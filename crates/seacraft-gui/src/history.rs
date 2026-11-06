//! Snapshot undo/redo.
//!
//! A `Surface` is a few thousand points, so cloning it per edit is cheaper and
//! far less error-prone than a command system with inverse operations.

use seacraft_core::Surface;

/// Bounded undo and redo stacks of whole models (or of whatever state `T`
/// travels with a model, such as the measurement draft).
#[derive(Debug, Default)]
pub struct History<T = Surface> {
    undo: Vec<T>,
    redo: Vec<T>,
    /// The redo stack the newest snapshot cleared, until the next change of
    /// history: [`Self::discard_last`] puts it back.
    cleared_redo: Vec<T>,
    limit: usize,
}

impl<T: Clone> History<T> {
    /// A history that keeps at most `limit` snapshots.
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            cleared_redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Record the state *before* a mutation.
    pub fn snapshot(&mut self, s: &T) {
        self.undo.push(s.clone());
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.cleared_redo = std::mem::take(&mut self.redo);
    }

    /// Swap `current` for the previous state; `current` goes on the redo stack.
    pub fn undo(&mut self, current: &T) -> Option<T> {
        let previous = self.undo.pop()?;
        self.cleared_redo.clear();
        self.redo.push(current.clone());
        Some(previous)
    }

    /// Swap `current` for the next state; `current` goes back on the undo stack.
    pub fn redo(&mut self, current: &T) -> Option<T> {
        let next = self.redo.pop()?;
        self.cleared_redo.clear();
        self.undo.push(current.clone());
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        Some(next)
    }

    /// Forget the newest snapshot: a click that never became a drag must not
    /// leave an undo entry that does nothing, nor cost the redo stack, since
    /// nothing was edited.
    pub fn discard_last(&mut self) -> bool {
        let discarded = self.undo.pop().is_some();
        if discarded && self.redo.is_empty() {
            self.redo = std::mem::take(&mut self.cleared_redo);
        }
        discarded
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.cleared_redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The state an undo would return to.
    pub fn peek_undo(&self) -> Option<&T> {
        self.undo.last()
    }

    /// The state a redo would return to.
    pub fn peek_redo(&self) -> Option<&T> {
        self.redo.last()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    pub fn truncate_undo(&mut self, len: usize) {
        self.undo.truncate(len);
    }

    pub fn clear_redo(&mut self) {
        self.redo.clear();
        self.cleared_redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seacraft_core::demo;

    #[test]
    fn undo_restores_point_positions_and_caps_history() {
        let mut surface = demo::demo_hull(false);
        let mut history = History::new(64);
        let original = surface.points[7].p;
        for _ in 0..100 {
            // Snapshot before mutating, as the app does.
            history.snapshot(&surface);
            surface.points[7].p.z += 0.01;
        }
        assert_eq!(history.undo_len(), 64, "history must be capped");
        let mut current = surface;
        for _ in 0..64 {
            current = history.undo(&current).expect("undo available");
        }
        assert!(history.undo(&current).is_none());
        // The oldest kept snapshot is 64 edits before the end, not the original.
        assert!((current.points[7].p.z - (original.z + 0.36)).abs() < 1e-9);
        assert_eq!(history.redo_len(), 64);
    }

    #[test]
    fn a_discarded_snapshot_keeps_the_redo_stack() {
        let mut history: History<i32> = History::new(8);
        history.snapshot(&1);
        assert_eq!(history.undo(&2), Some(1));
        // A click: snapshot on press, discarded on release.
        history.snapshot(&1);
        assert!(!history.can_redo());
        history.discard_last();
        assert_eq!(history.redo(&1), Some(2));
        // A real edit still ends the redo stack, for good.
        assert_eq!(history.undo(&2), Some(1));
        history.snapshot(&1);
        history.snapshot(&3);
        history.discard_last();
        assert!(!history.can_redo());
    }

    #[test]
    fn redo_replays_an_undone_edit() {
        let mut surface = demo::demo_hull(false);
        let mut history = History::new(8);
        history.snapshot(&surface);
        surface.points[3].p.z += 1.0;
        let edited = surface.points[3].p.z;
        let mut current = history.undo(&surface).expect("undo");
        assert!((current.points[3].p.z - (edited - 1.0)).abs() < 1e-12);
        assert!(history.can_redo());
        current = history.redo(&current).expect("redo");
        assert!((current.points[3].p.z - edited).abs() < 1e-12);
        assert!(!history.can_redo());
    }
}
