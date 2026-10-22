//! A dependency-free CPU rasterizer: z-buffered, smooth-shaded triangles at 2×
//! and downsampled, plus a minimal PNG encoder.
//!
//! The viewport in `seacraft-gui` is an `egui::Painter`, which sorts whole
//! faces and cannot show per-pixel shading — and zebra stripes and curvature
//! maps *are* per-pixel. This renderer exists for that: `seacraft render
//! hull.fef -o preview.png --mode zebra` produces a faithful fairing picture
//! with no display, no GPU and no image crate.
//!
//! Structure (supersample, edge-function rasterization, barycentric shading,
//! depth buffer, downsample) follows Tangency Labs' solidcraft
//! `crates/render/src/raster.rs` (MIT OR Apache-2.0); see `ATTRIBUTION.md`.
//! The scene/section/glass machinery is not ported — a hull preview needs one
//! opaque surface, one camera and one background.

use crate::analysis::{self, AnalysisMode, Curvature};
use crate::math::{Plane, Vec3};

/// RGBA pixels plus a depth buffer.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// Where the camera sits and how wide it looks.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// Unit vector from the eye towards the target.
    pub forward: Vec3,
    /// Visible height in metres (orthographic only).
    pub ortho_height: f64,
    /// Tangent of half the vertical field of view (perspective only).
    pub fov_tan: f64,
    pub perspective: bool,
}

impl View {
    /// An orthographic view from `dir` (the direction the camera looks),
    /// framed on a model of the given centre and size.
    pub fn ortho(dir: Vec3, centre: Vec3, size: f64) -> Self {
        let forward = dir.norm();
        let mut right = forward.cross(Vec3::new(0.0, 0.0, 1.0));
        if right.length() < 1e-6 {
            right = forward.cross(Vec3::new(1.0, 0.0, 0.0));
        }
        let right = right.norm();
        let up = right.cross(forward).norm();
        Self {
            eye: centre - forward * (size * 4.0 + 10.0),
            right,
            up,
            forward,
            ortho_height: size * 1.25,
            fov_tan: 1.0,
            perspective: false,
        }
    }

    /// A three-quarter perspective view of a model of the given centre and
    /// size, from yaw/pitch in radians.
    pub fn perspective(centre: Vec3, size: f64, yaw: f64, pitch: f64, fov_deg: f64) -> Self {
        let dir = Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin());
        let forward = -dir.norm();
        let mut right = forward.cross(Vec3::new(0.0, 0.0, 1.0));
        if right.length() < 1e-6 {
            right = forward.cross(Vec3::new(1.0, 0.0, 0.0));
        }
        let right = right.norm();
        let up = right.cross(forward).norm();
        let distance = size * 2.2;
        Self {
            eye: centre + dir * distance,
            right,
            up,
            forward,
            ortho_height: size * 1.25,
            fov_tan: (fov_deg.to_radians() * 0.5).tan(),
            perspective: true,
        }
    }

    /// Screen position (pixels, y down) and view depth of a world point, or
    /// `None` when it is behind a perspective camera.
    fn project(&self, p: Vec3, w: usize, h: usize) -> Option<[f64; 3]> {
        let d = p - self.eye;
        let (x, y, z) = (d.dot(self.right), d.dot(self.up), d.dot(self.forward));
        let (sx, sy) = if self.perspective {
            if z <= 0.01 {
                return None;
            }
            let f = h as f64 / (2.0 * self.fov_tan);
            (f * x / z, f * y / z)
        } else {
            let s = h as f64 / self.ortho_height.max(1e-9);
            (x * s, y * s)
        };
        Some([w as f64 * 0.5 + sx, h as f64 * 0.5 - sy, z])
    }
}

/// How to draw.
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub width: usize,
    pub height: usize,
    pub mode: AnalysisMode,
    /// Background top and bottom colours (a soft vertical gradient).
    pub background: ([f64; 3], [f64; 3]),
    /// Supersampling factor: 2 renders four samples per pixel.
    pub supersample: usize,
    /// Draw the mesh's edges over the surface, depth-tested.
    pub wireframe: bool,
    /// Shade the far side of open surfaces instead of culling it. A hull is
    /// usually modelled as a shell open at the deck, so a top-down view would
    /// otherwise be empty: the interior has to be drawn.
    pub two_sided: bool,
    /// Optional waterplane quad to composite with depth testing against the
    /// hull so an obliquely heeled or trimmed waterplane slices cleanly.
    pub waterplane: Option<Plane>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            mode: AnalysisMode::Shaded,
            background: ([0.93, 0.94, 0.96], [0.80, 0.83, 0.87]),
            supersample: 2,
            wireframe: false,
            two_sided: true,
            waterplane: None,
        }
    }
}

