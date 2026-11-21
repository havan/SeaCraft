//! Places outlines on sheets: true shapes, largest first, each at the lowest spot (then the
//! leftmost) where it keeps a gap to every placed outline and to the sheet's edges.
//!
//! A shape may be turned end for end (half a turn in its plane) or turned over (mirrored across
//! its length), never a quarter turn, so a plywood piece keeps its grain along the sheet. The
//! search is deterministic and good, not optimal.

use serde::Serialize;

/// A point, metres.
pub type Pt = (f64, f64);

/// The step of the search, metres.
const STEP_M: f64 = 0.01;
/// How finely the lowest spot is found between two steps, metres.
const REFINE_M: f64 = 0.001;
/// How far a simplified outline may stray from the true one, metres. The gap searched with is
/// widened by twice this, so the gap holds on the true outlines.
const SIMPLIFY_M: f64 = 0.00025;
/// The cell of the grid that finds a placed outline's nearby edges, metres.
const CELL_M: f64 = 0.05;

/// One thing to place: a closed outline in metres, any winding.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub outline: Vec<Pt>,
    /// Whether it may be turned over (mirrored); every item may be turned end for end.
    pub may_turn_over: bool,
}

/// Where and how an item lies on its bin.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Pose {
    /// The low corner of the turned outline's bounding box, metres from the bin's.
    pub x: f64,
    pub y: f64,
    /// Turned half a turn in its plane.
    pub end_for_end: bool,
    /// Mirrored across its length (y → −y), before any half turn.
    pub turned_over: bool,
}

impl Pose {
    /// The outline turned as the pose says, then moved so its bounding box's low corner is at
    /// `(x, y)`.
    pub fn place(&self, outline: &[Pt]) -> Vec<Pt> {
        let turned = turn(outline, self.end_for_end, self.turned_over);
        let b = bbox(&turned);
        turned
            .into_iter()
            .map(|(px, py)| (px - b[0] + self.x, py - b[1] + self.y))
            .collect()
    }
}

/// A sheet or blank to place on, metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Bin {
    pub length: f64,
    pub width: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Placement {
    pub item: usize,
    pub bin: usize,
    pub pose: Pose,
}

/// The bins opened, in order, where each item went, and the items not placed with why.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Packing {
    pub bins: Vec<Bin>,
    pub placements: Vec<Placement>,
    pub unplaced: Vec<(usize, String)>,
}

fn turn(outline: &[Pt], end_for_end: bool, turned_over: bool) -> Vec<Pt> {
    outline
        .iter()
        .map(|&(x, y)| {
            let y = if turned_over { -y } else { y };
            if end_for_end {
                (-x, -y)
            } else {
                (x, y)
            }
        })
        .collect()
}

pub(crate) fn bbox(p: &[Pt]) -> [f64; 4] {
    p.iter()
        .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| {
            [b[0].min(q.0), b[1].min(q.1), b[2].max(q.0), b[3].max(q.1)]
        })
}

pub(crate) fn signed_area(p: &[Pt]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f64>()
        / 2.0
}

fn area(p: &[Pt]) -> f64 {
    signed_area(p).abs()
}

fn point_segment(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

fn cross(o: Pt, a: Pt, b: Pt) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

fn segments_cross(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

fn segment_distance(a: Pt, b: Pt, c: Pt, d: Pt) -> f64 {
    if segments_cross(a, b, c, d) {
        return 0.0;
    }
    point_segment(a, c, d)
        .min(point_segment(b, c, d))
        .min(point_segment(c, a, b))
        .min(point_segment(d, a, b))
}

pub(crate) fn inside(p: Pt, poly: &[Pt]) -> bool {
    let mut odd = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            odd = !odd;
        }
    }
    odd
}

/// The least distance between two closed outlines; 0 when they cross or one holds the other.
pub fn min_distance(a: &[Pt], b: &[Pt]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return f64::INFINITY;
    }
    if inside(a[0], b) || inside(b[0], a) {
        return 0.0;
    }
    let mut d = f64::INFINITY;
    for i in 0..a.len() {
        let (p, q) = (a[i], a[(i + 1) % a.len()]);
        for j in 0..b.len() {
            d = d.min(segment_distance(p, q, b[j], b[(j + 1) % b.len()]));
        }
    }
    d
}

