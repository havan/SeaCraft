//! Moving control points: one rule for the window, the control channel and
//! the MCP server.
//!
//! A [`MovePlan`] is made once, when a move starts, from the points being
//! moved (the *seeds*) and, for connected editing, the points that follow them
//! with a weight. It remembers where every point started, so the same plan
//! answers "where is everything for this displacement" as often as a drag asks,
//! without drift, and a zero displacement puts every point back exactly.
//!
//! Two rules protect a half model. A symmetric layer is modelled for `y >= 0`
//! only and mirrored, so a point on one never crosses to `y < 0`. And a point
//! *on* the centreline of a symmetric layer stays on it when it moves as part
//! of a group or as a follower; only a centreline point moved on its own may
//! leave it.

use crate::math::Vec3;
use crate::mesh::Surface;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

/// One point's move: where it was and where it went.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Moved {
    pub point: usize,
    pub from: Vec3,
    pub to: Vec3,
}

/// Which coordinates a follower moves in. Written as a string of the letters
/// `x`, `y`, `z` in files and messages (`"xyz"`, `"y"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisMask {
    pub x: bool,
    pub y: bool,
    pub z: bool,
}

impl AxisMask {
    pub const ALL: AxisMask = AxisMask {
        x: true,
        y: true,
        z: true,
    };

    /// `v` with the coordinates outside the mask set to zero.
    pub fn apply(self, v: Vec3) -> Vec3 {
        let keep = |on: bool, c: f64| if on { c } else { 0.0 };
        Vec3::new(keep(self.x, v.x), keep(self.y, v.y), keep(self.z, v.z))
    }

    /// Read `"xyz"`, `"y"`, `"zx"`. Empty, or any other letter, is an error.
    pub fn parse(text: &str) -> Result<AxisMask, String> {
        let mut mask = AxisMask {
            x: false,
            y: false,
            z: false,
        };
        for letter in text.chars() {
            let axis = match letter.to_ascii_lowercase() {
                'x' => &mut mask.x,
                'y' => &mut mask.y,
                'z' => &mut mask.z,
                _ => return Err(bad_axes(text)),
            };
            if std::mem::replace(axis, true) {
                return Err(bad_axes(text));
            }
        }
        if mask.to_string().is_empty() {
            return Err(bad_axes(text));
        }
        Ok(mask)
    }
}

fn bad_axes(text: &str) -> String {
    format!(
        "axes must be one or more of \"x\", \"y\", \"z\", each once, like \"xy\"; got \"{text}\""
    )
}

impl Default for AxisMask {
    fn default() -> Self {
        AxisMask::ALL
    }
}

impl std::fmt::Display for AxisMask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (on, letter) in [(self.x, "x"), (self.y, "y"), (self.z, "z")] {
            if on {
                f.write_str(letter)?;
            }
        }
        Ok(())
    }
}

impl Serialize for AxisMask {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AxisMask {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        AxisMask::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// A point this close to `y = 0` on a symmetric layer is on the centreline.
pub const CENTRELINE_EPS: f64 = 1e-9;

/// Whether control point `point` belongs to a face on a symmetric layer
/// (only `y >= 0` modelled), so it must not cross the centreline.
pub fn point_on_symmetric_layer(surface: &Surface, point: usize) -> bool {
    surface.faces.iter().enumerate().any(|(fi, f)| {
        f.contains(&point)
            && surface
                .face_layer
                .get(fi)
                .and_then(|&li| surface.layers.get(li))
                .is_some_and(|l| l.symmetric)
    })
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    point: usize,
    start: Vec3,
    weight: f64,
    mask: AxisMask,
    is_seed: bool,
    /// On a symmetric layer: never below `y = 0`.
    sym: bool,
    /// On the centreline of a symmetric layer and not free to leave it.
    lock_centreline: bool,
}

impl Entry {
    /// Where this point is for the seeds' displacement `delta`; `exact` is the
    /// position asked for outright, for the anchor of a move to a position.
    fn moved(&self, delta: Vec3, exact: Option<Vec3>) -> Moved {
        let mut to = exact.unwrap_or_else(|| {
            if self.is_seed {
                self.start + delta
            } else {
                self.start + self.mask.apply(delta) * self.weight
            }
        });
        if self.lock_centreline {
            to.y = self.start.y;
        } else if self.sym {
            // Never below the centreline; a point that already was stays no
            // lower than it started.
            to.y = to.y.max(self.start.y.min(0.0));
        }
        Moved {
            point: self.point,
            from: self.start,
            to,
        }
    }
}

/// A move in progress: which points move, from where, and by how much of the
/// displacement each.
#[derive(Debug, Clone, Default)]
pub struct MovePlan {
    /// Sorted by point, one entry a point.
    entries: Vec<Entry>,
}

impl MovePlan {
    /// The seeds move together by the whole displacement; nothing follows.
    /// Repeated seeds count once; a seed that is not a point of the surface
    /// is left out.
    pub fn rigid(surface: &Surface, seeds: &[usize]) -> MovePlan {
        MovePlan::weighted(surface, seeds, &[], AxisMask::ALL)
    }

