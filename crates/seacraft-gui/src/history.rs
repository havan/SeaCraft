//! Snapshot undo/redo.
//!
//! A `Surface` is a few thousand points, so cloning it per edit is cheaper and
//! far less error-prone than a command system with inverse operations.

use seacraft_core::Surface;

/// Bounded undo and redo stacks of whole models.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Surface>,
    redo: Vec<Surface>,
    limit: usize,
}

impl History {
    /// A history that keeps at most `limit` snapshots.
    pub fn new(limit: usize) -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit: limit.max(1) }
    }

    /// Record the state *before* a mutation.
    pub fn snapshot(&mut self, s: &Surface) {
        self.undo.push(s.clone());
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Swap `current` for the previous state; `current` goes on the redo stack.
    pub fn undo(&mut self, current: &Surface) -> Option<Surface> {
        let previous = self.undo.pop()?;
        self.redo.push(current.clone());
        Some(previous)
    }

    /// Swap `current` for the next state; `current` goes back on the undo stack.
    pub fn redo(&mut self, current: &Surface) -> Option<Surface> {
        let next = self.redo.pop()?;
        self.undo.push(current.clone());
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        Some(next)
    }

    /// Forget the newest snapshot: a click that never became a drag must not
    /// leave an undo entry that does nothing.
    pub fn discard_last(&mut self) -> bool {
        self.undo.pop().is_some()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
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
            surface.points[7].p = surface.points[7].p + seacraft_core::math::Vec3::new(0.0, 0.0, 0.01);
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
