//! IMO 2008 IS Code Part A §2.3 severe wind and rolling (weather) criterion
//! (Resolution MSC.267(85), Part A, Chapter 2, Section 2.3).
//!
//! All linear dimensions are in metres, areas in m², masses/displacements in
//! tonnes, pressures in Pascals, angles in degrees, and righting/heeling levers
//! in metres. Vertical centroids (`KG`, `z_centroid`, `underwater_z`) are
//! measured above the keel datum (`hull.keel`).

use crate::hydro::{
    clipped_loop_moments, plane_segments, waterplane_for, Hull, HydroOptions, Hydrostatics,
};
use crate::math::{deg_to_rad, Plane, Vec3};
use crate::mesh::Surface;
use crate::stability::{cross_curves, hydrostatics_free_trim, StabilityCondition};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Standard acceleration of gravity in m/s² (MSC.267(85) §2.3.1.2).
pub const GRAVITY_M_S2: f64 = 9.81;

/// Default steady wind pressure P in Pa for unrestricted service (MSC.267(85) §2.3.1.2).
pub const DEFAULT_WIND_PRESSURE_PA: f64 = 504.0;

/// Default maximum angle of heel under steady wind φ0 in degrees (MSC.267(85) §2.3.1.1).
pub const DEFAULT_PHI0_LIMIT_DEG: f64 = 16.0;

fn default_wind_pressure_pa() -> f64 {
    DEFAULT_WIND_PRESSURE_PA
}

/// An additional above-waterline lateral windage item (e.g. superstructure, deckhouse, mast).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindageItem {
    pub name: String,
    /// Lateral projected area above the waterline (m²).
    pub area_m2: f64,
    /// Vertical centroid above the keel (m).
    pub z_centroid: f64,
}

impl WindageItem {
    /// Parse a `name:area:z` specification string (used by the CLI).
    pub fn parse_spec(spec: &str) -> Result<Self, String> {
        let parts: Vec<&str> = spec.rsplitn(3, ':').collect();
        if parts.len() != 3 {
            return Err(format!(
                "windage item `{spec}` must have the form `name:area:z`"
            ));
        }
        let name = parts[2].trim();
        if name.is_empty() {
            return Err(format!("windage item `{spec}` has an empty name"));
        }
        let area_m2: f64 = parts[1]
            .trim()
            .parse()
            .map_err(|_| format!("windage `{spec}`: invalid area `{}`", parts[1]))?;
        let z_centroid: f64 = parts[0]
            .trim()
            .parse()
            .map_err(|_| format!("windage `{spec}`: invalid z centroid `{}`", parts[0]))?;
        if !area_m2.is_finite() || area_m2 < 0.0 {
            return Err(format!("windage `{name}`: area must be finite and >= 0"));
        }
        if !z_centroid.is_finite() {
            return Err(format!("windage `{name}`: z centroid must be finite"));
        }
        Ok(Self {
            name: name.to_string(),
            area_m2,
            z_centroid,
        })
    }
}

/// Bilge configuration for Table 2.3.4-3 factor `k`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BilgeKind {
    /// Round-bilged hull having no bilge or bar keels (`k = 1.0`).
    #[default]
    Round,
    /// Hull with sharp (hard-chine) bilges (`k = 0.7`).
    Sharp,
    /// Hull with bilge keels and/or a bar keel of total lateral area `area_m2` (`A_k`, m²).
    Keels { area_m2: f64 },
}

impl BilgeKind {
    /// Parse `"round"`, `"sharp"`, or `"keels:<Ak>"`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        let lower = t.to_ascii_lowercase();
        match lower.as_str() {
            "round" => Ok(Self::Round),
            "sharp" | "hard_chine" | "chine" => Ok(Self::Sharp),
            _ if lower.starts_with("keels:") => {
                let val_str = t[6..].trim();
                let area_m2: f64 = val_str
                    .parse()
                    .map_err(|_| format!("invalid bilge keel area `{val_str}` in `{text}`"))?;
                if !area_m2.is_finite() || area_m2 < 0.0 {
                    return Err(format!(
                        "bilge keel area must be finite and non-negative, got `{val_str}`"
                    ));
                }
                Ok(Self::Keels { area_m2 })
            }
            _ => Err(format!(
                "unknown bilge kind `{text}` (expected `round`, `sharp`, or `keels:<Ak>`)"
            )),
        }
    }
}

impl Serialize for BilgeKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self {
            BilgeKind::Round => serializer.serialize_str("round"),
            BilgeKind::Sharp => serializer.serialize_str("sharp"),
            BilgeKind::Keels { area_m2 } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("keels", &serde_json::json!({ "area_m2": area_m2 }))?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for BilgeKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(deserializer)?;
        match v {
            serde_json::Value::String(s) => BilgeKind::parse(&s).map_err(serde::de::Error::custom),
            serde_json::Value::Object(map) => {
                if let Some(inner) = map.get("keels").or_else(|| map.get("Keels")) {
                    let area = inner
                        .get("area_m2")
                        .and_then(|x| x.as_f64())
                        .or_else(|| inner.as_f64())
                        .ok_or_else(|| {
                            serde::de::Error::custom("`keels` must have a numeric `area_m2`")
                        })?;
                    if !area.is_finite() || area < 0.0 {
                        return Err(serde::de::Error::custom(
                            "`area_m2` must be finite and non-negative",
                        ));
                    }
                    Ok(BilgeKind::Keels { area_m2: area })
                } else if let Some(kind) = map.get("kind").and_then(|k| k.as_str()) {
                    match kind.to_ascii_lowercase().as_str() {
                        "round" => Ok(BilgeKind::Round),
                        "sharp" => Ok(BilgeKind::Sharp),
                        "keels" => {
                            let area =
                                map.get("area_m2").and_then(|x| x.as_f64()).ok_or_else(|| {
                                    serde::de::Error::custom("`keels` bilge requires `area_m2`")
                                })?;
                            Ok(BilgeKind::Keels { area_m2: area })
                        }
                        other => Err(serde::de::Error::custom(format!(
                            "unknown bilge kind `{other}`"
                        ))),
                    }
                } else {
                    Err(serde::de::Error::custom(
                        "expected `\"round\"`, `\"sharp\"`, `\"keels:<Ak>\"`, or `{\"keels\":{\"area_m2\":...}}`",
                    ))
                }
            }
            _ => Err(serde::de::Error::custom(
                "bilge must be a string or an object",
            )),
        }
    }
}

