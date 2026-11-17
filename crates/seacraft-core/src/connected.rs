//! Connected editing: moving a control point draws its neighbours along.
//!
//! A [`ConnectedPreset`] says which neighbours follow a moved point and how
//! strongly. Distance is measured along the net's own edges from the nearest
//! moved point (a *seed*), over the edges the preset's [`Spread`] allows; a
//! follower's weight is the preset's [`Falloff`] of that distance over the
//! reach. Weights are worked out once, from where the points are when the move
//! starts, and handed to a [`MovePlan`], which applies them and keeps a half
//! model's centreline rules.
//!
//! The built-in presets live here. A user's own presets come from a JSON file
//! read with [`parse_presets`]; the file is the window's business, so a move
//! asked for by an agent names a built-in or carries its own preset
//! ([`preset_from_json`]) and never depends on what is on the user's disk.
//!
//! Nothing here claims a result is fair. A preset describes how points move.

use crate::mesh::Surface;
use crate::netlines::{LineKind, NetLines};
use crate::netmove::{AxisMask, MovePlan};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

/// How far followers are found.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Reach {
    /// A distance in metres along the net.
    #[serde(rename = "m")]
    Metres(f64),
    /// A fraction of the control net's extent in `x`, so one preset suits a
    /// dinghy and a ship.
    #[serde(rename = "of_length")]
    OfLength(f64),
}

/// The largest `of_length` a preset may ask for.
pub const MAX_OF_LENGTH: f64 = 10.0;
/// The largest reach in metres a preset may ask for.
pub const MAX_METRES: f64 = 1.0e6;

impl Reach {
    /// The reach in metres on `surface`.
    pub fn metres(&self, surface: &Surface) -> f64 {
        match *self {
            Reach::Metres(m) => m,
            Reach::OfLength(share) => {
                let (min, max) = surface.extents();
                share * (max.x - min.x)
            }
        }
    }
}

/// How a follower's weight falls with `t`, its distance over the reach.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Falloff {
    /// `1 - 3t² + 2t³`: level at the seed and at the reach.
    Smooth,
    /// `1 - t`.
    Linear,
    /// `(1 - t)²`: falls away quickly.
    Sharp,
    /// `1` all the way to the reach.
    Constant,
    /// Straight lines through `[t, weight]` points, in order of `t`; the end
    /// values hold outside them.
    Curve(Vec<[f64; 2]>),
}

impl Falloff {
    /// The weight at `t`, from 0 to 1. Zero beyond the reach (`t > 1`).
    pub fn weight(&self, t: f64) -> f64 {
        if !(0.0..=1.0).contains(&t) {
            // Beyond the reach, or not a distance at all.
            return 0.0;
        }
        let w = match self {
            Falloff::Smooth => 1.0 - 3.0 * t * t + 2.0 * t * t * t,
            Falloff::Linear => 1.0 - t,
            Falloff::Sharp => (1.0 - t) * (1.0 - t),
            Falloff::Constant => 1.0,
            Falloff::Curve(points) => curve_at(points, t),
        };
        if w.is_finite() {
            w.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// A curve's value at `t`: straight lines through its points in order of `t`,
/// the first and last values holding outside them.
fn curve_at(points: &[[f64; 2]], t: f64) -> f64 {
    let mut sorted: Vec<[f64; 2]> = points.to_vec();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let (Some(first), Some(last)) = (sorted.first(), sorted.last()) else {
        return 0.0;
    };
    if t < first[0] {
        return first[1];
    }
    if t >= last[0] {
        return last[1];
    }
    // The last point at or before `t`, and the one after it.
    let after = sorted.partition_point(|p| p[0] <= t);
    let (a, b) = (sorted[after - 1], sorted[after]);
    a[1] + (b[1] - a[1]) * (t - a[0]) / (b[0] - a[0])
}

/// Which neighbours follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Spread {
    /// Along the lengthwise lines through the seed.
    Lengthwise,
    /// Round the seed's station.
    Station,
    /// Over the whole net.
    Both,
}

/// One way of drawing neighbours along with a moved point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectedPreset {
    pub name: String,
    pub reach: Reach,
    pub falloff: Falloff,
    pub spread: Spread,
    /// The coordinates followers move in. Seeds always move in all three.
    #[serde(default)]
    pub follow_axes: AxisMask,
    /// Keep a move on one side of a crease: a path passes through a point on a
    /// crease only when it arrives and leaves along crease edges.
    #[serde(default)]
    pub stop_at_creases: bool,
}

impl ConnectedPreset {
    /// Whether the preset can be used; the error names it.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("a preset needs a name".into());
        }
        let fail = |why: String| Err(format!("preset \"{}\": {why}", self.name));
        match self.reach {
            Reach::Metres(m) if !(m.is_finite() && m > 0.0 && m <= MAX_METRES) => {
                return fail(format!(
                    "reach must be more than 0 and at most {MAX_METRES} m, got {m}"
                ));
            }
            Reach::OfLength(share)
                if !(share.is_finite() && share > 0.0 && share <= MAX_OF_LENGTH) =>
            {
                return fail(format!(
                    "reach must be more than 0 and at most {MAX_OF_LENGTH} of the length, got {share}"
                ));
            }
            _ => {}
        }
        if let Falloff::Curve(points) = &self.falloff {
            if points.is_empty() {
                return fail("a falloff curve needs at least one [t, weight] point".into());
            }
            if let Some([t, w]) = points
                .iter()
                .find(|p| p.iter().any(|v| !(0.0..=1.0).contains(v)))
            {
                return fail(format!(
                    "a falloff curve's t and weight must both be from 0 to 1, got [{t}, {w}]"
                ));
            }
        }
        Ok(())
    }
}

