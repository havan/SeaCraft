//! A zero-dependency AutoCAD 2000 (`AC1015`) DXF writer.
//!
//! Ported and adapted from Tangency Labs' `solidcraft` project,
//! `crates/io/src/dxf_out.rs` (MIT OR Apache-2.0) — see `ATTRIBUTION.md`. The
//! port drops the exact-conic/ellipse machinery (SeaCraft's linesplan is
//! polylines) and its millimetre-specific header, and adds 3D polylines so the
//! hull curves can be handed to Rhino or ShipConstructor in place.
//!
//! The file carries what release 2000 needs to read it: the version, the
//! handle seed, the units and the extents; the linetype, layer and style
//! tables; the model and paper space blocks; handles, owners and subclass
//! markers on every entity; and the root dictionary. Group codes follow
//! Autodesk's public DXF reference.
//!
//! Units: SeaCraft measures in metres, so the header declares `$INSUNITS` 6
//! (metres) and metric measurement rather than the original's millimetres.

use crate::math::Vec3;
use std::fmt::Write as _;

/// A point in the drawing plane. DXF's LWPOLYLINE and friends are 2D; use
/// [`Vec3`] and [`DxfShape::Polyline3`] for entities that carry height.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn dist(self, o: Vec2) -> f64 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }
}

/// A layer's linetype, defined in the file's own LTYPE table so a viewer needs
/// nothing else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DxfLinetype {
    #[default]
    Continuous,
    /// AutoCAD's DASHED: a dash of 12.7 mm and a gap of 6.35 mm.
    Dashed,
    /// Short dashes for hidden edges.
    Hidden,
}

impl DxfLinetype {
    fn name(self) -> &'static str {
        match self {
            DxfLinetype::Continuous => "Continuous",
            DxfLinetype::Dashed => "DASHED",
            DxfLinetype::Hidden => "HIDDEN",
        }
    }
}

/// A layer: its name, AutoCAD colour index (1 red, 3 green, 5 blue, 7 white or
/// black) and linetype.
#[derive(Clone, Debug, PartialEq)]
pub struct DxfLayer {
    pub name: String,
    pub color: i16,
    pub linetype: DxfLinetype,
}

impl DxfLayer {
    /// A continuous layer.
    pub fn new(name: impl Into<String>, color: i16) -> DxfLayer {
        DxfLayer { name: name.into(), color, linetype: DxfLinetype::Continuous }
    }

    /// The same layer with a dashed line.
    pub fn dashed(self) -> DxfLayer {
        DxfLayer { linetype: DxfLinetype::Dashed, ..self }
    }
}

/// The geometry of one entity, in metres.
#[derive(Clone, Debug, PartialEq)]
pub enum DxfShape {
    Line(Vec2, Vec2),
    /// Connected straight segments with continuous linetype generation, so a
    /// dash pattern runs through the vertices instead of restarting at each.
    Polyline { points: Vec<Vec2> },
    /// A 3D polyline (`POLYLINE` with 3D vertices), for hull curves in place.
    Polyline3 { points: Vec<Vec3> },
    Circle { c: Vec2, r: f64 },
    /// Counter-clockwise from `a0` to `a1` (degrees).
    Arc { c: Vec2, r: f64, a0: f64, a1: f64 },
    /// One line of text, centred on `at` (its baseline) and turned
    /// `angle_deg` counter-clockwise.
    Text { at: Vec2, height: f64, angle_deg: f64, text: String },
}