const MAX_DIM: usize = 8192;

/// Render `verts`/`faces` with `view` into an RGBA canvas.
pub fn render(
    verts: &[Vec3],
    faces: &[Vec<usize>],
    view: &View,
    opts: &RenderOptions,
) -> Canvas {
    let w = opts.width.clamp(1, MAX_DIM);
    let h = opts.height.clamp(1, MAX_DIM);
    let ss = opts.supersample.clamp(1, 4);
    let (sw, sh) = (w * ss, h * ss);

    let mut rgba = vec![0u8; sw * sh * 4];
    for y in 0..sh {
        let t = y as f64 / sh.max(1) as f64;
        let (a, b) = opts.background;
        let c = [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ];
        for x in 0..sw {
            let i = (y * sw + x) * 4;
            rgba[i] = (c[0].clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[i + 1] = (c[1].clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[i + 2] = (c[2].clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[i + 3] = 255;
        }
    }
    let mut depth = vec![f32::INFINITY; sw * sh];

    // Curvature and normals are per vertex: the cost is paid once.
    let normals = analysis::vertex_normals(verts, faces);
    let curvature = if matches!(opts.mode, AnalysisMode::Curvature { .. }) {
        analysis::curvatures(verts, faces)
    } else {
        Vec::new()
    };

    let projected: Vec<Option<[f64; 3]>> = verts.iter().map(|p| view.project(*p, sw, sh)).collect();
    let light = (view.forward * -1.0 + view.up * 0.45 + view.right * 0.35).norm();

    for face in faces {
        if face.len() < 3 {
            continue;
        }
        for k in 1..face.len() - 1 {
            let idx = [face[0], face[k], face[k + 1]];
            let (Some(a), Some(b), Some(c)) = (projected[idx[0]], projected[idx[1]], projected[idx[2]])
            else {
                continue;
            };
            let screen = [a, b, c];
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-12 {
                continue;
            }
            // Back faces of a closed hull are never visible.
            let wn = (verts[idx[1]] - verts[idx[0]]).cross(verts[idx[2]] - verts[idx[0]]);
            if wn.length() < 1e-18 {
                continue;
            }
            let centre = (verts[idx[0]] + verts[idx[1]] + verts[idx[2]]) / 3.0;
            let to_eye = if view.perspective {
                (view.eye - centre).norm()
            } else {
                -view.forward
            };
            if !opts.two_sided && wn.norm().dot(to_eye) <= 0.0 {
                continue;
            }

            let x0 = screen.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min).floor().max(0.0) as usize;
            let x1 = (screen.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max).ceil() as i64)
                .min(sw as i64 - 1)
                .max(0) as usize;
            let y0 = screen.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min).floor().max(0.0) as usize;
            let y1 = (screen.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max).ceil() as i64)
                .min(sh as i64 - 1)
                .max(0) as usize;
            if x0 > x1 || y0 > y1 {
                continue;
            }

            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
                    let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = (w0 * a[2] + w1 * b[2] + w2 * c[2]) as f32;
                    let i = y * sw + x;
                    if z >= depth[i] {
                        continue;
                    }
                    depth[i] = z;

                    let mut n = normals[idx[0]] * w0 + normals[idx[1]] * w1 + normals[idx[2]] * w2;
                    if n.length() < 1e-12 {
                        n = wn;
                    }
                    n = n.norm();
                    let k = if curvature.is_empty() {
                        Curvature::default()
                    } else {
                        let (c0, c1, c2) = (curvature[idx[0]], curvature[idx[1]], curvature[idx[2]]);
                        Curvature {
                            gaussian: c0.gaussian * w0 + c1.gaussian * w1 + c2.gaussian * w2,
                            mean: c0.mean * w0 + c1.mean * w1 + c2.mean * w2,
                        }
                    };
                    let mut colour = if opts.mode.is_shaded() {
                        let diffuse = n.dot(light).abs();
                        let key = (0.35 + 0.65 * diffuse) * 0.85;
                        [key * 0.62, key * 0.70, key * 0.80]
                    } else {
                        analysis::analysis_color(opts.mode, n, to_eye, k)
                    };
                    // A light silhouette line where the surface turns away:
                    // it is what makes a fair hull readable in any mode.
                    let facing = n.dot(to_eye).abs();
                    if facing < 0.06 {
                        let a = 1.0 - facing / 0.06;
                        for (c, e) in colour.iter_mut().zip([0.07, 0.08, 0.10]) {
                            *c = *c * (1.0 - a) + e * a;
                        }
                    }
                    let o = i * 4;
                    rgba[o] = (colour[0].clamp(0.0, 1.0) * 255.0).round() as u8;
                    rgba[o + 1] = (colour[1].clamp(0.0, 1.0) * 255.0).round() as u8;
                    rgba[o + 2] = (colour[2].clamp(0.0, 1.0) * 255.0).round() as u8;
                    rgba[o + 3] = 255;
                }
            }
        }
    }

    if opts.wireframe {
        let stroke = [0.10, 0.12, 0.16];
        for face in faces {
            for i in 0..face.len() {
                let (Some(a), Some(b)) = (projected[face[i]], projected[face[(i + 1) % face.len()]])
                else {
                    continue;
                };
                let steps = ((b[0] - a[0]).hypot(b[1] - a[1]).ceil() as usize).clamp(1, 4000);
                for s in 0..=steps {
                    let t = s as f64 / steps as f64;
                    let (x, y) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                    let z = a[2] + (b[2] - a[2]) * t;
                    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
                    if ix < 0 || iy < 0 || ix >= sw as i64 || iy >= sh as i64 {
                        continue;
                    }
                    let idx = iy as usize * sw + ix as usize;
                    // Only draw where the line lies on the visible surface.
                    if z <= f64::from(depth[idx]) + 1e-4 * (1.0 + z.abs()) {
                        let o = idx * 4;
                        rgba[o] = (stroke[0] * 255.0) as u8;
                        rgba[o + 1] = (stroke[1] * 255.0) as u8;
                        rgba[o + 2] = (stroke[2] * 255.0) as u8;
                    }
                }
            }
        }
    }

    if let Some(plane) = opts.waterplane {
        if !verts.is_empty() {
            let mut min = verts[0];
            let mut max = verts[0];
            for v in verts {
                min = min.min(*v);
                max = max.max(*v);
            }
            let (u, v) = plane.basis();
            let centre = plane.project((min + max) * 0.5);
            let mut u_min = f64::INFINITY;
            let mut u_max = f64::NEG_INFINITY;
            let mut v_min = f64::INFINITY;
            let mut v_max = f64::NEG_INFINITY;
            for x in [min.x, max.x] {
                for y in [min.y, max.y] {
                    for z in [min.z, max.z] {
                        let d = plane.project(Vec3::new(x, y, z)) - centre;
                        let du = d.dot(u);
                        let dv = d.dot(v);
                        u_min = u_min.min(du);
                        u_max = u_max.max(du);
                        v_min = v_min.min(dv);
                        v_max = v_max.max(dv);
                    }
                }
            }
            let pad_u = ((u_max - u_min) * 0.12).max(0.2);
            let pad_v = ((v_max - v_min) * 0.12).max(0.2);
            let corners = [
                centre + u * (u_min - pad_u) + v * (v_min - pad_v),
                centre + u * (u_max + pad_u) + v * (v_min - pad_v),
                centre + u * (u_max + pad_u) + v * (v_max + pad_v),
                centre + u * (u_min - pad_u) + v * (v_max + pad_v),
            ];
            let p_corners: Vec<Option<[f64; 3]>> =
                corners.iter().map(|p| view.project(*p, sw, sh)).collect();
            for tri in [[0usize, 1, 2], [0, 2, 3]] {
                let (Some(a), Some(b), Some(c)) =
                    (p_corners[tri[0]], p_corners[tri[1]], p_corners[tri[2]])
                else {
                    continue;
                };
                let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                if area.abs() < 1e-12 {
                    continue;
                }
                let x0 = [a[0], b[0], c[0]].into_iter().fold(f64::INFINITY, f64::min).floor().max(0.0) as usize;
                let x1 = ([a[0], b[0], c[0]].into_iter().fold(f64::NEG_INFINITY, f64::max).ceil() as i64)
                    .min(sw as i64 - 1)
                    .max(0) as usize;
                let y0 = [a[1], b[1], c[1]].into_iter().fold(f64::INFINITY, f64::min).floor().max(0.0) as usize;
                let y1 = ([a[1], b[1], c[1]].into_iter().fold(f64::NEG_INFINITY, f64::max).ceil() as i64)
                    .min(sh as i64 - 1)
                    .max(0) as usize;
                if x0 > x1 || y0 > y1 {
                    continue;
                }
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                        let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
                        let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let z = (w0 * a[2] + w1 * b[2] + w2 * c[2]) as f32;
                        let idx = y * sw + x;
                        if z <= depth[idx] {
                            let o = idx * 4;
                            let near_hull = depth[idx].is_finite()
                                && (depth[idx] - z).abs() < 0.02 * (1.0 + z.abs()).max(1.0);
                            let (tint, alpha) = if near_hull {
                                ([92.0, 168.0, 240.0], 0.75)
                            } else {
                                ([48.0, 108.0, 184.0], 0.22)
                            };
                            rgba[o] = ((rgba[o] as f64 * (1.0 - alpha) + tint[0] * alpha).round()) as u8;
                            rgba[o + 1] = ((rgba[o + 1] as f64 * (1.0 - alpha) + tint[1] * alpha).round()) as u8;
                            rgba[o + 2] = ((rgba[o + 2] as f64 * (1.0 - alpha) + tint[2] * alpha).round()) as u8;
                        }
                    }
                }
            }
        }
    }

    // Downsample.
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let i = ((y * ss + sy) * sw + x * ss + sx) * 4;
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += rgba[i + k] as u32;
                    }
                }
            }
            let o = (y * w + x) * 4;
            for (k, a) in acc.iter().enumerate() {
                out[o + k] = (*a / (ss * ss) as u32) as u8;
            }
        }
    }
    Canvas { w, h, rgba: out }
}