/// The preset a new installation starts with.
pub const DEFAULT_PRESET: &str = "Blend both ways";

/// The presets that are always there, in menu order.
pub fn builtin_presets() -> Vec<ConnectedPreset> {
    let all = AxisMask::ALL;
    let only = |x, y, z| AxisMask { x, y, z };
    [
        (DEFAULT_PRESET, 0.15, Falloff::Smooth, Spread::Both, all),
        (
            "Blend lengthwise",
            0.25,
            Falloff::Smooth,
            Spread::Lengthwise,
            all,
        ),
        (
            "Blend around station",
            0.15,
            Falloff::Smooth,
            Spread::Station,
            all,
        ),
        ("Nudge", 0.06, Falloff::Sharp, Spread::Both, all),
        ("Move station", 1.0, Falloff::Constant, Spread::Station, all),
        (
            "Beam only",
            0.15,
            Falloff::Smooth,
            Spread::Both,
            only(false, true, false),
        ),
        (
            "Height only",
            0.15,
            Falloff::Smooth,
            Spread::Both,
            only(false, false, true),
        ),
    ]
    .into_iter()
    .map(
        |(name, of_length, falloff, spread, follow_axes)| ConnectedPreset {
            name: name.into(),
            reach: Reach::OfLength(of_length),
            falloff,
            spread,
            follow_axes,
            stop_at_creases: false,
        },
    )
    .collect()
}

/// What a presets file must look like, for messages.
const FILE_SHAPE: &str = r#"a presets file is { "version": 1, "presets": [ ... ] }"#;

/// Read one preset from JSON; `label` names it in the error.
fn preset_of(value: &Value, label: &str) -> Result<ConnectedPreset, String> {
    if !value.is_object() {
        return Err(format!(
            "{label}: a preset is an object with a name, a reach, a falloff and a spread"
        ));
    }
    let preset: ConnectedPreset =
        serde_json::from_value(value.clone()).map_err(|e| format!("{label}: {e}"))?;
    preset.validate()?;
    Ok(preset)
}

/// How to call a preset in a message: by its name if it has one, else `nth`.
fn label_of(value: &Value, nth: impl FnOnce() -> String) -> String {
    match value.get("name").and_then(Value::as_str) {
        Some(name) if !name.trim().is_empty() => format!("preset \"{name}\""),
        _ => nth(),
    }
}

/// Read a presets file: `{ "version": 1, "presets": [ ... ] }`, with an
/// optional `"notes"` beside them that is ignored. The error says which preset
/// is at fault and why.
pub fn parse_presets(json: &str) -> Result<Vec<ConnectedPreset>, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("not valid JSON: {e}"))?;
    let Some(root) = root.as_object() else {
        return Err(FILE_SHAPE.into());
    };
    if let Some(key) = root
        .keys()
        .find(|k| !matches!(k.as_str(), "version" | "presets" | "notes"))
    {
        return Err(format!("unknown entry \"{key}\"; {FILE_SHAPE}"));
    }
    match root.get("version") {
        Some(v) if v.as_u64() == Some(1) => {}
        Some(v) => {
            return Err(format!(
                "version {v} is not one this program reads; {FILE_SHAPE}"
            ))
        }
        None => return Err(format!("no version; {FILE_SHAPE}")),
    }
    let Some(list) = root.get("presets").and_then(Value::as_array) else {
        return Err(format!("no list of presets; {FILE_SHAPE}"));
    };
    let mut presets: Vec<ConnectedPreset> = Vec::with_capacity(list.len());
    for (i, value) in list.iter().enumerate() {
        let nth = || format!("preset {}", i + 1);
        let label = label_of(value, nth);
        if value.is_object() && !label.starts_with("preset \"") {
            return Err(format!("{label}: a preset needs a name"));
        }
        let preset = preset_of(value, &label)?;
        if presets.iter().any(|p| p.name == preset.name) {
            return Err(format!("{label}: there are two presets of this name"));
        }
        presets.push(preset);
    }
    Ok(presets)
}

/// The built-ins with the user's presets: one with a built-in's name takes its
/// place, the rest follow in the user's order.
pub fn merged(user: Vec<ConnectedPreset>) -> Vec<ConnectedPreset> {
    let mut all = builtin_presets();
    for preset in user {
        match all.iter_mut().find(|p| p.name == preset.name) {
            Some(slot) => *slot = preset,
            None => all.push(preset),
        }
    }
    all
}

/// A presets file to start from, with a preset of each kind of reach and
/// falloff. [`parse_presets`] reads it.
pub fn example_file() -> String {
    EXAMPLE_FILE.to_string()
}