    /// The seeds move by the whole displacement; each follower moves by its
    /// weight times the displacement, in the coordinates of `mask` only.
    /// A follower that is also a seed is a seed. A follower whose weight is
    /// not a positive number, or that is not a point of the surface, is left
    /// out.
    pub(crate) fn weighted(
        surface: &Surface,
        seeds: &[usize],
        followers: &[(usize, f64)],
        mask: AxisMask,
    ) -> MovePlan {
        let mut weight: BTreeMap<usize, (f64, bool)> = BTreeMap::new();
        for &(point, w) in followers {
            if w.is_finite() && w > 0.0 {
                let best = weight.entry(point).or_insert((w, false));
                best.0 = best.0.max(w);
            }
        }
        for &seed in seeds {
            weight.insert(seed, (1.0, true));
        }
        weight.retain(|&point, _| point < surface.points.len());
        let seed_count = weight.values().filter(|w| w.1).count();
        let entries = weight
            .into_iter()
            .filter_map(|(point, (weight, is_seed))| {
                let start = surface.points.get(point)?.p;
                let sym = point_on_symmetric_layer(surface, point);
                Some(Entry {
                    point,
                    start,
                    weight,
                    mask,
                    is_seed,
                    sym,
                    lock_centreline: sym
                        && start.y.abs() <= CENTRELINE_EPS
                        && (!is_seed || seed_count > 1),
                })
            })
            .collect();
        MovePlan { entries }
    }

    /// Where every point of the plan is when the seeds have moved by `delta`
    /// from where they started, in point order.
    pub fn positions(&self, delta: Vec3) -> Vec<Moved> {
        self.entries.iter().map(|e| e.moved(delta, None)).collect()
    }

    /// As [`MovePlan::positions`], for the displacement that takes the seed
    /// `anchor` to `target`; the anchor lands on `target` exactly (no rounding
    /// from going by way of a difference) before the half-model rules apply.
    /// Empty when `anchor` is not a seed of the plan.
    pub fn positions_to(&self, anchor: usize, target: Vec3) -> Vec<Moved> {
        let Some(from) = self.entries.iter().find(|e| e.is_seed && e.point == anchor) else {
            return Vec::new();
        };
        let delta = target - from.start;
        self.entries
            .iter()
            .map(|e| e.moved(delta, (e.point == anchor).then_some(target)))
            .collect()
    }

    /// Write [`MovePlan::positions`] into `surface`. A point the surface no
    /// longer has is skipped.
    pub fn apply(&self, surface: &mut Surface, delta: Vec3) {
        for m in self.positions(delta) {
            if let Some(p) = surface.points.get_mut(m.point) {
                p.p = m.to;
            }
        }
    }

    /// Write [`MovePlan::positions_to`] into `surface`.
    pub fn apply_to(&self, surface: &mut Surface, anchor: usize, target: Vec3) {
        for m in self.positions_to(anchor, target) {
            if let Some(p) = surface.points.get_mut(m.point) {
                p.p = m.to;
            }
        }
    }