/// Configuration for the IMO severe wind and rolling (weather) criterion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherInput {
    /// Superstructure, deckhouse, or rig windage items added to the hull's own above-waterline profile.
    #[serde(default)]
    pub windage_extra: Vec<WindageItem>,
    /// Bilge type or bilge/bar keel area for factor `k`.
    #[serde(default)]
    pub bilge: BilgeKind,
    /// Optional angle of downflooding φf in degrees.
    #[serde(default)]
    pub downflooding_deg: Option<f64>,
    /// Steady wind pressure P in Pa (default 504 Pa for unrestricted service).
    #[serde(default = "default_wind_pressure_pa")]
    pub wind_pressure_pa: f64,
}

impl Default for WeatherInput {
    fn default() -> Self {
        Self {
            windage_extra: Vec::new(),
            bilge: BilgeKind::Round,
            downflooding_deg: None,
            wind_pressure_pa: DEFAULT_WIND_PRESSURE_PA,
        }
    }
}

/// Hydrostatic and mass inputs for [`evaluate_weather`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WeatherHydro {
    /// Displacement Δ in tonnes.
    pub displacement_t: f64,
    /// Waterline breadth B in metres.
    pub beam_m: f64,
    /// Mean moulded draught d in metres.
    pub draft_m: f64,
    /// Waterline length LWL in metres.
    pub lwl_m: f64,
    /// Block coefficient Cb.
    pub cb: f64,
    /// Vertical centre of gravity KG above the keel in metres (fluid KG when tanks are present).
    pub kg_m: f64,
    /// Initial transverse metacentric height GM in metres (fluid GM when tanks are present).
    pub gm_m: f64,
}

impl WeatherHydro {
    /// Build [`WeatherHydro`] from a [`Hydrostatics`] measurement and `kg_m` above the keel.
    pub fn from_hydrostatics(h: &Hydrostatics, kg_m: f64) -> Self {
        let beam_m = if h.bwl > 0.0 { h.bwl } else { h.submerged_beam };
        let lwl_m = if h.lwl > 0.0 {
            h.lwl
        } else {
            h.submerged_length
        };
        Self {
            displacement_t: h.displacement_t,
            beam_m,
            draft_m: h.draft,
            lwl_m,
            cb: h.block_coefficient,
            kg_m,
            gm_m: h.km_transverse - kg_m,
        }
    }
}

/// Complete IMO 2008 IS Code Part A §2.3 severe wind and rolling evaluation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherResult {
    /// Hull lateral windage area above the waterline (m²).
    pub hull_windage_area_m2: f64,
    /// Hull lateral windage vertical centroid above the keel (m).
    pub hull_windage_z_m: f64,
    /// Extra (superstructure) lateral windage area above the waterline (m²).
    pub extra_windage_area_m2: f64,
    /// Underwater lateral area (m²).
    pub underwater_area_m2: f64,
    /// Underwater lateral vertical centroid above the keel (m).
    pub underwater_z_m: f64,
    /// Total lateral windage area A above the waterline (m²).
    pub area_m2: f64,
    /// Combined above-waterline windage vertical centroid above the keel (m).
    pub windage_z_m: f64,
    /// Vertical distance Z from the centroid of A to the underwater lateral centroid (m).
    pub z_m: f64,
    /// Wind pressure P (Pa).
    pub wind_pressure_pa: f64,
    /// Steady wind heeling lever lw1 (m).
    pub lw1_m: f64,
    /// Gust wind heeling lever lw2 = 1.5 * lw1 (m).
    pub lw2_m: f64,
    /// Angle of heel under steady wind φ0 (deg), first intercept of lw1 with GZ.
    pub phi0_deg: Option<f64>,
    /// Maximum allowed steady-wind heel angle φ0 (deg, 16.0°).
    pub phi0_limit_deg: f64,
    /// Roll-back angle φ1 to windward (deg).
    pub phi1_deg: f64,
    /// Windward roll angle φ0 - φ1 (deg), lower integration limit of area a.
    pub phi_roll_deg: Option<f64>,
    /// First intercept φc of lw2 with GZ (deg).
    pub phic_deg: Option<f64>,
    /// Second intercept of lw2 with GZ (deg), if reached within the curve.
    pub phic2_deg: Option<f64>,
    /// Downflooding angle φf (deg), if given.
    pub downflooding_deg: Option<f64>,
    /// Upper integration limit φ2 = min(50°, φf, second intercept) (deg).
    pub phi2_deg: Option<f64>,
    /// Table 2.3.4-1 factor X1(B/d).
    pub x1: f64,
    /// Table 2.3.4-2 factor X2(Cb).
    pub x2: f64,
    /// Table 2.3.4-3 bilge/keel factor k.
    pub k: f64,
    /// OG = KG - d (m, positive when CG is above the waterline).
    pub og_m: f64,
    /// Factor r = 0.73 + 0.6 * OG / d.
    pub r: f64,
    /// Roll period coefficient C = 0.373 + 0.023*(B/d) - 0.043*(LWL/100).
    pub c: f64,
    /// Roll period T = 2 * C * B / sqrt(GM) (s).
    pub roll_period_s: f64,
    /// Table 2.3.4-4 wave steepness factor s(T).
    pub s: f64,
    /// Heeling energy area a between lw2 and GZ on [φ0 - φ1, φc] (m·rad).
    pub area_a_m_rad: f64,
    /// Righting energy area b between GZ and lw2 on [φc, φ2] (m·rad).
    pub area_b_m_rad: f64,
    /// Energy ratio b / a (infinity when a == 0 and b > 0, 0 when b == 0).
    pub area_ratio: f64,
    /// Whether φ0 <= 16° is satisfied.
    pub phi0_passed: bool,
    /// Whether area b >= area a (with valid φc and φ2) is satisfied.
    pub area_passed: bool,
    /// Whether the complete weather criterion is satisfied (`phi0_passed && area_passed`).
    pub passed: bool,
    /// Explanatory notes when φ0, φc, or φ2 cannot be found or inputs are degenerate.
    pub notes: Vec<String>,
}

const TABLE_X1: [(f64, f64); 11] = [
    (2.4, 1.00),
    (2.5, 0.98),
    (2.6, 0.96),
    (2.7, 0.95),
    (2.8, 0.93),
    (2.9, 0.91),
    (3.0, 0.90),
    (3.1, 0.88),
    (3.2, 0.86),
    (3.4, 0.82),
    (3.5, 0.80),
];

const TABLE_X2: [(f64, f64); 6] = [
    (0.45, 0.75),
    (0.50, 0.82),
    (0.55, 0.89),
    (0.60, 0.95),
    (0.65, 0.97),
    (0.70, 1.00),
];

