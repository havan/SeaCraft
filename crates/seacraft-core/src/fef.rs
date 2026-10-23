//! FREE!ship Exchange Format (`.fef`) reader / writer.
//!
//! The layout follows `TFreeEdit.File_ExportFEF`,
//! `TFreeSubdivisionSurface.ImportFEFFile` and the `SaveToStream` methods of
//! the control point / edge / face classes:
//!
//! ```text
//! <project name>
//! <designer>
//! <created by>
//! <comment>
//! <length> <beam> <draft> <water density> <appendage coeff> <units> <mainparticulars set> <precision>
//! <layer count>
//!   <layer name>
//!   <id> <color> <visible> <developable> <symmetric> <use for intersections> <use in hydrostatics> <show in linesplan> <density> <thickness>
//! <control point count>
//!   <x> <y> <z> <vertex type> <selected>
//! <control edge count>
//!   <a> <b> <crease> <selected>
//! <control face count>
//!   <n> <p1> ... <pn> <layer> <selected>
//! ```
//!
//! Booleans are written as `0`/`1`; the parser also accepts `true`/`false`.
//! Further trailing tokens are ignored, so files written by the original
//! application parse unchanged.
//!
//! **One project setting is deliberately not in the file**: the mainframe
//! station (`ProjectSettings::mainframe_location`). The settings line above is
//! the whole of what FREE!ship writes, and inventing a token for a value the
//! original application cannot read back would make the file mean one thing
//! here and another there. A model therefore loads with `0`, which every
//! measurement reads as "use the LCF"; a caller that wants the midship section
//! at a particular station passes it in (`--mainframe`, or `mainframe_x` over
//! MCP) rather than relying on state that would not survive the round trip.

use crate::math::Vec3;
use crate::mesh::{ControlEdge, ControlPoint, Layer, ProjectSettings, Surface, VertexKind};
use std::fmt::Write as _;

#[derive(Debug)]
pub struct FefError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for FefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for FefError {}

fn err(line: usize, msg: impl Into<String>) -> FefError {
    FefError {
        line,
        message: msg.into(),
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "t" | "yes" => Some(true),
        "0" | "false" | "f" | "no" => Some(false),
        _ => None,
    }
}

fn parse_f64(s: &str, line: usize, what: &str) -> Result<f64, FefError> {
    s.trim()
        .replace(',', ".")
        .parse::<f64>()
        .map_err(|_| err(line, format!("invalid {} value '{}'", what, s)))
}

fn parse_usize(s: &str, line: usize, what: &str) -> Result<usize, FefError> {
    s.trim()
        .parse::<usize>()
        .map_err(|_| err(line, format!("invalid {} value '{}'", what, s)))
}

fn parse_i32(s: &str, line: usize, what: &str) -> Result<i32, FefError> {
    s.trim()
        .parse::<i32>()
        .map_err(|_| err(line, format!("invalid {} value '{}'", what, s)))
}