/// Douglas–Peucker on a closed outline: every dropped point within `tol` of the outline kept.
fn simplify(p: &[Pt], tol: f64) -> Vec<Pt> {
    if p.len() <= 4 {
        return p.to_vec();
    }
    let far = (1..p.len())
        .max_by(|&i, &j| {
            let di = (p[i].0 - p[0].0).hypot(p[i].1 - p[0].1);
            let dj = (p[j].0 - p[0].0).hypot(p[j].1 - p[0].1);
            di.total_cmp(&dj)
        })
        .unwrap_or(1);
    fn chain(p: &[Pt], tol: f64, keep: &mut Vec<bool>, lo: usize, hi: usize) {
        if hi <= lo + 1 {
            return;
        }
        let (a, b) = (p[lo], p[hi % p.len()]);
        let (k, d) = (lo + 1..hi)
            .map(|k| (k, point_segment(p[k], a, b)))
            .fold((lo, -1.0), |m, x| if x.1 > m.1 { x } else { m });
        if d > tol {
            keep[k] = true;
            chain(p, tol, keep, lo, k);
            chain(p, tol, keep, k, hi);
        }
    }
    let mut keep = vec![false; p.len()];
    keep[0] = true;
    keep[far] = true;
    chain(p, tol, &mut keep, 0, far);
    chain(p, tol, &mut keep, far, p.len());
    p.iter()
        .zip(keep)
        .filter_map(|(q, k)| k.then_some(*q))
        .collect()
}

/// One way an item may lie: its pose flags, the size of its true box, and its simplified
/// outline with the true box's low corner at the origin.
struct Shape {
    end_for_end: bool,
    turned_over: bool,
    length: f64,
    width: f64,
    simple: Vec<Pt>,
}

fn shapes(item: &Item) -> Vec<Shape> {
    let mut flags = vec![(false, false), (true, false)];
    if item.may_turn_over {
        flags.extend([(false, true), (true, true)]);
    }
    flags
        .into_iter()
        .map(|(e, t)| {
            let turned = turn(&item.outline, e, t);
            let b = bbox(&turned);
            let moved: Vec<Pt> = turned.iter().map(|p| (p.0 - b[0], p.1 - b[1])).collect();
            Shape {
                end_for_end: e,
                turned_over: t,
                length: b[2] - b[0],
                width: b[3] - b[1],
                simple: simplify(&moved, SIMPLIFY_M),
            }
        })
        .collect()
}

/// A placed outline with a grid of its edges, for finding the ones near a candidate quickly.
struct Placed {
    pts: Vec<Pt>,
    bbox: [f64; 4],
    nx: usize,
    ny: usize,
    cells: Vec<Vec<usize>>,
}

impl Placed {
    fn new(pts: Vec<Pt>) -> Placed {
        let b = bbox(&pts);
        let nx = (((b[2] - b[0]) / CELL_M).floor() as usize + 1).min(10_000);
        let ny = (((b[3] - b[1]) / CELL_M).floor() as usize + 1).min(10_000);
        let mut placed = Placed {
            bbox: b,
            nx,
            ny,
            cells: vec![Vec::new(); nx * ny],
            pts,
        };
        for i in 0..placed.pts.len() {
            let (p, q) = (placed.pts[i], placed.pts[(i + 1) % placed.pts.len()]);
            let sb = [p.0.min(q.0), p.1.min(q.1), p.0.max(q.0), p.1.max(q.1)];
            let (c0, c1) = placed.cell_range(sb);
            for cy in c0.1..=c1.1 {
                for cx in c0.0..=c1.0 {
                    placed.cells[cy * nx + cx].push(i);
                }
            }
        }
        placed
    }

