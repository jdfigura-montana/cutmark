//! editor::history: undo/redo history (1,000 steps, session only).

use std::sync::Arc;

use crate::model::project::Project;

/// Maximum number of undo entries kept; the oldest drops off.
pub const MAX_HISTORY: usize = 1_000;

/// One undo step: the label shown in menus and the snapshot taken before the edit.
pub struct HistoryEntry {
    /// Menu label such as "Track Volume".
    pub label: &'static str,
    /// Whole-project snapshot taken before the edit.
    pub before: Arc<Project>,
}

/// Undo and redo stacks of whole-project snapshots, capped at `MAX_HISTORY`.
pub struct History {
    /// Snapshots to restore, newest last.
    pub undo: Vec<HistoryEntry>,
    /// Snapshots to restore after undo, newest last.
    pub redo: Vec<HistoryEntry>,
}

impl History {
    /// Creates empty undo and redo stacks.
    pub fn new() -> History {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Pushes one entry, clears redo and drops the oldest past the cap.
    pub fn push_undo(&mut self, entry: HistoryEntry) {
        self.redo.clear();
        self.undo.push(entry);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
    }

    /// Pops the top undo entry, pushing the current project onto redo.
    pub fn undo_step(&mut self, current: Arc<Project>) -> Option<HistoryEntry> {
        let entry = self.undo.pop()?;
        self.redo.push(HistoryEntry {
            label: entry.label,
            before: current,
        });
        Some(entry)
    }

    /// Pops the top redo entry, pushing the current project onto undo.
    pub fn redo_step(&mut self, current: Arc<Project>) -> Option<HistoryEntry> {
        let entry = self.redo.pop()?;
        self.undo.push(HistoryEntry {
            label: entry.label,
            before: current,
        });
        Some(entry)
    }

    /// Label of the next undo, if any.
    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|entry| entry.label)
    }

    /// Label of the next redo, if any.
    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|entry| entry.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::project::{BitDepth, Project};

    fn plain() -> Project {
        Project::new("T".to_string(), 48_000, BitDepth::Int24)
    }

    #[test]
    fn push_undo_clears_redo_and_caps_at_max() {
        let mut history = History::new();
        for _ in 0..(MAX_HISTORY + 1) {
            history.push_undo(HistoryEntry {
                label: "L",
                before: Arc::new(plain()),
            });
        }
        assert_eq!(history.undo.len(), MAX_HISTORY);
        history.push_undo(HistoryEntry {
            label: "L",
            before: Arc::new(plain()),
        });
        history.undo_step(Arc::new(plain()));
        assert_eq!(history.redo.len(), 1);
        history.push_undo(HistoryEntry {
            label: "L",
            before: Arc::new(plain()),
        });
        assert!(history.redo.is_empty());
    }
}
