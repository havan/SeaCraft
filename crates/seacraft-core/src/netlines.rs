//! Net lines: the control net read as lengthwise lines and stations.
//!
//! The control net is an unstructured polygon mesh with no rows or columns, but
//! a hull's net is mostly quads laid out as a grid, and a designer reads it as
//! two families of lines: *lengthwise* ones running along the hull and
//! *transverse* ones running round it. A transverse line through a point is the
//! point's **station**.
//!
//! A line is a maximal chain of edges, continued through a point the way
//! edge-loop selection continues: straight across. Only quads carry lines; any
//! other face (a triangle, a transom drawn as one polygon) is a hole, and its
//! rim is walked like an outer boundary. Creases and layers do not stop a line.
//! Every edge belongs to exactly one line.
//!
//! "Straight across" at a point is decided by the quads around it. Walk round
//! the point from quad to quad through the edges they share; the edges met on
//! the way form a *fan*. Two edges continue each other when they are two steps
//! apart in the fan and neither has another such partner:
//!
//! * four quads all the way round (a regular interior point): opposite edges;
//! * two quads (a regular boundary or rim point): the two outer edges, so a
//!   line follows the boundary and the edge between the quads ends there;
//! * three quads (a re-entrant corner, or the corner of a hole): both pairs,
//!   so a grid line stays whole where it runs along the side of a hole;
//! * anything else (one quad, a three- or five-quad pole, an edge on three
//!   quads): nothing continues, and the lines end at the point.
//!
//! The two families are told apart by where lines cross: two edges that are
//! neighbouring sides of one quad are of different families. Each connected
//! patch of quads is two-coloured that way, and the colour whose edges run
//! further along `x` than across it is named lengthwise. A line that touches
//! no quad, or sits in a patch that cannot be two-coloured, is named by its own
//! direction.

use crate::mesh::{ordered, Surface};
use std::collections::{BTreeSet, HashMap, VecDeque};

/// Which of the two families a net line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LineKind {
    /// Runs along the hull: a keel, a chine, a sheer, a buttock-like line.
    Lengthwise,
    /// Runs round the hull: a station.
    Transverse,
}

/// One line of the net.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetLine {
    /// The points in order along the line. A lengthwise line runs from smaller
    /// `x` to larger; a transverse one from the centreline outwards (smaller
    /// `|y|` first, then smaller `z`). A closed line does not repeat its first
    /// point at the end.
    pub points: Vec<usize>,
    /// Indices into `Surface::edges`, in the same order: `edges[k]` joins
    /// `points[k]` and `points[k + 1]` (wrapping for a closed line).
    pub edges: Vec<usize>,
    /// Whether the line returns to its first point.
    pub closed: bool,
    pub kind: LineKind,
}

/// Every line of a net, and which line each edge is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetLines {
    pub lines: Vec<NetLine>,
    /// The line of each edge, indexed like `Surface::edges`.
    pub edge_line: Vec<usize>,
    /// The lines through each point, indexed like `Surface::points`.
    point_lines: Vec<Vec<usize>>,
}

/// A flat grid of quads, for tests here and in the modules built on this one.
///
/// Point `i * ny + j` sits at `(i, j, 0)` for `i < nx`, `j < ny`. One layer,
/// symmetric, so the row `j = 0` is on the centreline. No creases.
#[doc(hidden)]
pub fn grid_net(nx: usize, ny: usize) -> Surface {
    let mut s = Surface::new();
    for i in 0..nx {
        for j in 0..ny {
            s.add_point(crate::math::Vec3::new(i as f64, j as f64, 0.0));
        }
    }
    for i in 0..nx.saturating_sub(1) {
        for j in 0..ny.saturating_sub(1) {
            let p = |i: usize, j: usize| i * ny + j;
            s.add_face(vec![p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j)], 0);
        }
    }
    s.rebuild_edges();
    s
}

