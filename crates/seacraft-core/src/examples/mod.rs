//! The example-hull library: ready-made models with a design sidecar.
//!
//! Each example is a hand-authored control net (a symmetric half model on a
//! station grid) plus a [`Project`] carrying the example's KG, its design
//! requirements and a "Design" loading condition. The nets were calibrated
//! once, at authoring time, against level-3 Loop hydrostatics and their
//! numbers frozen in the hull's station table: [`build`] runs no solver, so
//! it is cheap enough for the New design preview.
//!
//! Every example is checked in its own file against the targets of the
//! examples board (`.hermes/research/examples-board/20-chair-decision.md`):
//! Δ ±2 %, LWL and BWL ±1 %, Cb and Cp ±0.02, Cm and Cwp ±0.03, LCB ±0.5 %
//! of LWL, closed below the waterline, GM ≥ 0.15 m at the example's KG,
//! |GZ(0°)| < 1 mm, free-trim equilibrium in the Design condition within
//! 0.5 % of LWL, a lossless `.fef` round trip and a valid sidecar. The shared
//! checks live in [`checks`] (tests only). `Cm` and `Cp` are measured at the
//! middle of the waterline length (`mainframe_x = wl_aft_x + LWL / 2`).
//!
//! # Adding a hull
//!
//! A hull batch adds one file and flips one line here:
//!
//! 1. `examples/<hull>.rs` with `pub(super) fn build() -> (Surface, Project)`
//!    and its acceptance tests (use [`checks`]);
//! 2. `mod <hull>;` below, and `Some(<hull>::build)` in place of `None` in
//!    [`Example::builder`];
//! 3. `seacraft example --all -o examples/` to regenerate the committed
//!    `examples/<slug>.fef` / `.seacraft.json` (the disk-parity test fails
//!    until they match `build`).
//!
//! An example whose builder is `None` is not available: [`Example::all`],
//! [`Example::with_benchmarks`] and [`Example::from_slug`] leave it out, and
//! [`build`] refuses it, so nothing advertises a hull that is not there yet.
//!
//! # Conventions
//!
//! `x` forward, `y` to starboard, `z` up; the keel (the lowest hydrostatic
//! point) is at `z = 0`, so KG and VCG coincide; the aft end of the design
//! waterline is at `x = 0` unless the hull says otherwise. Coordinates are
//! rounded to the five decimals a `.fef` stores, so a saved example reads
//! back bit for bit.

use std::collections::HashMap;

use crate::loading::{LoadingCondition, WeightItem};
use crate::math::Vec3;
use crate::mesh::{ordered, Layer, ProjectSettings, Surface};
use crate::project::Project;

mod container;
mod kayik;
mod runabout;
mod trawler;
mod wigley;
mod yacht;

/// The example hulls. Variants whose hull file has not landed yet are not
/// available (see the module documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Example {
    /// 10 m fin-keel cruising yacht.
    Yacht10m,
    /// 9 m Aegean piyade kayık.
    Kayik9m,
    /// 6.5 m hard-chine outboard runabout.
    Runabout6m,
    /// "KCS-like" 230 m container ship.
    Container230m,
    /// 22 m steel fishing vessel.
    Trawler22m,
    /// The Wigley parabolic hull, a hydrostatics benchmark (CLI and tests).
    Wigley,
}

/// What [`build`] returns: the control net and its design sidecar.
pub type Built = (Surface, Project);

impl Example {
    /// Every variant, in menu order (available or not).
    pub const VARIANTS: [Example; 6] = [
        Example::Yacht10m,
        Example::Kayik9m,
        Example::Runabout6m,
        Example::Container230m,
        Example::Trawler22m,
        Example::Wigley,
    ];

    /// The hull's builder, or `None` while its file has not landed.
    fn builder(self) -> Option<fn() -> Built> {
        match self {
            Example::Yacht10m => Some(yacht::build),
            Example::Kayik9m => Some(kayik::build),
            Example::Runabout6m => Some(runabout::build),
            Example::Container230m => Some(container::build),
            Example::Trawler22m => Some(trawler::build),
            Example::Wigley => Some(wigley::build),
        }
    }