const EXAMPLE_FILE: &str = r#"{
  "version": 1,
  "notes": [
    "Your own presets for connected editing. SeaCraft reads this file when it starts and when you choose Reload presets.",
    "name: what the preset is called in the list. A preset named like a built-in one replaces it.",
    "reach: how far followers are found, along the net: { \"m\": 2.0 } in metres, or { \"of_length\": 0.2 } as a share of the hull's length.",
    "falloff: how the pull fades with distance: \"smooth\", \"linear\", \"sharp\", \"constant\", or { \"curve\": [[t, weight], ...] } with t and weight from 0 to 1.",
    "spread: which neighbours follow: \"lengthwise\", \"station\" or \"both\".",
    "follow_axes: the directions followers move in, any of x (along), y (beam), z (height). Left out, it is \"xyz\".",
    "stop_at_creases: true keeps a move on one side of a crease such as a chine. Left out, it is false."
  ],
  "presets": [
    {
      "name": "Wide blend",
      "reach": { "of_length": 0.3 },
      "falloff": "smooth",
      "spread": "both",
      "follow_axes": "xyz",
      "stop_at_creases": false
    },
    {
      "name": "Two metres along, this side of the chine",
      "reach": { "m": 2.0 },
      "falloff": "linear",
      "spread": "lengthwise",
      "stop_at_creases": true
    },
    {
      "name": "Shoulder round the station",
      "reach": { "of_length": 0.2 },
      "falloff": { "curve": [[0.0, 1.0], [0.5, 0.9], [1.0, 0.0]] },
      "spread": "station",
      "follow_axes": "yz"
    }
  ]
}
"#;

/// The preset an agent asked for: a built-in's name, or a preset object.
/// The user's presets file is never consulted.
pub fn preset_from_json(value: &Value) -> Result<ConnectedPreset, String> {
    match value {
        Value::String(name) => {
            let builtins = builtin_presets();
            let names: Vec<String> = builtins.iter().map(|p| format!("\"{}\"", p.name)).collect();
            builtins
                .into_iter()
                .find(|p| &p.name == name)
                .ok_or_else(|| {
                    format!(
                        "no built-in preset \"{name}\"; there are {}",
                        names.join(", ")
                    )
                })
        }
        Value::Object(fields) => {
            // An agent's one-off preset may leave its name out: "custom".
            let named = fields
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| !n.trim().is_empty());
            let mut value = value.clone();
            if !named {
                value["name"] = Value::String("custom".into());
            }
            let label = label_of(&value, || "the preset".to_string());
            preset_of(&value, &label)
        }
        _ => Err("a preset is the name of a built-in one or a preset object".into()),
    }
}

/// The points that follow `seeds` under `preset` with a reach of `reach_m`
/// metres, each with its weight in `(0, 1]`, in point order. The seeds
/// themselves are not listed.
pub fn follower_weights(
    surface: &Surface,
    lines: &NetLines,
    seeds: &[usize],
    preset: &ConnectedPreset,
    reach_m: f64,
) -> Vec<(usize, f64)> {
    let n = surface.points.len();
    if !(reach_m.is_finite() && reach_m > 0.0) {
        return Vec::new();
    }
    let mut is_seed = vec![false; n];
    for &seed in seeds {
        if let Some(flag) = is_seed.get_mut(seed) {
            *flag = true;
        }
    }
    // The edges the spread lets a path run along, from each point; and which
    // points have a crease at them (any crease, allowed or not).
    let mut from: Vec<Vec<(usize, f64, bool)>> = vec![Vec::new(); n];
    let mut on_crease = vec![false; n];
    for (ei, edge) in surface.edges.iter().enumerate() {
        if edge.a >= n || edge.b >= n {
            continue;
        }
        if edge.crease {
            on_crease[edge.a] = true;
            on_crease[edge.b] = true;
        }
        let kind = lines
            .edge_line
            .get(ei)
            .and_then(|&li| lines.lines.get(li))
            .map(|line| line.kind);
        let allowed = match preset.spread {
            Spread::Both => true,
            Spread::Lengthwise => kind == Some(LineKind::Lengthwise),
            Spread::Station => kind == Some(LineKind::Transverse),
        };
        let length = surface.points[edge.a].p.dist(surface.points[edge.b].p);
        if allowed && length.is_finite() {
            from[edge.a].push((edge.b, length, edge.crease));
            from[edge.b].push((edge.a, length, edge.crease));
        }
    }
    // Shortest paths from the seeds. A point is reached in two ways that
    // matter apart: along a crease edge, or along a plain one.
    let mut dist = vec![[f64::INFINITY; 2]; n];
    let mut queue: BinaryHeap<Reverse<(Distance, usize, bool)>> = BinaryHeap::new();
    for (seed, _) in is_seed.iter().enumerate().filter(|(_, &s)| s) {
        dist[seed] = [0.0, 0.0];
        queue.push(Reverse((Distance(0.0), seed, true)));
    }
    while let Some(Reverse((Distance(d), u, by_crease))) = queue.pop() {
        if d > dist[u][usize::from(by_crease)] {
            continue;
        }
        let held = preset.stop_at_creases && on_crease[u] && !is_seed[u];
        for &(v, length, crease) in &from[u] {
            // Through a point on a crease only along the crease.
            if held && !(by_crease && crease) {
                continue;
            }
            let next = d + length;
            let slot = &mut dist[v][usize::from(crease)];
            if next <= reach_m && next < *slot {
                *slot = next;
                queue.push(Reverse((Distance(next), v, crease)));
            }
        }
    }
    (0..n)
        .filter(|&v| !is_seed[v])
        .filter_map(|v| {
            let d = dist[v][0].min(dist[v][1]);
            let w = preset.falloff.weight(d / reach_m);
            (d <= reach_m && w > 0.0).then_some((v, w))
        })
        .collect()
}