impl DxfShape {
    /// Points on the shape (or its control polygon) for the drawing's extents.
    fn sample(&self, out: &mut Vec<Vec2>) {
        match self {
            DxfShape::Line(a, b) => out.extend([*a, *b]),
            DxfShape::Polyline { points } => out.extend(points.iter().copied()),
            DxfShape::Polyline3 { points } => {
                out.extend(points.iter().map(|p| Vec2::new(p.x, p.y)))
            }
            DxfShape::Circle { c, r } => out.extend([
                Vec2::new(c.x - r, c.y - r),
                Vec2::new(c.x + r, c.y + r),
            ]),
            DxfShape::Arc { c, r, a0, a1 } => {
                let sweep = (a1 - a0).rem_euclid(360.0);
                out.extend((0..=32).map(|i| {
                    let a = (a0 + sweep * i as f64 / 32.0).to_radians();
                    Vec2::new(c.x + a.cos() * r, c.y + a.sin() * r)
                }));
            }
            DxfShape::Text { at, height, angle_deg, text } => {
                let (c, s) = (
                    angle_deg.to_radians().cos(),
                    angle_deg.to_radians().sin(),
                );
                let half = 0.4 * height * text.chars().count() as f64;
                for along in [-half, half] {
                    for up in [0.0, *height] {
                        out.push(Vec2::new(
                            at.x + c * along - s * up,
                            at.y + s * along + c * up,
                        ));
                    }
                }
            }
        }
    }
}

/// An entity on a layer.
#[derive(Clone, Debug, PartialEq)]
pub struct DxfEntity {
    pub layer: String,
    pub shape: DxfShape,
}

impl DxfEntity {
    pub fn new(layer: impl Into<String>, shape: DxfShape) -> Self {
        Self { layer: layer.into(), shape }
    }
}

/// Group codes and handles, in file order.
struct Out {
    s: String,
    next: u64,
}

impl Out {
    fn code(&mut self, c: i32, v: impl std::fmt::Display) {
        let _ = write!(self.s, "{c:>3}\n{v}\n");
    }

    /// A real number: up to twelve decimals, at least one.
    fn num(&mut self, c: i32, v: f64) {
        let text = format!("{:.12}", if v.is_finite() { v } else { 0.0 });
        let text = text.trim_end_matches('0');
        let text = if text.ends_with('.') { format!("{text}0") } else { text.to_string() };
        self.code(c, if text == "-0.0" { "0.0" } else { text.as_str() });
    }

    /// A point in the plane at height 0: codes `c`, `c + 10`, `c + 20`.
    fn point(&mut self, c: i32, p: Vec2) {
        self.num(c, p.x);
        self.num(c + 10, p.y);
        self.num(c + 20, 0.0);
    }

    fn handle(&mut self) -> String {
        let h = format!("{:X}", self.next);
        self.next += 1;
        h
    }

    /// The start of a symbol table; returns its handle.
    fn table(&mut self, name: &str, entries: usize) -> String {
        let h = self.handle();
        self.code(0, "TABLE");
        self.code(2, name);
        self.code(5, &h);
        self.code(330, 0);
        self.code(100, "AcDbSymbolTable");
        self.code(70, entries);
        h
    }

    /// The start of a symbol table record.
    fn record(&mut self, kind: &str, subclass: &str, table: &str) {
        let h = self.handle();
        self.code(0, kind);
        self.code(5, h);
        self.code(330, table);
        self.code(100, "AcDbSymbolTableRecord");
        self.code(100, subclass);
    }

    /// A BLOCK and its ENDBLK, empty, owned by the block record `owner`.
    fn block(&mut self, owner: &str, name: &str, paper: bool) {
        for (kind, sub) in [("BLOCK", "AcDbBlockBegin"), ("ENDBLK", "AcDbBlockEnd")] {
            let h = self.handle();
            self.code(0, kind);
            self.code(5, h);
            self.code(330, owner);
            self.code(100, "AcDbEntity");
            if paper {
                self.code(67, 1);
            }
            self.code(8, "0");
            self.code(100, sub);
            if kind == "BLOCK" {
                self.code(2, name);
                self.code(70, 0);
                self.point(10, Vec2::ZERO);
                self.code(3, name);
                self.code(1, "");
            }
        }
    }
}