impl NetLines {
    /// Read the lines of `surface`'s control net. Never fails: an empty net has
    /// no lines, and an edge that no rule continues is a line of its own.
    pub fn of(surface: &Surface) -> NetLines {
        let net = Net::of(surface);
        let (mut lines, edge_line) = net.walk();
        net.name_families(&mut lines, &edge_line);
        for line in &mut lines {
            net.orient(line);
        }
        let mut point_lines = vec![Vec::new(); surface.points.len()];
        for (li, line) in lines.iter().enumerate() {
            for &p in &line.points {
                if let Some(at) = point_lines.get_mut(p) {
                    if !at.contains(&li) {
                        at.push(li);
                    }
                }
            }
        }
        NetLines {
            lines,
            edge_line,
            point_lines,
        }
    }

    /// The family of the line `edge` is on.
    ///
    /// # Panics
    /// If `edge` is not an edge of the surface these lines were read from.
    pub fn edge_kind(&self, edge: usize) -> LineKind {
        self.lines[self.edge_line[edge]].kind
    }

    /// The lines of `kind` through `point`, as indices into `lines`, ascending.
    /// Empty when there is none, or no such point.
    pub fn through(&self, point: usize, kind: LineKind) -> Vec<usize> {
        self.point_lines
            .get(point)
            .map(|at| {
                at.iter()
                    .copied()
                    .filter(|&li| self.lines[li].kind == kind)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The points of `point`'s station: every point on a transverse line
    /// through it, ascending. Where several transverse lines meet or end at
    /// the point, all of them count. Empty when the point is on none.
    pub fn station_points(&self, point: usize) -> Vec<usize> {
        let set: BTreeSet<usize> = self
            .through(point, LineKind::Transverse)
            .into_iter()
            .flat_map(|li| self.lines[li].points.iter().copied())
            .collect();
        set.into_iter().collect()
    }

    /// The smallest and largest `x` over `point`'s station; equal for an
    /// upright station, apart when it leans. `None` when the point has no
    /// station.
    pub fn station_x(&self, surface: &Surface, point: usize) -> Option<(f64, f64)> {
        let mut range: Option<(f64, f64)> = None;
        for p in self.station_points(point) {
            let x = surface.points.get(p)?.p.x;
            range = Some(range.map_or((x, x), |(lo, hi): (f64, f64)| (lo.min(x), hi.max(x))));
        }
        range
    }

    /// The next point from `point` along a line of `kind` through it:
    /// `forward` follows the line's order (towards the bow on a lengthwise
    /// line, away from the centreline on a transverse one). `None` at the end
    /// of the line, or when no such line passes through the point. Where
    /// several do, the first that has somewhere to go is followed.
    pub fn step(&self, point: usize, kind: LineKind, forward: bool) -> Option<usize> {
        self.through(point, kind).into_iter().find_map(|li| {
            let line = &self.lines[li];
            let n = line.points.len();
            let at = line.points.iter().position(|&p| p == point)?;
            let next = match (forward, line.closed) {
                (true, true) => (at + 1) % n,
                (false, true) => (at + n - 1) % n,
                (true, false) => at + 1,
                (false, false) => at.checked_sub(1)?,
            };
            line.points.get(next).copied().filter(|&p| p != point)
        })
    }
}

/// The net as the walk needs it: which quads each edge is on, and which edge
/// continues which through each point.
struct Net<'a> {
    surface: &'a Surface,
    /// The quads, as four edge indices in order round the face.
    quads: Vec<[usize; 4]>,
    /// `across[e]` is the edge that continues `e` through its end `a`, and
    /// through its end `b`.
    across: Vec<[Option<usize>; 2]>,
}

impl<'a> Net<'a> {
    fn of(surface: &'a Surface) -> Net<'a> {
        let n_points = surface.points.len();
        let mut index: HashMap<(usize, usize), usize> = HashMap::new();
        for (i, e) in surface.edges.iter().enumerate() {
            if e.a != e.b && e.a < n_points && e.b < n_points {
                index.entry(e.key()).or_insert(i);
            }
        }
        // A quad is a face of four different points, all of whose sides are
        // edges of the net; anything else is a hole.
        let mut quads: Vec<[usize; 4]> = Vec::new();
        for f in &surface.faces {
            if f.len() != 4 || (0..4).any(|i| (i + 1..4).any(|j| f[i] == f[j])) {
                continue;
            }
            let side = |i: usize| index.get(&ordered(f[i], f[(i + 1) % 4])).copied();
            if let (Some(a), Some(b), Some(c), Some(d)) = (side(0), side(1), side(2), side(3)) {
                quads.push([a, b, c, d]);
            }
        }
        // Round each point, every quad links the two of its sides that meet
        // there.
        let mut links: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n_points];
        for q in &quads {
            let f = quad_points(surface, q);
            for i in 0..4 {
                // Sides `i - 1` and `i` meet at the quad's point `i`.
                links[f[i]].push((q[(i + 3) % 4], q[i]));
            }
        }
        let mut across = vec![[None, None]; surface.edges.len()];
        for (v, pairs) in links.iter().enumerate() {
            for (e, f) in straight_across(pairs) {
                for (from, to) in [(e, f), (f, e)] {
                    let end = usize::from(surface.edges[from].b == v);
                    across[from][end] = Some(to);
                }
            }
        }
        Net {
            surface,
            quads,
            across,
        }
    }

    /// Chain the edges into lines. Returns the lines (not yet named or
    /// oriented) and the line of each edge.
    fn walk(&self) -> (Vec<NetLine>, Vec<usize>) {
        let edges = &self.surface.edges;
        let mut edge_line = vec![usize::MAX; edges.len()];
        let mut lines: Vec<NetLine> = Vec::new();
        for start in 0..edges.len() {
            if edge_line[start] != usize::MAX {
                continue;
            }
            let li = lines.len();
            edge_line[start] = li;
            let mut points: VecDeque<usize> = VecDeque::from([edges[start].a, edges[start].b]);
            let mut chain: VecDeque<usize> = VecDeque::from([start]);
            let mut closed = false;
            // Out through end `b`, then, unless that came round, out through `a`.
            for forward in [true, false] {
                let (mut e, mut v) = (
                    start,
                    if forward {
                        edges[start].b
                    } else {
                        edges[start].a
                    },
                );
                while let Some(next) = self.across[e][usize::from(edges[e].b == v)] {
                    if next == start {
                        closed = forward;
                        break;
                    }
                    if edge_line[next] != usize::MAX {
                        break;
                    }
                    edge_line[next] = li;
                    v = if edges[next].a == v {
                        edges[next].b
                    } else {
                        edges[next].a
                    };
                    e = next;
                    if forward {
                        points.push_back(v);
                        chain.push_back(e);
                    } else {
                        points.push_front(v);
                        chain.push_front(e);
                    }
                }
                if closed {
                    break;
                }
            }
            if closed {
                // The walk came back to the start edge: the last point pushed is
                // the first one again.
                points.pop_back();
            }
            lines.push(NetLine {
                points: points.into(),
                edges: chain.into(),
                closed,
                kind: LineKind::Transverse,
            });
        }
        (lines, edge_line)
    }

    /// How much further along `x` than across it the edges of a line run.
    fn lean(&self, line: &NetLine) -> f64 {
        line.edges
            .iter()
            .map(|&e| {
                let edge = &self.surface.edges[e];
                let points = &self.surface.points;
                match (points.get(edge.a), points.get(edge.b)) {
                    (Some(a), Some(b)) => {
                        let d = b.p - a.p;
                        d.x.abs() - d.y.hypot(d.z)
                    }
                    _ => 0.0,
                }
            })
            .sum()
    }

    /// Name each line lengthwise or transverse.
    fn name_families(&self, lines: &mut [NetLine], edge_line: &[usize]) {
        // Lines that are neighbouring sides of a quad are of different families.
        let mut cross: Vec<Vec<usize>> = vec![Vec::new(); lines.len()];
        for q in &self.quads {
            for i in 0..4 {
                let (a, b) = (edge_line[q[i]], edge_line[q[(i + 1) % 4]]);
                cross[a].push(b);
                cross[b].push(a);
            }
        }
        let lean: Vec<f64> = lines.iter().map(|l| self.lean(l)).collect();
        let own = |li: usize| {
            if lean[li] >= 0.0 {
                LineKind::Lengthwise
            } else {
                LineKind::Transverse
            }
        };
        let mut side: Vec<Option<bool>> = vec![None; lines.len()];
        for root in 0..lines.len() {
            if side[root].is_some() {
                continue;
            }
            // Two-colour the patch of lines that cross each other, from `root`.
            side[root] = Some(false);
            let mut patch = vec![root];
            let mut two_coloured = true;
            let mut at = 0;
            while at < patch.len() {
                let li = patch[at];
                at += 1;
                for &other in &cross[li] {
                    match side[other] {
                        None => {
                            side[other] = side[li].map(|s| !s);
                            patch.push(other);
                        }
                        Some(s) if Some(s) == side[li] => two_coloured = false,
                        Some(_) => {}
                    }
                }
            }
            let score = |want: bool| -> f64 {
                patch
                    .iter()
                    .filter(|&&li| side[li] == Some(want))
                    .map(|&li| lean[li])
                    .sum()
            };
            let (dark, light) = (score(false), score(true));
            for &li in &patch {
                lines[li].kind = if !two_coloured || patch.len() == 1 || dark == light {
                    own(li)
                } else if (side[li] == Some(false)) == (dark > light) {
                    LineKind::Lengthwise
                } else {
                    LineKind::Transverse
                };
            }
        }
    }

    /// Put a line's points in the documented order.
    fn orient(&self, line: &mut NetLine) {
        let end = |p: Option<&usize>| p.and_then(|&p| self.surface.points.get(p)).map(|c| c.p);
        let (Some(a), Some(b)) = (end(line.points.first()), end(line.points.last())) else {
            return;
        };
        let backwards = if line.closed {
            // A ring has no ends; make it start at its lowest-numbered point
            // and run towards the lower-numbered of that point's neighbours.
            let n = line.points.len();
            let at = (0..n).min_by_key(|&k| line.points[k]).unwrap_or(0);
            line.points.rotate_left(at);
            line.edges.rotate_left(at);
            n > 2 && line.points[n - 1] < line.points[1]
        } else {
            match line.kind {
                LineKind::Lengthwise => (b.x, b.y.abs(), b.z) < (a.x, a.y.abs(), a.z),
                LineKind::Transverse => (b.y.abs(), b.z, b.x) < (a.y.abs(), a.z, a.x),
            }
        };
        if backwards {
            if line.closed {
                // Keep the first point first: reverse the rest.
                line.points[1..].reverse();
                line.edges.reverse();
            } else {
                line.points.reverse();
                line.edges.reverse();
            }
        }
    }
}

/// The four corners of a quad given by its edges, so that sides `i - 1` and
/// `i` meet at corner `i`.
fn quad_points(surface: &Surface, q: &[usize; 4]) -> [usize; 4] {
    let mut corners = [0usize; 4];
    for i in 0..4 {
        let (prev, this) = (&surface.edges[q[(i + 3) % 4]], &surface.edges[q[i]]);
        corners[i] = if this.a == prev.a || this.a == prev.b {
            this.a
        } else {
            this.b
        };
    }
    corners
}

/// Which edges continue each other through one point.
///
/// `pairs` lists, for every quad at the point, the two of its sides that meet
/// there. Walking from quad to quad through shared sides orders the edges into
/// fans. Only three shapes of fan have a "straight across": four quads all the
/// way round, and two or three quads with open ends. In those, two edges
/// continue each other when they are two steps apart.
fn straight_across(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut beside: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(e, f) in pairs {
        beside.entry(e).or_default().push(f);
        beside.entry(f).or_default().push(e);
    }
    // An edge on three or more quads here, or twice beside the same edge,
    // leaves no fan to speak of.
    if beside
        .values()
        .any(|n| n.len() > 2 || (n.len() == 2 && n[0] == n[1]))
    {
        return Vec::new();
    }
    let mut starts: Vec<usize> = beside.keys().copied().collect();
    // Open fans first, from an end, so each is walked once from end to end;
    // what is left after them is closed.
    starts.sort_unstable_by_key(|e| (beside[e].len(), *e));
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    let mut out = Vec::new();
    for start in starts {
        if !seen.insert(start) {
            continue;
        }
        let mut fan = vec![start];
        let mut at = start;
        while let Some(&next) = beside[&at].iter().find(|n| !seen.contains(n)) {
            seen.insert(next);
            fan.push(next);
            at = next;
        }
        let closed = beside[&start].len() == 2;
        match (closed, fan.len()) {
            (true, 4) => out.extend([(fan[0], fan[2]), (fan[1], fan[3])]),
            (false, 3) => out.push((fan[0], fan[2])),
            (false, 4) => out.extend([(fan[0], fan[2]), (fan[1], fan[3])]),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;

    fn count(nl: &NetLines, kind: LineKind) -> usize {
        nl.lines.iter().filter(|l| l.kind == kind).count()
    }

    /// A tube of quads: `nx` rings of `ny` points, ring `i` at `x = i` on the
    /// unit circle. Point `i * ny + j`.
    fn tube_net(nx: usize, ny: usize) -> Surface {
        let mut s = Surface::new();
        s.layers[0].symmetric = false;
        for i in 0..nx {
            for j in 0..ny {
                let a = std::f64::consts::TAU * j as f64 / ny as f64;
                s.add_point(Vec3::new(i as f64, a.cos(), a.sin()));
            }
        }
        for i in 0..nx - 1 {
            for j in 0..ny {
                let p = |i: usize, j: usize| i * ny + j % ny;
                s.add_face(vec![p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j)], 0);
            }
        }
        s.rebuild_edges();
        s
    }

    #[test]
    fn grid_5x4_has_five_transverse_and_four_lengthwise_lines() {
        // Closed form by hand: a 5x4 grid (x = 0..4, y = 0..3) has 5 transverse lines of 4 points at
        // x = i and 4 lengthwise lines of 5 points at y = j. Its outline is four of those nine, not one loop.
        let s = grid_net(5, 4);
        let nl = NetLines::of(&s);
        assert_eq!(nl.lines.len(), 9);
        assert_eq!(count(&nl, LineKind::Transverse), 5);
        assert_eq!(count(&nl, LineKind::Lengthwise), 4);
        for i in 0..5 {
            let p = i * 4 + 2;
            assert_eq!(nl.station_x(&s, p), Some((i as f64, i as f64)));
            assert_eq!(
                nl.station_points(p),
                vec![i * 4, i * 4 + 1, i * 4 + 2, i * 4 + 3]
            );
        }
        assert_eq!(
            nl.step(2 * 4 + 1, LineKind::Lengthwise, true),
            Some(3 * 4 + 1)
        );
        assert_eq!(nl.step(2 * 4 + 1, LineKind::Lengthwise, false), Some(4 + 1));
        assert_eq!(
            nl.step(4 * 4 + 1, LineKind::Lengthwise, true),
            None,
            "the bow end"
        );
    }

    #[test]
    fn a_line_lists_its_points_in_order_with_the_edges_between_them() {
        // By hand on the 5x4 grid: the lengthwise line y = 1 is points 1, 5, 9, 13, 17 (x rising), and the
        // station x = 2 is points 8, 9, 10, 11 (from the centreline row j = 0 outwards).
        let s = grid_net(5, 4);
        let nl = NetLines::of(&s);
        let along = &nl.lines[nl.through(9, LineKind::Lengthwise)[0]];
        assert_eq!(along.points, vec![1, 5, 9, 13, 17]);
        let across = &nl.lines[nl.through(9, LineKind::Transverse)[0]];
        assert_eq!(across.points, vec![8, 9, 10, 11]);
        for line in &nl.lines {
            assert!(!line.closed);
            assert_eq!(line.points.len(), line.edges.len() + 1);
            for (k, &e) in line.edges.iter().enumerate() {
                assert_eq!(
                    s.edges[e].key(),
                    ordered(line.points[k], line.points[k + 1])
                );
                assert_eq!(
                    nl.edge_line[e],
                    nl.lines.iter().position(|l| l == line).unwrap()
                );
            }
        }
        assert_eq!(nl.step(9, LineKind::Transverse, true), Some(10));
        assert_eq!(nl.step(9, LineKind::Transverse, false), Some(8));
        assert_eq!(
            nl.edge_kind(nl.lines[nl.through(9, LineKind::Transverse)[0]].edges[0]),
            LineKind::Transverse
        );
        assert!(
            nl.through(99, LineKind::Transverse).is_empty(),
            "no such point"
        );
        assert!(nl.station_points(99).is_empty());
        assert_eq!(nl.station_x(&s, 99), None);
    }

    #[test]
    fn creases_and_layers_do_not_stop_a_line() {
        // The design: a station that runs up the side and across a deck is one station.
        let plain = grid_net(5, 4);
        let mut marked = plain.clone();
        for e in &mut marked.edges {
            e.crease = true;
        }
        marked.layers.push(crate::mesh::Layer::default());
        for (f, layer) in marked.face_layer.iter_mut().enumerate() {
            *layer = f % 2;
        }
        assert_eq!(NetLines::of(&plain).lines.len(), 9);
        assert_eq!(NetLines::of(&marked), NetLines::of(&plain));
    }

    #[test]
    fn a_ring_of_a_tube_is_one_closed_line() {
        // By hand: a tube of 4 rings of 6 points has 4 closed transverse rings (6 points, 6 edges each, at
        // x = i) and 6 open lengthwise lines of 4 points.
        let s = tube_net(4, 6);
        let nl = NetLines::of(&s);
        assert_eq!(count(&nl, LineKind::Transverse), 4);
        assert_eq!(count(&nl, LineKind::Lengthwise), 6);
        for line in &nl.lines {
            match line.kind {
                LineKind::Transverse => {
                    assert!(line.closed);
                    assert_eq!((line.points.len(), line.edges.len()), (6, 6));
                    for (k, &e) in line.edges.iter().enumerate() {
                        assert_eq!(
                            s.edges[e].key(),
                            ordered(line.points[k], line.points[(k + 1) % 6])
                        );
                    }
                }
                LineKind::Lengthwise => {
                    assert!(!line.closed);
                    assert_eq!(line.points.len(), 4);
                }
            }
        }
        assert_eq!(nl.station_points(2 * 6 + 3), (12..18).collect::<Vec<_>>());
        // Round a closed line a step never runs out.
        let mut p = 12;
        for _ in 0..6 {
            p = nl.step(p, LineKind::Transverse, true).unwrap();
        }
        assert_eq!(p, 12);
    }

    #[test]
    fn demo_hull_stations_follow_its_station_table() {
        // `demo.rs` STATIONS has 9 rows and SECTION_T has 6 ordinates, so the half model has 6 points per
        // station (keel + 5). The transom (x = 0) is one polygon with an extra apex point on the centreline,
        // so its rim is walked as a boundary and the two apex edges, which touch no quad, are lines of their
        // own.
        let s = crate::demo::demo_hull(true);
        let nl = NetLines::of(&s);
        assert_eq!(nl.edge_line.len(), s.edges.len());
        for x in [1.60, 3.40, 5.20, 7.00, 8.60, 10.00, 11.20, 12.00] {
            let p = s.points.iter().position(|c| c.p.x == x).unwrap();
            let st = nl.station_points(p);
            assert_eq!(st.len(), 6, "station at x = {x}");
            assert!(st.iter().all(|&q| s.points[q].p.x == x));
            assert_eq!(nl.station_x(&s, p), Some((x, x)));
        }
        assert!(
            nl.lines.iter().any(|l| l.kind == LineKind::Transverse
                && l.points.len() == 6
                && l.points.iter().all(|&q| s.points[q].p.x == 0.0)),
            "the transom rim is one line"
        );
        // The keel is one lengthwise line through all nine stations; so is each of the five lines above it.
        let keel = nl
            .lines
            .iter()
            .find(|l| {
                l.kind == LineKind::Lengthwise && l.points.iter().all(|&q| s.points[q].p.y == 0.0)
            })
            .expect("a keel line");
        assert_eq!(keel.points.len(), 9);
        assert!(keel
            .points
            .windows(2)
            .all(|w| s.points[w[0]].p.x < s.points[w[1]].p.x));
        assert_eq!(
            nl.lines
                .iter()
                .filter(|l| l.kind == LineKind::Lengthwise && l.points.len() == 9)
                .count(),
            6
        );
        // 9 stations + 6 lengthwise + the 2 apex edges.
        assert_eq!(nl.lines.len(), 17);
    }

    #[test]
    fn the_full_demo_hull_has_whole_stations_port_to_starboard() {
        // The unmirrored demo hull has 5 points a side and the keel: 11 points per station. The transom
        // polygon's rim is the station at x = 0.
        let s = crate::demo::demo_hull(false);
        let nl = NetLines::of(&s);
        for x in [0.0, 1.60, 7.00, 12.00] {
            let p = s.points.iter().position(|c| c.p.x == x).unwrap();
            assert_eq!(nl.station_points(p).len(), 11, "station at x = {x}");
            assert_eq!(nl.station_x(&s, p), Some((x, x)));
        }
    }

    #[test]
    fn every_shipped_example_classifies_every_edge() {
        // Review Focus 1: every built-in example and every examples/*.fef gives each edge a line, and each
        // line is consistent with the edges it lists.
        let mut nets: Vec<(String, Surface)> = Vec::new();
        for ex in crate::examples::Example::with_benchmarks() {
            nets.push((ex.to_string(), crate::examples::build(ex).unwrap().0));
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "fef") {
                let text = std::fs::read_to_string(&path).unwrap();
                nets.push((
                    path.display().to_string(),
                    crate::fef::parse(&text).unwrap(),
                ));
            }
        }
        assert!(nets.len() >= 8, "found only {} nets", nets.len());
        for (name, s) in &nets {
            let nl = NetLines::of(s);
            assert_eq!(nl.edge_line.len(), s.edges.len(), "{name}");
            let mut seen = vec![0usize; s.edges.len()];
            for (li, line) in nl.lines.iter().enumerate() {
                let n = line.points.len();
                assert_eq!(
                    line.edges.len(),
                    if line.closed { n } else { n - 1 },
                    "{name}"
                );
                for (k, &e) in line.edges.iter().enumerate() {
                    assert_eq!(nl.edge_line[e], li, "{name}");
                    assert_eq!(
                        s.edges[e].key(),
                        ordered(line.points[k], line.points[(k + 1) % n]),
                        "{name}"
                    );
                    seen[e] += 1;
                }
            }
            assert!(
                seen.iter().all(|&c| c == 1),
                "{name}: an edge is on no line or on two"
            );
            // A hull's net has stations: most points are on one.
            let on_station = (0..s.points.len())
                .filter(|&p| !nl.station_points(p).is_empty())
                .count();
            assert!(
                on_station * 2 > s.points.len(),
                "{name}: {on_station} of {} points",
                s.points.len()
            );
        }
    }

    #[test]
    fn a_split_face_fan_ends_lines_without_panicking_and_empty_surface_gives_no_lines() {
        // Review Focus 1 and 2: a triangle fan from topology::split_face, and no net at all.
        let mut s = grid_net(5, 4);
        crate::topology::split_face(&mut s, 5).unwrap();
        let nl = NetLines::of(&s);
        assert_eq!(nl.edge_line.len(), s.edges.len());
        assert!(nl.edge_line.iter().all(|&l| l < nl.lines.len()));
        let empty = NetLines::of(&Surface::new());
        assert!(empty.lines.is_empty() && empty.edge_line.is_empty());
        assert!(empty.station_points(0).is_empty());
        assert_eq!(empty.step(0, LineKind::Lengthwise, true), None);
        // Points and no faces: still nothing, and no panic.
        let mut loose = Surface::new();
        loose.add_point(Vec3::ZERO);
        assert!(NetLines::of(&loose)
            .through(0, LineKind::Transverse)
            .is_empty());
    }

    #[test]
    fn a_hole_inside_the_net_leaves_the_grid_lines_whole() {
        // By hand on a 5x5 grid: face 5 is the quad with corners (1,1) (1,2) (2,2) (2,1). Split into a fan it
        // is a hole of four triangles. Each corner keeps three quads, so the lines x = 1, x = 2, y = 1, y = 2
        // run on along the hole's sides: still 5 stations and 5 lengthwise lines of 5 points, and the four
        // spokes to the centre, which touch no quad, are four lines of one edge.
        let mut s = grid_net(5, 5);
        let centre = crate::topology::split_face(&mut s, 5).unwrap().point;
        let nl = NetLines::of(&s);
        let whole = |kind| {
            nl.lines
                .iter()
                .filter(|l| l.kind == kind && l.points.len() == 5)
                .count()
        };
        assert_eq!(whole(LineKind::Transverse), 5);
        assert_eq!(whole(LineKind::Lengthwise), 5);
        assert_eq!(nl.lines.len(), 14);
        assert_eq!(nl.station_points(5 + 1), vec![5, 6, 7, 8, 9]);
        let spokes: Vec<&NetLine> = nl
            .lines
            .iter()
            .filter(|l| l.points.contains(&centre))
            .collect();
        assert_eq!(spokes.len(), 4);
        assert!(spokes.iter().all(|l| l.edges.len() == 1));
    }

    #[test]
    fn a_pole_ends_the_lines_that_reach_it() {
        // Three quads round a point (a cube corner) have no "straight across": by hand, the three edges at
        // the pole end there, and each of the nine edges still gets a line.
        let mut s = Surface::new();
        let pts = [
            (0.0, 0.0, 0.0),
            (1.0, 0.0, 0.0),
            (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0),
            (1.0, 1.0, 0.0),
            (0.0, 1.0, 1.0),
            (1.0, 0.0, 1.0),
        ];
        for (x, y, z) in pts {
            s.add_point(Vec3::new(x, y, z));
        }
        s.add_face(vec![0, 1, 4, 2], 0);
        s.add_face(vec![0, 2, 5, 3], 0);
        s.add_face(vec![0, 3, 6, 1], 0);
        s.rebuild_edges();
        let nl = NetLines::of(&s);
        assert_eq!(nl.edge_line.len(), 9);
        for line in &nl.lines {
            let at = line.points.iter().position(|&p| p == 0);
            assert!(at.is_none() || at == Some(0) || at == Some(line.points.len() - 1));
        }
    }
    #[test]
    fn a_boundary_point_with_four_quads_ends_its_lines() {
        // Four quads fanned round a boundary point (five edges r0..r4 from the point, a gap between r4 and
        // r0) is irregular: by hand, no edge there has one "straight across", so all five lines end at it.
        let mut s = Surface::new();
        s.add_point(Vec3::ZERO);
        let at = |deg: f64, r: f64| {
            let a = deg.to_radians();
            Vec3::new(r * a.cos(), r * a.sin(), 0.0)
        };
        for k in 0..5 {
            s.add_point(at(60.0 * k as f64, 1.0)); // r_k is point 1 + k
        }
        for k in 0..4 {
            s.add_point(at(60.0 * k as f64 + 30.0, 1.4)); // the far corner of quad k is point 6 + k
        }
        for k in 0..4 {
            s.add_face(vec![0, 1 + k, 6 + k, 2 + k], 0);
        }
        s.rebuild_edges();
        let nl = NetLines::of(&s);
        assert_eq!(nl.edge_line.len(), 13);
        for line in &nl.lines {
            let at = line.points.iter().position(|&p| p == 0);
            assert!(
                at.is_none() || at == Some(0) || at == Some(line.points.len() - 1),
                "{:?} runs through the point",
                line.points
            );
        }
    }
}
