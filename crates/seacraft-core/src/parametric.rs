//! Parametric hull control-net generator from principal dimensions.
//!
//! Builds a symmetric half-model (`y >= 0`, one layer with `symmetric = true`
//! and `use_in_hydrostatics = true`) as a clean station-by-station quad grid
//! with a closed, creased transom face at `x = 0` and a creased stem profile
//! along `y = 0` at `x = lwl`.
//!
//! All target fitting (`LWL`, `BWL`, `T`, `Cb`, `LCB`) is measured on the
//! level-3 refined mesh via [`crate::hydro::Hull`], reusing
//! [`crate::transform::fit_to_targets`] for principal-dimension scaling and
//! [`crate::lackenby::shift`] for longitudinal station shifting.

use crate::hydro::{Hull, HydroOptions};
use crate::lackenby;
use crate::math::Vec3;
use crate::mesh::{ordered, Layer, Surface, VertexKind};
use crate::subdiv::{SubdivisionMode, SubdivisionOptions};
use crate::transform::{self, Targets};
use serde::{Deserialize, Serialize};
use std::f64::consts::FRAC_PI_2;
use std::fmt;
use std::str::FromStr;

/// Family of parametric hull forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HullKind {
    /// Round-bilge displacement hull with fine entry and moderate run.
    #[default]
    Displacement,
    /// Hard-chine planing hull with straight deadrise bottom, near-vertical
    /// topsides, and a wide immersed transom.
    Planing,
    /// Canoe-body sailing yacht with slack bilges, short overhangs, and a
    /// small transom.
    Yacht,
}

impl HullKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Displacement => "displacement",
            Self::Planing => "planing",
            Self::Yacht => "yacht",
        }
    }
}

impl fmt::Display for HullKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for HullKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "displacement" => Ok(Self::Displacement),
            "planing" => Ok(Self::Planing),
            "yacht" => Ok(Self::Yacht),
            other => Err(format!(
                "unknown hull kind '{other}' (expected displacement, planing or yacht)"
            )),
        }
    }
}

/// Principal dimensions and form targets for parametric hull generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HullSpec {
    pub kind: HullKind,
    /// Waterline length `LWL` (m).
    pub lwl: f64,
    /// Waterline beam `BWL` (m).
    pub beam: f64,
    /// Canoe-body draft `T` at midship (m).
    pub draft: f64,
    /// Moulded depth from keel to sheer at midship (m).
    pub depth: f64,
    /// Optional target block coefficient `Cb` in `0.35..=0.85`.
    pub cb: Option<f64>,
    /// Optional target longitudinal centre of buoyancy as a fraction of `LWL`
    /// from the aft perpendicular (`0.35..=0.60`).
    pub lcb_frac: Option<f64>,
    /// Bottom deadrise angle in degrees (used by `HullKind::Planing`, default 15).
    pub deadrise_deg: f64,
    /// Transom immersion / beam ratio in `0.0..=1.0`.
    pub transom_frac: f64,
    /// Number of longitudinal control stations (default 9).
    pub stations: usize,
    /// Number of control points per station from keel to sheer (default 5).
    pub points_per_station: usize,
}

impl Default for HullSpec {
    fn default() -> Self {
        Self::default_for(HullKind::Displacement)
    }
}

impl HullSpec {
    /// Sensible default specification for the given [`HullKind`].
    pub fn default_for(kind: HullKind) -> Self {
        match kind {
            HullKind::Displacement => Self {
                kind,
                lwl: 12.0,
                beam: 3.6,
                draft: 0.75,
                depth: 1.8,
                cb: None,
                lcb_frac: None,
                deadrise_deg: 15.0,
                transom_frac: 0.35,
                stations: 9,
                points_per_station: 5,
            },
            HullKind::Planing => Self {
                kind,
                lwl: 10.0,
                beam: 3.2,
                draft: 0.60,
                depth: 1.8,
                cb: None,
                lcb_frac: None,
                deadrise_deg: 15.0,
                transom_frac: 0.85,
                stations: 9,
                points_per_station: 5,
            },
            HullKind::Yacht => Self {
                kind,
                lwl: 11.0,
                beam: 3.4,
                draft: 0.65,
                depth: 1.7,
                cb: None,
                lcb_frac: None,
                deadrise_deg: 15.0,
                transom_frac: 0.25,
                stations: 9,
                points_per_station: 5,
            },
        }
    }

    /// Validate all dimensions, ratios and grid counts.
    pub fn validate(&self) -> Result<(), String> {
        if !self.lwl.is_finite() || self.lwl <= 0.0 {
            return Err(format!("lwl must be a positive number, got {}", self.lwl));
        }
        if !self.beam.is_finite() || self.beam <= 0.0 {
            return Err(format!("beam must be a positive number, got {}", self.beam));
        }
        if !self.draft.is_finite() || self.draft <= 0.0 {
            return Err(format!("draft must be a positive number, got {}", self.draft));
        }
        if !self.depth.is_finite() || self.depth <= 0.0 {
            return Err(format!("depth must be a positive number, got {}", self.depth));
        }
        if self.depth <= self.draft {
            return Err(format!(
                "depth ({}) must be strictly greater than draft ({})",
                self.depth, self.draft
            ));
        }
        if let Some(cb) = self.cb {
            if !cb.is_finite() || !(0.35..=0.85).contains(&cb) {
                return Err(format!("cb must be in 0.35..=0.85, got {cb}"));
            }
        }
        if let Some(lcb) = self.lcb_frac {
            if !lcb.is_finite() || !(0.35..=0.60).contains(&lcb) {
                return Err(format!("lcb_frac must be in 0.35..=0.60, got {lcb}"));
            }
        }
        if !self.deadrise_deg.is_finite() || !(0.0..=45.0).contains(&self.deadrise_deg) {
            return Err(format!(
                "deadrise_deg must be in 0..=45 degrees, got {}",
                self.deadrise_deg
            ));
        }
        if !self.transom_frac.is_finite() || !(0.0..=1.0).contains(&self.transom_frac) {
            return Err(format!(
                "transom_frac must be in 0..=1, got {}",
                self.transom_frac
            ));
        }
        if !(5..=65).contains(&self.stations) {
            return Err(format!(
                "stations must be in 5..=65, got {}",
                self.stations
            ));
        }
        if !(3..=33).contains(&self.points_per_station) {
            return Err(format!(
                "points_per_station must be in 3..=33, got {}",
                self.points_per_station
            ));
        }
        Ok(())
    }
}