    /// The command-line name, also the file stem under `examples/`.
    pub fn slug(self) -> &'static str {
        match self {
            Example::Yacht10m => "yacht-10m",
            Example::Kayik9m => "kayik-9m",
            Example::Runabout6m => "runabout-6m",
            Example::Container230m => "container-230m",
            Example::Trawler22m => "trawler-22m",
            Example::Wigley => "wigley",
        }
    }

    /// A short title for a picker.
    pub fn title(self) -> &'static str {
        match self {
            Example::Yacht10m => "10 m cruising yacht",
            Example::Kayik9m => "9 m Aegean kayık (piyade)",
            Example::Runabout6m => "6.5 m outboard runabout",
            Example::Container230m => "230 m container ship (KCS-like)",
            Example::Trawler22m => "22 m fishing vessel",
            Example::Wigley => "Wigley hull (benchmark)",
        }
    }

    /// One line: what the hull is.
    pub fn description(self) -> &'static str {
        match self {
            Example::Yacht10m => "Fin-keel cruiser with spade rudder and deck",
            Example::Kayik9m => {
                "Round-bilge workboat on a deep keel, raked stem, small transom, inboard diesel, 7 kn; means of 12 surveyed piyade boats (Turan 2022), lines our own; KG 0.95 m estimated"
            }
            Example::Runabout6m => {
                "Hard-chine V-bottom planing boat, flat raked transom for an outboard, 30 kn; KG 0.65 m assumed"
            }
            Example::Container230m => "Bulbous bow and transom stern, KCS particulars, own lines",
            Example::Trawler22m => {
                "Steel round-bilge stern trawler: skeg, raked transom, bulwark; KG 2.50 m assumed"
            }
            Example::Wigley => {
                "Parabolic benchmark hull, L 10 m, B 1 m, T 0.625 m; closed-form hydrostatics"
            }
        }
    }

    /// Where the target particulars come from.
    pub fn source(self) -> &'static str {
        match self {
            Example::Kayik9m => "Turan (2022), mean of 12 piyade boats",
            Example::Container230m => "KCS benchmark particulars; lines our own",
            Example::Trawler22m => "representative of vessels of this size",
            Example::Wigley => "closed form",
            _ => "representative of production boats of this size",
        }
    }

    /// Whether this build has the hull.
    pub fn is_available(self) -> bool {
        self.builder().is_some()
    }

    /// A benchmark for the command line and the tests, not for the GUI.
    pub fn is_benchmark(self) -> bool {
        self == Example::Wigley
    }

    /// The examples the GUI offers: available, benchmarks left out.
    pub fn all() -> Vec<Example> {
        Self::VARIANTS
            .into_iter()
            .filter(|e| e.is_available() && !e.is_benchmark())
            .collect()
    }

    /// Every available example, benchmarks included (the CLI's list).
    pub fn with_benchmarks() -> Vec<Example> {
        Self::VARIANTS
            .into_iter()
            .filter(|e| e.is_available())
            .collect()
    }

    /// The available example called `slug`; the error lists the names.
    pub fn from_slug(slug: &str) -> Result<Example, String> {
        let slug = slug.trim();
        Self::with_benchmarks()
            .into_iter()
            .find(|e| e.slug().eq_ignore_ascii_case(slug))
            .ok_or_else(|| {
                let names: Vec<&str> = Self::with_benchmarks().iter().map(|e| e.slug()).collect();
                format!(
                    "no example named `{slug}` (available: {})",
                    names.join(", ")
                )
            })
    }
}