/// Control characters become spaces and letters outside ASCII become
/// `\U+XXXX`, as a TEXT entity holds them.
fn text_value(s: &str) -> String {
    s.chars()
        .take(250)
        .map(|c| match c {
            c if c.is_ascii_control() => " ".to_string(),
            c if c.is_ascii() => c.to_string(),
            c => u16::try_from(u32::from(c))
                .map_or_else(|_| "?".to_string(), |u| format!("\\U+{u:04X}")),
        })
        .collect()
}

/// A DXF drawing (AutoCAD 2000, metres) of `entities`. `layers` are the layers
/// they are on; layer `0` is always present.
pub fn write_dxf(layers: &[DxfLayer], entities: &[DxfEntity]) -> String {
    let mut pts = Vec::new();
    for e in entities {
        e.shape.sample(&mut pts);
    }
    pts.retain(|p| p.x.is_finite() && p.y.is_finite());
    let (lo, hi) = pts.iter().fold(None, |acc: Option<(Vec2, Vec2)>, p| {
        Some(acc.map_or((*p, *p), |(lo, hi)| {
            (
                Vec2::new(lo.x.min(p.x), lo.y.min(p.y)),
                Vec2::new(hi.x.max(p.x), hi.y.max(p.y)),
            )
        }))
    })
    .unwrap_or((Vec2::ZERO, Vec2::ZERO));

    let mut layer_list = vec![DxfLayer::new("0", 7)];
    for l in layers {
        if !layer_list.iter().any(|x| x.name == l.name) {
            layer_list.push(l.clone());
        }
    }

    // Everything but the header, which needs the handle seed.
    let mut o = Out { s: String::new(), next: 0x20 };
    let (model, paper) = (o.handle(), o.handle());
    let root = o.handle();

    o.code(0, "SECTION");
    o.code(2, "TABLES");
    // The three every drawing has, plus DASHED, which the dashed layers name:
    // dash and gap lengths (a gap is negative) after the pattern's total.
    let mut linetypes: Vec<(&str, &str, &[f64])> = vec![
        ("ByBlock", "", &[]),
        ("ByLayer", "", &[]),
        ("Continuous", "Solid line", &[]),
        ("DASHED", "Dashed __ __ __ __ __ __ __ __ __ __", &[12.7, -6.35]),
    ];
    if layer_list.iter().any(|l| l.linetype == DxfLinetype::Hidden) {
        linetypes.push(("HIDDEN", "Hidden edges", &[2.0, -1.0]));
    }
    let t = o.table("LTYPE", linetypes.len());
    for (name, description, pattern) in linetypes {
        o.record("LTYPE", "AcDbLinetypeTableRecord", &t);
        o.code(2, name);
        o.code(70, 0);
        o.code(3, description);
        o.code(72, 65);
        o.code(73, pattern.len());
        o.num(40, pattern.iter().map(|d| d.abs()).sum());
        for d in pattern {
            o.num(49, *d);
            o.code(74, 0);
        }
    }
    o.code(0, "ENDTAB");
    let t = o.table("LAYER", layer_list.len());
    for l in &layer_list {
        o.record("LAYER", "AcDbLayerTableRecord", &t);
        o.code(2, &l.name);
        o.code(70, 0);
        o.code(62, l.color);
        o.code(6, l.linetype.name());
    }
    o.code(0, "ENDTAB");
    // The one text style, which a TEXT entity uses when it names none.
    let t = o.table("STYLE", 1);
    o.record("STYLE", "AcDbTextStyleTableRecord", &t);
    o.code(2, "Standard");
    o.code(70, 0);
    o.num(40, 0.0);
    o.num(41, 1.0);
    o.num(50, 0.0);
    o.code(71, 0);
    o.num(42, 2.5);
    o.code(3, "txt");
    o.code(4, "");
    o.code(0, "ENDTAB");
    let t = o.table("BLOCK_RECORD", 2);
    for (h, name) in [(&model, "*Model_Space"), (&paper, "*Paper_Space")] {
        o.code(0, "BLOCK_RECORD");
        o.code(5, h);
        o.code(330, &t);
        o.code(100, "AcDbSymbolTableRecord");
        o.code(100, "AcDbBlockTableRecord");
        o.code(2, name);
    }
    o.code(0, "ENDTAB");
    o.code(0, "ENDSEC");

    o.code(0, "SECTION");
    o.code(2, "BLOCKS");
    o.block(&model, "*Model_Space", false);
    o.block(&paper, "*Paper_Space", true);
    o.code(0, "ENDSEC");

    o.code(0, "SECTION");
    o.code(2, "ENTITIES");
    for e in entities {
        let kind = match &e.shape {
            DxfShape::Line(..) => "LINE",
            DxfShape::Polyline { .. } => "LWPOLYLINE",
            DxfShape::Polyline3 { .. } => "POLYLINE",
            DxfShape::Circle { .. } => "CIRCLE",
            DxfShape::Arc { .. } => "ARC",
            DxfShape::Text { .. } => "TEXT",
        };
        let h = o.handle();
        o.code(0, kind);
        o.code(5, h);
        o.code(330, &model);
        o.code(100, "AcDbEntity");
        o.code(8, if e.layer.trim().is_empty() { "0" } else { e.layer.as_str() });
        match &e.shape {
            DxfShape::Polyline { points } => {
                o.code(100, "AcDbPolyline");
                o.code(90, points.len());
                // Bit 128 (PLINEGEN) continues the dash pattern.
                o.code(70, 128);
                for p in points {
                    o.num(10, p.x);
                    o.num(20, p.y);
                }
            }
            DxfShape::Polyline3 { points } => {
                o.code(100, "AcDb3dPolyline");
                o.code(66, 1);
                o.point(10, Vec2::ZERO);
                // Bit 8 (3D polyline).
                o.code(70, 8);
                for p in points {
                    let vh = o.handle();
                    o.code(0, "VERTEX");
                    o.code(5, vh);
                    o.code(330, &model);
                    o.code(100, "AcDbEntity");
                    o.code(8, if e.layer.trim().is_empty() { "0" } else { e.layer.as_str() });
                    o.code(100, "AcDbVertex");
                    o.code(100, "AcDb3dPolylineVertex");
                    o.num(10, p.x);
                    o.num(20, p.y);
                    o.num(30, p.z);
                    // Bit 32 (3D polyline vertex).
                    o.code(70, 32);
                }
                let sh = o.handle();
                o.code(0, "SEQEND");
                o.code(5, sh);
                o.code(330, &model);
                o.code(100, "AcDbEntity");
                o.code(8, if e.layer.trim().is_empty() { "0" } else { e.layer.as_str() });
            }
            DxfShape::Line(a, b) => {
                o.code(100, "AcDbLine");
                o.point(10, *a);
                o.point(11, *b);
            }
            DxfShape::Circle { c, r } => {
                o.code(100, "AcDbCircle");
                o.point(10, *c);
                o.num(40, *r);
            }
            DxfShape::Arc { c, r, a0, a1 } => {
                o.code(100, "AcDbCircle");
                o.point(10, *c);
                o.num(40, *r);
                o.code(100, "AcDbArc");
                o.num(50, *a0);
                o.num(51, *a1);
            }
            DxfShape::Text { at, height, angle_deg, text } => {
                o.code(100, "AcDbText");
                o.point(10, *at);
                o.num(40, *height);
                o.code(1, text_value(text));
                o.num(50, *angle_deg);
                // Centred (72 = 1): a reader that places the text by the second
                // alignment point and one that takes the first both put its
                // middle on `at`.
                o.code(72, 1);
                o.point(11, *at);
                o.code(100, "AcDbText");
                o.code(73, 0);
            }
        }
    }
    o.code(0, "ENDSEC");

    o.code(0, "SECTION");
    o.code(2, "OBJECTS");
    o.code(0, "DICTIONARY");
    o.code(5, root.as_str());
    o.code(330, 0);
    o.code(100, "AcDbDictionary");
    o.code(281, 1);
    o.code(0, "ENDSEC");
    o.code(0, "EOF");

    let mut head = Out { s: String::new(), next: 0 };
    head.code(0, "SECTION");
    head.code(2, "HEADER");
    head.code(9, "$ACADVER");
    head.code(1, "AC1015");
    head.code(9, "$HANDSEED");
    head.code(5, format!("{:X}", o.next));
    // 6 = metres; measurement 1 = metric.
    head.code(9, "$INSUNITS");
    head.code(70, 6);
    head.code(9, "$MEASUREMENT");
    head.code(70, 1);
    head.code(9, "$EXTMIN");
    head.point(10, lo);
    head.code(9, "$EXTMAX");
    head.point(10, hi);
    head.code(0, "ENDSEC");
    head.s + &o.s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Group codes and values in file order.
    fn pairs(dxf: &str) -> Vec<(i32, String)> {
        let lines: Vec<&str> = dxf.lines().collect();
        lines
            .chunks(2)
            .map(|c| (c[0].trim().parse().unwrap(), c.get(1).unwrap_or(&"").trim().to_string()))
            .collect()
    }

    /// (kind, group codes) of each entity in the ENTITIES section.
    fn entities(dxf: &str) -> Vec<(String, Vec<(i32, String)>)> {
        let p = pairs(dxf);
        let start = p.iter().position(|(c, v)| (*c, v.as_str()) == (2, "ENTITIES")).unwrap();
        let mut out: Vec<(String, Vec<(i32, String)>)> = Vec::new();
        for (c, v) in &p[start + 1..] {
            match (*c, v.as_str()) {
                (0, "ENDSEC") => break,
                (0, kind) => out.push((kind.to_string(), Vec::new())),
                _ => out.last_mut().unwrap().1.push((*c, v.clone())),
            }
        }
        out
    }

    fn values(e: &(String, Vec<(i32, String)>), code: i32) -> Vec<f64> {
        e.1.iter().filter(|(c, _)| *c == code).map(|(_, v)| v.parse().unwrap()).collect()
    }

    #[test]
    fn drawings_carry_version_units_extents_and_handles() {
        let items = vec![
            DxfEntity::new("OUTLINE", DxfShape::Line(Vec2::new(-1.0, 2.0), Vec2::new(10.0, 2.0))),
            DxfEntity::new("OUTLINE", DxfShape::Circle { c: Vec2::new(5.0, 5.0), r: 4.0 }),
        ];
        let dxf = write_dxf(&[DxfLayer::new("OUTLINE", 7)], &items);
        let p = pairs(&dxf);
        let after = |var: &str| {
            &p[p.iter().position(|(c, v)| (*c, v.as_str()) == (9, var)).unwrap() + 1..]
        };
        assert_eq!(after("$ACADVER")[0], (1, "AC1015".to_string()));
        assert_eq!(after("$INSUNITS")[0], (70, "6".to_string()));
        assert_eq!(after("$EXTMIN")[0], (10, "-1.0".to_string()));
        assert_eq!(after("$EXTMAX")[1], (20, "9.0".to_string()));

        // Handles are unique and the seed is above all of them.
        let seed_at = p.iter().position(|(c, v)| (*c, v.as_str()) == (9, "$HANDSEED")).unwrap() + 1;
        let handles: Vec<u64> = p
            .iter()
            .enumerate()
            .filter(|(i, (c, _))| *i != seed_at && *c == 5)
            .map(|(_, (_, v))| u64::from_str_radix(v, 16).unwrap())
            .collect();
        let mut sorted = handles.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), handles.len());
        assert!(handles.iter().all(|h| *h < u64::from_str_radix(&p[seed_at].1, 16).unwrap()));
        assert!(dxf.ends_with("  0\nEOF\n"));

        // Layers, linetypes and the entities' owner.
        let layer_table = p.iter().position(|(c, v)| (*c, v.as_str()) == (2, "LAYER")).unwrap();
        let names: Vec<&str> = p[layer_table..]
            .iter()
            .take_while(|(c, v)| (*c, v.as_str()) != (0, "ENDTAB"))
            .filter(|(c, _)| *c == 2)
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(names, ["LAYER", "0", "OUTLINE"]);
        let model_at = p.iter().position(|(c, v)| (*c, v.as_str()) == (2, "*Model_Space")).unwrap();
        let model = p[model_at - 4].1.clone();
        for e in entities(&dxf) {
            assert!(e.1.contains(&(330, model.clone())), "{e:?}");
            assert!(e.1.contains(&(100, "AcDbEntity".to_string())));
        }
        // No entities: still a valid, empty drawing.
        assert_eq!(entities(&write_dxf(&[], &[])).len(), 0);
    }

    #[test]
    fn polylines_keep_their_dash_pattern_and_3d_vertices() {
        let points = vec![
            Vec2::ZERO,
            Vec2::new(1.0, 0.0),
            Vec2::new(2.0, 1.0),
            Vec2::new(3.0, 1.0),
        ];
        let trace = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.5, 0.2),
            Vec3::new(2.0, 0.0, 0.4),
        ];
        let layer = DxfLayer { linetype: DxfLinetype::Hidden, ..DxfLayer::new("HIDDEN", 8) };
        let dxf = write_dxf(
            &[layer],
            &[
                DxfEntity::new("HIDDEN", DxfShape::Polyline { points }),
                DxfEntity::new("HIDDEN", DxfShape::Polyline3 { points: trace }),
            ],
        );
        let e = entities(&dxf);
        let kinds: Vec<&str> = e.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(kinds, ["LWPOLYLINE", "POLYLINE", "VERTEX", "VERTEX", "VERTEX", "SEQEND"]);
        assert_eq!(values(&e[0], 90), [4.0]);
        assert_eq!(values(&e[0], 70), [128.0]);
        assert_eq!(values(&e[1], 70), [8.0]);
        assert_eq!(values(&e[1], 66), [1.0]);
        // Each vertex carries its own height, so the curve is in place in 3D.
        assert_eq!(values(&e[2], 30), [0.0]);
        assert_eq!(values(&e[3], 30), [0.2]);
        assert_eq!(values(&e[4], 30), [0.4]);
        assert_eq!(values(&e[3], 20), [0.5]);
        for v in &e[2..5] {
            assert_eq!(values(v, 70), [32.0]);
        }
    }

    #[test]
    fn text_is_centred_and_escaped() {
        let dxf = write_dxf(
            &[DxfLayer::new("NOTES", 8)],
            &[DxfEntity::new(
                "NOTES",
                DxfShape::Text {
                    at: Vec2::new(30.0, 12.0),
                    height: 3.0,
                    angle_deg: 90.0,
                    text: "AB%%d\nC\u{b0}".into(),
                },
            )],
        );
        let e = &entities(&dxf)[0];
        assert_eq!(e.0, "TEXT");
        assert_eq!(
            e.1.iter().find(|(c, _)| *c == 1).map(|(_, v)| v.as_str()),
            Some("AB%%d C\\U+00B0")
        );
        assert_eq!(values(e, 72), [1.0]);
        assert_eq!(values(e, 50), [90.0]);
    }

    #[test]
    fn the_numbers_are_plain_and_never_negative_zero() {
        let dxf = write_dxf(
            &[],
            &[DxfEntity::new(
                "0",
                DxfShape::Line(Vec2::new(-0.0, 0.0), Vec2::new(1.0 / 3.0, 0.0)),
            )],
        );
        assert!(dxf.contains("\n0.0\n"), "negative zero is normalised");
        assert!(dxf.contains("\n0.333333333333\n"), "twelve decimals, trailing zeros trimmed");
        let counts: HashMap<i32, usize> = pairs(&dxf).into_iter().fold(HashMap::new(), |mut m, (c, _)| {
            *m.entry(c).or_default() += 1;
            m
        });
        assert!(counts.get(&0).copied().unwrap_or(0) > 10);
    }
}