/// Targets, achieved level-3 hydrostatic values and solver iteration count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerateReport {
    pub kind: HullKind,
    pub target_lwl: f64,
    pub target_bwl: f64,
    pub target_draft: f64,
    pub target_depth: f64,
    pub target_cb: Option<f64>,
    pub target_lcb_frac: Option<f64>,
    pub lwl: f64,
    pub bwl: f64,
    pub t: f64,
    pub cb: f64,
    pub lcb_frac: f64,
    pub volume: f64,
    pub displacement_t: f64,
    pub fullness: f64,
    pub iterations: usize,
}

#[derive(Debug, Clone, Copy)]
struct Measured {
    lwl: f64,
    bwl: f64,
    t: f64,
    cb: f64,
    lcb_frac: f64,
    volume: f64,
    displacement_t: f64,
    wl_min_x: f64,
}

fn measure_hull(surface: &Surface, draft: f64) -> Measured {
    let subdiv = SubdivisionOptions {
        level: 3,
        mode: SubdivisionMode::Loop,
    };
    let hull = Hull::new(surface, subdiv);
    let h = hull.hydrostatics(&HydroOptions {
        draft,
        trim: 0.0,
        heel_deg: 0.0,
        water_density: surface.settings.water_density,
        appendage_coefficient: surface.settings.appendage_coefficient,
        mainframe_x: None,
        stations: 0,
        subdivision: subdiv,
    });
    let lwl = if h.lwl > 0.0 { h.lwl } else { h.submerged_length };
    let bwl = if h.bwl > 0.0 { h.bwl } else { h.submerged_beam };
    let t = if h.absolute_draft > 0.0 {
        h.absolute_draft
    } else {
        h.draft
    };
    // Measure waterline aft perpendicular (min x on the waterline) so LCB
    // fraction is always referenced to the aft perpendicular of LWL.
    let wp = crate::hydro::waterplane_for(hull.keel, hull.length, draft, 0.0, 0.0);
    let segs = crate::hydro::plane_segments(&hull.verts, &hull.tris, &wp);
    let wl_min_x = if segs.is_empty() {
        0.0
    } else {
        segs.iter()
            .flat_map(|(a, b)| [a.x, b.x])
            .fold(f64::INFINITY, f64::min)
    };
    let lcb_frac = if lwl > 1e-9 {
        (h.lcb.x - wl_min_x) / lwl
    } else {
        0.5
    };
    Measured {
        lwl,
        bwl,
        t,
        cb: h.block_coefficient,
        lcb_frac,
        volume: h.volume_m3,
        displacement_t: h.displacement_t,
        wl_min_x,
    }
}

fn polygon_normal(s: &Surface, idx: &[usize]) -> Vec3 {
    if idx.len() < 3 {
        return Vec3::ZERO;
    }
    let p0 = s.points[idx[0]].p;
    let mut n = Vec3::ZERO;
    for i in 1..idx.len() - 1 {
        let a = s.points[idx[i]].p - p0;
        let b = s.points[idx[i + 1]].p - p0;
        n += a.cross(b);
    }
    n
}

fn default_fullness(kind: HullKind) -> f64 {
    match kind {
        HullKind::Displacement => 0.46,
        HullKind::Planing => 0.48,
        HullKind::Yacht => 0.38,
    }
}

