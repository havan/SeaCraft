//! Loading conditions: weight items, partially filled tanks, free-surface
//! correction, equilibrium floating condition and intact stability.
//!
//! Conventions (see `AGENTS.md`):
//! * Axes: `x` forward, `y` to starboard, `z` up; keel at low `z`.
//! * Units: metres and tonnes (`mass_t`, `displacement_t`, `density_t_m3`).
//! * Datum: `KG`, `KB`, `BM`, `KM`, `GM` are measured above the keel
//!   ([`crate::mesh::Surface::lowest_hydrostatics_point`]); `LCG`, `LCB`, `LCF`
//!   are in model `x`, and `TCG` in model `y` (+ to starboard).
//! * Free-surface moment of a slack tank (`0 < fill < 1`):
//!   `FSM_t = density_t_m3 * i_t`, where `i_t` is the transverse second moment
//!   of the liquid free-surface area about its own centroidal fore-aft axis
//!   (`l * b^3 / 12` for a rectangular box). For `fill <= 0` or `fill >= 1`
//!   (pressed-up tank), `FSM_t = 0`.
//! * Free-surface correction: `FSC = sum(FSM_t) / displacement_t`, raising the
//!   effective vertical centre of gravity to `KG_fluid = KG + FSC` and
//!   reducing the transverse metacentric height to `GM_fluid = KM - KG_fluid`.

use crate::curves::station_polygons;
use crate::hydro::{Hull, HydroOptions};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::stability::{
    cross_curves, evaluate_imo_criteria, solve_draft, solve_floating, CrossCurve,
    FloatingCondition, ImoCriteria, StabilityCondition,
};
use crate::weather::{
    evaluate_weather, hull_windage_on_hull, sample_weather_gz, BilgeKind, WeatherHydro,
    WeatherInput, WeatherResult, WindageItem,
};
use serde::{Deserialize, Serialize};

fn default_permeability() -> f64 {
    0.98
}

/// A discrete weight item with its centroid in model coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightItem {
    pub name: String,
    /// Mass in tonnes.
    pub mass_t: f64,
    /// Longitudinal centroid in model `x` (m).
    pub x: f64,
    /// Transverse centroid in model `y` (m, positive to starboard).
    pub y: f64,
    /// Vertical centroid in model `z` (m).
    pub z: f64,
}

/// A liquid tank bounded by a box `[x_min..x_max] × [y_min..y_max] × [z_min..z_max]`,
/// optionally clipped to the hull's moulded volume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tank {
    pub name: String,
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub z_min: f64,
    pub z_max: f64,
    /// Fill fraction (`0..=1` by volume).
    pub fill: f64,
    /// Liquid density in tonnes per cubic metre (e.g. `0.85` fuel, `1.00` fresh water, `1.025` sea water).
    pub density_t_m3: f64,
    /// Structural permeability (`0..=1`, default `0.98`).
    #[serde(default = "default_permeability")]
    pub permeability: f64,
    /// Intersect the bounding box with the hull's moulded volume when `true`.
    #[serde(default)]
    pub clip_to_hull: bool,
}

/// A complete ship loading condition: lightship, deadweight weight items, and liquid tanks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadingCondition {
    pub name: String,
    pub lightship: WeightItem,
    #[serde(default)]
    pub items: Vec<WeightItem>,
    #[serde(default)]
    pub tanks: Vec<Tank>,
    /// Optional superstructure/extra windage items for the IMO weather criterion.
    #[serde(default, alias = "windage")]
    pub windage_extra: Vec<WindageItem>,
    /// Bilge kind for the IMO weather criterion (default `Round`).
    #[serde(default)]
    pub bilge: BilgeKind,
    /// Optional downflooding angle φf in degrees for the IMO weather criterion.
    #[serde(default)]
    pub downflooding_deg: Option<f64>,
}

/// One row of the weight and centre-of-gravity breakdown table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakdownRow {
    pub name: String,
    /// `"lightship"`, `"item"`, or `"tank"`.
    pub kind: String,
    pub mass_t: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub fsm_t_m: f64,
}

