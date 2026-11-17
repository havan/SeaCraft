//! Multi-point control net selection: an ordered list of picked point indices
//! with one active point (the one shown in the Inspector).

/// Picked control points in pick order, with one active point.
#[derive(Default, Clone, PartialEq, Eq, Debug)]
pub struct Selection {
    points: Vec<usize>,
    active: Option<usize>,
}

impl Selection {
    /// The active point (the most recently added, or the last remaining after
    /// removal), shown in the Inspector.
    pub fn active(&self) -> Option<usize> {
        self.active
    }

    /// All selected points in pick order.
    pub fn points(&self) -> &[usize] {
        &self.points
    }

    /// Whether `p` is in the selection.
    pub fn contains(&self, p: usize) -> bool {
        self.points.contains(&p)
    }

    /// How many points are selected.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether no points are selected.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Replace the selection with `p` alone and make it active.
    pub fn set_only(&mut self, p: usize) {
        self.points.clear();
        self.points.push(p);
        self.active = Some(p);
    }

    /// Add `p` (making it active) if not selected, or remove it (making the
    /// last remaining point active) if already selected.
    pub fn toggle(&mut self, p: usize) {
        if let Some(pos) = self.points.iter().position(|&x| x == p) {
            self.points.remove(pos);
            self.active = self.points.last().copied();
        } else {
            self.points.push(p);
            self.active = Some(p);
        }
    }

    /// Append each point in `iter` that is not already selected, in the order
    /// given, and make the last item given active.
    pub fn add_all(&mut self, iter: impl IntoIterator<Item = usize>) {
        for p in iter {
            if !self.points.contains(&p) {
                self.points.push(p);
            }
            self.active = Some(p);
        }
    }

    /// Empty the selection.
    pub fn clear(&mut self) {
        self.points.clear();
        self.active = None;
    }

    /// The two selected points in pick order when `len() == 2`.
    pub fn pair(&self) -> Option<[usize; 2]> {
        match self.points.as_slice() {
            &[a, b] => Some([a, b]),
            _ => None,
        }
    }

    /// Drop any point index `>= n_points`; if the active point was dropped,
    /// make the last remaining point active.
    pub fn retain_valid(&mut self, n_points: usize) {
        self.points.retain(|&p| p < n_points);
        if self.active.is_some_and(|a| !self.points.contains(&a)) {
            self.active = self.points.last().copied();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_toggle_pair_and_retain_valid_behave_as_specified() {
        let mut s = Selection::default();
        s.set_only(3);
        assert_eq!(
            (s.active(), s.points(), s.pair()),
            (Some(3), &[3][..], None)
        );
        s.toggle(7);
        assert_eq!(
            (s.active(), s.points(), s.pair()),
            (Some(7), &[3, 7][..], Some([3, 7]))
        );
        s.toggle(7);
        assert_eq!((s.active(), s.points()), (Some(3), &[3][..]));
        s.add_all([5, 3, 9]);
        assert_eq!((s.active(), s.points()), (Some(9), &[3, 5, 9][..]));
        s.retain_valid(6);
        assert_eq!((s.active(), s.points()), (Some(5), &[3, 5][..]));
    }
}