/// Build the unscaled symmetric half-model control net for `spec` and
/// dimensionless fullness parameter `fullness` in `0.02..=0.98`.
fn build_raw_hull(spec: &HullSpec, fullness: f64) -> Surface {
    let f = fullness.clamp(0.02, 0.98);
    let ns = spec.stations;
    let np = spec.points_per_station - 1;

    // Position of maximum beam as fraction of station span:
    // planing ~0.40, yacht ~0.46, displacement ~0.48.
    let u_max = match spec.kind {
        HullKind::Displacement => 0.48,
        HullKind::Planing => 0.40,
        HullKind::Yacht => 0.46,
    };
    let keel_min_i = ((u_max * (ns - 1) as f64).round() as usize).clamp(1, ns - 2);
    let u_keel_min = keel_min_i as f64 / (ns - 1) as f64;

    let mut s = Surface::new();
    s.settings.name = format!("Parametric {} {:.1}m", spec.kind, spec.lwl);
    s.settings.designer = "SeaCraft".into();
    s.settings.length = spec.lwl;
    s.settings.beam = spec.beam;
    s.settings.draft = spec.draft;
    s.settings.mainframe_location = spec.lwl * 0.5;
    s.settings.main_particulars_set = true;

    s.layers.clear();
    s.layers.push(Layer {
        id: 0,
        name: "Hull".into(),
        symmetric: true,
        use_in_hydrostatics: true,
        visible: true,
        use_for_intersections: true,
        show_in_linesplan: true,
        ..Layer::default()
    });

    let half_beam = spec.beam * 0.5;
    let freeboard = (spec.depth - spec.draft).max(0.05);

    // Build stations i = 0..ns from transom (u = 0) to stem (u = 1).
    let mut sb = vec![vec![0usize; np + 1]; ns];
    let mut sheer_z_0 = spec.depth;
    let mut keel_z_0 = 0.0;

    // Stern transom rake (aft overhang from bottom of transom to sheer at u = 0).
    let stern_rake_dx = match spec.kind {
        HullKind::Displacement => -0.025 * spec.lwl,
        HullKind::Planing => -0.012 * spec.lwl,
        HullKind::Yacht => -0.045 * spec.lwl,
    };

    for (i, sb_row) in sb.iter_mut().enumerate() {
        let u = i as f64 / (ns - 1) as f64;

        // Sheer profile: lowest near midship (spec.depth), rising toward bow & stern.
        let z_sheer = if u >= u_max {
            let t = (u - u_max) / (1.0 - u_max);
            spec.depth + 0.20 * freeboard * t * t
        } else {
            let t = (u_max - u) / u_max;
            spec.depth + 0.09 * freeboard * t * t
        };

        // Keel rocker profile: exactly 0.0 at keel_min_i, rising smoothly toward
        // the bow (into the raked stem forefoot) and toward the stern.
        let z_keel = if i == keel_min_i {
            0.0
        } else if u < u_keel_min {
            let s_aft = (u_keel_min - u) / u_keel_min;
            match spec.kind {
                HullKind::Displacement => {
                    let stern_rise =
                        (0.45 * (1.0 - 0.50 * spec.transom_frac) * (1.0 - 0.90 * f))
                            .clamp(0.01, 0.58);
                    spec.draft * stern_rise * s_aft.powf(2.1)
                }
                HullKind::Planing => {
                    let stern_rise =
                        (0.07 * (1.0 - 0.65 * spec.transom_frac) * (1.0 - 0.85 * f))
                            .clamp(0.0, 0.10);
                    spec.draft * stern_rise * s_aft.powi(2)
                }
                HullKind::Yacht => {
                    // Canoe-body counter stern: rises to just touch the design
                    // waterline at the transom when fullness is moderate/low.
                    let stern_rise =
                        (0.92 * (1.0 - 0.30 * spec.transom_frac) * (1.0 - 0.88 * f))
                            .clamp(0.02, 0.92);
                    spec.draft * stern_rise * s_aft.powf(1.85)
                }
            }
        } else {
            let s_fwd = (u - u_keel_min) / (1.0 - u_keel_min);
            match spec.kind {
                HullKind::Displacement => {
                    let bow_rise = (0.34 * (1.0 - 0.88 * f)).clamp(0.02, 0.38);
                    spec.draft * bow_rise * s_fwd.powf(2.2)
                }
                HullKind::Planing => {
                    let bow_rise = (0.28 * (1.0 - 0.85 * f)).clamp(0.02, 0.34);
                    spec.draft * bow_rise * s_fwd.powf(2.4)
                }
                HullKind::Yacht => {
                    let bow_rise = (0.54 * (1.0 - 0.85 * f)).clamp(0.03, 0.58);
                    spec.draft * bow_rise * s_fwd.powf(2.0)
                }
            }
        };

        if i == 0 {
            sheer_z_0 = z_sheer;
            keel_z_0 = z_keel;
        }

        // Smooth C1 half-beam envelope b_env(u) in [0, 1] with ZERO slope at
        // u = u_max from both sides:
        //   forward: 1 - (1 - xi)^q_fwd   where xi = (1 - u) / (1 - u_max)
        //   aft:     b_tr + (1 - b_tr) * (1 - (1 - eta)^q_aft)  where eta = u / u_max
        // with q_fwd, q_aft >= 1.5 increasing monotonically with fullness f.
        let b_env = if i == ns - 1 {
            0.0
        } else if u >= u_max {
            let xi = ((1.0 - u) / (1.0 - u_max)).clamp(0.0, 1.0);
            let q_fwd = match spec.kind {
                HullKind::Displacement => 1.50 + 4.20 * f.powf(1.35),
                HullKind::Planing => 1.55 + 3.80 * f.powf(1.25),
                HullKind::Yacht => 1.50 + 3.90 * f.powf(1.35),
            };
            1.0 - (1.0 - xi).powf(q_fwd)
        } else {
            let eta = (u / u_max).clamp(0.0, 1.0);
            let (b_tr, q_aft) = match spec.kind {
                HullKind::Displacement => {
                    let b_tr = ((0.16 + 0.45 * spec.transom_frac) * (1.0 - f) + 0.93 * f)
                        .clamp(0.08, 0.95);
                    let q_aft = 1.50 + 3.90 * f.powf(1.35);
                    (b_tr, q_aft)
                }
                HullKind::Planing => {
                    let b_tr = ((0.62 + 0.28 * spec.transom_frac) * (1.0 - 0.45 * f) + 0.44 * f)
                        .clamp(0.52, 0.96);
                    let q_aft = 1.60 + 3.50 * f.powf(1.25);
                    (b_tr, q_aft)
                }
                HullKind::Yacht => {
                    let b_tr = ((0.12 + 0.35 * spec.transom_frac) * (1.0 - f) + 0.88 * f)
                        .clamp(0.06, 0.92);
                    let q_aft = 1.50 + 3.60 * f.powf(1.35);
                    (b_tr, q_aft)
                }
            };
            b_tr + (1.0 - b_tr) * (1.0 - (1.0 - eta).powf(q_aft))
        };

        let station_half_beam = half_beam * b_env;
        let dz_total = (z_sheer - z_keel).max(1e-4);
        let z_wl = (spec.draft * 1.06).clamp(z_keel + 0.22 * dz_total, z_sheer - 0.06 * dz_total);
        let dz_sub = (z_wl - z_keel).max(1e-4);
        let chine_j = (np / 2).max(1);

        for (j, slot) in sb_row.iter_mut().enumerate() {
            // Compute section (y, z) with consistent vertical placement up to the
            // stem so longitudinal grid lines flow smoothly into the bow profile.
            let (y, z) = match spec.kind {
                HullKind::Displacement => {
                    let flare = 1.0 + 0.14 * (1.0 - f);
                    let (y_sec, z_sec) = if j == 0 {
                        (0.0, z_keel)
                    } else if j == np {
                        (station_half_beam * flare, z_sheer)
                    } else {
                        let tau = j as f64 / (np - 1).max(1) as f64;
                        let theta = tau * FRAC_PI_2;
                        let alpha = (0.06 + 2.05 * (1.0 - f).powf(1.20)).clamp(0.06, 2.2);
                        let beta = (0.36 + 2.40 * f.powf(1.20)).clamp(0.36, 2.8);
                        let y_norm = theta.sin().powf(alpha);
                        let z_norm = (1.0 - theta.cos()).powf(beta);
                        (station_half_beam * y_norm, z_keel + dz_sub * z_norm)
                    };
                    (if i == ns - 1 { 0.0 } else { y_sec }, z_sec)
                }
                HullKind::Yacht => {
                    let flare = 1.0 + 0.18 * (1.0 - f);
                    let (y_sec, z_sec) = if j == 0 {
                        (0.0, z_keel)
                    } else if j == np {
                        (station_half_beam * flare, z_sheer)
                    } else {
                        let tau = j as f64 / (np - 1).max(1) as f64;
                        let theta = tau * FRAC_PI_2;
                        let alpha = (0.10 + 2.15 * (1.0 - f).powf(1.15)).clamp(0.10, 2.3);
                        let beta = (0.34 + 2.20 * f.powf(1.20)).clamp(0.34, 2.6);
                        let y_norm = theta.sin().powf(alpha);
                        let z_norm = (1.0 - theta.cos()).powf(beta);
                        (station_half_beam * y_norm, z_keel + dz_sub * z_norm)
                    };
                    (if i == ns - 1 { 0.0 } else { y_sec }, z_sec)
                }
                HullKind::Planing => {
                    let y_chine = station_half_beam * (0.90 + 0.08 * f);
                    let y_sheer = station_half_beam;
                    let tan_beta = spec.deadrise_deg.to_radians().tan().max(0.02)
                        * (1.85 * (1.0 - f) + 0.10).clamp(0.10, 2.3);
                    let chine_bow_lift = if u > u_max {
                        0.12 * spec.draft * (1.0 - 0.6 * f) * ((u - u_max) / (1.0 - u_max)).powi(2)
                    } else {
                        0.0
                    };
                    // Keep the chine below the design waterline all the way to
                    // the stem so the waterline never kinks across the crease.
                    let max_dz_chine = (0.76 * (spec.draft - z_keel)).max(0.04 * dz_total);
                    let dz_chine = (y_chine * tan_beta + chine_bow_lift)
                        .clamp(0.04 * dz_total, max_dz_chine.min(0.55 * dz_total));
                    let z_chine = z_keel + dz_chine;
                    let (y_sec, z_sec) = if j == 0 {
                        (0.0, z_keel)
                    } else if j <= chine_j {
                        let s_bot = j as f64 / chine_j as f64;
                        (y_chine * s_bot, z_keel + (z_chine - z_keel) * s_bot)
                    } else {
                        let s_top = (j - chine_j) as f64 / (np - chine_j) as f64;
                        (
                            y_chine + (y_sheer - y_chine) * s_top,
                            z_chine + (z_sheer - z_chine) * s_top,
                        )
                    };
                    (if i == ns - 1 { 0.0 } else { y_sec }, z_sec)
                }
            };

            // Longitudinal x coordinate with stem rake, rounded forefoot, and
            // stern overhang/rake.
            let h_z = ((z - z_keel) / dz_total).clamp(0.0, 1.0);
            let dx_rake = if u >= u_max {
                let s_fwd = (u - u_max) / (1.0 - u_max);
                let w_fwd = s_fwd * s_fwd;
                let (bow_overhang, forefoot_setback) = match spec.kind {
                    // Modest stem rake (~20 deg from vertical) + rounded forefoot.
                    HullKind::Displacement => (
                        0.36 * (z_sheer - spec.draft),
                        0.032 * spec.lwl * (1.0 - 0.65 * f),
                    ),
                    // Raked planing stem (~25 deg from vertical).
                    HullKind::Planing => (
                        0.46 * (z_sheer - spec.draft),
                        0.024 * spec.lwl * (1.0 - 0.60 * f),
                    ),
                    // Yacht: ~0.08 * LWL bow overhang above the waterline + canoe forefoot.
                    HullKind::Yacht => (
                        0.08 * spec.lwl,
                        0.038 * spec.lwl * (1.0 - 0.65 * f),
                    ),
                };
                let rake_slope = bow_overhang / (z_sheer - spec.draft).max(0.05);
                let stem_dx =
                    rake_slope * (z - spec.draft) - forefoot_setback * (1.0 - h_z).powi(2);
                w_fwd * stem_dx
            } else {
                let s_aft = (u_max - u) / u_max;
                let w_aft = s_aft * s_aft;
                // Linear in z at u = 0 so the transom polygon stays strictly planar.
                let h_0 = (z - keel_z_0) / (sheer_z_0 - keel_z_0).max(1e-4);
                w_aft * stern_rake_dx * h_0
            };

            let x = u * spec.lwl + dx_rake;
            *slot = s.add_point(Vec3::new(x, y, z));
        }
    }

    // Starboard hull quad grid. Winding [sb[i][j], sb[i][j+1], sb[i+1][j+1], sb[i+1][j]]
    // points outward (+y / -z), matching demo.rs and Invariant 2.
    for i in 0..ns - 1 {
        for j in 0..np {
            s.add_face(
                vec![sb[i][j], sb[i][j + 1], sb[i + 1][j + 1], sb[i + 1][j]],
                0,
            );
        }
    }

    // Closed transom face at i = 0 required by Invariant 3 so the mirrored
    // half-model closes cleanly across the centreline. `apex` shares the exact
    // same x(sheer_z_0) as `sb[0][np]` so the transom polygon is planar.
    let apex = s.add_point(Vec3::new(stern_rake_dx, 0.0, sheer_z_0));
    let mut half: Vec<usize> = (0..=np).rev().map(|j| sb[0][j]).collect();
    half.push(apex);
    if polygon_normal(&s, &half).x > 0.0 {
        half.reverse();
    }
    s.add_face(half.clone(), 0);

    s.initialize_trace();

    let set_crease = |s: &mut Surface, a: usize, b: usize| {
        let key = ordered(a, b);
        if let Some(e) = s.edges.iter_mut().find(|e| e.key() == key) {
            e.crease = true;
        }
    };

    // Crease the transom boundary ring.
    for w in 0..half.len() {
        set_crease(&mut s, half[w], half[(w + 1) % half.len()]);
    }

    // Crease the stem profile at y = 0 (i = ns - 1).
    for j in 0..np {
        set_crease(&mut s, sb[ns - 1][j], sb[ns - 1][j + 1]);
    }
    for &pt_idx in &sb[ns - 1][1..np] {
        s.points[pt_idx].kind = VertexKind::Crease;
    }

    // Crease the keel centreline edges (j = 0).
    for i in 0..ns - 1 {
        set_crease(&mut s, sb[i][0], sb[i + 1][0]);
    }

    // For planing hulls, crease the longitudinal chine edge row.
    if spec.kind == HullKind::Planing {
        let chine_j = (np / 2).max(1);
        for i in 0..ns - 1 {
            set_crease(&mut s, sb[i][chine_j], sb[i + 1][chine_j]);
        }
    }

    s
}