const TABLE_K: [(f64, f64); 8] = [
    (0.0, 1.00),
    (1.0, 0.98),
    (1.5, 0.95),
    (2.0, 0.88),
    (2.5, 0.79),
    (3.0, 0.74),
    (3.5, 0.72),
    (4.0, 0.70),
];

const TABLE_S: [(f64, f64); 8] = [
    (6.0, 0.100),
    (7.0, 0.098),
    (8.0, 0.093),
    (12.0, 0.065),
    (14.0, 0.053),
    (16.0, 0.044),
    (18.0, 0.038),
    (20.0, 0.035),
];

fn interp_table(table: &[(f64, f64)], x: f64) -> f64 {
    if !x.is_finite() {
        return table[0].1;
    }
    if x <= table[0].0 {
        return table[0].1;
    }
    for w in table.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            let t = (x - x0) / (x1 - x0);
            return y0 + t * (y1 - y0);
        }
    }
    table[table.len() - 1].1
}

/// Table 2.3.4-1 factor `X1` as a function of `B / d`.
pub fn x1_factor(b_over_d: f64) -> f64 {
    interp_table(&TABLE_X1, b_over_d)
}

/// Table 2.3.4-2 factor `X2` as a function of block coefficient `Cb`.
pub fn x2_factor(cb: f64) -> f64 {
    interp_table(&TABLE_X2, cb)
}

/// Table 2.3.4-3 factor `k` for round bilge (`1.0`), sharp bilge (`0.7`), or
/// bilge/bar keels interpolated on `Ak * 100 / (LWL * B)`.
pub fn k_factor(bilge: BilgeKind, lwl_m: f64, beam_m: f64) -> f64 {
    match bilge {
        BilgeKind::Round => 1.0,
        BilgeKind::Sharp => 0.7,
        BilgeKind::Keels { area_m2 } => {
            let denom = lwl_m * beam_m;
            if denom <= 1e-12 || area_m2 <= 0.0 {
                1.0
            } else {
                let ratio = area_m2.max(0.0) * 100.0 / denom;
                interp_table(&TABLE_K, ratio)
            }
        }
    }
}

/// Table 2.3.4-4 wave steepness factor `s` as a function of roll period `T` (s).
pub fn s_factor(roll_period_s: f64) -> f64 {
    interp_table(&TABLE_S, roll_period_s)
}

/// Roll period coefficient `C` and roll period `T` (s) from MSC.267(85) §2.3.4:
/// `C = 0.373 + 0.023 * (B / d) - 0.043 * (LWL / 100)`, `T = 2 * C * B / sqrt(GM)`.
pub fn roll_period(beam_m: f64, draft_m: f64, lwl_m: f64, gm_m: f64) -> (f64, f64) {
    let b_over_d = if draft_m > 1e-12 {
        beam_m / draft_m
    } else {
        0.0
    };
    let c = (0.373 + 0.023 * b_over_d - 0.043 * (lwl_m / 100.0)).max(0.0);
    let t = if gm_m > 1e-12 {
        2.0 * c * beam_m / gm_m.sqrt()
    } else {
        0.0
    };
    (c, t)
}

/// Roll-back angle `φ1 = 109 * k * X1 * X2 * sqrt(r * s)` in degrees.
pub fn rollback_angle_deg(k: f64, x1: f64, x2: f64, r: f64, s: f64) -> f64 {
    109.0 * k * x1 * x2 * (r.max(0.0) * s.max(0.0)).sqrt()
}

/// Maximum `z` of the refined hull mesh at longitudinal coordinate `x`.
fn max_z_at_x(hull: &Hull, x: f64) -> Option<f64> {
    let mut z_max = f64::NEG_INFINITY;
    for t in &hull.tris {
        let tri = [hull.verts[t[0]], hull.verts[t[1]], hull.verts[t[2]]];
        for i in 0..3 {
            let a = tri[i];
            let b = tri[(i + 1) % 3];
            if (a.x - x).abs() <= 1e-7 {
                z_max = z_max.max(a.z);
            }
            if (a.x < x && b.x > x) || (b.x < x && a.x > x) {
                let u = (x - a.x) / (b.x - a.x);
                let z = a.z + u * (b.z - a.z);
                z_max = z_max.max(z);
            }
        }
    }
    z_max.is_finite().then_some(z_max)
}