/// Evaluated volume, mass, liquid centroid and free-surface moment of one tank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TankSummary {
    pub name: String,
    pub fill: f64,
    pub density_t_m3: f64,
    pub permeability: f64,
    pub clip_to_hull: bool,
    /// Geometric moulded volume of the tank (box or hull-clipped box), m³.
    pub moulded_volume_m3: f64,
    /// Net 100 % liquid capacity (`moulded_volume_m3 * permeability`), m³.
    pub capacity_m3: f64,
    /// Filled liquid volume (`capacity_m3 * fill`), m³.
    pub volume_m3: f64,
    /// Liquid mass (`volume_m3 * density_t_m3`), tonnes.
    pub mass_t: f64,
    /// Liquid centroid `x` in model coordinates, metres.
    pub x: f64,
    /// Liquid centroid `y` in model coordinates, metres.
    pub y: f64,
    /// Liquid centroid `z` in model coordinates, metres.
    pub z: f64,
    /// Upright fluid surface height in model `z`, metres.
    pub surface_z: f64,
    /// Transverse second moment of the free-surface area about its centroidal fore-aft axis, m⁴.
    pub it_m4: f64,
    /// Free-surface moment `FSM_t = density_t_m3 * it_m4`, t·m (0 when `fill == 0` or `fill == 1`).
    pub fsm_t_m: f64,
}

/// Full loading-condition evaluation: weight breakdown, solid and fluid KG/GM,
/// solved floating condition, GZ cross curves and IMO A.749 criteria.
#[derive(Debug, Clone, Serialize)]
pub struct LoadingResult {
    pub name: String,
    /// Total mass (lightship + items + tanks) in tonnes.
    pub displacement_t: f64,
    /// Longitudinal centre of gravity in model `x`, metres.
    pub lcg: f64,
    /// Transverse centre of gravity in model `y`, metres (+ to starboard).
    pub tcg: f64,
    /// Vertical centre of gravity in model `z`, metres.
    pub vcg_z: f64,
    /// Solid vertical centre of gravity above the keel, metres.
    pub kg: f64,
    /// Solid vertical centre of gravity above the keel, metres (alias of `kg`).
    pub kg_solid: f64,
    /// Total free-surface moment across all tanks, t·m.
    pub fsm_t_m: f64,
    /// Free-surface correction `FSC = FSM / displacement_t`, metres.
    pub fsc_m: f64,
    /// Virtual (fluid) vertical centre of gravity above the keel `KG_fluid = KG + FSC`, metres.
    pub kg_fluid: f64,
    /// Per-item and per-tank breakdown table.
    pub breakdown: Vec<BreakdownRow>,
    /// Detailed per-tank breakdown.
    pub tanks: Vec<TankSummary>,
    /// Solved upright equilibrium floating condition (`draft`, `trim` positive bow-up).
    pub floating: FloatingCondition,
    /// Solved draft above the keel, metres.
    pub draft: f64,
    /// Solved trim, metres (positive bow-up).
    pub trim_m: f64,
    /// Estimated list angle in degrees (+ to starboard) from `tan φ = TCG / GM_fluid`.
    pub list_deg: f64,
    /// Submerged volume at the solved floating condition, m³.
    pub volume_m3: f64,
    /// Hydrostatic displacement at the solved floating condition, tonnes.
    pub hydro_displacement_t: f64,
    /// Longitudinal centre of buoyancy in model `x`, metres.
    pub lcb_x: f64,
    /// Longitudinal centre of flotation in model `x`, metres.
    pub lcf_x: f64,
    /// Waterline length, metres.
    pub lwl: f64,
    /// Waterline beam, metres.
    pub bwl: f64,
    /// Vertical centre of buoyancy above the keel, metres.
    pub kb: f64,
    /// Transverse metacentric radius, metres.
    pub bm_transverse: f64,
    /// Transverse metacentre height above the keel, metres.
    pub km_transverse: f64,
    /// Solid transverse metacentric height `KM - KG_solid`, metres.
    pub gm_solid: f64,
    /// Fluid transverse metacentric height `KM - KG_fluid`, metres.
    pub gm_fluid: f64,
    /// Cross curves evaluated at `displacement_t` with `KG_fluid` and free trim about `LCG`.
    pub cross_curves: Vec<CrossCurve>,
    /// IMO A.749 / 2008 IS Code Part A general intact stability criteria evaluated with `KG_fluid`.
    pub imo_criteria: Option<ImoCriteria>,
    /// IMO 2008 IS Code Part A §2.3 severe wind and rolling (weather) criterion evaluated with `KG_fluid`.
    pub weather: Option<WeatherResult>,
}