/// Apply longitudinal station shift (reusing [`crate::lackenby::shift`]) and
/// fit `LWL`, `BWL` and `T` on the level-3 refined mesh via
/// [`crate::transform::fit_to_targets`].
fn build_and_fit_dims(
    spec: &HullSpec,
    fullness: f64,
    lcb_shift_a: f64,
) -> Result<(Surface, Measured, usize), String> {
    let mut s = build_raw_hull(spec, fullness);

    // Reuse lackenby::shift for longitudinal station shifting. When |a| > 0.7,
    // compose two half-steps so each step has |a_step| < 0.85 and strictly
    // preserves station ordering.
    if lcb_shift_a.abs() > 1e-9 {
        let (min, max) = s.extents();
        let span = (max.x - min.x).max(1e-6);
        if lcb_shift_a.abs() <= 0.7 {
            s = lackenby::shift(&s, lcb_shift_a, 0.0, min.x, span);
        } else {
            let half_a = (lcb_shift_a * 0.5).clamp(-0.85, 0.85);
            s = lackenby::shift(&s, half_a, 0.0, min.x, span);
            s = lackenby::shift(&s, half_a, 0.0, min.x, span);
        }
    }

    let subdiv = SubdivisionOptions {
        level: 3,
        mode: SubdivisionMode::Loop,
    };
    let targets = Targets {
        lwl: Some(spec.lwl),
        bwl: Some(spec.beam),
        draft: Some(spec.draft),
    };
    let (mut s, fit_rep) = transform::fit_to_targets(&s, targets, subdiv, 1e-3, 6)?;

    // Ensure the waterline aft perpendicular sits at x = 0 and the keel sits at
    // z = 0, and refine BWL / LWL / T if the waterline shifted slightly.
    let mut m = measure_hull(&s, spec.draft);
    let (min_ext, _) = s.extents();
    if m.wl_min_x.abs() > 1e-9 || min_ext.z.abs() > 1e-12 {
        transform::apply(
            &mut s,
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-m.wl_min_x, 0.0, -min_ext.z),
        )?;
        m = measure_hull(&s, spec.draft);
    }
    let mut extra_iters = 0usize;
    for _ in 0..3 {
        let err_l = ((m.lwl - spec.lwl) / spec.lwl).abs();
        let err_b = ((m.bwl - spec.beam) / spec.beam).abs();
        let err_t = ((m.t - spec.draft) / spec.draft).abs();
        if err_l <= 1e-3 && err_b <= 1e-3 && err_t <= 1e-3 {
            break;
        }
        let sx = if m.lwl > 1e-9 { spec.lwl / m.lwl } else { 1.0 };
        let sy = if m.bwl > 1e-9 { spec.beam / m.bwl } else { 1.0 };
        let sz = if m.t > 1e-9 { spec.draft / m.t } else { 1.0 };
        transform::apply(&mut s, Vec3::new(sx, sy, sz), Vec3::ZERO)?;
        m = measure_hull(&s, spec.draft);
        let (min_ext, _) = s.extents();
        if m.wl_min_x.abs() > 1e-9 || min_ext.z.abs() > 1e-12 {
            transform::apply(
                &mut s,
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(-m.wl_min_x, 0.0, -min_ext.z),
            )?;
        }
        // Keep keel centreline and stem strictly at y = 0.0.
        for p in &mut s.points {
            if p.p.y.abs() < 1e-12 {
                p.p.y = 0.0;
            }
        }
        m = measure_hull(&s, spec.draft);
        extra_iters += 1;
    }
    s.settings.length = spec.lwl;
    s.settings.beam = spec.beam;
    s.settings.draft = spec.draft;
    s.settings.mainframe_location = spec.lwl * 0.5;

    Ok((s, m, fit_rep.iterations + extra_iters))
}

