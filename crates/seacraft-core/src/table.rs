//! Hydrostatics table: the classic "offsets of the form" page, one row per
//! draft.
//!
//! Every surveyor's set of lines comes with a table of hydrostatics: draft down
//! the side, and across the page displacement, `KB`, `BM`, `KM`, `LCB`, `LCF`,
//! the coefficients, tonnes per centimetre of immersion and the moment to
//! change trim by one centimetre. This module produces exactly that, by running
//! [`hydro::compute`] once per draft on the refined mesh — the table is a
//! *report* of the same measurement the single-point command makes, so a row
//! can never disagree with `seacraft hydro`.
//!
//! Two derived columns are computed here rather than in [`hydro`] because they
//! are table quantities, not hull properties:
//!
//! ```text
//! TPC  = A_w · ρ / 100          tonnes per centimetre of immersion
//! MTC  = ρ · I_L / (100 · L)    tonnes·metres to change trim one centimetre
//! ```
//!
//! `TPC` follows from the waterplane area and the density; `MTC` from the
//! longitudinal waterplane inertia and the waterline length (equivalently
//! `Δ · GM_L / (100 · L)`), the standard small-trim result. The density inside
//! the core is kg/m³, so both carry a `/1000` to land in tonnes — without it
//! every column reads a thousand times too large.

use std::fmt::Write as _;

use serde::Serialize;

use crate::hydro::{self, HydroOptions};
use crate::mesh::Surface;

/// One draft's worth of the table.
#[derive(Debug, Clone, Serialize)]
pub struct HydrostaticRow {
    pub draft: f64,
    pub volume_m3: f64,
    pub displacement_t: f64,
    pub kb: f64,
    pub bm_transverse: f64,
    pub bm_longitudinal: f64,
    pub km_transverse: f64,
    pub lcb_x: f64,
    pub lcf_x: f64,
    pub block_coefficient: f64,
    pub prismatic_coefficient: f64,
    pub waterplane_coefficient: f64,
    pub waterplane_area_m2: f64,
    pub lwl: f64,
    pub bwl: f64,
    /// Tonnes per centimetre of immersion, `A_w · ρ / 100`.
    pub tpc: f64,
    /// Moment to change trim one centimetre, `ρ · I_L / (100 · L)`.
    pub mtc: f64,
    pub wetted_surface_m2: f64,
}

/// The whole table.
#[derive(Debug, Clone, Serialize)]
pub struct HydrostaticTable {
    pub name: String,
    pub water_density: f64,
    pub rows: Vec<HydrostaticRow>,
}

/// Drafts from `0` to `draft_max` in `steps` equal increments, skipping zero
/// (a hull displaces nothing at the waterline and every coefficient is 0/0).
pub fn draft_range(draft_max: f64, steps: usize) -> Vec<f64> {
    let n = steps.max(1);
    let mut out = Vec::with_capacity(n);
    for i in 1..=n {
        out.push(draft_max * i as f64 / n as f64);
    }
    out
}

/// Build the table. `opts` supplies density, trim, heel and the subdivision;
/// its `draft` is ignored in favour of `drafts`.
pub fn table(surface: &Surface, opts: &HydroOptions, drafts: &[f64]) -> HydrostaticTable {
    let mut rows = Vec::with_capacity(drafts.len());
    for &draft in drafts {
        if !(draft.is_finite() && draft > 0.0) {
            continue;
        }
        let mut o = opts.clone();
        o.draft = draft;
        let h = hydro::compute(surface, &o);
        // The core works in kg (the hull-numerics skill: "metres and kg inside the core"),
        // so both table quantities need the kilogram-to-tonne factor or they
        // read a thousand times too large.
        let rho_t = h.water_density / 1000.0;
        let tpc = h.waterplane_area_m2 * rho_t / 100.0;
        let mtc = if h.lwl > 1e-9 {
            rho_t * h.waterplane_inertia_longitudinal / (100.0 * h.lwl)
        } else {
            0.0
        };
        rows.push(HydrostaticRow {
            draft,
            volume_m3: h.volume_m3,
            displacement_t: h.displacement_t,
            kb: h.kb,
            bm_transverse: h.bm_transverse,
            bm_longitudinal: h.bm_longitudinal,
            km_transverse: h.km_transverse,
            lcb_x: h.lcb.x,
            lcf_x: h.lcf.x,
            block_coefficient: h.block_coefficient,
            prismatic_coefficient: h.prismatic_coefficient,
            waterplane_coefficient: h.waterplane_coefficient,
            waterplane_area_m2: h.waterplane_area_m2,
            lwl: h.lwl,
            bwl: h.bwl,
            tpc,
            mtc,
            wetted_surface_m2: h.wetted_surface_m2,
        });
    }
    HydrostaticTable {
        name: surface.settings.name.clone(),
        water_density: opts.water_density,
        rows,
    }
}