/// Validate a [`LoadingCondition`] before evaluation.
pub fn validate(cond: &LoadingCondition) -> Result<(), String> {
    let check_item = |w: &WeightItem, label: &str| -> Result<(), String> {
        if !w.mass_t.is_finite() || w.mass_t < 0.0 {
            return Err(format!(
                "{label} `{}`: `mass_t` must be finite and non-negative",
                w.name
            ));
        }
        if !w.x.is_finite() || !w.y.is_finite() || !w.z.is_finite() {
            return Err(format!(
                "{label} `{}`: centroid coordinates must be finite",
                w.name
            ));
        }
        Ok(())
    };
    check_item(&cond.lightship, "lightship")?;
    for item in &cond.items {
        check_item(item, "item")?;
    }
    for t in &cond.tanks {
        let coords = [t.x_min, t.x_max, t.y_min, t.y_max, t.z_min, t.z_max];
        if coords.iter().any(|v| !v.is_finite()) {
            return Err(format!("tank `{}`: box bounds must be finite", t.name));
        }
        if t.x_max < t.x_min || t.y_max < t.y_min || t.z_max < t.z_min {
            return Err(format!(
                "tank `{}`: max bounds must be >= min bounds",
                t.name
            ));
        }
        if !t.fill.is_finite() || !(0.0..=1.0).contains(&t.fill) {
            return Err(format!("tank `{}`: `fill` must be in 0..=1", t.name));
        }
        if !t.density_t_m3.is_finite() || t.density_t_m3 < 0.0 {
            return Err(format!(
                "tank `{}`: `density_t_m3` must be finite and non-negative",
                t.name
            ));
        }
        if !t.permeability.is_finite() || !(0.0..=1.0).contains(&t.permeability) {
            return Err(format!(
                "tank `{}`: `permeability` must be in 0..=1",
                t.name
            ));
        }
    }
    for w in &cond.windage_extra {
        if !w.area_m2.is_finite() || w.area_m2 < 0.0 {
            return Err(format!(
                "windage `{}`: `area_m2` must be finite and non-negative",
                w.name
            ));
        }
        if !w.z_centroid.is_finite() {
            return Err(format!("windage `{}`: `z_centroid` must be finite", w.name));
        }
    }
    if let BilgeKind::Keels { area_m2 } = cond.bilge {
        if !area_m2.is_finite() || area_m2 < 0.0 {
            return Err("bilge `keels`: `area_m2` must be finite and non-negative".into());
        }
    }
    if let Some(df) = cond.downflooding_deg {
        if !df.is_finite() || df <= 0.0 {
            return Err("`downflooding_deg` must be positive and finite".into());
        }
    }
    let total_mass = cond.lightship.mass_t
        + cond.items.iter().map(|i| i.mass_t).sum::<f64>()
        + cond
            .tanks
            .iter()
            .map(|t| {
                (t.x_max - t.x_min).max(0.0)
                    * (t.y_max - t.y_min).max(0.0)
                    * (t.z_max - t.z_min).max(0.0)
                    * t.fill.clamp(0.0, 1.0)
                    * t.permeability.clamp(0.0, 1.0)
                    * t.density_t_m3.max(0.0)
            })
            .sum::<f64>();
    if total_mass <= 0.0 {
        return Err("loading condition has zero total displacement".into());
    }
    Ok(())
}

/// Find the hull's transverse span `(y_min, y_max)` where the closed station
/// polygons cross height `z`.
fn hull_y_span_at(polys: &[Vec<Vec3>], z: f64) -> Option<(f64, f64)> {
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;
    for poly in polys {
        let n = poly.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            let za = a.z - z;
            let zb = b.z - z;
            if (za < 0.0 && zb >= 0.0) || (za >= 0.0 && zb < 0.0) {
                let t = za / (za - zb);
                let y = a.y + (b.y - a.y) * t;
                if y.is_finite() {
                    y_min = y_min.min(y);
                    y_max = y_max.max(y);
                }
            }
        }
    }
    if y_max > y_min {
        Some((y_min, y_max))
    } else {
        None
    }
}