/// Parse a `.fef` document.
pub fn parse(text: &str) -> Result<Surface, FefError> {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0usize;
    let next = |i: &mut usize| -> Result<String, FefError> {
        if *i >= lines.len() {
            return Err(err(lines.len(), "unexpected end of file"));
        }
        let s = lines[*i].trim_end_matches('\r').to_string();
        *i += 1;
        Ok(s)
    };

    // Field initialisers run in written order, so the four lines are read in
    // the order the format stores them.
    let mut settings = ProjectSettings {
        name: next(&mut i)?,
        designer: next(&mut i)?,
        created_by: next(&mut i)?,
        comment: next(&mut i)?,
        ..Default::default()
    };

    let hdr_line = i;
    let hdr = next(&mut i)?;
    let t: Vec<&str> = hdr.split_whitespace().collect();
    if t.len() < 5 {
        return Err(err(hdr_line, "project line needs at least 5 numbers"));
    }
    settings.length = parse_f64(t[0], hdr_line, "length")?;
    settings.beam = parse_f64(t[1], hdr_line, "beam")?;
    settings.draft = parse_f64(t[2], hdr_line, "draft")?;
    settings.water_density = parse_f64(t[3], hdr_line, "water density")?;
    settings.appendage_coefficient = parse_f64(t[4], hdr_line, "appendage coefficient")?;
    if t.len() > 5 {
        settings.imperial = parse_usize(t[5], hdr_line, "unit system")? == 1;
    }
    if t.len() > 6 {
        settings.main_particulars_set = parse_bool(t[6]).unwrap_or(false);
    }

    // ---- layers ---------------------------------------------------------
    let nlayers_line = i;
    let nlayers = parse_usize(&next(&mut i)?, nlayers_line, "layer count")?;
    let mut layers: Vec<Layer> = Vec::with_capacity(nlayers);
    for _ in 0..nlayers {
        let name_line = i;
        let name = next(&mut i)?;
        let data_line = i;
        let data = next(&mut i)?;
        let t: Vec<&str> = data.split_whitespace().collect();
        if t.len() < 5 {
            return Err(err(data_line, "layer line needs at least 5 fields"));
        }
        let mut l = Layer {
            name,
            id: parse_i32(t[0], data_line, "layer id")?,
            color: parse_i32(t[1], data_line, "layer colour")?,
            visible: parse_bool(t[2]).unwrap_or(true),
            developable: parse_bool(t[3]).unwrap_or(false),
            symmetric: parse_bool(t[4]).unwrap_or(true),
            ..Default::default()
        };
        if t.len() > 5 {
            l.use_for_intersections = parse_bool(t[5]).unwrap_or(true);
        }
        if t.len() > 6 {
            l.use_in_hydrostatics = parse_bool(t[6]).unwrap_or(true);
        }
        if t.len() > 7 {
            l.show_in_linesplan = parse_bool(t[7]).unwrap_or(true);
        }
        if t.len() > 8 {
            l.density = parse_f64(t[8], data_line, "layer density")?;
        }
        if t.len() > 9 {
            l.thickness = parse_f64(t[9], data_line, "layer thickness")?;
        }
        let _ = name_line;
        layers.push(l);
    }
    if layers.is_empty() {
        layers.push(Layer::default());
    }

    // ---- control points -------------------------------------------------
    let npoints_line = i;
    let npoints = parse_usize(&next(&mut i)?, npoints_line, "control point count")?;
    let mut points = Vec::with_capacity(npoints);
    for _ in 0..npoints {
        let line = i;
        let s = next(&mut i)?;
        let t: Vec<&str> = s.split_whitespace().collect();
        if t.len() < 3 {
            return Err(err(line, "control point needs x y z"));
        }
        let p = Vec3::new(
            parse_f64(t[0], line, "x")?,
            parse_f64(t[1], line, "y")?,
            parse_f64(t[2], line, "z")?,
        );
        let kind = if t.len() > 3 {
            VertexKind::from_code(parse_usize(t[3], line, "vertex type")? as u8)
        } else {
            VertexKind::Regular
        };
        points.push(ControlPoint { p, kind });
    }

    // ---- control edges --------------------------------------------------
    let nedges_line = i;
    let nedges = parse_usize(&next(&mut i)?, nedges_line, "control edge count")?;
    let mut edges = Vec::with_capacity(nedges);
    let mut bad_edges = Vec::new();
    for _ in 0..nedges {
        let line = i;
        let s = next(&mut i)?;
        let t: Vec<&str> = s.split_whitespace().collect();
        if t.len() < 2 {
            return Err(err(line, "control edge needs two point indices"));
        }
        let a = parse_usize(t[0], line, "edge start")?;
        let b = parse_usize(t[1], line, "edge end")?;
        if a >= points.len() || b >= points.len() {
            bad_edges.push(line);
            continue;
        }
        let crease = t.get(2).and_then(|s| parse_bool(s)).unwrap_or(false);
        edges.push(ControlEdge { a, b, crease });
    }
    if let Some(line) = bad_edges.first() {
        return Err(err(*line, "control edge references a missing point"));
    }

    // ---- control faces --------------------------------------------------
    let nfaces_line = i;
    let nfaces = parse_usize(&next(&mut i)?, nfaces_line, "control face count")?;
    let mut faces = Vec::with_capacity(nfaces);
    let mut face_layer = Vec::with_capacity(nfaces);
    for _ in 0..nfaces {
        let line = i;
        let s = next(&mut i)?;
        let t: Vec<&str> = s.split_whitespace().collect();
        if t.is_empty() {
            return Err(err(line, "empty control face line"));
        }
        let n = parse_usize(t[0], line, "face point count")?;
        if n < 3 {
            return Err(err(line, "control face needs at least 3 points"));
        }
        if t.len() < n + 1 {
            return Err(err(line, "control face point list is truncated"));
        }
        let mut pts = Vec::with_capacity(n);
        for k in 0..n {
            let idx = parse_usize(t[k + 1], line, "face point index")?;
            if idx >= points.len() {
                return Err(err(line, format!("face references missing point {}", idx)));
            }
            pts.push(idx);
        }
        let mut layer = 0usize;
        if t.len() > n + 1 {
            let l = parse_i32(t[n + 1], line, "layer index")?;
            layer = if l < 0 { 0 } else { l as usize };
        }
        if layer >= layers.len() {
            layer = 0;
        }
        faces.push(pts);
        face_layer.push(layer);
    }

    let mut surface = Surface {
        points,
        edges,
        faces,
        face_layer,
        layers,
        settings,
    };
    surface.rebuild_edges();
    surface.validate().map_err(|m| err(nfaces_line, m))?;
    Ok(surface)
}