/// Chain coplanar segments into polylines `(points, closed)` without dropping
/// the final corner of a closed ring.
fn chain_profile_paths(segs: &[(Vec3, Vec3)], tol: f64) -> Vec<(Vec<Vec3>, bool)> {
    let mut points: Vec<Vec3> = Vec::new();
    let mut find_or_insert = |p: Vec3| -> usize {
        for (i, q) in points.iter().enumerate() {
            if (*q - p).length() <= tol {
                return i;
            }
        }
        let idx = points.len();
        points.push(p);
        idx
    };

    let mut seen = std::collections::HashSet::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for (a, b) in segs {
        let ia = find_or_insert(*a);
        let ib = find_or_insert(*b);
        if ia != ib {
            let key = if ia < ib { (ia, ib) } else { (ib, ia) };
            if seen.insert(key) {
                edges.push((ia, ib));
            }
        }
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for (i, (a, b)) in edges.iter().enumerate() {
        adj[*a].push(i);
        adj[*b].push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut paths: Vec<(Vec<Vec3>, bool)> = Vec::new();

    let walk = |mut cur: usize,
                target_vertex: usize,
                used: &mut Vec<bool>,
                adj: &[Vec<usize>],
                edges: &[(usize, usize)]|
     -> (Vec<usize>, bool) {
        let mut seq = vec![cur];
        let mut closed = false;
        loop {
            let next = adj[cur].iter().copied().find(|&e| !used[e]);
            match next {
                Some(e) => {
                    used[e] = true;
                    let (x, y) = edges[e];
                    cur = if x == cur { y } else { x };
                    if cur == target_vertex {
                        closed = true;
                        break;
                    }
                    seq.push(cur);
                }
                None => break,
            }
            if seq.len() > edges.len() + 2 {
                break;
            }
        }
        (seq, closed)
    };

    for start_edge in 0..edges.len() {
        if used[start_edge] {
            continue;
        }
        used[start_edge] = true;
        let (a, b) = edges[start_edge];
        let (fwd, closed) = walk(b, a, &mut used, &adj, &edges);
        let mut poly: Vec<Vec3> = Vec::with_capacity(fwd.len() + 1);
        poly.push(points[a]);
        for i in &fwd {
            poly.push(points[*i]);
        }
        if !closed {
            let (mut back, _) = walk(a, b, &mut used, &adj, &edges);
            back.remove(0);
            back.reverse();
            let mut full: Vec<Vec3> = back.iter().map(|i| points[*i]).collect();
            full.extend(poly);
            poly = full;
        }
        if poly.len() >= 2 {
            paths.push((poly, closed));
        }
    }
    paths
}

/// Build the closed lateral (`y = 0` profile) polygon loops of `hull`.
///
/// For a closed-deck hull (e.g. a box barge with a deck), the `y = 0` cut
/// produces a closed loop directly. For an open-deck hull (whose `y = 0` cut is
/// an open polyline along the transom, keel and stem), the open polyline is
/// closed along the hull's upper sheer profile projected onto `y = 0`, so the
/// above-waterline area covers the hull side up to the sheer while the
/// underwater clipped area matches [`Hydrostatics::lateral_area_m2`].
fn profile_loops(hull: &Hull) -> (Plane, Vec<Vec<Vec3>>) {
    let centre = Plane::new(Vec3::new(0.0, 1.0, 0.0), 0.0);
    let lsegs = plane_segments(&hull.verts, &hull.tris, &centre);
    let lpaths = chain_profile_paths(&lsegs, 1e-7);

    let mut loops = Vec::with_capacity(lpaths.len().max(1));
    let x_lo = hull.model_min.x;
    let x_hi = hull.model_max.x;
    let span = (x_hi - x_lo).max(1e-6);

    for (mut pts, closed) in lpaths {
        if pts.len() < 2 {
            continue;
        }
        if !closed {
            let first = pts[0];
            let last = pts[pts.len() - 1];
            const STEPS: usize = 48;
            for k in 0..=STEPS {
                let t = k as f64 / STEPS as f64;
                let x_raw = last.x + t * (first.x - last.x);
                let x = x_raw.clamp(x_lo + 1e-6 * span, x_hi - 1e-6 * span);
                if let Some(z_sheer) = max_z_at_x(hull, x) {
                    let z_end = last.z + t * (first.z - last.z);
                    pts.push(Vec3::new(x_raw, 0.0, z_sheer.max(z_end)));
                }
            }
            pts.push(first);
        }
        loops.push(pts);
    }

    (centre, loops)
}

/// Compute the hull's lateral (`y = 0` / profile) projected windage area and
/// centroid above the waterline, and the underwater lateral area and centroid
/// below the waterline:
/// `(area_m2, z_centroid, underwater_area_m2, underwater_z)`.
///
/// Both `z_centroid` and `underwater_z` are measured above the keel
/// (`hull.keel`), matching `draft`, `KB`, `KM`, and `KG`.
pub fn hull_windage_on_hull(hull: &Hull, draft: f64, trim: f64) -> (f64, f64, f64, f64) {
    let (centre, loops) = profile_loops(hull);
    let wp = waterplane_for(hull.keel, hull.length, draft, trim, 0.0);
    let above_wp = Plane { n: -wp.n, d: -wp.d };
    let (u, v) = centre.basis();

    let (underwater_area_m2, underwater_z) =
        if let Some(m) = clipped_loop_moments(&loops, &centre, &wp) {
            let area = m.area.abs();
            let cx = m.mx / m.area;
            let cy = m.my / m.area;
            let c = u * cx + v * cy;
            (area, c.z - hull.keel)
        } else {
            (0.0, 0.5 * draft.max(0.0))
        };

    let (area_m2, z_centroid) = if let Some(m) = clipped_loop_moments(&loops, &centre, &above_wp) {
        let area = m.area.abs();
        let cx = m.mx / m.area;
        let cy = m.my / m.area;
        let c = u * cx + v * cy;
        (area, c.z - hull.keel)
    } else {
        (0.0, draft.max(0.0))
    };

    (area_m2, z_centroid, underwater_area_m2, underwater_z)
}

/// Measure the hull's above-waterline windage and underwater lateral profile
/// `(area_m2, z_centroid, underwater_area_m2, underwater_z)` on the refined
/// mesh specified by `opts.subdivision`.
pub fn hull_windage(
    surface: &Surface,
    opts: &HydroOptions,
    draft: f64,
    trim: f64,
) -> (f64, f64, f64, f64) {
    let hull = Hull::new(surface, opts.subdivision);
    hull_windage_on_hull(&hull, draft, trim)
}

/// Sample the GZ curve at 2° heel steps from −30° (mirrored down to −60° for a
/// symmetric hull with `GZ(-φ) = -GZ(φ)`) up to +60° at a single displacement.
pub fn sample_weather_gz(
    hull: &Hull,
    displacement_t: f64,
    cond: &StabilityCondition,
    base: &HydroOptions,
) -> Vec<(f64, f64)> {
    let pos_heels: Vec<f64> = (0..=30).map(|i| (i * 2) as f64).collect();
    let curves = cross_curves(hull, &pos_heels, &[displacement_t], cond, base);
    let mut pos: Vec<(f64, f64)> = Vec::with_capacity(curves.len());
    for c in &curves {
        if let Some(gz) = c.points.first().and_then(|p| p.gz) {
            if gz.is_finite() && c.heel_deg.is_finite() {
                pos.push((c.heel_deg, gz));
            }
        }
    }
    let mut full: Vec<(f64, f64)> = Vec::with_capacity(pos.len() * 2);
    for &(h, gz) in pos.iter().rev() {
        if h > 0.0 && h <= 30.0 {
            full.push((-h, -gz));
        }
    }
    full.extend(pos);
    full
}

/// Linearly interpolate `GZ` at `heel_deg` on a sorted `(heel_deg, gz)` table.
fn gz_interp(pts: &[(f64, f64)], heel_deg: f64) -> f64 {
    let Some(&(h0, g0)) = pts.first() else {
        return 0.0;
    };
    if heel_deg <= h0 {
        return g0;
    }
    for w in pts.windows(2) {
        let ((ha, ga), (hb, gb)) = (w[0], w[1]);
        if heel_deg <= hb {
            if (hb - ha).abs() <= 1e-12 {
                return gb;
            }
            let t = (heel_deg - ha) / (hb - ha);
            return ga + t * (gb - ga);
        }
    }
    pts.last().map(|&(_, g)| g).unwrap_or(0.0)
}

/// First upward crossing where `GZ(φ)` rises from `<= level` to `>= level` at `φ >= min_heel`.
fn find_upward_intercept(pts: &[(f64, f64)], level: f64, min_heel: f64) -> Option<f64> {
    if pts.len() < 2 {
        return None;
    }
    for w in pts.windows(2) {
        let ((mut ha, mut ga), (hb, gb)) = (w[0], w[1]);
        if hb < min_heel - 1e-12 {
            continue;
        }
        if ha < min_heel {
            ga = gz_interp(pts, min_heel);
            ha = min_heel;
        }
        if ga <= level + 1e-12 && gb >= level - 1e-12 && gb > ga + 1e-15 {
            let t = ((level - ga) / (gb - ga)).clamp(0.0, 1.0);
            return Some(ha + t * (hb - ha));
        }
    }
    None
}

/// First downward crossing where `GZ(φ)` falls from `>= level` to `<= level` strictly after `after_heel`.
fn find_downward_intercept(pts: &[(f64, f64)], level: f64, after_heel: f64) -> Option<f64> {
    if pts.len() < 2 {
        return None;
    }
    for w in pts.windows(2) {
        let ((mut ha, mut ga), (hb, gb)) = (w[0], w[1]);
        if hb <= after_heel + 1e-9 {
            continue;
        }
        if ha < after_heel {
            ga = gz_interp(pts, after_heel);
            ha = after_heel;
        }
        if ga >= level - 1e-12 && gb <= level + 1e-12 && ga > gb + 1e-15 {
            let t = ((ga - level) / (ga - gb)).clamp(0.0, 1.0);
            let h = ha + t * (hb - ha);
            if h > after_heel + 1e-7 {
                return Some(h);
            }
        }
    }
    None
}

/// Trapezoidal integral of `max(0, f(φ))` on `[a_deg, b_deg]` in `m·rad`.
fn integrate_positive<F: Fn(f64) -> f64>(pts: &[(f64, f64)], a_deg: f64, b_deg: f64, f: F) -> f64 {
    if pts.len() < 2 || !a_deg.is_finite() || !b_deg.is_finite() || b_deg <= a_deg {
        return 0.0;
    }
    let mut xs: Vec<f64> = Vec::with_capacity(pts.len() + 2);
    xs.push(a_deg);
    for &(h, _) in pts {
        if h > a_deg + 1e-9 && h < b_deg - 1e-9 {
            xs.push(h);
        }
    }
    xs.push(b_deg);

    let mut area_deg = 0.0;
    for w in xs.windows(2) {
        let (h0, h1) = (w[0], w[1]);
        let y0 = f(h0).max(0.0);
        let y1 = f(h1).max(0.0);
        area_deg += 0.5 * (y0 + y1) * (h1 - h0);
    }
    deg_to_rad(area_deg)
}

/// Evaluate the IMO 2008 IS Code Part A §2.3 severe wind and rolling (weather)
/// criterion on a tabulated `curve_gz` (`&[(heel_deg, gz)]`).
///
/// If `curve_gz` contains only non-negative heels (`φ >= 0°`), negative heels
/// are mirrored automatically via `GZ(-φ) = -GZ(φ)` so the windward roll
/// excursion down to `φ0 - φ1` is integrated without truncation.
pub fn evaluate_weather(
    curve_gz: &[(f64, f64)],
    hydro: &WeatherHydro,
    windage: (f64, f64, f64, f64),
    input: &WeatherInput,
) -> WeatherResult {
    let mut notes = Vec::new();
    let (hull_a, hull_z, uw_a, uw_z_raw) = windage;
    let hull_area = hull_a.max(0.0);

    let mut extra_area = 0.0;
    let mut extra_mz = 0.0;
    for item in &input.windage_extra {
        if item.area_m2.is_finite() && item.area_m2 > 0.0 && item.z_centroid.is_finite() {
            extra_area += item.area_m2;
            extra_mz += item.area_m2 * item.z_centroid;
        }
    }

    let area_m2 = hull_area + extra_area;
    let windage_z_m = if area_m2 > 1e-12 {
        (hull_area * hull_z + extra_mz) / area_m2
    } else {
        hydro.draft_m.max(0.0)
    };

    let underwater_z_m = if uw_z_raw.is_finite() && (uw_a > 1e-9 || uw_z_raw > 0.0) {
        uw_z_raw
    } else {
        0.5 * hydro.draft_m.max(0.0)
    };

    let z_m = (windage_z_m - underwater_z_m).max(0.0);
    let wind_pressure_pa = if input.wind_pressure_pa.is_finite() && input.wind_pressure_pa >= 0.0 {
        input.wind_pressure_pa
    } else {
        DEFAULT_WIND_PRESSURE_PA
    };

    let lw1_m = if hydro.displacement_t > 1e-12 {
        wind_pressure_pa * area_m2 * z_m / (1000.0 * GRAVITY_M_S2 * hydro.displacement_t)
    } else {
        notes.push("displacement is zero or negative; lw1 set to 0".into());
        0.0
    };
    let lw2_m = 1.5 * lw1_m;

    let b_over_d = if hydro.draft_m > 1e-12 {
        hydro.beam_m / hydro.draft_m
    } else {
        0.0
    };
    let x1 = x1_factor(b_over_d);
    let x2 = x2_factor(hydro.cb);
    let k = k_factor(input.bilge, hydro.lwl_m, hydro.beam_m);
    let og_m = hydro.kg_m - hydro.draft_m;
    let r = if hydro.draft_m > 1e-12 {
        0.73 + 0.6 * (og_m / hydro.draft_m)
    } else {
        0.73
    };
    let (c, roll_period_s) = roll_period(hydro.beam_m, hydro.draft_m, hydro.lwl_m, hydro.gm_m);
    if hydro.gm_m <= 0.0 {
        notes.push("GM <= 0: roll period T cannot be evaluated from positive GM".into());
    }
    let s = s_factor(roll_period_s);
    let phi1_deg = rollback_angle_deg(k, x1, x2, r, s);

    // Prepare sorted GZ curve, mirroring negative heels if only >= 0° was supplied.
    let mut raw: Vec<(f64, f64)> = curve_gz
        .iter()
        .copied()
        .filter(|(h, g)| h.is_finite() && g.is_finite())
        .collect();
    raw.sort_by(|a, b| a.0.total_cmp(&b.0));

    let has_negative = raw.first().is_some_and(|(h, _)| *h < -1e-9);
    let mut pts = Vec::with_capacity(raw.len() * 2);
    if !has_negative {
        for &(h, g) in raw.iter().rev() {
            if h > 1e-9 {
                pts.push((-h, -g));
            }
        }
    }
    pts.extend(raw);

    let phi0_limit_deg = DEFAULT_PHI0_LIMIT_DEG;
    let phi0_deg = if pts.len() < 2 {
        notes.push("insufficient GZ samples to evaluate weather criterion".into());
        None
    } else {
        let p0 = find_upward_intercept(&pts, lw1_m, 0.0);
        if p0.is_none() {
            notes.push(format!(
                "steady wind lever lw1 ({lw1_m:.4} m) exceeds maximum GZ; phi0 not found"
            ));
        }
        p0
    };

    let phi_roll_deg = phi0_deg.map(|p0| p0 - phi1_deg);

    // If the windward roll excursion extends past the lowest tabulated negative
    // heel, mirror further positive samples so the curve covers `phi_roll_deg`.
    if let Some(p_roll) = phi_roll_deg {
        if let Some(&(min_h, _)) = pts.first() {
            if p_roll < min_h {
                let mut extra_neg: Vec<(f64, f64)> = pts
                    .iter()
                    .copied()
                    .filter(|&(h, _)| h > -min_h)
                    .map(|(h, g)| (-h, -g))
                    .collect();
                extra_neg.sort_by(|a, b| a.0.total_cmp(&b.0));
                extra_neg.extend(pts);
                pts = extra_neg;
            }
        }
    }

    let phic_deg = match phi0_deg {
        Some(p0) => {
            let search_from = phi_roll_deg.unwrap_or(0.0).min(p0);
            let pc = find_upward_intercept(&pts, lw2_m, search_from);
            if pc.is_none() {
                notes.push(format!(
                    "gust lever lw2 ({lw2_m:.4} m) exceeds maximum GZ; first intercept phi_c not found"
                ));
            }
            pc
        }
        None => None,
    };

    let phic2_deg = phic_deg.and_then(|pc| find_downward_intercept(&pts, lw2_m, pc));

    let phi2_deg = phic_deg.and_then(|pc| {
        let mut limit = 50.0_f64;
        if let Some(df) = input.downflooding_deg.filter(|d| d.is_finite() && *d > 0.0) {
            limit = limit.min(df);
        }
        if let Some(pc2) = phic2_deg {
            limit = limit.min(pc2);
        }
        let max_curve_heel = pts.last().map(|&(h, _)| h).unwrap_or(0.0);
        if max_curve_heel < limit && phic2_deg.is_none() {
            notes.push(format!(
                "GZ curve ends at {max_curve_heel:.1} deg before reaching {limit:.1} deg or second lw2 intercept; clamping phi2"
            ));
            limit = limit.min(max_curve_heel);
        }
        if limit > pc {
            Some(limit)
        } else {
            notes.push(format!(
                "upper limit phi2 ({limit:.2} deg) is not above phi_c ({pc:.2} deg)"
            ));
            None
        }
    });

    let area_a_m_rad = match (phi_roll_deg, phic_deg) {
        (Some(p_start), Some(pc)) if pc > p_start => {
            integrate_positive(&pts, p_start, pc, |h| lw2_m - gz_interp(&pts, h))
        }
        _ => 0.0,
    };

    let area_b_m_rad = match (phic_deg, phi2_deg) {
        (Some(pc), Some(p2)) if p2 > pc => {
            integrate_positive(&pts, pc, p2, |h| gz_interp(&pts, h) - lw2_m)
        }
        _ => 0.0,
    };

    let area_ratio = if area_a_m_rad > 1e-12 {
        area_b_m_rad / area_a_m_rad
    } else if area_b_m_rad > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };

    let phi0_passed = phi0_deg.is_some_and(|p0| p0 <= phi0_limit_deg);
    let area_passed = phic_deg.is_some()
        && phi2_deg.is_some()
        && area_b_m_rad >= area_a_m_rad
        && area_b_m_rad > 0.0;
    let passed = phi0_passed && area_passed;

    WeatherResult {
        hull_windage_area_m2: hull_area,
        hull_windage_z_m: hull_z,
        extra_windage_area_m2: extra_area,
        underwater_area_m2: uw_a.max(0.0),
        underwater_z_m,
        area_m2,
        windage_z_m,
        z_m,
        wind_pressure_pa,
        lw1_m,
        lw2_m,
        phi0_deg,
        phi0_limit_deg,
        phi1_deg,
        phi_roll_deg,
        phic_deg,
        phic2_deg,
        downflooding_deg: input.downflooding_deg,
        phi2_deg,
        x1,
        x2,
        k,
        og_m,
        r,
        c,
        roll_period_s,
        s,
        area_a_m_rad,
        area_b_m_rad,
        area_ratio,
        phi0_passed,
        area_passed,
        passed,
        notes,
    }
}

/// Evaluate the IMO weather criterion on a refined [`Hull`] at the floating
/// condition defined by `opts` and `cond` (sampling GZ at 2° steps from −30° to 60°).
pub fn evaluate_hull_weather(
    hull: &Hull,
    opts: &HydroOptions,
    cond: &StabilityCondition,
    input: &WeatherInput,
) -> WeatherResult {
    let (h, fc) = if cond.is_free() {
        hydrostatics_free_trim(hull, opts, cond.lcg_x)
    } else {
        (
            hull.hydrostatics(opts),
            crate::stability::FloatingCondition {
                draft: opts.draft,
                trim: opts.trim,
            },
        )
    };
    let mut solved_opts = opts.clone();
    solved_opts.draft = fc.draft;
    solved_opts.trim = fc.trim;
    let mut solved_cond = *cond;
    solved_cond.trim = fc.trim;

    let wh = WeatherHydro::from_hydrostatics(&h, cond.kg);
    let windage = hull_windage_on_hull(hull, fc.draft, fc.trim);
    let curve_gz = sample_weather_gz(hull, h.displacement_t, &solved_cond, &solved_opts);
    evaluate_weather(&curve_gz, &wh, windage, input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subdiv::{SubdivisionMode, SubdivisionOptions};

    #[test]
    fn table_interpolation_at_and_between_entries() {
        // X1(B/d): Table 2.3.4-1
        assert!((x1_factor(2.0) - 1.00).abs() < 1e-12);
        assert!((x1_factor(2.4) - 1.00).abs() < 1e-12);
        assert!((x1_factor(2.45) - 0.99).abs() < 1e-12);
        assert!((x1_factor(2.5) - 0.98).abs() < 1e-12);
        assert!((x1_factor(2.75) - 0.94).abs() < 1e-12);
        assert!((x1_factor(3.0) - 0.90).abs() < 1e-12);
        // 3.3 is halfway between 3.2 (0.86) and 3.4 (0.82) -> 0.84
        assert!((x1_factor(3.3) - 0.84).abs() < 1e-12);
        assert!((x1_factor(3.5) - 0.80).abs() < 1e-12);
        assert!((x1_factor(4.2) - 0.80).abs() < 1e-12);

        // X2(Cb): Table 2.3.4-2
        assert!((x2_factor(0.40) - 0.75).abs() < 1e-12);
        assert!((x2_factor(0.45) - 0.75).abs() < 1e-12);
        assert!((x2_factor(0.475) - 0.785).abs() < 1e-12);
        assert!((x2_factor(0.50) - 0.82).abs() < 1e-12);
        assert!((x2_factor(0.575) - 0.92).abs() < 1e-12);
        assert!((x2_factor(0.625) - 0.96).abs() < 1e-12);
        assert!((x2_factor(0.70) - 1.00).abs() < 1e-12);
        assert!((x2_factor(0.85) - 1.00).abs() < 1e-12);

        // k: Table 2.3.4-3
        assert!((k_factor(BilgeKind::Round, 20.0, 5.0) - 1.0).abs() < 1e-12);
        assert!((k_factor(BilgeKind::Sharp, 20.0, 5.0) - 0.7).abs() < 1e-12);
        // LWL * B = 100 m2 -> Ak * 100 / (LWL * B) == Ak
        assert!((k_factor(BilgeKind::Keels { area_m2: 0.0 }, 20.0, 5.0) - 1.00).abs() < 1e-12);
        assert!((k_factor(BilgeKind::Keels { area_m2: 0.5 }, 20.0, 5.0) - 0.99).abs() < 1e-12);
        assert!((k_factor(BilgeKind::Keels { area_m2: 1.0 }, 20.0, 5.0) - 0.98).abs() < 1e-12);
        assert!((k_factor(BilgeKind::Keels { area_m2: 1.75 }, 20.0, 5.0) - 0.915).abs() < 1e-12);
        assert!((k_factor(BilgeKind::Keels { area_m2: 2.25 }, 20.0, 5.0) - 0.835).abs() < 1e-12);
        assert!((k_factor(BilgeKind::Keels { area_m2: 4.0 }, 20.0, 5.0) - 0.70).abs() < 1e-12);
        assert!((k_factor(BilgeKind::Keels { area_m2: 6.0 }, 20.0, 5.0) - 0.70).abs() < 1e-12);

        // s(T): Table 2.3.4-4
        assert!((s_factor(4.0) - 0.100).abs() < 1e-12);
        assert!((s_factor(6.0) - 0.100).abs() < 1e-12);
        assert!((s_factor(6.5) - 0.099).abs() < 1e-12);
        assert!((s_factor(7.5) - 0.0955).abs() < 1e-12);
        assert!((s_factor(10.0) - 0.079).abs() < 1e-12);
        assert!((s_factor(13.0) - 0.059).abs() < 1e-12);
        assert!((s_factor(15.0) - 0.0485).abs() < 1e-12);
        assert!((s_factor(17.0) - 0.041).abs() < 1e-12);
        assert!((s_factor(19.0) - 0.0365).abs() < 1e-12);
        assert!((s_factor(20.0) - 0.035).abs() < 1e-12);
        assert!((s_factor(25.0) - 0.035).abs() < 1e-12);
    }

    #[test]
    fn roll_period_and_phi1_hand_computed() {
        // Hand-computed case:
        // B = 10.0 m, d = 4.0 m -> B/d = 2.5 -> X1 = 0.98
        // LWL = 50.0 m
        // Cb = 0.55 -> X2 = 0.89
        // KG = 4.4 m -> OG = 0.4 m -> r = 0.73 + 0.6 * (0.4 / 4.0) = 0.79
        // GM = 1.00 m
        // C = 0.373 + 0.023 * 2.5 - 0.043 * 0.50 = 0.373 + 0.0575 - 0.0215 = 0.409
        // T = 2 * 0.409 * 10.0 / 1.0 = 8.18 s
        // s(8.18): linear between 8.0 (0.093) and 12.0 (0.065):
        //   0.093 + (0.18 / 4.0) * (0.065 - 0.093) = 0.09174
        // k = 1.0 (round bilge)
        // phi1 = 109 * 1.0 * 0.98 * 0.89 * sqrt(0.79 * 0.09174)
        let (c, t) = roll_period(10.0, 4.0, 50.0, 1.0);
        assert!((c - 0.409).abs() < 1e-12, "C = {c}");
        assert!((t - 8.18).abs() < 1e-12, "T = {t}");

        let x1 = x1_factor(10.0 / 4.0);
        let x2 = x2_factor(0.55);
        let k = k_factor(BilgeKind::Round, 50.0, 10.0);
        let r: f64 = 0.73 + 0.6 * ((4.4 - 4.0) / 4.0);
        let s = s_factor(t);
        assert!((x1 - 0.98).abs() < 1e-12);
        assert!((x2 - 0.89).abs() < 1e-12);
        assert!((r - 0.79).abs() < 1e-12);
        assert!((s - 0.09174).abs() < 1e-12);

        let want_phi1 = 109.0 * 1.0 * 0.98 * 0.89 * (0.79_f64 * 0.09174).sqrt();
        let got_phi1 = rollback_angle_deg(k, x1, x2, r, s);
        assert!(
            (got_phi1 - want_phi1).abs() < 1e-12,
            "{got_phi1} vs {want_phi1}"
        );
    }

    #[test]
    fn box_barge_windage_and_lw1_match_analytic() {
        // Closed box barge: L = 10 m, B = 4 m, depth D = 3 m, draft d = 1.2 m.
        // Underwater lateral profile: 10 m x 1.2 m = 12.0 m2, centroid z = 0.6 m.
        // Above-waterline windage profile: 10 m x 1.8 m = 18.0 m2, centroid z = 2.1 m.
        // Lever Z = 2.1 - 0.6 = 1.5 m (= D / 2).
        let mut s = Surface::new();
        s.layers.push(crate::mesh::Layer::default());
        let l = 10.0;
        let hb = 2.0;
        let d = 3.0;
        let p000 = s.add_point(Vec3::new(0.0, -hb, 0.0));
        let p010 = s.add_point(Vec3::new(0.0, hb, 0.0));
        let p110 = s.add_point(Vec3::new(l, hb, 0.0));
        let p100 = s.add_point(Vec3::new(l, -hb, 0.0));
        let p001 = s.add_point(Vec3::new(0.0, -hb, d));
        let p011 = s.add_point(Vec3::new(0.0, hb, d));
        let p111 = s.add_point(Vec3::new(l, hb, d));
        let p101 = s.add_point(Vec3::new(l, -hb, d));
        s.add_face(vec![p000, p100, p110, p010], 0);
        s.add_face(vec![p001, p011, p111, p101], 0);
        s.add_face(vec![p010, p110, p111, p011], 0);
        s.add_face(vec![p100, p000, p001, p101], 0);
        s.add_face(vec![p000, p010, p011, p001], 0);
        s.add_face(vec![p110, p100, p101, p111], 0);
        s.initialize_trace();
        for e in &mut s.edges {
            e.crease = true;
        }

        let mut opts = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 1,
                mode: SubdivisionMode::CatmullClark,
            },
        );
        opts.draft = 1.2;
        let (a_above, z_above, a_under, z_under) = hull_windage(&s, &opts, 1.2, 0.0);
        assert!((a_above - 18.0).abs() < 1e-6, "a_above = {a_above}");
        assert!((z_above - 2.1).abs() < 1e-6, "z_above = {z_above}");
        assert!((a_under - 12.0).abs() < 1e-6, "a_under = {a_under}");
        assert!((z_under - 0.6).abs() < 1e-6, "z_under = {z_under}");

        let disp_t = 10.0 * 4.0 * 1.2 * 1.025; // 49.2 t
        let wh = WeatherHydro {
            displacement_t: disp_t,
            beam_m: 4.0,
            draft_m: 1.2,
            lwl_m: 10.0,
            cb: 1.0,
            kg_m: 1.5,
            gm_m: 0.8,
        };
        let curve: Vec<(f64, f64)> = (-30..=60)
            .map(|deg| (deg as f64, 0.8 * deg_to_rad(deg as f64).sin()))
            .collect();
        let res = evaluate_weather(
            &curve,
            &wh,
            (a_above, z_above, a_under, z_under),
            &WeatherInput::default(),
        );
        let want_lw1 = 504.0 * 18.0 * 1.5 / (1000.0 * 9.81 * disp_t);
        assert!((res.z_m - 1.5).abs() < 1e-6, "Z = {}", res.z_m);
        assert!(
            (res.lw1_m - want_lw1).abs() < 1e-9,
            "lw1 = {} vs {want_lw1}",
            res.lw1_m
        );
        assert!((res.lw2_m - 1.5 * want_lw1).abs() < 1e-9);
    }

    #[test]
    fn synthetic_sine_gz_areas_match_analytic_within_one_percent() {
        // Synthetic GZ = GM * sin(phi) with GM = 1.0 m.
        // Pick inputs so lw1 = 0.08 m, lw2 = 0.12 m.
        let gm = 1.0_f64;
        let curve: Vec<(f64, f64)> = (-30..=60)
            .map(|deg| {
                let d = deg as f64;
                (d, gm * deg_to_rad(d).sin())
            })
            .collect();

        let disp_t = 100.0;
        let target_lw1 = 0.08;
        // lw1 = P * A * Z / (1000 * g * disp_t); with Z = 2.0 m, P = 504 Pa:
        let z_m = 2.0;
        let area_m2 =
            target_lw1 * 1000.0 * GRAVITY_M_S2 * disp_t / (DEFAULT_WIND_PRESSURE_PA * z_m);

        let wh = WeatherHydro {
            displacement_t: disp_t,
            beam_m: 6.0,
            draft_m: 2.0,
            lwl_m: 24.0,
            cb: 0.60,
            kg_m: 2.0,
            gm_m: gm,
        };
        let res = evaluate_weather(
            &curve,
            &wh,
            (area_m2, 3.0, 20.0, 1.0), // z_wind = 3.0, z_under = 1.0 -> Z = 2.0
            &WeatherInput::default(),
        );

        let lw1 = target_lw1;
        let lw2 = 1.5 * lw1;
        let phi0_rad = (lw1 / gm).asin();
        let phic_rad = (lw2 / gm).asin();
        let phi1_rad = deg_to_rad(res.phi1_deg);
        let phi_start_rad = phi0_rad - phi1_rad;
        let phi2_rad = deg_to_rad(50.0);

        // Analytic integrals of (lw2 - GM sin phi) on [phi_start, phic]
        // and (GM sin phi - lw2) on [phic, phi2]:
        let want_a = lw2 * (phic_rad - phi_start_rad) + gm * (phic_rad.cos() - phi_start_rad.cos());
        let want_b = gm * (phic_rad.cos() - phi2_rad.cos()) - lw2 * (phi2_rad - phic_rad);

        let rel_err_a = (res.area_a_m_rad - want_a).abs() / want_a;
        let rel_err_b = (res.area_b_m_rad - want_b).abs() / want_b;
        assert!(
            rel_err_a < 0.01,
            "area a numerical {} vs analytic {} (rel err {})",
            res.area_a_m_rad,
            want_a,
            rel_err_a
        );
        assert!(
            rel_err_b < 0.01,
            "area b numerical {} vs analytic {} (rel err {})",
            res.area_b_m_rad,
            want_b,
            rel_err_b
        );
        assert!(res.passed);
    }

    #[test]
    fn demo_hull_with_demo_loadcase_produces_finite_weather_result() {
        let surface = crate::demo::demo_hull(false);
        let opts = HydroOptions::from_surface(
            &surface,
            SubdivisionOptions {
                level: 2,
                mode: SubdivisionMode::Loop,
            },
        );
        let json_text = include_str!("../../../examples/demo-loadcase.json");
        let cond: crate::loading::LoadingCondition =
            serde_json::from_str(json_text).expect("valid demo-loadcase.json");
        let res = crate::loading::evaluate(&surface, &opts, &cond);
        let w = res
            .weather
            .expect("weather result present in LoadingResult");
        assert!(w.area_m2.is_finite() && w.area_m2 > 0.0, "{w:?}");
        assert!(w.z_m.is_finite() && w.z_m > 0.0, "{w:?}");
        assert!(w.lw1_m.is_finite() && w.lw1_m > 0.0, "{w:?}");
        assert!(w.lw2_m.is_finite() && w.lw2_m > w.lw1_m, "{w:?}");
        assert!(
            w.phi0_deg.is_some_and(|p| p.is_finite() && p >= 0.0),
            "{w:?}"
        );
        assert!(w.phi1_deg.is_finite() && w.phi1_deg > 0.0, "{w:?}");
        assert!(
            w.phic_deg.is_some_and(|p| p.is_finite() && p > 0.0),
            "{w:?}"
        );
        assert!(
            w.phi2_deg.is_some_and(|p| p.is_finite() && p > 0.0),
            "{w:?}"
        );
        assert!(
            w.roll_period_s.is_finite() && w.roll_period_s > 0.0,
            "{w:?}"
        );
        assert!(w.area_a_m_rad.is_finite() && w.area_a_m_rad > 0.0, "{w:?}");
        assert!(w.area_b_m_rad.is_finite() && w.area_b_m_rad > 0.0, "{w:?}");
    }
}