/// Solve for the fullness parameter `f` in `[0.02, 0.98]` that hits `target_cb`
/// within `tol` at a fixed longitudinal shift `lcb_shift_a` using bracketed
/// secant / bisection.
fn solve_fullness_for_cb(
    spec: &HullSpec,
    target_cb: f64,
    lcb_shift_a: f64,
    tol: f64,
) -> Result<(Surface, Measured, f64, usize), String> {
    let mut total_iters = 0usize;
    let f_lo = 0.02;
    let f_hi = 0.98;

    let (s_lo, m_lo, it_lo) = build_and_fit_dims(spec, f_lo, lcb_shift_a)?;
    total_iters += it_lo;
    if (m_lo.cb - target_cb).abs() <= tol {
        return Ok((s_lo, m_lo, f_lo, total_iters));
    }

    let (s_hi, m_hi, it_hi) = build_and_fit_dims(spec, f_hi, lcb_shift_a)?;
    total_iters += it_hi;
    if (m_hi.cb - target_cb).abs() <= tol {
        return Ok((s_hi, m_hi, f_hi, total_iters));
    }

    if target_cb < m_lo.cb.min(m_hi.cb) - tol || target_cb > m_lo.cb.max(m_hi.cb) + tol {
        return Err(format!(
            "target Cb {:.3} is outside achievable range [{:.3}, {:.3}] for {} hull",
            target_cb,
            m_lo.cb.min(m_hi.cb),
            m_lo.cb.max(m_hi.cb),
            spec.kind
        ));
    }

    let mut a = f_lo;
    let mut fa = m_lo.cb - target_cb;
    let mut b = f_hi;
    let mut fb = m_hi.cb - target_cb;
    let mut best = if fa.abs() < fb.abs() {
        (s_lo, m_lo, f_lo, fa.abs())
    } else {
        (s_hi, m_hi, f_hi, fb.abs())
    };

    for step in 0..18 {
        // Hybrid secant + bisection (Dekker/Brent style): use secant when it
        // lands well inside the bracket, otherwise bisect.
        let mid = 0.5 * (a + b);
        let sec = if (fb - fa).abs() > 1e-12 {
            b - fb * (b - a) / (fb - fa)
        } else {
            mid
        };
        let margin = 0.1 * (b - a).abs();
        let c = if step % 3 == 2 || sec <= a + margin || sec >= b - margin {
            mid
        } else {
            sec
        };

        let (sc, mc, it_c) = build_and_fit_dims(spec, c, lcb_shift_a)?;
        total_iters += it_c;
        let fc = mc.cb - target_cb;
        if fc.abs() < best.3 {
            best = (sc.clone(), mc, c, fc.abs());
        }
        if fc.abs() <= tol || (b - a).abs() < 1e-4 {
            return Ok((sc, mc, c, total_iters));
        }
        if (fa < 0.0 && fc < 0.0) || (fa > 0.0 && fc > 0.0) {
            a = c;
            fa = fc;
        } else {
            b = c;
            fb = fc;
        }
    }

    Ok((best.0, best.1, best.2, total_iters))
}