/// A distance, ordered so a heap can hold it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Distance(f64);

impl Eq for Distance {}

impl PartialOrd for Distance {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Distance {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl MovePlan {
    /// A move of `seeds` that draws their neighbours along as `preset` says,
    /// with a reach of `reach_m` metres.
    pub fn connected(
        surface: &Surface,
        lines: &NetLines,
        seeds: &[usize],
        preset: &ConnectedPreset,
        reach_m: f64,
    ) -> MovePlan {
        let followers = follower_weights(surface, lines, seeds, preset, reach_m);
        MovePlan::weighted(surface, seeds, &followers, preset.follow_axes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;
    use crate::netlines::grid_net;
    use std::collections::BTreeMap;

    // grid_net(5, 5): point i * 5 + j at (i, j, 0), unit spacing; lengthwise lines run in i, stations in j;
    // the row j = 0 is the centreline of its one symmetric layer.

    fn preset(falloff: Falloff, spread: Spread, reach: f64) -> ConnectedPreset {
        ConnectedPreset {
            name: "test".into(),
            reach: Reach::Metres(reach),
            falloff,
            spread,
            follow_axes: AxisMask::ALL,
            stop_at_creases: false,
        }
    }

    fn weights(s: &Surface, seeds: &[usize], p: &ConnectedPreset) -> BTreeMap<usize, f64> {
        let Reach::Metres(reach) = p.reach else {
            panic!("these tests give the reach in metres");
        };
        follower_weights(s, &NetLines::of(s), seeds, p, reach)
            .into_iter()
            .collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn falloff_and_grid_dijkstra_weights_match_closed_forms() {
        // Closed forms by hand on the unit grid with reach = 3:
        // linear(t) = 1 - t -> at d = 1: 2/3, at d = 2: 1/3, at d = 3: 0 (left out).
        // smooth(0.5) = 1 - 3 * 0.25 + 2 * 0.125 = 0.5; sharp(0.5) = (1 - 0.5)^2 = 0.25.
        // curve [[0,1],[0.5,0.8],[1,0]]: at t = 0.25 -> 0.9, at t = 0.75 -> 0.4.
        assert!(close(Falloff::Smooth.weight(0.5), 0.5));
        assert!(close(Falloff::Sharp.weight(0.5), 0.25));
        assert!(close(Falloff::Linear.weight(0.25), 0.75));
        let curve = Falloff::Curve(vec![[0.0, 1.0], [0.5, 0.8], [1.0, 0.0]]);
        assert!(close(curve.weight(0.25), 0.9));
        assert!(close(curve.weight(0.75), 0.4));
        for f in [
            Falloff::Smooth,
            Falloff::Linear,
            Falloff::Sharp,
            Falloff::Constant,
            curve,
        ] {
            assert_eq!(f.weight(0.0), 1.0, "{f:?} at the seed");
            assert_eq!(f.weight(1.0 + 1e-9), 0.0, "{f:?} beyond the reach");
            assert_eq!(f.weight(f64::NAN), 0.0, "{f:?} of nonsense");
        }

        let s = grid_net(5, 5);
        let along = preset(Falloff::Linear, Spread::Lengthwise, 3.0);
        let w = weights(&s, &[0], &along);
        assert!(close(w[&5], 2.0 / 3.0) && close(w[&10], 1.0 / 3.0));
        assert!(!w.contains_key(&15) && !w.contains_key(&1) && !w.contains_key(&0));
        assert_eq!(w.len(), 2);

        // Round the station instead: the lengthwise neighbours stay.
        let round = preset(Falloff::Linear, Spread::Station, 3.0);
        let w = weights(&s, &[0], &round);
        assert!(close(w[&1], 2.0 / 3.0) && close(w[&2], 1.0 / 3.0));
        assert_eq!(w.len(), 2);

        // Both ways the distance is the Manhattan distance: (1, 1) is 2 away.
        let both = preset(Falloff::Linear, Spread::Both, 3.0);
        let w = weights(&s, &[0], &both);
        assert!(close(w[&6], 1.0 / 3.0));
        assert!(close(w[&5], 2.0 / 3.0) && close(w[&1], 2.0 / 3.0));
        // Points at Manhattan distance 1 and 2 from the corner: 2 + 3.
        assert_eq!(w.len(), 5);
    }

    #[test]
    fn a_curve_is_read_in_order_of_t_and_holds_its_end_values() {
        // By hand: points given out of order, [[0.8, 0.2], [0.2, 0.6]]. Before t = 0.2 the weight holds at
        // 0.6, after 0.8 at 0.2 (until the reach), and half way between, t = 0.5, it is 0.4. A step (two
        // points at one t) takes the later value from there on.
        let curve = Falloff::Curve(vec![[0.8, 0.2], [0.2, 0.6]]);
        assert!(close(curve.weight(0.0), 0.6));
        assert!(close(curve.weight(0.5), 0.4));
        assert!(close(curve.weight(0.9), 0.2));
        assert!(close(curve.weight(1.0), 0.2));
        let step = Falloff::Curve(vec![[0.5, 1.0], [0.5, 0.25]]);
        assert!(close(step.weight(0.4), 1.0) && close(step.weight(0.6), 0.25));
    }

    #[test]
    fn stop_at_creases_holds_a_move_on_one_side() {
        // By hand: crease the lengthwise row j = 2, edges (i,2)-(i+1,2) for i = 0..3. Linear, reach 10, both.
        // Seed (2,0) = 10: (2,1) = 11 at d 1 -> 0.9; the crease point (2,2) = 12 at d 2 -> 0.8, and (1,2) = 7
        // at d 3 -> 0.7; nothing with j >= 3, because every way there passes a crease point that was entered
        // along a plain edge.
        // Seed (2,2) = 12, on the crease: along it (3,2) = 17 -> 0.9 and (4,2) = 22 -> 0.8; into both sides,
        // (2,1) = 11 and (2,3) = 13 -> 0.9.
        // With stop_at_creases off, seed 10 reaches 13 at d 3 -> 0.7.
        let mut s = grid_net(5, 5);
        for i in 0..4 {
            crate::topology::set_crease(&mut s, i * 5 + 2, (i + 1) * 5 + 2, true).unwrap();
        }
        let mut lin = preset(Falloff::Linear, Spread::Both, 10.0);
        lin.stop_at_creases = true;
        let w = weights(&s, &[10], &lin);
        assert!(close(w[&11], 0.9) && close(w[&12], 0.8) && close(w[&7], 0.7));
        assert!(w.keys().all(|p| p % 5 <= 2), "{:?}", w.keys());
        // Every point of the rows j = 0, 1, 2 but the seed.
        assert_eq!(w.len(), 14);

        let w = weights(&s, &[12], &lin);
        assert!(close(w[&17], 0.9) && close(w[&22], 0.8));
        assert!(close(w[&11], 0.9) && close(w[&13], 0.9));
        assert_eq!(w.len(), 24, "a seed on the crease reaches both sides");

        lin.stop_at_creases = false;
        let w = weights(&s, &[10], &lin);
        assert!(close(w[&13], 0.7));
        assert_eq!(w.len(), 24);
    }

    #[test]
    fn two_seeds_use_the_nearer_and_constant_reaches_the_rim() {
        // Lengthwise, linear, reach 3, seeds 0 and 20 = (0,0), (4,0): point 10 is 2 from both -> 1/3; points
        // 5 and 15 are 1 from one of them -> 2/3.
        let s = grid_net(5, 5);
        let w = weights(
            &s,
            &[0, 20],
            &preset(Falloff::Linear, Spread::Lengthwise, 3.0),
        );
        assert!(close(w[&10], 1.0 / 3.0) && close(w[&5], 2.0 / 3.0) && close(w[&15], 2.0 / 3.0));
        assert_eq!(w.len(), 3);
        // Constant holds 1 up to and at the reach, and 0 beyond: seed 0, lengthwise, reach 2 -> points 5 and
        // 10 at weight 1, and 15 not at all.
        assert_eq!(Falloff::Constant.weight(1.0), 1.0);
        assert_eq!(Falloff::Constant.weight(1.0 + 1e-9), 0.0);
        let w = weights(
            &s,
            &[0],
            &preset(Falloff::Constant, Spread::Lengthwise, 2.0),
        );
        assert_eq!(w, BTreeMap::from([(5, 1.0), (10, 1.0)]));
    }

    #[test]
    fn a_centreline_follower_stays_on_the_centreline() {
        // Linear, reach 3, both. Seed (2,1) = 11, delta (0, 0.6, 0.3). Followers at d 1 weigh 2/3:
        // (2,0) = 10 is on the centreline -> (2, 0, 0.2); (2,2) = 12 -> (2, 2.4, 0.2); the seed -> (2, 1.6, 0.3).
        // With follow_axes "z": 12 -> (2, 2, 0.2) and the seed still (2, 1.6, 0.3).
        let s = grid_net(5, 5);
        let nl = NetLines::of(&s);
        let mut p = preset(Falloff::Linear, Spread::Both, 3.0);
        let delta = Vec3::new(0.0, 0.6, 0.3);
        let at = |plan: &MovePlan, point: usize| {
            plan.positions(delta)
                .into_iter()
                .find(|m| m.point == point)
                .unwrap()
                .to
        };
        let plan = MovePlan::connected(&s, &nl, &[11], &p, 3.0);
        assert!((at(&plan, 10) - Vec3::new(2.0, 0.0, 0.2)).length() < 1e-12);
        assert_eq!(at(&plan, 10).y, 0.0);
        assert!((at(&plan, 12) - Vec3::new(2.0, 2.4, 0.2)).length() < 1e-12);
        assert!((at(&plan, 11) - Vec3::new(2.0, 1.6, 0.3)).length() < 1e-12);
        assert_eq!(plan.seed_count(), 1);
        // Manhattan distance 1 and 2 from (2, 1): 4 + 7 followers.
        assert_eq!(plan.followers().len(), 11);

        p.follow_axes = AxisMask::parse("z").unwrap();
        let plan = MovePlan::connected(&s, &nl, &[11], &p, 3.0);
        assert!((at(&plan, 12) - Vec3::new(2.0, 2.0, 0.2)).length() < 1e-12);
        assert!((at(&plan, 11) - Vec3::new(2.0, 1.6, 0.3)).length() < 1e-12);
    }

    #[test]
    fn a_split_face_fan_still_yields_follower_weights() {
        // Review Focus 1: split face 5 of the grid, the quad (1,1) (1,2) (2,2) (2,1), into a fan. Seed its
        // corner (1,1) = 6, linear, reach 3, both. The new centre point at (1.5, 1.5) is one spoke away,
        // sqrt(0.5), so it weighs 1 - sqrt(0.5) / 3; every weight is a number in (0, 1].
        let mut s = grid_net(5, 5);
        let centre = crate::topology::split_face(&mut s, 5).unwrap().point;
        let w = weights(&s, &[6], &preset(Falloff::Linear, Spread::Both, 3.0));
        assert!(close(w[&centre], 1.0 - 0.5_f64.sqrt() / 3.0));
        assert!(w.values().all(|&x| x.is_finite() && x > 0.0 && x <= 1.0));
        // Across the hole by way of the centre: (2,2) = 12 is 2 * sqrt(0.5) away, nearer than round the rim.
        assert!(close(w[&12], 1.0 - 2.0 * 0.5_f64.sqrt() / 3.0));
    }

    #[test]
    fn followers_are_none_without_a_reach_a_seed_or_a_net() {
        let s = grid_net(5, 5);
        let nl = NetLines::of(&s);
        let p = preset(Falloff::Linear, Spread::Both, 3.0);
        for reach in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                follower_weights(&s, &nl, &[12], &p, reach).is_empty(),
                "reach {reach}"
            );
        }
        assert!(follower_weights(&s, &nl, &[], &p, 3.0).is_empty());
        assert!(
            follower_weights(&s, &nl, &[99], &p, 3.0).is_empty(),
            "no such point"
        );
        let empty = Surface::new();
        assert!(follower_weights(&empty, &NetLines::of(&empty), &[0], &p, 3.0).is_empty());
        assert!(MovePlan::connected(&empty, &NetLines::of(&empty), &[0], &p, 3.0).is_empty());
        // Lines read from another net (too few edges) are no reason to panic either.
        let _ = follower_weights(&s, &NetLines::of(&grid_net(2, 2)), &[12], &p, 3.0);
    }

    #[test]
    fn move_station_on_the_demo_hull_moves_the_station_and_nothing_else() {
        // The demo half hull has 6 points per station (`demo.rs`: SECTION_T). "Move station" is constant
        // round the station with a reach of the whole length (12 m), longer than any half girth, so a seed
        // on the station at x = 5.20 has exactly the station's other 5 points as followers, each at weight 1.
        let s = crate::demo::demo_hull(true);
        let nl = NetLines::of(&s);
        let p = builtin_presets()
            .into_iter()
            .find(|p| p.name == "Move station")
            .unwrap();
        assert!(close(p.reach.metres(&s), 12.0));
        let seed = s
            .points
            .iter()
            .position(|c| c.p.x == 5.20 && c.p.y > 1.0)
            .unwrap();
        let w = follower_weights(&s, &nl, &[seed], &p, p.reach.metres(&s));
        let mut station = nl.station_points(seed);
        station.retain(|&q| q != seed);
        assert_eq!(station.len(), 5);
        assert_eq!(w, station.iter().map(|&q| (q, 1.0)).collect::<Vec<_>>());
    }

    #[test]
    fn a_reach_is_metres_or_a_share_of_the_length() {
        // grid_net(5, 5) spans x = 0..4: a quarter of the length is 1 m.
        let s = grid_net(5, 5);
        assert_eq!(Reach::Metres(2.0).metres(&s), 2.0);
        assert_eq!(Reach::OfLength(0.25).metres(&s), 1.0);
        assert_eq!(Reach::OfLength(0.25).metres(&Surface::new()), 0.0);
    }

    #[test]
    fn the_built_in_presets_are_these_seven() {
        let b = builtin_presets();
        let row = |p: &ConnectedPreset| {
            (
                p.name.clone(),
                p.reach,
                p.falloff.clone(),
                p.spread,
                p.follow_axes.to_string(),
            )
        };
        let want = [
            (
                "Blend both ways",
                0.15,
                Falloff::Smooth,
                Spread::Both,
                "xyz",
            ),
            (
                "Blend lengthwise",
                0.25,
                Falloff::Smooth,
                Spread::Lengthwise,
                "xyz",
            ),
            (
                "Blend around station",
                0.15,
                Falloff::Smooth,
                Spread::Station,
                "xyz",
            ),
            ("Nudge", 0.06, Falloff::Sharp, Spread::Both, "xyz"),
            (
                "Move station",
                1.0,
                Falloff::Constant,
                Spread::Station,
                "xyz",
            ),
            ("Beam only", 0.15, Falloff::Smooth, Spread::Both, "y"),
            ("Height only", 0.15, Falloff::Smooth, Spread::Both, "z"),
        ];
        assert_eq!(b.len(), want.len());
        for (p, (name, of_length, falloff, spread, axes)) in b.iter().zip(want) {
            assert_eq!(
                row(p),
                (
                    name.to_string(),
                    Reach::OfLength(of_length),
                    falloff,
                    spread,
                    axes.to_string()
                )
            );
            assert!(!p.stop_at_creases);
            assert_eq!(p.validate(), Ok(()));
            let lower = p.name.to_lowercase();
            for claim in ["fair", "verif", "valid", "compl", "approv"] {
                assert!(!lower.contains(claim), "{} claims a result", p.name);
            }
        }
        assert_eq!(b[0].name, DEFAULT_PRESET);
        // They survive a trip through JSON unchanged.
        let text = serde_json::to_string(&b).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<ConnectedPreset>>(&text).unwrap(),
            b
        );
    }