impl std::fmt::Display for Example {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

/// Build an example: its control net and its design sidecar.
pub fn build(kind: Example) -> Result<Built, String> {
    kind.builder()
        .map(|b| b())
        .ok_or_else(|| format!("the `{kind}` example is not available in this build"))
}

// ---- net primitives -------------------------------------------------------

// ---- plating materials (kg/m³), for a layer's weight estimate -------------
//
// A layer's density and thickness feed only the plating weight estimate
// (area × thickness × density); no hydrostatic number depends on them.

/// Mild and high-tensile shipbuilding steel.
pub(crate) const STEEL: f64 = 7850.0;
/// Hand-laid glass-reinforced polyester laminate.
pub(crate) const GRP: f64 = 1900.0;
/// Planking timber (larch, pine, iroko).
pub(crate) const TIMBER: f64 = 600.0;
/// Heavy structural timber for keels and deadwood (oak).
pub(crate) const OAK: f64 = 800.0;
/// Cast lead, for a ballast keel.
pub(crate) const LEAD: f64 = 11300.0;

/// Plating of a layer: material density (kg/m³) and thickness (mm).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Plating {
    pub density: f64,
    pub thickness_mm: f64,
}

/// `density` kg/m³, `thickness_mm` millimetres.
pub(crate) const fn plating(density: f64, thickness_mm: f64) -> Plating {
    Plating {
        density,
        thickness_mm,
    }
}

/// A symmetric hydrostatic layer with its plating.
pub(crate) fn layer(id: i32, name: &str, color: i32, developable: bool, plate: Plating) -> Layer {
    Layer {
        name: name.into(),
        id,
        color,
        visible: true,
        developable,
        symmetric: true,
        use_for_intersections: true,
        use_in_hydrostatics: true,
        show_in_linesplan: true,
        density: plate.density,
        thickness: plate.thickness_mm / 1000.0,
    }
}

/// Project settings for an example: the design draft is the hull's `T`
/// above the keel, `length` its LWL (the trim lever), and no mainframe
/// station (the `.fef` cannot carry one).
pub(crate) fn settings(kind: Example, lwl: f64, bwl: f64, draft: f64) -> ProjectSettings {
    ProjectSettings {
        name: kind.title().into(),
        designer: "SeaCraft example library".into(),
        created_by: format!("SeaCraft example `{}`", kind.slug()),
        comment: format!("{} ({})", kind.description(), kind.source()),
        length: lwl,
        beam: bwl,
        draft,
        water_density: 1025.0,
        appendage_coefficient: 1.0,
        imperial: false,
        main_particulars_set: true,
        mainframe_location: 0.0,
    }
}

/// Add a station grid of quads. `stations[i][j]` runs aft to forward in `i`
/// and keel to sheer in `j` on the starboard side (`y >= 0`); every station
/// has the same number of points. The winding
/// `[p(i, j), p(i, j+1), p(i+1, j+1), p(i+1, j)]` points outward for that
/// ordering. `layer_of(i, j)` names the layer of the face whose lower-aft
/// corner is `(i, j)`. Returns the point indices `[i][j]`.
pub(crate) fn station_grid(
    s: &mut Surface,
    stations: &[Vec<Vec3>],
    layer_of: impl Fn(usize, usize) -> usize,
) -> Vec<Vec<usize>> {
    let idx: Vec<Vec<usize>> = stations
        .iter()
        .map(|st| st.iter().map(|&p| s.add_point(p)).collect())
        .collect();
    for i in 0..idx.len().saturating_sub(1) {
        assert_eq!(idx[i].len(), idx[i + 1].len(), "ragged station grid");
        for j in 0..idx[i].len() - 1 {
            s.add_face(
                vec![idx[i][j], idx[i][j + 1], idx[i + 1][j + 1], idx[i + 1][j]],
                layer_of(i, j),
            );
        }
    }
    idx
}