    /// Where `point` was when the plan was made, if it is in the plan.
    pub fn start(&self, point: usize) -> Option<Vec3> {
        self.entries
            .binary_search_by_key(&point, |e| e.point)
            .ok()
            .map(|at| self.entries[at].start)
    }

    /// Every point of the plan with its weight, in point order. Seeds weigh 1.
    pub fn weights(&self) -> Vec<(usize, f64)> {
        self.entries.iter().map(|e| (e.point, e.weight)).collect()
    }

    /// The followers alone, with their weights, in point order.
    pub fn followers(&self) -> Vec<(usize, f64)> {
        self.entries
            .iter()
            .filter(|e| !e.is_seed)
            .map(|e| (e.point, e.weight))
            .collect()
    }

    pub fn seed_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_seed).count()
    }

    /// Whether the plan moves nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netlines::grid_net;

    #[test]
    fn centreline_and_symmetric_layer_rules_hold_for_single_and_multi_seed_moves() {
        // By hand on grid_net(2, 2): point 0 at (0, 0, 0) is on the centreline, point 1 at (0, 1, 0).
        // Two seeds, delta (0, 0.5, 0.2): point 0 keeps y = 0 -> (0, 0, 0.2); point 1 -> (0, 1.5, 0.2).
        // Point 0 alone: (0, 0.5, 0.2), as before this module. Point 1 alone with delta (0, -9, 0): y stops
        // at 0.
        let mut s = grid_net(2, 2); // symmetric layer by default
        let two = MovePlan::rigid(&s, &[0, 1]);
        let moved = two.positions(Vec3::new(0.0, 0.5, 0.2));
        assert_eq!(moved.len(), 2);
        assert_eq!(moved[0].to, Vec3::new(0.0, 0.0, 0.2));
        assert_eq!(moved[1].to, Vec3::new(0.0, 1.5, 0.2));
        assert_eq!((moved[0].point, moved[0].from), (0, Vec3::ZERO));
        assert_eq!(two.seed_count(), 2);

        let lone = MovePlan::rigid(&s, &[0]);
        assert_eq!(
            lone.positions(Vec3::new(0.0, 0.5, 0.2))[0].to,
            Vec3::new(0.0, 0.5, 0.2)
        );
        assert_eq!(
            MovePlan::rigid(&s, &[1]).positions(Vec3::new(0.0, -9.0, 0.0))[0].to,
            Vec3::new(0.0, 0.0, 0.0)
        );

        // Review Focus 5: a model with no symmetric layer has no centreline rule and no y >= 0 floor.
        s.layers[0].symmetric = false;
        let unsym = MovePlan::rigid(&s, &[0, 1]);
        let moved = unsym.positions(Vec3::new(0.0, -0.4, 0.0));
        assert_eq!(moved[0].to.y, -0.4);
        assert_eq!(moved[1].to.y, 0.6);
    }

    #[test]
    fn a_point_is_on_a_symmetric_layer_when_any_of_its_faces_is() {
        // grid_net(3, 2) has two quads: face 0 on points 0 1 3 2, face 1 on points 2 3 5 4. Put face 1 on a
        // second, unsymmetric layer: points 4 and 5 touch only that one; 2 and 3 touch both.
        let mut s = grid_net(3, 2);
        s.layers.push(crate::mesh::Layer {
            symmetric: false,
            ..Default::default()
        });
        s.face_layer[1] = 1;
        assert!(point_on_symmetric_layer(&s, 0));
        assert!(
            point_on_symmetric_layer(&s, 2),
            "one symmetric face is enough"
        );
        assert!(!point_on_symmetric_layer(&s, 4));
        assert!(!point_on_symmetric_layer(&s, 99), "no such point");
        let loose = s.add_point(Vec3::ZERO);
        assert!(!point_on_symmetric_layer(&s, loose), "a point on no face");
    }