    fn cell_range(&self, b: [f64; 4]) -> ((usize, usize), (usize, usize)) {
        let c =
            |v: f64, lo: f64, n: usize| (((v - lo) / CELL_M).floor().max(0.0) as usize).min(n - 1);
        (
            (
                c(b[0], self.bbox[0], self.nx),
                c(b[1], self.bbox[1], self.ny),
            ),
            (
                c(b[2], self.bbox[0], self.nx),
                c(b[3], self.bbox[1], self.ny),
            ),
        )
    }

    /// Whether `cand` moved by `at` keeps `gap` from this outline.
    fn clear_of(&self, cand: &[Pt], cb: [f64; 4], at: Pt, gap: f64) -> bool {
        let cb = [cb[0] + at.0, cb[1] + at.1, cb[2] + at.0, cb[3] + at.1];
        if cb[0] > self.bbox[2] + gap
            || cb[2] < self.bbox[0] - gap
            || cb[1] > self.bbox[3] + gap
            || cb[3] < self.bbox[1] - gap
        {
            return true;
        }
        let n = cand.len();
        let m = self.pts.len();
        let mut seen: Vec<usize> = Vec::new();
        for i in 0..n {
            let p = (cand[i].0 + at.0, cand[i].1 + at.1);
            let q = (cand[(i + 1) % n].0 + at.0, cand[(i + 1) % n].1 + at.1);
            let sb = [
                p.0.min(q.0) - gap,
                p.1.min(q.1) - gap,
                p.0.max(q.0) + gap,
                p.1.max(q.1) + gap,
            ];
            if sb[0] > self.bbox[2]
                || sb[2] < self.bbox[0]
                || sb[1] > self.bbox[3]
                || sb[3] < self.bbox[1]
            {
                continue;
            }
            let (c0, c1) = self.cell_range(sb);
            seen.clear();
            for cy in c0.1..=c1.1 {
                for cx in c0.0..=c1.0 {
                    for &j in &self.cells[cy * self.nx + cx] {
                        if seen.contains(&j) {
                            continue;
                        }
                        seen.push(j);
                        if segment_distance(p, q, self.pts[j], self.pts[(j + 1) % m]) < gap {
                            return false;
                        }
                    }
                }
            }
        }
        // No edges near: one may still hold the other whole.
        let c0 = (cand[0].0 + at.0, cand[0].1 + at.1);
        if inside(c0, &self.pts) {
            return false;
        }
        let moved: Vec<Pt> = cand.iter().map(|p| (p.0 + at.0, p.1 + at.1)).collect();
        !inside(self.pts[0], &moved)
    }
}

/// Grid sample coordinates from `lo` to `hi` in `STEP_M` increments, always including `hi` so
/// slack smaller than one step at the far edge is never skipped.
fn samples(lo: f64, hi: f64) -> Vec<f64> {
    let hi = hi.max(lo);
    let n = ((hi - lo) / STEP_M + 1e-9).floor() as usize;
    let mut out: Vec<f64> = (0..=n).map(|i| (lo + i as f64 * STEP_M).min(hi)).collect();
    if out.last().is_none_or(|&last| hi - last > 1e-12) {
        out.push(hi);
    }
    out
}

