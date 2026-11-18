//! Contour lines of a scalar on a regular grid, by marching squares.

/// One contour segment: two points in the grid's coordinates.
pub type Segment = [(f64, f64); 2];

/// The segments where `values` crosses `level` on the grid `xs` x `ys` (both ascending), with
/// `values[iy * xs.len() + ix]`. A cell with any corner not `valid` is skipped. A saddle cell (two
/// opposite corners above the level) is split by the mean of its corners. Segments are not joined.
pub fn segments(
    xs: &[f64],
    ys: &[f64],
    values: &[f64],
    valid: &[bool],
    level: f64,
) -> Vec<Segment> {
    let nx = xs.len();
    let mut out = Vec::new();
    if nx < 2 || ys.len() < 2 || values.len() != nx * ys.len() || valid.len() != values.len() {
        return out;
    }
    for iy in 0..ys.len() - 1 {
        for ix in 0..nx - 1 {
            // Corners counter-clockwise from the lower left: a (ix, iy), b, c, d.
            let idx = [
                iy * nx + ix,
                iy * nx + ix + 1,
                (iy + 1) * nx + ix + 1,
                (iy + 1) * nx + ix,
            ];
            if idx.iter().any(|&k| !valid[k] || !values[k].is_finite()) {
                continue;
            }
            let v = idx.map(|k| values[k]);
            let p = [
                (xs[ix], ys[iy]),
                (xs[ix + 1], ys[iy]),
                (xs[ix + 1], ys[iy + 1]),
                (xs[ix], ys[iy + 1]),
            ];
            let above = v.map(|value| value > level);
            // Edge e joins corners e and e + 1: bottom, right, top, left.
            let cross = |e: usize| {
                let (i, j) = (e, (e + 1) % 4);
                let t = (level - v[i]) / (v[j] - v[i]);
                (
                    p[i].0 + t * (p[j].0 - p[i].0),
                    p[i].1 + t * (p[j].1 - p[i].1),
                )
            };
            let crossed: Vec<usize> = (0..4).filter(|&e| above[e] != above[(e + 1) % 4]).collect();
            match crossed.len() {
                2 => out.push([cross(crossed[0]), cross(crossed[1])]),
                4 => {
                    // Corner k lies between edges k - 1 and k. A centre on the same side as corner a
                    // joins a and c, so the level cuts off b and d; otherwise it cuts off a and c.
                    let mean = v.iter().sum::<f64>() / 4.0;
                    let cut_off = if (mean > level) == above[0] {
                        [1, 3]
                    } else {
                        [0, 2]
                    };
                    for k in cut_off {
                        out.push([cross((k + 3) % 4), cross(k)]);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// values = x on a 5 x 4 grid: the level 2.5 crosses each of the 3 rows of cells once.
    fn plane() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let xs: Vec<f64> = (0..5).map(f64::from).collect();
        let ys: Vec<f64> = (0..4).map(f64::from).collect();
        let values = (0..4).flat_map(|_| xs.clone()).collect();
        (xs, ys, values)
    }

    #[test]
    fn contour_of_a_plane_is_one_straight_line() {
        let (xs, ys, v) = plane();
        let segs = segments(&xs, &ys, &v, &vec![true; v.len()], 2.5);
        assert_eq!(segs.len(), 3, "{segs:?}");
        for s in &segs {
            for p in s {
                assert!((p.0 - 2.5).abs() < 1e-12, "{segs:?}");
            }
        }
    }

    #[test]
    fn contour_skips_invalid_cells() {
        let (xs, ys, v) = plane();
        let mut valid = vec![true; v.len()];
        valid[xs.len() + 2] = false; // the corner (2, 1) touches cells (1..=2, 0..=1)
        let segs = segments(&xs, &ys, &v, &valid, 2.5);
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert!(segs[0].iter().all(|p| p.1 >= 2.0), "{segs:?}");
    }

    /// A saddle: corners above at (0, 0) and (1, 1). A high centre joins them, so the level cuts off
    /// the two low corners; a low centre cuts off the two high ones.
    #[test]
    fn saddles_are_split_by_the_cell_mean() {
        let (xs, ys) = (vec![0.0, 1.0], vec![0.0, 1.0]);
        let valid = vec![true; 4];
        let high = segments(&xs, &ys, &[1.0, 0.0, 0.0, 1.0], &valid, 0.4);
        let low = segments(&xs, &ys, &[1.0, 0.0, 0.0, 1.0], &valid, 0.6);
        assert_eq!((high.len(), low.len()), (2, 2));
        // Each segment of `high` encloses a low corner: (1, 0) or (0, 1).
        let near = |s: &[(f64, f64); 2], c: (f64, f64)| {
            s.iter()
                .all(|p| (p.0 - c.0).abs() + (p.1 - c.1).abs() < 1.0 + 1e-12)
        };
        assert!(
            high.iter().any(|s| near(s, (1.0, 0.0))) && high.iter().any(|s| near(s, (0.0, 1.0)))
        );
        assert!(low.iter().any(|s| near(s, (0.0, 0.0))) && low.iter().any(|s| near(s, (1.0, 1.0))));
    }
}