    #[test]
    fn a_users_preset_replaces_a_built_in_of_the_same_name_and_new_ones_follow() {
        let mine = |name: &str| ConnectedPreset {
            name: name.into(),
            ..preset(Falloff::Linear, Spread::Both, 2.0)
        };
        let all = merged(vec![mine("Mine"), mine("Nudge"), mine("Other")]);
        let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Blend both ways",
                "Blend lengthwise",
                "Blend around station",
                "Nudge",
                "Move station",
                "Beam only",
                "Height only",
                "Mine",
                "Other"
            ]
        );
        assert_eq!(all[3].reach, Reach::Metres(2.0), "the user's Nudge");
        assert_eq!(merged(Vec::new()), builtin_presets());
    }

    fn file(presets: &str) -> String {
        format!(r#"{{ "version": 1, "presets": [{presets}] }}"#)
    }

    fn bad(fields: &str) -> String {
        file(&format!(
            r#"{{ "name": "Bad", "reach": {{ "m": 2.0 }}, "falloff": "linear", "spread": "both"{fields} }}"#
        ))
    }

    #[test]
    fn parse_presets_reads_both_reaches_every_falloff_and_the_example_file() {
        let text = file(
            r#"{ "name": "A", "reach": { "m": 2.0 }, "falloff": "sharp", "spread": "station" },
               { "name": "B", "reach": { "of_length": 0.2 }, "falloff": { "curve": [[0, 1], [1, 0]] },
                 "spread": "lengthwise", "follow_axes": "zx", "stop_at_creases": true }"#,
        );
        let got = parse_presets(&text).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0],
            ConnectedPreset {
                name: "A".into(),
                ..preset(Falloff::Sharp, Spread::Station, 2.0)
            }
        );
        assert_eq!(got[1].reach, Reach::OfLength(0.2));
        assert_eq!(got[1].falloff, Falloff::Curve(vec![[0.0, 1.0], [1.0, 0.0]]));
        assert_eq!(
            (
                got[1].follow_axes.to_string().as_str(),
                got[1].stop_at_creases
            ),
            ("xz", true)
        );
        // Notes beside the presets are allowed and ignored; no presets at all is a valid file.
        assert_eq!(
            parse_presets(r#"{ "version": 1, "notes": ["mine"], "presets": [] }"#),
            Ok(Vec::new())
        );

        let example = parse_presets(&example_file()).unwrap();
        assert!(example.len() >= 2);
        let builtin: Vec<String> = builtin_presets().into_iter().map(|p| p.name).collect();
        assert!(
            example.iter().all(|p| !builtin.contains(&p.name)),
            "the example replaces no built-in"
        );
        assert!(example.iter().any(|p| matches!(p.reach, Reach::Metres(_))));
        assert!(example
            .iter()
            .any(|p| matches!(p.reach, Reach::OfLength(_))));
        assert!(example
            .iter()
            .any(|p| matches!(p.falloff, Falloff::Curve(_))));
    }