/// Evaluate one [`Tank`] in the upright condition, either as an exact box
/// (`clip_to_hull == false` or `hull == None`) or intersected with the refined
/// hull's moulded volume (`clip_to_hull == true`).
pub fn evaluate_tank(tank: &Tank, hull: Option<&Hull>) -> TankSummary {
    let l = (tank.x_max - tank.x_min).max(0.0);
    let b = (tank.y_max - tank.y_min).max(0.0);
    let h = (tank.z_max - tank.z_min).max(0.0);
    let fill = tank.fill.clamp(0.0, 1.0);
    let perm = tank.permeability.clamp(0.0, 1.0);
    let rho = tank.density_t_m3.max(0.0);
    let slack = fill > 0.0 && fill < 1.0;

    if !tank.clip_to_hull || hull.is_none() {
        let moulded_volume_m3 = l * b * h;
        let capacity_m3 = moulded_volume_m3 * perm;
        let volume_m3 = capacity_m3 * fill;
        let mass_t = volume_m3 * rho;
        let surface_z = tank.z_min + fill * h;
        let x = 0.5 * (tank.x_min + tank.x_max);
        let y = 0.5 * (tank.y_min + tank.y_max);
        let z = tank.z_min + 0.5 * fill * h;
        let it_m4 = if slack { l * b * b * b / 12.0 } else { 0.0 };
        let fsm_t_m = rho * it_m4;
        return TankSummary {
            name: tank.name.clone(),
            fill,
            density_t_m3: rho,
            permeability: perm,
            clip_to_hull: tank.clip_to_hull,
            moulded_volume_m3,
            capacity_m3,
            volume_m3,
            mass_t,
            x,
            y,
            z,
            surface_z,
            it_m4,
            fsm_t_m,
        };
    }

    let hull = hull.unwrap();
    let xa = tank.x_min.max(hull.model_min.x);
    let xb = tank.x_max.min(hull.model_max.x);
    let za = tank.z_min.max(hull.keel);
    let zb = tank.z_max.min(hull.model_max.z);

    if xb <= xa || zb <= za || b <= 0.0 {
        let x = 0.5 * (tank.x_min + tank.x_max);
        let y = 0.5 * (tank.y_min + tank.y_max);
        return TankSummary {
            name: tank.name.clone(),
            fill,
            density_t_m3: rho,
            permeability: perm,
            clip_to_hull: true,
            moulded_volume_m3: 0.0,
            capacity_m3: 0.0,
            volume_m3: 0.0,
            mass_t: 0.0,
            x,
            y,
            z: tank.z_min,
            surface_z: tank.z_min,
            it_m4: 0.0,
            fsm_t_m: 0.0,
        };
    }

    const NX: usize = 32;
    const NZ: usize = 40;
    let dx = (xb - xa) / NX as f64;
    let dz = (zb - za) / NZ as f64;

    let stations: Vec<(f64, Vec<Vec<Vec3>>)> = (0..NX)
        .map(|i| {
            let x = xa + (i as f64 + 0.5) * dx;
            let (_, polys) = station_polygons(hull, x);
            (x, polys)
        })
        .collect();

    // Per-z-layer volume and first moments (dv, mx, my, z_mid).
    let mut layers: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(NZ);
    let mut raw_moulded_vol = 0.0;

    for k in 0..NZ {
        let z = za + (k as f64 + 0.5) * dz;
        let mut area = 0.0;
        let mut mx_area = 0.0;
        let mut my_area = 0.0;
        for (x, polys) in &stations {
            if let Some((hy0, hy1)) = hull_y_span_at(polys, z) {
                let y0 = tank.y_min.max(hy0);
                let y1 = tank.y_max.min(hy1);
                if y1 > y0 {
                    let w = y1 - y0;
                    let yc = 0.5 * (y0 + y1);
                    let da = w * dx;
                    area += da;
                    mx_area += x * da;
                    my_area += yc * da;
                }
            }
        }
        let dv = area * dz;
        raw_moulded_vol += dv;
        layers.push((dv, mx_area * dz, my_area * dz, z));
    }

    // The clipped tank sits inside the hull slab [za, zb] and inside the box,
    // so its geometric volume can never exceed either the hull's moulded volume
    // in that slab or the unclipped box volume.
    let hull_slab_vol = (hull.displaced_volume(zb - hull.keel, 0.0, 0.0)
        - hull.displaced_volume((za - hull.keel).max(0.0), 0.0, 0.0))
    .max(0.0);
    let max_geom_vol = hull_slab_vol.min(l * b * h);
    let vol_scale = if raw_moulded_vol > max_geom_vol && raw_moulded_vol > 1e-12 {
        max_geom_vol / raw_moulded_vol
    } else {
        1.0
    };
    let moulded_volume_m3 = raw_moulded_vol * vol_scale;
    let capacity_m3 = moulded_volume_m3 * perm;
    let volume_m3 = capacity_m3 * fill;
    let mass_t = volume_m3 * rho;

    if fill <= 0.0 || raw_moulded_vol <= 1e-12 {
        return TankSummary {
            name: tank.name.clone(),
            fill,
            density_t_m3: rho,
            permeability: perm,
            clip_to_hull: true,
            moulded_volume_m3,
            capacity_m3,
            volume_m3: 0.0,
            mass_t: 0.0,
            x: 0.5 * (xa + xb),
            y: 0.5 * (tank.y_min + tank.y_max),
            z: za,
            surface_z: za,
            it_m4: 0.0,
            fsm_t_m: 0.0,
        };
    }

    let target_raw_vol = fill * raw_moulded_vol;
    let mut acc_vol = 0.0;
    let mut acc_mx = 0.0;
    let mut acc_my = 0.0;
    let mut acc_mz = 0.0;
    let mut surface_z = zb;

    for (k, &(dv, mx, my, z_mid)) in layers.iter().enumerate() {
        if dv <= 1e-15 {
            continue;
        }
        if acc_vol + dv >= target_raw_vol - 1e-12 {
            let alpha = ((target_raw_vol - acc_vol) / dv).clamp(0.0, 1.0);
            let z_base = za + k as f64 * dz;
            surface_z = z_base + alpha * dz;
            let z_part = z_base + 0.5 * alpha * dz;
            acc_vol += alpha * dv;
            acc_mx += alpha * mx;
            acc_my += alpha * my;
            acc_mz += alpha * dv * z_part;
            break;
        }
        acc_vol += dv;
        acc_mx += mx;
        acc_my += my;
        acc_mz += dv * z_mid;
    }

    let (x, y, z) = if acc_vol > 1e-12 {
        (acc_mx / acc_vol, acc_my / acc_vol, acc_mz / acc_vol)
    } else {
        (0.5 * (xa + xb), 0.5 * (tank.y_min + tank.y_max), za)
    };

    // Free-surface transverse second moment about the liquid surface's own
    // centroidal fore-aft axis at `surface_z`.
    let it_m4 = if slack {
        let z_eval = surface_z.clamp(za + 0.5 * dz, zb - 0.5 * dz);
        let mut strips: Vec<(f64, f64)> = Vec::with_capacity(NX);
        let mut fs_area = 0.0;
        let mut fs_my = 0.0;
        for (_, polys) in &stations {
            if let Some((hy0, hy1)) = hull_y_span_at(polys, z_eval) {
                let y0 = tank.y_min.max(hy0);
                let y1 = tank.y_max.min(hy1);
                if y1 > y0 {
                    let w = y1 - y0;
                    let yc = 0.5 * (y0 + y1);
                    fs_area += w * dx;
                    fs_my += yc * w * dx;
                    strips.push((w, yc));
                }
            }
        }
        if fs_area > 1e-12 {
            let y_bar = fs_my / fs_area;
            strips
                .iter()
                .map(|&(w, yc)| {
                    let dy = yc - y_bar;
                    (w * w * w / 12.0 + w * dy * dy) * dx
                })
                .sum()
        } else {
            0.0
        }
    } else {
        0.0
    };
    let fsm_t_m = rho * it_m4;

    TankSummary {
        name: tank.name.clone(),
        fill,
        density_t_m3: rho,
        permeability: perm,
        clip_to_hull: true,
        moulded_volume_m3,
        capacity_m3,
        volume_m3,
        mass_t,
        x,
        y,
        z,
        surface_z,
        it_m4,
        fsm_t_m,
    }
}