/// Generate a parametric symmetric half-model control net and level-3
/// verification report from `spec`.
pub fn generate(spec: &HullSpec) -> Result<(Surface, GenerateReport), String> {
    spec.validate()?;

    let mut total_iters = 0usize;
    let mut fullness = default_fullness(spec.kind);

    // Step 1 & 2: build, scale to (LWL, BWL, T), and if `cb` is given solve
    // fullness so measured Cb matches within 0.005 (we solve to 0.002).
    let (mut surface, mut measured) = if let Some(target_cb) = spec.cb {
        let (s, m, f_sol, iters) = solve_fullness_for_cb(spec, target_cb, 0.0, 0.002)?;
        fullness = f_sol;
        total_iters += iters;
        (s, m)
    } else {
        let (s, m, iters) = build_and_fit_dims(spec, fullness, 0.0)?;
        total_iters += iters;
        (s, m)
    };

    // Step 3: if `lcb_frac` is given, shift stations longitudinally via
    // lackenby::shift until LCB/LWL is within 0.5% of LWL while keeping
    // displacement within 1% (and Cb within 0.005 if specified).
    if let Some(target_lcb) = spec.lcb_frac {
        let cb_keep = spec.cb.unwrap_or(measured.cb);
        if (measured.lcb_frac - target_lcb).abs() > 0.002 {
            let mut a_lo = -1.65;
            let mut a_hi = 1.65;

            let evaluate_shift =
                |shift_a: f64, f_hint: f64| -> Result<(Surface, Measured, f64, usize), String> {
                    if spec.cb.is_some() {
                        solve_fullness_for_cb(spec, cb_keep, shift_a, 0.002)
                    } else {
                        // First evaluate at f_hint; if volume drifted by > 0.6%,
                        // adjust fullness to hold displacement within 1%.
                        let (s, m, it) = build_and_fit_dims(spec, f_hint, shift_a)?;
                        if ((m.cb - cb_keep) / cb_keep.max(1e-6)).abs() <= 0.006 {
                            Ok((s, m, f_hint, it))
                        } else {
                            solve_fullness_for_cb(spec, cb_keep, shift_a, 0.002)
                        }
                    }
                };

            let (s_lo, m_lo, f_lo, it_lo) = evaluate_shift(a_lo, fullness)?;
            total_iters += it_lo;
            let (s_hi, m_hi, f_hi, it_hi) = evaluate_shift(a_hi, fullness)?;
            total_iters += it_hi;

            let mut flo = m_lo.lcb_frac - target_lcb;
            let mut fhi = m_hi.lcb_frac - target_lcb;
            let mut best = if flo.abs() < fhi.abs() {
                (s_lo, m_lo, f_lo, flo.abs())
            } else {
                (s_hi, m_hi, f_hi, fhi.abs())
            };

            for step in 0..14 {
                if best.3 <= 0.002 {
                    break;
                }
                let mid = 0.5 * (a_lo + a_hi);
                let sec = if (fhi - flo).abs() > 1e-12 {
                    a_hi - fhi * (a_hi - a_lo) / (fhi - flo)
                } else {
                    mid
                };
                let margin = 0.1 * (a_hi - a_lo).abs();
                let a_mid = if step % 3 == 2 || sec <= a_lo + margin || sec >= a_hi - margin {
                    mid
                } else {
                    sec
                };
                let (sm, mm, fm, it_m) = evaluate_shift(a_mid, best.2)?;
                total_iters += it_m;
                let err = mm.lcb_frac - target_lcb;
                if err.abs() < best.3 {
                    best = (sm, mm, fm, err.abs());
                }
                if err.abs() <= 0.002 || (a_hi - a_lo).abs() < 1e-4 {
                    break;
                }
                if (flo < 0.0 && err < 0.0) || (flo > 0.0 && err > 0.0) {
                    a_lo = a_mid;
                    flo = err;
                } else {
                    a_hi = a_mid;
                    fhi = err;
                }
            }

            surface = best.0;
            measured = best.1;
            fullness = best.2;
        }
    }

    let report = GenerateReport {
        kind: spec.kind,
        target_lwl: spec.lwl,
        target_bwl: spec.beam,
        target_draft: spec.draft,
        target_depth: spec.depth,
        target_cb: spec.cb,
        target_lcb_frac: spec.lcb_frac,
        lwl: measured.lwl,
        bwl: measured.bwl,
        t: measured.t,
        cb: measured.cb,
        lcb_frac: measured.lcb_frac,
        volume: measured.volume,
        displacement_t: measured.displacement_t,
        fullness,
        iterations: total_iters,
    };

    Ok((surface, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fef;
    use crate::subdiv::evaluate_full;
    use crate::threemf::weld;

    /// Check that a generated half-model has no submerged boundary edges (neither
    /// on the control net off the symmetry plane nor on the welded level-3 full
    /// mesh) and that its raw signed divergence volume is positive (outward normals).
    fn assert_closed_and_outward(surface: &Surface, draft: f64) {
        assert_eq!(surface.layers.len(), 1);
        assert!(surface.layers[0].symmetric);
        assert!(surface.layers[0].use_in_hydrostatics);

        // Keel / stem boundary edges on the half-model may only lie on the
        // symmetry plane y = 0 or above the waterline (sheer / transom top).
        let counts = surface.edge_face_counts();
        for (ei, e) in surface.edges.iter().enumerate() {
            if counts[ei] == 1 {
                let a = surface.points[e.a].p;
                let b = surface.points[e.b].p;
                let on_centreline = a.y.abs() < 1e-9 && b.y.abs() < 1e-9;
                let above_waterline = a.z > draft && b.z > draft;
                assert!(
                    on_centreline || above_waterline,
                    "submerged off-centreline boundary edge on half-model: {a:?} -> {b:?}"
                );
            }
        }

        // On the mirrored + welded level-3 mesh, every edge with a vertex at or
        // below the waterline must be shared by 2 triangles (no leaks).
        let subdiv = SubdivisionOptions {
            level: 3,
            mode: SubdivisionMode::Loop,
        };
        let full = evaluate_full(surface, subdiv);
        let tris_u32: Vec<[u32; 3]> = full
            .triangles()
            .into_iter()
            .map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
            .collect();
        let (verts, tris) = weld(&full.verts, &tris_u32, 1e-6);
        let mut edge_counts: std::collections::HashMap<(u32, u32), usize> =
            std::collections::HashMap::new();
        for t in &tris {
            for k in 0..3 {
                let u = t[k];
                let v = t[(k + 1) % 3];
                let key = if u < v { (u, v) } else { (v, u) };
                *edge_counts.entry(key).or_insert(0) += 1;
            }
        }
        for (&(u, v), &c) in &edge_counts {
            let za = verts[u as usize].z;
            let zb = verts[v as usize].z;
            if za <= draft + 1e-4 || zb <= draft + 1e-4 {
                assert_eq!(
                    c, 2,
                    "submerged non-manifold/boundary edge on welded full mesh (count={c}): {:?} -> {:?}",
                    verts[u as usize], verts[v as usize]
                );
            }
        }

        // Verify outward normals: raw signed divergence integral below the
        // waterplane must be strictly positive without sign flipping.
        let wp = crate::hydro::waterplane(surface, draft, 0.0, 0.0);
        let origin = wp.project(Vec3::ZERO);
        let mut raw_vol = 0.0;
        for t in &full.triangles() {
            let tri = [full.verts[t[0]], full.verts[t[1]], full.verts[t[2]]];
            if tri.iter().all(|p| wp.side(*p) <= 0.0) {
                let p0 = tri[0] - origin;
                let p1 = tri[1] - origin;
                let p2 = tri[2] - origin;
                raw_vol += p0.dot(p1.cross(p2)) / 6.0;
            }
        }
        assert!(
            raw_vol > 0.0,
            "expected positive raw divergence volume (outward normals), got {raw_vol}"
        );

        let mut opts = HydroOptions::from_surface(surface, subdiv);
        opts.draft = draft;
        let h = crate::hydro::compute(surface, &opts);
        assert!(h.warnings.is_empty(), "hydro warnings: {:?}", h.warnings);
        assert!(h.volume_m3 > 0.0);
        assert!(h.waterplane_area_m2 > 0.0);
    }

    #[test]
    fn default_specs_for_all_three_kinds_succeed_and_match_dims() {
        for kind in [HullKind::Displacement, HullKind::Planing, HullKind::Yacht] {
            let spec = HullSpec::default_for(kind);
            let (surface, rep) = generate(&spec).expect("generate default spec");
            assert_closed_and_outward(&surface, spec.draft);
            assert!(rep.volume > 0.0, "{kind}: volume={}", rep.volume);
            assert!(
                ((rep.lwl - spec.lwl) / spec.lwl).abs() <= 0.005,
                "{kind}: lwl={} target={}",
                rep.lwl,
                spec.lwl
            );
            assert!(
                ((rep.bwl - spec.beam) / spec.beam).abs() <= 0.005,
                "{kind}: bwl={} target={}",
                rep.bwl,
                spec.beam
            );
            assert!(
                ((rep.t - spec.draft) / spec.draft).abs() <= 0.005,
                "{kind}: t={} target={}",
                rep.t,
                spec.draft
            );
            println!(
                "DEFAULT {kind}: target=({:.2}, {:.2}, {:.2}) achieved=({:.4}, {:.4}, {:.4}) Cb={:.4} LCB/LWL={:.4} V={:.3} disp={:.3}t f={:.4} iters={}",
                spec.lwl, spec.beam, spec.draft, rep.lwl, rep.bwl, rep.t, rep.cb, rep.lcb_frac, rep.volume, rep.displacement_t, rep.fullness, rep.iterations
            );
        }
    }

    #[test]
    fn displacement_cb_targets_met_within_tolerance() {
        for target_cb in [0.45, 0.55] {
            let spec = HullSpec {
                cb: Some(target_cb),
                ..HullSpec::default_for(HullKind::Displacement)
            };
            let (surface, rep) = generate(&spec).expect("generate with cb target");
            assert_closed_and_outward(&surface, spec.draft);
            assert!(
                (rep.cb - target_cb).abs() <= 0.005,
                "target_cb={target_cb} got cb={}",
                rep.cb
            );
            assert!(((rep.lwl - spec.lwl) / spec.lwl).abs() <= 0.005);
            assert!(((rep.bwl - spec.beam) / spec.beam).abs() <= 0.005);
            assert!(((rep.t - spec.draft) / spec.draft).abs() <= 0.005);
        }
    }

    #[test]
    fn lcb_target_met_while_preserving_displacement() {
        let base_spec = HullSpec::default_for(HullKind::Displacement);
        let (_, base_rep) = generate(&base_spec).expect("baseline generate");

        for target_lcb in [0.44, 0.52] {
            let spec = HullSpec {
                lcb_frac: Some(target_lcb),
                ..base_spec.clone()
            };
            let (surface, rep) = generate(&spec).expect("generate with lcb target");
            assert_closed_and_outward(&surface, spec.draft);
            assert!(
                (rep.lcb_frac - target_lcb).abs() <= 0.005,
                "target_lcb={target_lcb} got lcb_frac={}",
                rep.lcb_frac
            );
            let disp_diff = ((rep.displacement_t - base_rep.displacement_t)
                / base_rep.displacement_t)
                .abs();
            assert!(
                disp_diff <= 0.01,
                "displacement drifted by {:.2}% (base={}, shifted={})",
                disp_diff * 100.0,
                base_rep.displacement_t,
                rep.displacement_t
            );
        }
    }

    #[test]
    fn invalid_specs_are_rejected_with_descriptive_errors() {
        let base = HullSpec::default();
        assert!(generate(&HullSpec { lwl: 0.0, ..base.clone() }).is_err());
        assert!(generate(&HullSpec { beam: -1.0, ..base.clone() }).is_err());
        assert!(generate(&HullSpec { draft: 0.0, ..base.clone() }).is_err());
        assert!(generate(&HullSpec { depth: 0.5, draft: 0.8, ..base.clone() }).is_err());
        assert!(generate(&HullSpec { cb: Some(0.25), ..base.clone() }).is_err());
        assert!(generate(&HullSpec { cb: Some(0.90), ..base.clone() }).is_err());
        assert!(generate(&HullSpec { lcb_frac: Some(0.30), ..base.clone() }).is_err());
        assert!(generate(&HullSpec { lcb_frac: Some(0.65), ..base.clone() }).is_err());
    }

    #[test]
    fn fef_round_trip_preserves_generated_hull() {
        let spec = HullSpec::default_for(HullKind::Planing);
        let (surface, _) = generate(&spec).expect("generate planing");
        let text = fef::write(&surface);
        let parsed = fef::parse(&text).expect("parse generated .fef");
        assert_eq!(parsed.points.len(), surface.points.len());
        assert_eq!(parsed.faces.len(), surface.faces.len());
        assert_eq!(parsed.layers.len(), 1);
        assert!(parsed.layers[0].symmetric);
    }

    fn max_waterline_turning_angle_deg(surface: &Surface, draft: f64) -> (f64, Vec3) {
        let subdiv = SubdivisionOptions {
            level: 3,
            mode: SubdivisionMode::Loop,
        };
        let hull = Hull::new(surface, subdiv);
        let wp = crate::hydro::waterplane_for(hull.keel, hull.length, draft, 0.0, 0.0);
        let segs = crate::hydro::plane_segments(&hull.verts, &hull.tris, &wp);
        let scale = (hull.model_max.x - hull.model_min.x)
            .max(hull.model_max.y - hull.model_min.y)
            .max(1.0);
        let paths = crate::hydro::chain_paths(&segs, scale * 1e-7);
        let (poly, closed) = paths
            .iter()
            .max_by_key(|(p, _)| p.len())
            .expect("waterline path");
        assert!(*closed, "waterline loop should be closed");

        let xmin = poly.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let xmax = poly.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let lwl = (xmax - xmin).max(1e-6);
        let margin = 0.03 * lwl;

        let n = poly.len();
        let mut max_deg = 0.0f64;
        let mut worst_pt = Vec3::ZERO;
        for i in 0..n {
            let p_prev = poly[(i + n - 1) % n];
            let p_curr = poly[i];
            let p_next = poly[(i + 1) % n];
            if p_curr.x <= xmin + margin || p_curr.x >= xmax - margin {
                continue;
            }
            let v1 = (p_curr.x - p_prev.x, p_curr.y - p_prev.y);
            let v2 = (p_next.x - p_curr.x, p_next.y - p_curr.y);
            let l1 = (v1.0 * v1.0 + v1.1 * v1.1).sqrt();
            let l2 = (v2.0 * v2.0 + v2.1 * v2.1).sqrt();
            if l1 < 1e-6 || l2 < 1e-6 {
                continue;
            }
            let cos_a = ((v1.0 * v2.0 + v1.1 * v2.1) / (l1 * l2)).clamp(-1.0, 1.0);
            let deg = cos_a.acos().to_degrees();
            if deg > max_deg {
                max_deg = deg;
                worst_pt = p_curr;
            }
        }
        (max_deg, worst_pt)
    }

    #[test]
    fn waterline_is_fair_for_all_three_kinds() {
        for kind in [HullKind::Displacement, HullKind::Planing, HullKind::Yacht] {
            let spec = HullSpec::default_for(kind);
            let (surface, _) = generate(&spec).expect("generate default spec");
            let (max_deg, worst_pt) = max_waterline_turning_angle_deg(&surface, spec.draft);
            println!("{kind}: max waterline turning angle = {max_deg:.3} deg at {worst_pt:?}");
            assert!(
                max_deg < 10.0,
                "{kind}: waterline has a kink of {max_deg:.2} deg at {worst_pt:?} (>= 10 deg)"
            );
        }
    }
}