    #[test]
    fn followers_move_by_their_weight_in_the_masked_axes_and_seeds_by_all_of_it() {
        // By hand on grid_net(3, 3), point i * 3 + j at (i, j, 0). Seed 4 = (1, 1, 0), delta (0.2, 0.4, 0.6).
        // Follower 7 = (2, 1, 0), weight 0.5, mask "yz": (2, 1 + 0.2, 0.3). The seed takes all of it,
        // whatever the mask: (1.2, 1.4, 0.6). Follower 5 = (1, 2, 0), weight 0.25: (1, 2.1, 0.15).
        let s = grid_net(3, 3);
        let mask = AxisMask::parse("yz").unwrap();
        let plan = MovePlan::weighted(&s, &[4], &[(7, 0.5), (5, 0.25)], mask);
        let moved = plan.positions(Vec3::new(0.2, 0.4, 0.6));
        let at = |p: usize| moved.iter().find(|m| m.point == p).unwrap().to;
        assert_eq!(
            moved.iter().map(|m| m.point).collect::<Vec<_>>(),
            vec![4, 5, 7]
        );
        assert!((at(4) - Vec3::new(1.2, 1.4, 0.6)).length() < 1e-12);
        assert!((at(7) - Vec3::new(2.0, 1.2, 0.3)).length() < 1e-12);
        assert!((at(5) - Vec3::new(1.0, 2.1, 0.15)).length() < 1e-12);
        assert_eq!(plan.weights(), vec![(4, 1.0), (5, 0.25), (7, 0.5)]);
        assert_eq!(plan.followers(), vec![(5, 0.25), (7, 0.5)]);
        assert_eq!(plan.seed_count(), 1);
        assert_eq!(plan.start(7), Some(Vec3::new(2.0, 1.0, 0.0)));
        assert_eq!(plan.start(0), None);
    }

    #[test]
    fn a_follower_on_the_centreline_stays_there_and_a_lone_seed_may_leave() {
        // grid_net(3, 3): row j = 0 is the centreline. Seed 4 = (1, 1, 0), follower 3 = (1, 0, 0) at weight
        // 0.5, delta (0, 0.6, 0.2): the follower would go to y = 0.3 but stays at y = 0 -> (1, 0, 0.1).
        // Seed 3 alone with the same delta leaves the centreline -> (1, 0.6, 0.2).
        let s = grid_net(3, 3);
        let delta = Vec3::new(0.0, 0.6, 0.2);
        let plan = MovePlan::weighted(&s, &[4], &[(3, 0.5)], AxisMask::ALL);
        let moved = plan.positions(delta);
        assert_eq!(moved[0].point, 3);
        assert!((moved[0].to - Vec3::new(1.0, 0.0, 0.1)).length() < 1e-12);
        assert_eq!(moved[0].to.y, 0.0);
        assert_eq!(
            MovePlan::rigid(&s, &[3]).positions(delta)[0].to,
            Vec3::new(1.0, 0.6, 0.2)
        );
    }

    #[test]
    fn a_plan_leaves_out_what_cannot_move() {
        // Repeated seeds count once; a seed listed as a follower too stays a seed; a follower with weight 0,
        // a negative or a non-finite weight, and any point the surface does not have, are not in the plan.
        let s = grid_net(3, 3);
        let plan = MovePlan::weighted(
            &s,
            &[4, 4, 99],
            &[
                (4, 0.5),
                (5, 0.0),
                (6, -0.5),
                (7, f64::NAN),
                (8, 0.5),
                (8, 0.25),
                (99, 0.5),
            ],
            AxisMask::ALL,
        );
        assert_eq!(plan.weights(), vec![(4, 1.0), (8, 0.5)]);
        assert_eq!(plan.seed_count(), 1);
        // A seed the surface does not have does not make a lone centreline seed part of a group.
        let lone = MovePlan::rigid(&s, &[3, 99]).positions(Vec3::new(0.0, 0.5, 0.0));
        assert_eq!(lone[0].to, Vec3::new(1.0, 0.5, 0.0));
        assert!(MovePlan::rigid(&s, &[99]).is_empty());
        assert!(MovePlan::rigid(&Surface::new(), &[0])
            .positions(Vec3::ZERO)
            .is_empty());
    }