    #[test]
    fn parse_presets_rejects_each_invalid_input_naming_the_preset() {
        // Review Focus 4. Each of these is refused with a message that names the preset "Bad".
        let named_bad = [
            bad(r#", "reach": { "m": 0.0 }"#),
            bad(r#", "reach": { "m": -1.0 }"#),
            bad(r#", "reach": { "of_length": 11.0 }"#),
            bad(r#", "reach": { "km": 1.0 }"#),
            bad(r#", "reach": 2.0"#),
            bad(r#", "falloff": { "curve": [] }"#),
            bad(r#", "falloff": { "curve": [[1.5, 1.0]] }"#),
            bad(r#", "falloff": { "curve": [[0.5, 2.0]] }"#),
            bad(r#", "falloff": "gentle""#),
            bad(r#", "spread": "sideways""#),
            bad(r#", "follow_axes": """#),
            bad(r#", "follow_axes": "xq""#),
            bad(r#", "stop_at_creases": "yes""#),
            bad(r#", "colour": "red""#),
            // Two presets of one name.
            file(
                r#"{ "name": "Bad", "reach": { "m": 2.0 }, "falloff": "linear", "spread": "both" },
                   { "name": "Bad", "reach": { "m": 3.0 }, "falloff": "linear", "spread": "both" }"#,
            ),
        ];
        for text in &named_bad {
            let err = parse_presets(text).expect_err(text);
            assert!(err.contains("\"Bad\""), "{err}\n  for {text}");
        }
        // A duplicated key is the JSON reader's complaint; serde_json keeps the last, so the test above uses
        // the later "reach" and "falloff". Without a usable name the message counts presets from 1.
        for text in [
            file(r#"{ "name": "", "reach": { "m": 2.0 }, "falloff": "linear", "spread": "both" }"#),
            file(
                r#"{ "name": "  ", "reach": { "m": 2.0 }, "falloff": "linear", "spread": "both" }"#,
            ),
            file(r#"{ "reach": { "m": 2.0 }, "falloff": "linear", "spread": "both" }"#),
            file(r#"{ "name": 7, "reach": { "m": 2.0 }, "falloff": "linear", "spread": "both" }"#),
            file(r#""Nudge""#),
        ] {
            let err = parse_presets(&text).expect_err(&text);
            assert!(err.contains("preset 1"), "{err}\n  for {text}");
        }
        // The file as a whole.
        for (text, says) in [
            (r#"{ "version": 2, "presets": [] }"#.to_string(), "version"),
            (r#"{ "presets": [] }"#.to_string(), "version"),
            (r#"{ "version": 1 }"#.to_string(), "presets"),
            (r#"{ "version": 1, "presets": {} }"#.to_string(), "presets"),
            (
                r#"{ "version": 1, "presets": [], "preset": [] }"#.to_string(),
                "\"preset\"",
            ),
            (r#"[]"#.to_string(), "version"),
            ("not json at all".to_string(), "line 1"),
            (String::new(), "line 1"),
            // A number too large for JSON is refused by the reader, with its place in the file.
            (bad(r#", "reach": { "m": 1e999 }"#), "line 1"),
        ] {
            let err = parse_presets(&text).expect_err(&text);
            assert!(err.contains(says), "{err}\n  for {text}");
        }
        // A preset made in code is checked the same way, including what JSON cannot carry.
        for reach in [
            Reach::Metres(f64::NAN),
            Reach::Metres(f64::INFINITY),
            Reach::OfLength(-0.1),
        ] {
            let p = ConnectedPreset {
                name: "Bad".into(),
                reach,
                ..preset(Falloff::Linear, Spread::Both, 1.0)
            };
            assert!(p.validate().unwrap_err().contains("\"Bad\""));
        }
        let nan_curve = ConnectedPreset {
            name: "Bad".into(),
            ..preset(Falloff::Curve(vec![[0.5, f64::NAN]]), Spread::Both, 1.0)
        };
        assert!(nan_curve.validate().unwrap_err().contains("\"Bad\""));
    }

    #[test]
    fn an_agent_names_a_built_in_or_sends_a_preset() {
        let nudge = preset_from_json(&serde_json::json!("Nudge")).unwrap();
        assert_eq!(
            nudge,
            builtin_presets()
                .into_iter()
                .find(|p| p.name == "Nudge")
                .unwrap()
        );
        let own = preset_from_json(&serde_json::json!({
            "name": "Mine", "reach": { "m": 1.5 }, "falloff": "linear", "spread": "station"
        }))
        .unwrap();
        assert_eq!(
            own,
            ConnectedPreset {
                name: "Mine".into(),
                ..preset(Falloff::Linear, Spread::Station, 1.5)
            }
        );
        // An unknown name lists the ones there are; a bad preset says why; anything else says what is wanted.
        let err = preset_from_json(&serde_json::json!("Smudge")).unwrap_err();
        assert!(
            err.contains("\"Smudge\"")
                && err.contains("\"Blend both ways\"")
                && err.contains("\"Nudge\""),
            "{err}"
        );
        let err = preset_from_json(&serde_json::json!({
            "name": "Mine", "reach": { "m": -1.0 }, "falloff": "linear", "spread": "station"
        }))
        .unwrap_err();
        assert!(err.contains("\"Mine\"") && err.contains("reach"), "{err}");
        let err = preset_from_json(&serde_json::json!(3)).unwrap_err();
        assert!(err.contains("name") && err.contains("object"), "{err}");
        // Final review of feat/hull-editing, M8: the plan gives an inline
        // preset without a name the name "custom".
        for unnamed in [
            serde_json::json!({ "reach": { "m": 1.5 }, "falloff": "linear", "spread": "station" }),
            serde_json::json!({ "name": " ", "reach": { "m": 1.5 }, "falloff": "linear", "spread": "station" }),
        ] {
            assert_eq!(
                preset_from_json(&unnamed).unwrap(),
                ConnectedPreset {
                    name: "custom".into(),
                    ..preset(Falloff::Linear, Spread::Station, 1.5)
                }
            );
        }
    }
}