/// Newell normal of a control polygon (not normalised).
pub(crate) fn polygon_normal(s: &Surface, idx: &[usize]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for i in 0..idx.len() {
        let a = s.points[idx[i]].p;
        let b = s.points[idx[(i + 1) % idx.len()]].p;
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n
}

/// Add a face wound so its normal has a positive component along `outward`.
/// Consecutive duplicate points are dropped; fewer than three remaining adds
/// nothing. Returns the face's points in their final order.
pub(crate) fn add_oriented(
    s: &mut Surface,
    mut pts: Vec<usize>,
    layer: usize,
    outward: Vec3,
) -> Vec<usize> {
    pts.dedup();
    while pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 3 {
        return Vec::new();
    }
    if polygon_normal(s, &pts).dot(outward) < 0.0 {
        pts.reverse();
    }
    s.add_face(pts.clone(), layer);
    pts
}

/// Close the stern of a half model: one polygon from the aft station's
/// points (keel to sheer) to `apex`, its point on the centreline at the top,
/// facing aft. A planar transom needs every point (apex included) on one
/// plane. Returns the ring, to be creased after [`Surface::initialize_trace`].
pub(crate) fn close_half_transom(
    s: &mut Surface,
    layer: usize,
    stern: &[usize],
    apex: usize,
) -> Vec<usize> {
    let mut ring: Vec<usize> = stern.to_vec();
    ring.push(apex);
    add_oriented(s, ring, layer, Vec3::new(-1.0, 0.0, 0.0))
}

/// Close the top of a half model with a flat deck from the sheer to the
/// centreline at sheer height. Where the sheer already lies on the
/// centreline (a stem head, a pointed stern) the deck ends in a triangle.
/// Returns the deck's centreline points, one per sheer point.
pub(crate) fn close_half_deck(s: &mut Surface, layer: usize, sheer: &[usize]) -> Vec<usize> {
    let centre: Vec<usize> = sheer
        .iter()
        .map(|&i| {
            let p = s.points[i].p;
            if p.y.abs() < 1e-9 {
                i
            } else {
                s.add_point(Vec3::new(p.x, 0.0, p.z))
            }
        })
        .collect();
    for i in 0..sheer.len().saturating_sub(1) {
        add_oriented(
            s,
            vec![sheer[i], sheer[i + 1], centre[i + 1], centre[i]],
            layer,
            Vec3::new(0.0, 0.0, 1.0),
        );
    }
    centre
}

/// Crease the edges along `path` (consecutive points). Call after
/// [`Surface::initialize_trace`]; a pair that is not a control edge is
/// skipped.
pub(crate) fn crease_path(s: &mut Surface, path: &[usize]) {
    let index: HashMap<(usize, usize), usize> = s
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| (e.key(), i))
        .collect();
    for w in path.windows(2) {
        if let Some(&e) = index.get(&ordered(w[0], w[1])) {
            s.edges[e].crease = true;
        }
    }
}

/// Crease a closed ring of edges (a transom outline).
pub(crate) fn crease_loop(s: &mut Surface, ring: &[usize]) {
    let mut path = ring.to_vec();
    if let Some(&first) = ring.first() {
        path.push(first);
    }
    crease_path(s, &path);
}

/// Make every point where three or more crease edges meet a corner: the
/// chine or the sheer meeting the transom outline. Call after the creases.
///
/// The kernel's crease rule only moves a point with exactly two crease
/// edges; a control point with more keeps its own kind, and a `Regular`
/// one is averaged as a smooth point, pulled towards the centres of the
/// transom, side and bottom faces around it. On the runabout that rounded
/// the transom's chine corner off by 0.2 m and rolled the bottom and side
/// inward towards the transom. The kind is stored in the `.fef`, so
/// FREE!ship keeps the corner too.
pub(crate) fn pin_crease_junctions(s: &mut Surface) {
    let mut creases = vec![0usize; s.points.len()];
    for e in s.edges.iter().filter(|e| e.crease) {
        creases[e.a] += 1;
        creases[e.b] += 1;
    }
    for (p, &n) in s.points.iter_mut().zip(&creases) {
        if n >= 3 {
            p.kind = crate::mesh::VertexKind::Corner;
        }
    }
}