/// Serialise to the FREE!ship exchange format.
pub fn write(surface: &Surface) -> String {
    let mut out = String::with_capacity(64 * 1024);
    let s = &surface.settings;
    let _ = writeln!(out, "{}", s.name);
    let _ = writeln!(out, "{}", s.designer);
    let _ = writeln!(out, "{}", s.created_by);
    let _ = writeln!(out, "{}", s.comment);
    let _ = writeln!(
        out,
        "{:.4} {:.4} {:.4} {:.5} {:.5} {} {} 0",
        s.length,
        s.beam,
        s.draft,
        s.water_density,
        s.appendage_coefficient,
        if s.imperial { 1 } else { 0 },
        if s.main_particulars_set { 1 } else { 0 }
    );

    let _ = writeln!(out, "{}", surface.layers.len());
    for l in &surface.layers {
        let _ = writeln!(out, "{}", if l.name.is_empty() { "Layer" } else { &l.name });
        let _ = writeln!(
            out,
            "{} {} {} {} {} {} {} {} {:.8} {:.8}",
            l.id,
            l.color,
            b(l.visible),
            b(l.developable),
            b(l.symmetric),
            b(l.use_for_intersections),
            b(l.use_in_hydrostatics),
            b(l.show_in_linesplan),
            l.density,
            l.thickness
        );
    }

    let _ = writeln!(out, "{}", surface.points.len());
    for p in &surface.points {
        let _ = writeln!(
            out,
            "{:.5} {:.5} {:.5} {} 0",
            p.p.x,
            p.p.y,
            p.p.z,
            p.kind.code()
        );
    }

    // Free!ship's FEF stores edges explicitly; the list is regenerated from
    // the faces so the file is always self consistent.
    let mut edges = surface.edges.clone();
    if edges.is_empty() {
        let mut tmp = surface.clone();
        tmp.rebuild_edges();
        edges = tmp.edges;
    }
    let _ = writeln!(out, "{}", edges.len());
    for e in &edges {
        let _ = writeln!(out, "{} {} {} 0", e.a, e.b, b(e.crease));
    }

    let _ = writeln!(out, "{}", surface.faces.len());
    for (fi, f) in surface.faces.iter().enumerate() {
        let mut line = format!("{}", f.len());
        for &p in f {
            let _ = write!(line, " {}", p);
        }
        let _ = writeln!(out, "{} {} 0", line, surface.face_layer[fi]);
    }
    out
}

fn b(v: bool) -> u8 {
    if v {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut s = Surface::new();
        s.settings.name = "Test".into();
        s.settings.length = 10.0;
        for p in [
            (0.0, 0.0, 0.0),
            (1.0, 0.0, 0.0),
            (1.0, 1.0, 0.5),
            (0.0, 1.0, 0.5),
        ] {
            s.add_point(Vec3::new(p.0, p.1, p.2));
        }
        s.add_face(vec![0, 1, 2, 3], 0);
        s.points[3].kind = VertexKind::Corner;
        let text = write(&s);
        let back = parse(&text).expect("parse");
        assert_eq!(back.points.len(), 4);
        assert_eq!(back.faces.len(), 1);
        assert_eq!(back.points[3].kind, VertexKind::Corner);
        assert!((back.settings.length - 10.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_missing_point() {
        let text = "n\nd\nc\n\n0 0 0 1025 1 0 0 0\n1\nLayer\n1 0 1 0 1 1 1 1 7850 0.005\n3\n0 0 0 0 0\n1 0 0 0 0\n2 0 0 0 0\n2\n0 1 0 0\n1 2 0 0\n1\n3 0 1 5 0 0\n";
        assert!(parse(text).is_err());
    }
}