/// Evaluate a [`LoadingCondition`] on `surface` with hydrostatic options `opts`.
pub fn evaluate(surface: &Surface, opts: &HydroOptions, cond: &LoadingCondition) -> LoadingResult {
    let hull = Hull::new(surface, opts.subdivision);

    let mut breakdown = Vec::with_capacity(1 + cond.items.len() + cond.tanks.len());
    let mut tanks = Vec::with_capacity(cond.tanks.len());

    let mut total_mass = 0.0;
    let mut mx = 0.0;
    let mut my = 0.0;
    let mut mz = 0.0;
    let mut total_fsm = 0.0;

    let mut add_weight =
        |name: String, kind: &str, mass_t: f64, x: f64, y: f64, z: f64, fsm: f64| {
            let m = mass_t.max(0.0);
            total_mass += m;
            mx += m * x;
            my += m * y;
            mz += m * z;
            total_fsm += fsm;
            breakdown.push(BreakdownRow {
                name,
                kind: kind.to_string(),
                mass_t: m,
                x,
                y,
                z,
                fsm_t_m: fsm,
            });
        };

    add_weight(
        cond.lightship.name.clone(),
        "lightship",
        cond.lightship.mass_t,
        cond.lightship.x,
        cond.lightship.y,
        cond.lightship.z,
        0.0,
    );

    for item in &cond.items {
        add_weight(
            item.name.clone(),
            "item",
            item.mass_t,
            item.x,
            item.y,
            item.z,
            0.0,
        );
    }

    for tank in &cond.tanks {
        let ts = evaluate_tank(tank, Some(&hull));
        add_weight(
            ts.name.clone(),
            "tank",
            ts.mass_t,
            ts.x,
            ts.y,
            ts.z,
            ts.fsm_t_m,
        );
        tanks.push(ts);
    }

    let (lcg, tcg, vcg_z) = if total_mass > 1e-12 {
        (mx / total_mass, my / total_mass, mz / total_mass)
    } else {
        (0.0, 0.0, hull.keel)
    };

    let kg_solid = vcg_z - hull.keel;
    let fsc_m = if total_mass > 1e-12 {
        total_fsm / total_mass
    } else {
        0.0
    };
    let kg_fluid = kg_solid + fsc_m;

    let density = opts.water_density.max(1e-6);
    let app_coeff = opts.appendage_coefficient.max(1e-6);
    let target_volume = total_mass * 1000.0 / (density * app_coeff);

    let floating = if target_volume > 1e-12 {
        solve_floating(&hull, target_volume, 0.0, lcg, opts.trim)
            .or_else(|| {
                solve_draft(&hull, target_volume, opts.trim, 0.0).map(|draft| FloatingCondition {
                    draft,
                    trim: opts.trim,
                })
            })
            .unwrap_or(FloatingCondition {
                draft: opts.draft,
                trim: opts.trim,
            })
    } else {
        FloatingCondition {
            draft: 0.0,
            trim: 0.0,
        }
    };

    let mut solved_opts = opts.clone();
    solved_opts.draft = floating.draft;
    solved_opts.trim = floating.trim;
    solved_opts.heel_deg = 0.0;
    let hydro = hull.hydrostatics(&solved_opts);

    let gm_solid = hydro.km_transverse - kg_solid;
    let gm_fluid = hydro.km_transverse - kg_fluid;
    let list_deg = if tcg.abs() <= 1e-12 {
        0.0
    } else if gm_fluid > 1e-9 {
        (tcg / gm_fluid).atan().to_degrees()
    } else {
        90.0_f64.copysign(tcg)
    };

    let (curves, imo_criteria, weather) = if total_mass > 1e-12 {
        let stab_cond = StabilityCondition {
            kg: kg_fluid,
            trim: floating.trim,
            lcg_x: Some(lcg),
        };
        let heels = [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 75.0, 90.0];
        let c = cross_curves(&hull, &heels, &[total_mass], &stab_cond, &solved_opts);
        // GM0 is the metacentric GM_fluid, known exactly here.
        let imo = evaluate_imo_criteria(&c, 0).map(|i| i.with_metacentric_gm(gm_fluid));

        let wh = WeatherHydro::from_hydrostatics(&hydro, kg_fluid);
        let windage = hull_windage_on_hull(&hull, floating.draft, floating.trim);
        let fine_gz = sample_weather_gz(&hull, total_mass, &stab_cond, &solved_opts);
        let w_input = WeatherInput {
            windage_extra: cond.windage_extra.clone(),
            bilge: cond.bilge,
            downflooding_deg: cond.downflooding_deg,
            ..WeatherInput::default()
        };
        let w_res = evaluate_weather(&fine_gz, &wh, windage, &w_input);
        (c, imo, Some(w_res))
    } else {
        (Vec::new(), None, None)
    };

    LoadingResult {
        name: cond.name.clone(),
        displacement_t: total_mass,
        lcg,
        tcg,
        vcg_z,
        kg: kg_solid,
        kg_solid,
        fsm_t_m: total_fsm,
        fsc_m,
        kg_fluid,
        breakdown,
        tanks,
        floating,
        draft: floating.draft,
        trim_m: floating.trim,
        list_deg,
        volume_m3: hydro.volume_m3,
        hydro_displacement_t: hydro.displacement_t,
        lcb_x: hydro.lcb.x,
        lcf_x: hydro.lcf.x,
        lwl: hydro.lwl,
        bwl: hydro.bwl,
        kb: hydro.kb,
        bm_transverse: hydro.bm_transverse,
        km_transverse: hydro.km_transverse,
        gm_solid,
        gm_fluid,
        cross_curves: curves,
        imo_criteria,
        weather,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subdiv::{SubdivisionMode, SubdivisionOptions};

    #[test]
    fn box_tank_volume_and_centroid_at_half_fill() {
        let tank = Tank {
            name: "Day tank".into(),
            x_min: 2.0,
            x_max: 6.0,
            y_min: -1.5,
            y_max: 1.5,
            z_min: 0.4,
            z_max: 2.4,
            fill: 0.5,
            density_t_m3: 0.85,
            permeability: 1.0,
            clip_to_hull: false,
        };
        let eval = evaluate_tank(&tank, None);
        // Box is 4 m x 3 m x 2 m = 24 m3; at fill 0.5 -> 12 m3, mass 10.2 t.
        assert!((eval.moulded_volume_m3 - 24.0).abs() < 1e-12);
        assert!((eval.volume_m3 - 12.0).abs() < 1e-12);
        assert!((eval.mass_t - 10.2).abs() < 1e-12);
        // Centroid of bottom half: x = 4.0, y = 0.0, z = 0.4 + 0.5 * (0.5 * 2.0) = 0.9 m.
        assert!((eval.x - 4.0).abs() < 1e-12);
        assert!((eval.y - 0.0).abs() < 1e-12);
        assert!((eval.z - 0.9).abs() < 1e-12);
        assert!((eval.surface_z - 1.4).abs() < 1e-12);
    }

    #[test]
    fn rectangular_tank_fsm_is_exact_and_zero_when_empty_or_pressed_up() {
        let make = |fill: f64| Tank {
            name: "Ballast".into(),
            x_min: 1.0,
            x_max: 5.0, // l = 4.0
            y_min: -1.2,
            y_max: 1.2, // b = 2.4
            z_min: 0.0,
            z_max: 1.5,
            fill,
            density_t_m3: 1.025,
            permeability: 0.98,
            clip_to_hull: false,
        };
        let want_it = 4.0 * 2.4_f64.powi(3) / 12.0;
        let want_fsm = 1.025 * want_it;

        for fill in [0.1, 0.5, 0.9] {
            let e = evaluate_tank(&make(fill), None);
            assert!(
                (e.it_m4 - want_it).abs() < 1e-12,
                "fill {fill}: it {}",
                e.it_m4
            );
            assert!(
                (e.fsm_t_m - want_fsm).abs() < 1e-12,
                "fill {fill}: fsm {}",
                e.fsm_t_m
            );
        }

        // Empty (fill == 0) and pressed-up (fill == 1) tanks have zero free-surface moment.
        let empty = evaluate_tank(&make(0.0), None);
        assert_eq!(empty.fsm_t_m, 0.0);
        let full = evaluate_tank(&make(1.0), None);
        assert_eq!(full.fsm_t_m, 0.0);
    }

    #[test]
    fn kg_of_items_is_mass_weighted_mean_and_fsc_is_fsm_over_displacement() {
        let surface = crate::demo::demo_hull(false);
        let opts = HydroOptions::from_surface(
            &surface,
            SubdivisionOptions {
                level: 2,
                mode: SubdivisionMode::Loop,
            },
        );
        let cond = LoadingCondition {
            name: "Analytic weights".into(),
            lightship: WeightItem {
                name: "Lightship".into(),
                mass_t: 6.0,
                x: 4.0,
                y: 0.0,
                z: 1.2,
            },
            items: vec![
                WeightItem {
                    name: "Cargo A".into(),
                    mass_t: 2.0,
                    x: 5.0,
                    y: 0.3,
                    z: 0.6,
                },
                WeightItem {
                    name: "Cargo B".into(),
                    mass_t: 2.0,
                    x: 4.0,
                    y: -0.3,
                    z: 1.5,
                },
            ],
            tanks: vec![Tank {
                name: "Slack tank".into(),
                x_min: 3.0,
                x_max: 5.0, // l = 2.0
                y_min: -0.6,
                y_max: 0.6, // b = 1.2
                z_min: 0.1,
                z_max: 1.1, // h = 1.0
                fill: 0.5,
                density_t_m3: 1.0,
                permeability: 1.0,
                clip_to_hull: false,
            }],
            windage_extra: Vec::new(),
            bilge: BilgeKind::Round,
            downflooding_deg: None,
        };
        let res = evaluate(&surface, &opts, &cond);

        // Tank: V = 2 * 1.2 * 1.0 * 0.5 = 1.2 m3, mass = 1.2 t, x = 4.0, y = 0.0, z = 0.1 + 0.25 = 0.35 m.
        // Total mass = 6 + 2 + 2 + 1.2 = 11.2 t.
        let want_mass = 11.2;
        let want_lcg = (6.0 * 4.0 + 2.0 * 5.0 + 2.0 * 4.0 + 1.2 * 4.0) / want_mass;
        let want_tcg = (6.0 * 0.0 + 2.0 * 0.3 + 2.0 * (-0.3) + 1.2 * 0.0) / want_mass;
        let want_kg = (6.0 * 1.2 + 2.0 * 0.6 + 2.0 * 1.5 + 1.2 * 0.35) / want_mass;
        let want_fsm = 1.0 * (2.0 * 1.2_f64.powi(3) / 12.0);
        let want_fsc = want_fsm / want_mass;

        assert!((res.displacement_t - want_mass).abs() < 1e-12);
        assert!((res.lcg - want_lcg).abs() < 1e-12);
        assert!((res.tcg - want_tcg).abs() < 1e-12);
        assert!((res.kg_solid - want_kg).abs() < 1e-12);
        assert!((res.fsm_t_m - want_fsm).abs() < 1e-12);
        assert!((res.fsc_m - want_fsc).abs() < 1e-12);
        assert!((res.kg_fluid - (want_kg + want_fsc)).abs() < 1e-12);
        assert!((res.gm_solid - res.gm_fluid - want_fsc).abs() < 1e-12);
        assert!(res.gm_fluid < res.gm_solid);
    }

    #[test]
    fn hull_clipped_tank_volume_never_exceeds_hull_moulded_volume() {
        let surface = crate::demo::demo_hull(false);
        let subdiv = SubdivisionOptions {
            level: 3,
            mode: SubdivisionMode::Loop,
        };
        let hull = Hull::new(&surface, subdiv);
        let hull_vol_at_062 = hull.displaced_volume(0.62, 0.0, 0.0);

        // Tank bounding box encloses the entire hull up to z = 0.62 m.
        let huge_tank = Tank {
            name: "Bounding tank".into(),
            x_min: -5.0,
            x_max: 20.0,
            y_min: -10.0,
            y_max: 10.0,
            z_min: 0.0,
            z_max: 0.62,
            fill: 1.0,
            density_t_m3: 1.025,
            permeability: 1.0,
            clip_to_hull: true,
        };
        let clipped = evaluate_tank(&huge_tank, Some(&hull));
        let unclipped = evaluate_tank(
            &Tank {
                clip_to_hull: false,
                ..huge_tank.clone()
            },
            Some(&hull),
        );

        assert!(
            clipped.moulded_volume_m3 <= hull_vol_at_062 + 1e-12,
            "clipped volume {} exceeded hull moulded volume {}",
            clipped.moulded_volume_m3,
            hull_vol_at_062
        );
        assert!(clipped.volume_m3 <= hull_vol_at_062 + 1e-12);
        assert!(
            (clipped.moulded_volume_m3 - hull_vol_at_062).abs() / hull_vol_at_062 < 0.02,
            "clipped volume {} should closely match hull moulded volume {}",
            clipped.moulded_volume_m3,
            hull_vol_at_062
        );
        assert!(clipped.moulded_volume_m3 < unclipped.moulded_volume_m3 * 0.1);
    }

    #[test]
    fn demo_loadcase_floats_at_equilibrium_displacement_and_lcb_matches_lcg() {
        let surface = crate::demo::demo_hull(false);
        let opts = HydroOptions::from_surface(
            &surface,
            SubdivisionOptions {
                level: 3,
                mode: SubdivisionMode::Loop,
            },
        );
        let json_text = include_str!("../../../examples/demo-loadcase.json");
        let cond: LoadingCondition =
            serde_json::from_str(json_text).expect("valid demo-loadcase.json");
        validate(&cond).expect("valid condition");

        let res = evaluate(&surface, &opts, &cond);
        let rel_disp_err =
            (res.hydro_displacement_t - res.displacement_t).abs() / res.displacement_t;
        assert!(
            rel_disp_err < 0.005,
            "hydro displacement {} vs summed mass {} (rel err {})",
            res.hydro_displacement_t,
            res.displacement_t,
            rel_disp_err
        );
        let lcb_err_over_lwl = (res.lcb_x - res.lcg).abs() / res.lwl;
        assert!(
            lcb_err_over_lwl < 0.01,
            "LCB {} vs LCG {} (LWL {}, ratio {})",
            res.lcb_x,
            res.lcg,
            res.lwl,
            lcb_err_over_lwl
        );
        assert!(
            (res.draft - 0.62).abs() < 0.02,
            "draft {} near 0.62 m",
            res.draft
        );
        assert!(res.gm_fluid < res.gm_solid);
        assert!(res.imo_criteria.is_some());
        // The GM0 criterion uses the metacentric GM_fluid, not the GZ secant.
        assert_eq!(res.imo_criteria.as_ref().unwrap().gm0_m, res.gm_fluid);
    }
}