/// Round every coordinate to the five decimals a `.fef` stores, so a
/// written example reads back to exactly the same numbers.
pub(crate) fn round_to_fef(s: &mut Surface) {
    let r = |v: f64| -> f64 {
        let t = format!("{v:.5}").parse::<f64>().unwrap_or(v);
        if t == 0.0 {
            0.0 // no negative zero
        } else {
            t
        }
    };
    for p in &mut s.points {
        p.p = Vec3::new(r(p.p.x), r(p.p.y), r(p.p.z));
    }
}

/// Round to `decimals` places (sidecar numbers).
pub(crate) fn round(v: f64, decimals: i32) -> f64 {
    let k = 10f64.powi(decimals);
    (v * k).round() / k
}

/// The "Design" loading condition: the deadweight `items` plus a lightship
/// that makes the total `displacement_t` with its centre of gravity at
/// (`lcg`, 0, `kg`). The lightship is the balance, rounded to the kilogram
/// and the millimetre.
pub(crate) fn design_condition(
    displacement_t: f64,
    lcg: f64,
    kg: f64,
    lightship_name: &str,
    items: Vec<WeightItem>,
) -> LoadingCondition {
    let dw: f64 = items.iter().map(|w| w.mass_t).sum();
    let mx: f64 = items.iter().map(|w| w.mass_t * w.x).sum();
    let mz: f64 = items.iter().map(|w| w.mass_t * w.z).sum();
    let light = displacement_t - dw;
    assert!(light > 0.0, "deadweight exceeds the displacement");
    LoadingCondition {
        name: "Design".into(),
        lightship: WeightItem {
            name: lightship_name.into(),
            mass_t: round(light, 3),
            x: round((displacement_t * lcg - mx) / light, 3),
            y: 0.0,
            z: round((displacement_t * kg - mz) / light, 3),
        },
        items,
        tanks: Vec::new(),
        windage_extra: Vec::new(),
        bilge: Default::default(),
        downflooding_deg: None,
    }
}

/// A deadweight item on the centreline.
pub(crate) fn item(name: &str, mass_t: f64, x: f64, z: f64) -> WeightItem {
    WeightItem {
        name: name.into(),
        mass_t,
        x,
        y: 0.0,
        z,
    }
}