/// The lowest spot, then the leftmost, for `shape` on a bin of `bin` among `placed`, keeping
/// `gap` from the bin's edges (on the true box) and `search_gap` from the placed outlines.
fn lowest_spot(
    shape: &Shape,
    bin: Bin,
    placed: &[Placed],
    gap: f64,
    search_gap: f64,
) -> Option<Pt> {
    if !bin.length.is_finite()
        || !bin.width.is_finite()
        || bin.length - 2.0 * gap <= 0.0
        || bin.width - 2.0 * gap <= 0.0
    {
        return None;
    }
    let cb = bbox(&shape.simple);
    let x_max = bin.length - gap - shape.length;
    let y_max = bin.width - gap - shape.width;
    if !x_max.is_finite() || !y_max.is_finite() || x_max < gap - 1e-9 || y_max < gap - 1e-9 {
        return None;
    }
    let clear = |x: f64, y: f64| {
        placed
            .iter()
            .all(|p| p.clear_of(&shape.simple, cb, (x, y), search_gap))
    };
    let xs = samples(gap, x_max.max(gap));
    let ys = samples(gap, y_max.max(gap));
    let mut best: Option<Pt> = None;
    for &x in &xs {
        let mut found = None;
        let mut prev_y = gap;
        for (k, &y) in ys.iter().enumerate() {
            let (probe_y, stop_after) = match best {
                Some(b) if y > b.1 => {
                    if k == 0 || prev_y >= b.1 - 1e-12 {
                        break;
                    }
                    (b.1, true)
                }
                _ => (y, false),
            };
            if clear(x, probe_y) {
                // Down to the last spot that was not clear.
                let (mut lo, mut hi) = (if k == 0 { probe_y } else { prev_y }, probe_y);
                while hi - lo > REFINE_M {
                    let mid = (lo + hi) / 2.0;
                    if clear(x, mid) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                found = Some((x, hi));
                break;
            }
            if stop_after {
                break;
            }
            prev_y = y;
        }
        if let Some(f) = found {
            if best.is_none_or(|b| f.1 < b.1 - 1e-12) {
                best = Some(f);
            }
        }
    }
    best
}

/// Places `items` on bins, largest first. `new_bin(length)` gives the bin to open for an item
/// of that length (its turned box's) that fits no open bin, or `None` when no bin is long
/// enough. `gap` is kept between items and from every bin edge.
pub fn pack(items: &[Item], new_bin: &dyn Fn(f64) -> Option<Bin>, gap: f64) -> Packing {
    let mut out = Packing::default();
    if !gap.is_finite() || gap < 0.0 {
        out.unplaced = (0..items.len())
            .map(|i| {
                (
                    i,
                    "the gap between pieces must be a length of 0 or more".to_string(),
                )
            })
            .collect();
        return out;
    }
    let search_gap = gap + 2.0 * SIMPLIFY_M;
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| area(&items[b].outline).total_cmp(&area(&items[a].outline)));
    let mut placed: Vec<Vec<Placed>> = Vec::new();
    for i in order {
        let item = &items[i];
        if item.outline.len() < 3
            || item
                .outline
                .iter()
                .any(|p| !p.0.is_finite() || !p.1.is_finite())
        {
            out.unplaced.push((i, "its outline is not a shape".into()));
            continue;
        }
        let shapes = shapes(item);
        let try_bin = |bin: Bin, placed: &[Placed]| -> Option<(usize, Pt)> {
            let mut best: Option<(usize, Pt)> = None;
            for (s, shape) in shapes.iter().enumerate() {
                if let Some(at) = lowest_spot(shape, bin, placed, gap, search_gap) {
                    let better = best.is_none_or(|(_, b)| {
                        at.1 < b.1 - 1e-12 || (at.1 <= b.1 + 1e-12 && at.0 < b.0 - 1e-12)
                    });
                    if better {
                        best = Some((s, at));
                    }
                }
            }
            best
        };
        let mut spot = None;
        for (b, bin) in out.bins.iter().enumerate() {
            if let Some(found) = try_bin(*bin, &placed[b]) {
                spot = Some((b, found));
                break;
            }
        }
        if spot.is_none() {
            let length = shapes[0].length;
            let width = shapes[0].width;
            let Some(bin) = new_bin(length) else {
                out.unplaced.push((
                    i,
                    format!(
                        "longer than the longest sheet or blank ({} mm)",
                        crate::units::millimetres(length)
                    ),
                ));
                continue;
            };
            if !bin.length.is_finite()
                || !bin.width.is_finite()
                || bin.length <= 0.0
                || bin.width <= 0.0
            {
                out.unplaced
                    .push((i, "the sheet or blank has no usable size".into()));
                continue;
            }
            if bin.width - 2.0 * gap <= 0.0 || width > bin.width - 2.0 * gap + 1e-9 {
                out.unplaced.push((
                    i,
                    format!(
                        "wider than the sheet less the gaps ({} mm > {} mm)",
                        crate::units::millimetres(width),
                        crate::units::millimetres((bin.width - 2.0 * gap).max(0.0))
                    ),
                ));
                continue;
            }
            match try_bin(bin, &[]) {
                Some(found) => {
                    out.bins.push(bin);
                    placed.push(Vec::new());
                    spot = Some((out.bins.len() - 1, found));
                }
                None => {
                    out.unplaced.push((
                        i,
                        format!(
                            "longer than the sheet or blank less the gaps ({} mm > {} mm)",
                            crate::units::millimetres(length),
                            crate::units::millimetres((bin.length - 2.0 * gap).max(0.0))
                        ),
                    ));
                    continue;
                }
            }
        }
        let Some((b, (s, at))) = spot else { continue };
        let shape = &shapes[s];
        placed[b].push(Placed::new(
            shape
                .simple
                .iter()
                .map(|p| (p.0 + at.0, p.1 + at.1))
                .collect(),
        ));
        out.placements.push(Placement {
            item: i,
            bin: b,
            pose: Pose {
                x: at.0,
                y: at.1,
                end_for_end: shape.end_for_end,
                turned_over: shape.turned_over,
            },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(l: f64, w: f64) -> Vec<Pt> {
        vec![(0.0, 0.0), (l, 0.0), (l, w), (0.0, w)]
    }

    fn area(p: &[Pt]) -> f64 {
        let n = p.len();
        (0..n)
            .map(|i| {
                let (a, b) = (p[i], p[(i + 1) % n]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum::<f64>()
            .abs()
            / 2.0
    }

    fn sides(p: &[Pt]) -> Vec<f64> {
        let n = p.len();
        (0..n)
            .map(|i| {
                let (a, b) = (p[i], p[(i + 1) % n]);
                (b.0 - a.0).hypot(b.1 - a.1)
            })
            .collect()
    }

    fn bbox(p: &[Pt]) -> [f64; 4] {
        p.iter()
            .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| {
                [b[0].min(q.0), b[1].min(q.1), b[2].max(q.0), b[3].max(q.1)]
            })
    }

    /// An arch 3 m long: the band between y = 0.4 (1 − u²) and 0.1 m above it, u from −1 to 1
    /// along the length; its box is 3 m by 0.5 m.
    fn arch() -> Vec<Pt> {
        let n = 60;
        let lo = |i: usize| {
            let u = -1.0 + 2.0 * i as f64 / n as f64;
            (1.5 * (u + 1.0), 0.4 * (1.0 - u * u))
        };
        let mut pts: Vec<Pt> = (0..=n).map(lo).collect();
        pts.extend((0..=n).rev().map(|i| {
            let (x, y) = lo(i);
            (x, y + 0.1)
        }));
        pts
    }

    /// Every placed outline inside its bin by the gap and every pair at least the gap apart.
    fn assert_sound(items: &[Item], p: &Packing, gap: f64) {
        let placed: Vec<(usize, Vec<Pt>)> = p
            .placements
            .iter()
            .map(|pl| (pl.bin, pl.pose.place(&items[pl.item].outline)))
            .collect();
        for (bin, pts) in &placed {
            let b = p.bins[*bin];
            for q in pts {
                assert!(
                    q.0 >= gap - 1e-9
                        && q.0 <= b.length - gap + 1e-9
                        && q.1 >= gap - 1e-9
                        && q.1 <= b.width - gap + 1e-9,
                    "{q:?} outside {b:?} by the gap"
                );
            }
        }
        for i in 0..placed.len() {
            for j in i + 1..placed.len() {
                if placed[i].0 == placed[j].0 {
                    let d = min_distance(&placed[i].1, &placed[j].1);
                    assert!(d >= gap - 1e-9, "pieces {i} and {j} {d} m apart");
                }
            }
        }
    }

    #[test]
    fn rectangles_fill_a_bin_without_overlap() {
        let items: Vec<Item> = (0..12)
            .map(|_| Item {
                outline: rect(0.5, 0.3),
                may_turn_over: true,
            })
            .collect();
        let bin = Bin {
            length: 2.44,
            width: 1.22,
        };
        let p = pack(&items, &|_| Some(bin), 0.01);
        assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
        assert_eq!(p.placements.len(), 12);
        assert_eq!(p.bins.len(), 1);
        assert_sound(&items, &p, 0.01);
    }

    #[test]
    fn two_arches_share_a_width() {
        let items = vec![
            Item {
                outline: arch(),
                may_turn_over: true,
            },
            Item {
                outline: arch(),
                may_turn_over: true,
            },
        ];
        let bin = Bin {
            length: 3.2,
            width: 0.85,
        };
        // Their boxes, 0.5 m each, would need 1.03 m with the gaps.
        assert!(2.0 * 0.5 + 3.0 * 0.01 > bin.width);
        let started = std::time::Instant::now();
        let p = pack(&items, &|_| Some(bin), 0.01);
        println!("two arches packed in {:?}", started.elapsed());
        assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
        assert_eq!(p.bins.len(), 1);
        assert_sound(&items, &p, 0.01);
    }

    #[test]
    fn poses_keep_the_shape_and_anchor_the_box() {
        let tri = vec![(0.3, 0.1), (1.5, 0.4), (0.7, 0.9)];
        for (end_for_end, turned_over) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let pose = Pose {
                x: 0.25,
                y: 0.75,
                end_for_end,
                turned_over,
            };
            let got = pose.place(&tri);
            let b = bbox(&got);
            assert!(
                (b[0] - 0.25).abs() < 1e-12 && (b[1] - 0.75).abs() < 1e-12,
                "{b:?}"
            );
            assert!((area(&got) - area(&tri)).abs() < 1e-12);
            for (a, b) in sides(&got).iter().zip(sides(&tri)) {
                assert!((a - b).abs() < 1e-12);
            }
        }
        // Turned over is the mirror image: the winding reverses.
        let signed = |p: &[Pt]| {
            (0..p.len())
                .map(|i| {
                    let (a, b) = (p[i], p[(i + 1) % p.len()]);
                    a.0 * b.1 - b.0 * a.1
                })
                .sum::<f64>()
        };
        let over = Pose {
            x: 0.0,
            y: 0.0,
            end_for_end: false,
            turned_over: true,
        };
        assert!(signed(&tri) * signed(&over.place(&tri)) < 0.0);
    }

    #[test]
    fn too_wide_is_unplaced_by_name() {
        let items = vec![Item {
            outline: rect(1.0, 1.3),
            may_turn_over: true,
        }];
        let p = pack(
            &items,
            &|_| {
                Some(Bin {
                    length: 2.44,
                    width: 1.22,
                })
            },
            0.01,
        );
        assert!(p.placements.is_empty());
        assert_eq!(p.unplaced.len(), 1);
        assert_eq!(p.unplaced[0].0, 0);
        assert!(
            p.unplaced[0].1.contains("wider than"),
            "{}",
            p.unplaced[0].1
        );
    }

    #[test]
    fn no_bin_long_enough_is_unplaced() {
        let items = vec![
            Item {
                outline: rect(5.0, 0.3),
                may_turn_over: true,
            },
            Item {
                outline: rect(1.0, 0.3),
                may_turn_over: true,
            },
        ];
        let p = pack(
            &items,
            &|need| {
                (need <= 2.4).then_some(Bin {
                    length: 2.44,
                    width: 1.22,
                })
            },
            0.01,
        );
        assert_eq!(p.placements.len(), 1);
        assert_eq!(p.placements[0].item, 1);
        assert_eq!(p.unplaced.len(), 1);
        assert_eq!(p.unplaced[0].0, 0);
        assert!(
            p.unplaced[0].1.contains("longer than"),
            "{}",
            p.unplaced[0].1
        );
    }

    #[test]
    fn a_bad_gap_places_nothing_and_says_why() {
        let items = vec![Item {
            outline: rect(1.0, 0.3),
            may_turn_over: true,
        }];
        let bin = Bin {
            length: 2.44,
            width: 1.22,
        };
        for gap in [-0.01, f64::NAN, f64::INFINITY, 0.7] {
            let p = pack(&items, &|_| Some(bin), gap);
            assert!(p.placements.is_empty(), "gap {gap}");
            assert_eq!(p.unplaced.len(), 1, "gap {gap}");
        }
        // A gap of nothing is allowed.
        let p = pack(&items, &|_| Some(bin), 0.0);
        assert_eq!(p.placements.len(), 1);
    }

    #[test]
    fn packing_is_deterministic() {
        let items: Vec<Item> = [arch(), rect(0.7, 0.4), arch(), rect(1.2, 0.2)]
            .into_iter()
            .map(|outline| Item {
                outline,
                may_turn_over: true,
            })
            .collect();
        let bins = |_| {
            Some(Bin {
                length: 3.2,
                width: 1.22,
            })
        };
        assert_eq!(pack(&items, &bins, 0.01), pack(&items, &bins, 0.01));
    }

    #[test]
    fn the_gap_is_kept_from_bin_edges() {
        let items = vec![Item {
            outline: vec![(5.0, 5.0), (6.0, 5.0), (5.5, 5.4)],
            may_turn_over: false,
        }];
        let p = pack(
            &items,
            &|_| {
                Some(Bin {
                    length: 2.44,
                    width: 1.22,
                })
            },
            0.02,
        );
        let pose = p.placements[0].pose;
        assert_eq!((pose.x, pose.y), (0.02, 0.02));
        assert!(!pose.turned_over);
    }

    #[test]
    fn tight_fit_within_one_step_of_bin_edges() {
        let bin = Bin {
            length: 2.44,
            width: 1.22,
        };
        // Two 1000 x 592 mm pieces on 2440 x 1220 mm with 10 mm gap leave 5.5 mm (< STEP_M) of
        // width slack; probing y_max must fit both on one bin.
        let stacked = vec![
            Item {
                outline: rect(1.0, 0.592),
                may_turn_over: false,
            },
            Item {
                outline: rect(1.0, 0.592),
                may_turn_over: false,
            },
        ];
        let p = pack(&stacked, &|_| Some(bin), 0.01);
        assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
        assert_eq!(p.bins.len(), 1, "both pieces must fit on one bin");
        assert_sound(&stacked, &p, 0.01);

        // Two 1202 x 800 mm pieces leave 5.5 mm (< STEP_M) of length slack; probing x_max fits both.
        let side_by_side = vec![
            Item {
                outline: rect(1.202, 0.8),
                may_turn_over: false,
            },
            Item {
                outline: rect(1.202, 0.8),
                may_turn_over: false,
            },
        ];
        let p = pack(&side_by_side, &|_| Some(bin), 0.01);
        assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
        assert_eq!(
            p.bins.len(),
            1,
            "both pieces must fit end to end on one bin"
        );
        assert_sound(&side_by_side, &p, 0.01);
    }

    #[test]
    fn non_finite_bin_and_zero_usable_width_are_unplaced() {
        let items = vec![Item {
            outline: rect(1.0, 0.3),
            may_turn_over: false,
        }];
        for bad_bin in [
            Bin {
                length: f64::NAN,
                width: 1.22,
            },
            Bin {
                length: 2.44,
                width: f64::NAN,
            },
            Bin {
                length: 2.44,
                width: -1.0,
            },
        ] {
            let p = pack(&items, &|_| Some(bad_bin), 0.01);
            assert!(p.placements.is_empty(), "{bad_bin:?}");
            assert_eq!(p.unplaced.len(), 1, "{bad_bin:?}");
        }
        // 2g == W leaves zero usable width and must refuse even a degenerate zero-width shape.
        let flat = vec![Item {
            outline: vec![(0.0, 0.0), (1.0, 0.0), (0.5, 0.0)],
            may_turn_over: false,
        }];
        let p = pack(
            &flat,
            &|_| {
                Some(Bin {
                    length: 2.44,
                    width: 1.22,
                })
            },
            0.61,
        );
        assert!(p.placements.is_empty());
        assert_eq!(p.unplaced.len(), 1);
    }
}