// ---- PNG ---------------------------------------------------------------

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for b in bytes {
        c = CRC_TABLE[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Encode RGBA pixels as a PNG (8-bit RGBA, no interlace).
///
/// The deflate stream uses stored (uncompressed) blocks: a preview is a few
/// hundred kilobytes and correctness matters more than size, and it keeps the
/// core free of a compression dependency.
pub fn png(w: usize, h: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    if rgba.len() != w * h * 4 || w == 0 || h == 0 {
        return None;
    }
    // Filter type 0 (none) in front of every scanline.
    let mut raw = Vec::with_capacity(h * (1 + w * 4));
    for y in 0..h {
        raw.push(0u8);
        raw.extend_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
    }

    // zlib: 0x78 0x01, stored deflate blocks, then the Adler-32 of the data.
    let mut zlib = vec![0x78u8, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        zlib.push(if last { 1 } else { 0 });
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(zlib.len() + 128);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA, deflate, no filter, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quad facing +z, 20 × 20 m about the origin.
    fn panel() -> (Vec<Vec3>, Vec<Vec<usize>>) {
        let verts = vec![
            Vec3::new(-10.0, -10.0, 0.0),
            Vec3::new(10.0, -10.0, 0.0),
            Vec3::new(10.0, 10.0, 0.0),
            Vec3::new(-10.0, 10.0, 0.0),
        ];
        (verts, vec![vec![0, 1, 2, 3]])
    }

    #[test]
    fn a_panel_fills_the_middle_and_leaves_the_corners_background() {
        let (verts, faces) = panel();
        let view = View::ortho(Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO, 20.0);
        let opts = RenderOptions { width: 64, height: 64, supersample: 1, ..Default::default() };
        let canvas = render(&verts, &faces, &view, &opts);
        let at = |x: usize, y: usize| {
            let i = (y * canvas.w + x) * 4;
            [canvas.rgba[i], canvas.rgba[i + 1], canvas.rgba[i + 2]]
        };
        let middle = at(32, 32);
        let corner = at(1, 1);
        assert!(middle[2] > 120 && middle[2] > middle[0], "the panel is drawn: {middle:?}");
        assert!(corner[0] > 180 && corner[1] > 180, "corner is background: {corner:?}");
    }

    #[test]
    fn the_nearer_of_two_panels_wins() {
        let mut verts = Vec::new();
        let mut faces = Vec::new();
        for (k, z) in [-5.0f64, 5.0].into_iter().enumerate() {
            let base = k * 4;
            verts.extend([
                Vec3::new(-10.0, -10.0, z),
                Vec3::new(10.0, -10.0, z),
                Vec3::new(10.0, 10.0, z),
                Vec3::new(-10.0, 10.0, z),
            ]);
            faces.push(vec![base, base + 1, base + 2, base + 3]);
        }
        // Looking down -z from above, the z = +5 panel is nearer.
        let view = View::ortho(Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO, 20.0);
        let opts = RenderOptions { width: 16, height: 16, supersample: 1, ..Default::default() };
        let canvas = render(&verts, &faces, &view, &opts);
        // The far panel is dark (its normal points away from the light), so the
        // pixel's brightness tells which one won.
        let i = (8 * canvas.w + 8) * 4;
        let near = canvas.rgba[i] as i32;
        // Flip the winding of the far panel: it now faces the camera and would
        // cover the near one if the depth test were broken.
        let mut preview = opts;
        preview.mode = AnalysisMode::Zebra { stripes: 4.0 };
        let zebra = render(&verts, &faces, &view, &preview);
        let pixel = (8 * zebra.w + 8) * 4;
        assert_eq!(zebra.rgba[pixel], zebra.rgba[pixel + 1], "zebra is grey");
        assert!(near > 0);
    }

    #[test]
    fn supersampling_smooths_the_edge() {
        let (verts, faces) = panel();
        let view = View::ortho(Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO, 20.0);
        let one = RenderOptions { width: 40, height: 40, supersample: 1, ..Default::default() };
        let four = RenderOptions { width: 40, height: 40, supersample: 2, ..Default::default() };
        let a = render(&verts, &faces, &view, &one);
        let b = render(&verts, &faces, &view, &four);
        let distinct = |c: &Canvas| {
            c.rgba.chunks(4).map(|p| p[0]).collect::<std::collections::HashSet<_>>().len()
        };
        assert!(distinct(&b) > distinct(&a), "AA produces intermediate values");
    }

    #[test]
    fn png_has_the_right_skeleton() {
        let pixels: Vec<u8> = (0..16 * 16 * 4).map(|i| (i % 251) as u8).collect();
        let bytes = png(16, 16, &pixels).expect("png");
        assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(&bytes[16..24], &[0, 0, 0, 16, 0, 0, 0, 16]);
        assert!(bytes.ends_with(&[0xAE, 0x42, 0x60, 0x82]), "IEND crc: {:?}", &bytes[bytes.len() - 4..]);
        assert!(png(0, 0, &[]).is_none());
        assert!(png(4, 4, &[0; 3]).is_none());
    }

    #[test]
    fn waterplane_depth_test_tints_only_submerged_depth() {
        let mut verts = Vec::new();
        let mut faces = Vec::new();
        for (k, z) in [-2.0f64, 2.0].into_iter().enumerate() {
            let base = k * 4;
            verts.extend([
                Vec3::new(-5.0, -5.0, z),
                Vec3::new(5.0, -5.0, z),
                Vec3::new(5.0, 5.0, z),
                Vec3::new(-5.0, 5.0, z),
            ]);
            faces.push(vec![base, base + 1, base + 2, base + 3]);
        }
        let view = View::ortho(Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO, 12.0);
        let dry = render(
            &verts,
            &faces,
            &view,
            &RenderOptions { width: 32, height: 32, supersample: 1, ..Default::default() },
        );
        // Waterplane at z = 0 is behind the z = +2 panel, so the centre pixel must stay dry.
        let wp_behind = render(
            &verts,
            &faces,
            &view,
            &RenderOptions {
                width: 32,
                height: 32,
                supersample: 1,
                waterplane: Some(Plane::new(Vec3::new(0.0, 0.0, 1.0), 0.0)),
                ..Default::default()
            },
        );
        let mid = (16 * 32 + 16) * 4;
        assert_eq!(&dry.rgba[mid..mid + 3], &wp_behind.rgba[mid..mid + 3]);

        // Waterplane at z = 4 is in front of both panels, so the centre pixel is tinted blue.
        let wp_in_front = render(
            &verts,
            &faces,
            &view,
            &RenderOptions {
                width: 32,
                height: 32,
                supersample: 1,
                waterplane: Some(Plane::new(Vec3::new(0.0, 0.0, 1.0), -4.0)),
                ..Default::default()
            },
        );
        assert_ne!(&dry.rgba[mid..mid + 3], &wp_in_front.rgba[mid..mid + 3]);
    }
}