#[cfg(test)]
pub(crate) mod checks;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_round_trip_and_only_available_examples_are_listed() {
        for e in Example::VARIANTS {
            if e.is_available() {
                assert_eq!(Example::from_slug(e.slug()), Ok(e));
                assert!(build(e).is_ok(), "{e}");
            } else {
                let err = Example::from_slug(e.slug()).unwrap_err();
                assert!(err.contains("available:"), "{err}");
                assert!(build(e).unwrap_err().contains("not available"));
            }
            assert!(!e.title().is_empty() && !e.description().is_empty());
        }
        assert!(Example::all().iter().all(|e| !e.is_benchmark()));
        assert!(Example::with_benchmarks().contains(&Example::Wigley));
        assert!(!Example::all().contains(&Example::Wigley));
        assert!(Example::all().contains(&Example::Runabout6m));
        assert_eq!(Example::from_slug(" RUNABOUT-6M "), Ok(Example::Runabout6m));
        assert!(Example::from_slug("catamaran").is_err());
    }

    /// The committed `examples/<slug>.fef` and `.seacraft.json` are what
    /// `build` makes (`seacraft example --all -o examples/` regenerates them).
    #[test]
    fn committed_example_files_match_build() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        for e in Example::with_benchmarks() {
            let (surface, project) = build(e).unwrap();
            let fef_path = dir.join(format!("{}.fef", e.slug()));
            let json_path = dir.join(format!("{}.seacraft.json", e.slug()));
            let on_disk = std::fs::read_to_string(&fef_path)
                .unwrap_or_else(|err| panic!("{}: {err}", fef_path.display()));
            assert!(
                on_disk == crate::fef::write(&surface),
                "{} differs from build({e}); run `seacraft example --all -o examples/`",
                fef_path.display()
            );
            let on_disk = std::fs::read_to_string(&json_path)
                .unwrap_or_else(|err| panic!("{}: {err}", json_path.display()));
            assert!(
                on_disk == project.to_json(),
                "{} differs from build({e}); run `seacraft example --all -o examples/`",
                json_path.display()
            );
        }
    }

    #[test]
    fn the_design_condition_balances_to_the_requested_centre() {
        let c = design_condition(
            2.0,
            2.4,
            0.7,
            "Lightship",
            vec![item("Crew", 0.3, 3.0, 1.0), item("Fuel", 0.2, 1.0, 0.3)],
        );
        let all: Vec<&WeightItem> = std::iter::once(&c.lightship).chain(&c.items).collect();
        let m: f64 = all.iter().map(|w| w.mass_t).sum();
        let x = all.iter().map(|w| w.mass_t * w.x).sum::<f64>() / m;
        let z = all.iter().map(|w| w.mass_t * w.z).sum::<f64>() / m;
        assert!((m - 2.0).abs() < 1e-3 && (x - 2.4).abs() < 2e-3 && (z - 0.7).abs() < 2e-3);
    }

    /// Plating weight per layer, t: refined area (both sides, level 3)
    /// times thickness times density.
    fn plating_weights(s: &Surface) -> Vec<f64> {
        let m = crate::export::render_mesh(s, 3, crate::subdiv::SubdivisionMode::Loop);
        let mut area = vec![0.0; s.layers.len()];
        for (f, &l) in m.faces.iter().zip(&m.face_layer) {
            let poly: Vec<Vec3> = f.iter().map(|&i| m.verts[i]).collect();
            area[l] += crate::math::polygon_area3(&poly);
        }
        s.layers
            .iter()
            .zip(&area)
            .map(|(l, a)| a * l.thickness * l.density / 1000.0)
            .collect()
    }

    /// Every GUI example carries its own plating from the material table
    /// (only steel layers may keep the default density), and the plating
    /// weight is a plausible share of the lightship: more than a tenth, less
    /// than three quarters (the yacht's share includes its lead keel).
    #[test]
    fn examples_carry_realistic_plating() {
        let default = Layer::default();
        for e in Example::all() {
            let (s, p) = build(e).unwrap();
            for l in &s.layers {
                assert!(
                    [STEEL, GRP, TIMBER, OAK, LEAD].contains(&l.density),
                    "{e} {}: density {}",
                    l.name,
                    l.density
                );
                assert!(
                    l.density == STEEL || l.density != default.density,
                    "{e} {}: default plating",
                    l.name
                );
                assert!(
                    (0.004..=0.08).contains(&l.thickness),
                    "{e} {}: {} m",
                    l.name,
                    l.thickness
                );
            }
            let shell: f64 = plating_weights(&s).iter().sum();
            let light = p.loading_conditions[0].lightship.mass_t;
            let share = shell / light;
            println!(
                "{e}: plating {shell:.3} t of lightship {light:.3} t ({:.0} %)",
                100.0 * share
            );
            assert!((0.10..0.75).contains(&share), "{e}: plating share {share}");
        }
    }

    /// Layer density and thickness are only for the weight estimate: every
    /// example measures the same with the default plating.
    #[test]
    fn plating_does_not_change_the_hydrostatics() {
        let d = Layer::default();
        for e in Example::with_benchmarks() {
            let (s, _) = build(e).unwrap();
            let mut plain = s.clone();
            for l in &mut plain.layers {
                (l.density, l.thickness) = (d.density, d.thickness);
            }
            let a = serde_json::to_string(&checks::measure(&s)).unwrap();
            let b = serde_json::to_string(&checks::measure(&plain)).unwrap();
            assert_eq!(a, b, "{e}");
        }
    }
}