    #[test]
    fn a_zero_move_puts_every_point_back_exactly() {
        // A drag that changes its plan half way resets with a zero move first, so this must be exact even
        // for a point just off the centreline (within CENTRELINE_EPS) or just below it.
        let mut s = grid_net(3, 3);
        s.points[0].p.y = 5e-10;
        s.points[3].p.y = -2e-10;
        s.points[4].p = Vec3::new(1.1, 0.7, 0.3);
        let before: Vec<Vec3> = s.points.iter().map(|p| p.p).collect();
        let plan = MovePlan::weighted(&s, &[4, 0], &[(3, 0.5), (5, 0.3)], AxisMask::ALL);
        plan.apply(&mut s, Vec3::new(0.3, -0.2, 0.9));
        assert_ne!(s.points[4].p, before[4]);
        assert_eq!(
            s.points[8].p, before[8],
            "a point outside the plan never moves"
        );
        plan.apply(&mut s, Vec3::ZERO);
        let after: Vec<Vec3> = s.points.iter().map(|p| p.p).collect();
        assert_eq!(after, before);
    }

    #[test]
    fn moving_the_anchor_to_a_position_lands_on_it_exactly() {
        // 0.3 + (0.9 - 0.3) is not 0.9 in floating point; the anchor must still land on the target itself.
        let mut s = grid_net(3, 3);
        s.points[4].p = Vec3::new(0.3, 0.3, 0.3);
        let target = Vec3::new(0.9, 0.9, 0.9);
        assert_ne!(
            s.points[4].p + (target - s.points[4].p),
            target,
            "the premise of this test"
        );
        let plan = MovePlan::rigid(&s, &[4, 8]);
        let moved = plan.positions_to(4, target);
        assert_eq!(moved[0].to, target);
        // Point 8 = (2, 2, 0) moves by the same displacement (0.6, 0.6, 0.6).
        assert!((moved[1].to - Vec3::new(2.6, 2.6, 0.6)).length() < 1e-12);
        assert!(
            plan.positions_to(0, target).is_empty(),
            "point 0 is not a seed of this plan"
        );
        plan.apply_to(&mut s, 4, target);
        assert_eq!(s.points[4].p, target);
        // The half-model floor still applies to an exact target.
        let below = MovePlan::rigid(&s, &[4]).positions_to(4, Vec3::new(0.9, -3.0, 0.9));
        assert_eq!(below[0].to, Vec3::new(0.9, 0.0, 0.9));
    }

    #[test]
    fn an_axis_mask_reads_and_writes_as_letters() {
        assert_eq!(AxisMask::parse("xyz").unwrap(), AxisMask::ALL);
        assert_eq!(AxisMask::default(), AxisMask::ALL);
        let zx = AxisMask::parse("zx").unwrap();
        assert_eq!(
            zx,
            AxisMask {
                x: true,
                y: false,
                z: true
            }
        );
        assert_eq!(zx.to_string(), "xz");
        assert_eq!(zx.apply(Vec3::new(1.0, 2.0, 3.0)), Vec3::new(1.0, 0.0, 3.0));
        assert_eq!(
            AxisMask::parse("Y").unwrap(),
            AxisMask {
                x: false,
                y: true,
                z: false
            }
        );
        for bad in ["", "xq", "x y", "xx"] {
            let err = AxisMask::parse(bad).unwrap_err();
            assert!(err.contains("\"x\", \"y\"") && err.contains(bad), "{err}");
        }
        assert_eq!(serde_json::to_string(&zx).unwrap(), "\"xz\"");
        assert_eq!(
            serde_json::from_str::<AxisMask>("\"y\"")
                .unwrap()
                .to_string(),
            "y"
        );
        assert!(serde_json::from_str::<AxisMask>("\"\"").is_err());
        assert!(serde_json::from_str::<AxisMask>("7").is_err());
    }
}