impl HydrostaticTable {
    /// The row nearest a draft, for a caller that wants to check one point.
    pub fn row_at(&self, draft: f64) -> Option<&HydrostaticRow> {
        self.rows.iter().min_by(|a, b| {
            (a.draft - draft)
                .abs()
                .partial_cmp(&(b.draft - draft).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Long-format CSV: one line per row, fixed column order.
    pub fn csv(&self) -> String {
        let mut out = String::with_capacity(256 + 128 * self.rows.len());
        out.push_str(
            "draft_m,volume_m3,displacement_t,kb_m,bm_m,bml_m,km_m,lcb_x,lcf_x,cb,cp,cw,\
             aw_m2,lwl_m,bwl_m,tpc_t_cm,mtc_t_m_cm,wetted_m2\n",
        );
        for r in &self.rows {
            let _ = writeln!(
                out,
                "{:.3},{:.3},{:.2},{:.4},{:.4},{:.4},{:.4},{:.3},{:.3},{:.4},{:.4},{:.4},\
                 {:.3},{:.3},{:.3},{:.4},{:.3},{:.2}",
                r.draft,
                r.volume_m3,
                r.displacement_t,
                r.kb,
                r.bm_transverse,
                r.bm_longitudinal,
                r.km_transverse,
                r.lcb_x,
                r.lcf_x,
                r.block_coefficient,
                r.prismatic_coefficient,
                r.waterplane_coefficient,
                r.waterplane_area_m2,
                r.lwl,
                r.bwl,
                r.tpc,
                r.mtc,
                r.wetted_surface_m2
            );
        }
        out
    }

    /// A standalone HTML page, the way a lines plan would carry it.
    pub fn html(&self) -> String {
        let mut out = String::with_capacity(2048 + 96 * self.rows.len());
        let _ = write!(
            out,
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <title>Hydrostatics - {name}</title>\n<style>\n\
             body {{ font-family: system-ui, sans-serif; margin: 2rem; color: #1a1a1a; }}\n\
             h1 {{ font-size: 1.2rem; }}\n\
             table {{ border-collapse: collapse; font-variant-numeric: tabular-nums; }}\n\
             th, td {{ padding: 0.25rem 0.6rem; text-align: right; border-bottom: 1px solid #ddd; }}\n\
             th {{ background: #f4f4f4; font-weight: 600; }}\n\
             caption {{ text-align: left; padding-bottom: 0.5rem; font-size: 0.85rem; color: #555; }}\n\
             </style>\n</head>\n<body>\n<h1>{name} - hydrostatics</h1>\n<table>\n\
             <caption>Density {dens:.1} kg/m3. Lengths in metres, displacement in tonnes, \
             TPC in t/cm, MTC in t&middot;m/cm.</caption>\n\
             <thead><tr><th>T</th><th>&nabla;</th><th>&Delta;</th><th>KB</th><th>BM</th>\
             <th>KM</th><th>LCB</th><th>LCF</th><th>Cb</th><th>Cp</th><th>Cw</th>\
             <th>Aw</th><th>TPC</th><th>MTC</th></tr></thead>\n<tbody>\n",
            name = esc(&self.name),
            dens = self.water_density
        );
        for r in &self.rows {
            let _ = writeln!(
                out,
                "<tr><td>{:.2}</td><td>{:.2}</td><td>{:.2}</td><td>{:.3}</td>\
                 <td>{:.3}</td><td>{:.3}</td><td>{:.3}</td><td>{:.3}</td>\
                 <td>{:.3}</td><td>{:.3}</td><td>{:.3}</td><td>{:.2}</td>\
                 <td>{:.3}</td><td>{:.3}</td></tr>",
                r.draft,
                r.volume_m3,
                r.displacement_t,
                r.kb,
                r.bm_transverse,
                r.km_transverse,
                r.lcb_x,
                r.lcf_x,
                r.block_coefficient,
                r.prismatic_coefficient,
                r.waterplane_coefficient,
                r.waterplane_area_m2,
                r.tpc,
                r.mtc
            );
        }
        out.push_str("</tbody>\n</table>\n</body>\n</html>\n");
        out
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::subdiv::SubdivisionOptions;

    fn opts(s: &Surface) -> HydroOptions {
        HydroOptions::from_surface(
            s,
            SubdivisionOptions {
                level: 3,
                mode: Default::default(),
            },
        )
    }

    #[test]
    fn a_row_matches_the_single_point_measurement() {
        let s = demo::demo_hull(false);
        let o = opts(&s);
        let t = table(&s, &o, &[0.62]);
        let row = t.row_at(0.62).expect("a row");
        let mut single = o.clone();
        single.draft = 0.62;
        let h = hydro::compute(&s, &single);
        assert!((row.volume_m3 - h.volume_m3).abs() < 1e-9, "volume");
        assert!((row.km_transverse - h.km_transverse).abs() < 1e-9, "KM");
        assert!((row.lcb_x - h.lcb.x).abs() < 1e-9, "LCB");
        assert!(
            (row.block_coefficient - h.block_coefficient).abs() < 1e-9,
            "Cb"
        );
        // The published demo reference, so the table cannot drift from the
        // single-point command.
        assert!(
            (row.volume_m3 - 11.8508).abs() < 0.05,
            "{:.4}",
            row.volume_m3
        );
        assert!(
            (row.km_transverse - 2.6245).abs() < 0.02,
            "{:.4}",
            row.km_transverse
        );
    }

    #[test]
    fn tpc_and_mtc_are_the_inertia_quantities_and_grow_with_draft() {
        // TPC and MTC are pure functions of the measured waterplane, so they
        // are checked against the `hydro` fields they come from — including
        // `BM_L · V` for the longitudinal inertia, which is a different code
        // path inside `hydro` than the inertia itself. (A sharp box would make
        // the absolute check possible, but a subdivided box is rounded; the
        // analytic box lives in `tests/analytic.rs` where creases are set up
        // properly.)
        let s = demo::demo_hull(false);
        let o = opts(&s);
        let t = table(&s, &o, &draft_range(0.62, 6));
        assert_eq!(t.rows.len(), 6);
        for r in &t.rows {
            assert!(
                (r.tpc - r.waterplane_area_m2 * (t.water_density / 1000.0) / 100.0).abs() < 1e-12,
                "{r:?}"
            );
            let il = r.bm_longitudinal * r.volume_m3;
            let mtc = t.water_density / 1000.0 * il / (100.0 * r.lwl);
            assert!(
                (r.mtc - mtc).abs() / mtc.abs().max(1.0) < 0.02,
                "MTC {:.4} vs {:.4} from BML*V",
                r.mtc,
                mtc
            );
            // Physical magnitudes: a 12 m boat immerses a few tonnes per metre
            // of length, so 0.1..1.0 t/cm and 0.05..5 t*m/cm. A missing
            // kg-to-tonne factor is a factor of 1000 and would fail here.
            assert!(
                (0.05..1.5).contains(&r.tpc),
                "TPC {:.4} t/cm is not plausible",
                r.tpc
            );
            assert!(
                (0.02..5.0).contains(&r.mtc),
                "MTC {:.4} t*m/cm is not plausible",
                r.mtc
            );
            assert!(r.displacement_t > 0.0);
        }
        // Volume and displacement rise with draft, and the top row is the
        // design draft the single-point command reports.
        assert!(t.rows.windows(2).all(|w| w[1].volume_m3 > w[0].volume_m3));
        let top = t.rows.last().expect("a row");
        assert!((top.draft - 0.62).abs() < 1e-12);
        assert!(
            (top.volume_m3 - 11.8508).abs() < 0.05,
            "{:.4}",
            top.volume_m3
        );
    }

    #[test]
    fn draft_range_skips_zero_and_reaches_the_top() {
        let d = draft_range(0.6, 6);
        assert_eq!(d.len(), 6);
        assert!((d[0] - 0.1).abs() < 1e-12);
        assert!((d[5] - 0.6).abs() < 1e-12);
        assert!(d.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn csv_and_html_are_well_formed() {
        let s = demo::demo_hull(false);
        let t = table(&s, &opts(&s), &[0.2, 0.62]);
        let csv = t.csv();
        assert_eq!(csv.lines().count(), 3, "header plus two rows");
        assert!(csv.starts_with("draft_m,volume_m3"));
        assert!(csv.lines().nth(2).expect("row").starts_with("0.620,"));
        let html = t.html();
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert_eq!(html.matches("<tr>").count(), 3, "header plus two rows");
        assert!(html.contains("Hydrostatics"));
        assert!(html.trim_end().ends_with("</html>"));
    }

    #[test]
    fn non_positive_drafts_are_skipped() {
        let s = demo::demo_hull(false);
        let t = table(&s, &opts(&s), &[0.0, -1.0, 0.3]);
        assert_eq!(t.rows.len(), 1);
        assert!((t.rows[0].draft - 0.3).abs() < 1e-12);
    }
}
