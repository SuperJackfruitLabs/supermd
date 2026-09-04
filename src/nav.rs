//! Back/forward history for followed links. Pure: the workspace drives
//! it and owns the tabs, this owns only the order things were visited.

use std::path::PathBuf;

/// A browser-style visit stack: entries plus a cursor into them.
#[derive(Default)]
pub struct History {
    entries: Vec<PathBuf>,
    /// Index of the current entry. Meaningless when `entries` is empty.
    at: usize,
}

impl History {
    /// Record a newly opened file. Re-opening the current file is not a
    /// move, so it does not stack. Visiting after going back discards
    /// the forward branch, which is what every browser does.
    pub fn visit(&mut self, path: PathBuf) {
        if self.entries.get(self.at) == Some(&path) {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.at + 1);
            self.at += 1;
        }
        self.entries.push(path);
        self.at = self.entries.len() - 1;
    }

    pub fn can_back(&self) -> bool { self.at > 0 }

    pub fn can_forward(&self) -> bool {
        !self.entries.is_empty() && self.at + 1 < self.entries.len()
    }

    pub fn back(&mut self) -> Option<PathBuf> {
        if !self.can_back() {
            return None;
        }
        self.at -= 1;
        self.entries.get(self.at).cloned()
    }

    pub fn forward(&mut self) -> Option<PathBuf> {
        if !self.can_forward() {
            return None;
        }
        self.at += 1;
        self.entries.get(self.at).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf { PathBuf::from(s) }

    #[test]
    fn back_and_forward_walk_the_visit_order() {
        let mut h = History::default();
        h.visit(p("/a"));
        h.visit(p("/b"));
        h.visit(p("/c"));
        assert_eq!(h.back(), Some(p("/b")));
        assert_eq!(h.back(), Some(p("/a")));
        assert_eq!(h.back(), None, "cannot go before the first visit");
        assert_eq!(h.forward(), Some(p("/b")));
        assert_eq!(h.forward(), Some(p("/c")));
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn visiting_after_going_back_truncates_the_forward_branch() {
        let mut h = History::default();
        h.visit(p("/a"));
        h.visit(p("/b"));
        h.back();
        h.visit(p("/z"));
        assert_eq!(h.forward(), None, "the old forward entry is gone");
        assert_eq!(h.back(), Some(p("/a")));
    }

    #[test]
    fn revisiting_the_current_file_does_not_stack_duplicates() {
        let mut h = History::default();
        h.visit(p("/a"));
        h.visit(p("/a"));
        assert_eq!(h.back(), None, "re-opening the same file is not a move");
    }

    #[test]
    fn back_does_not_itself_record_a_visit() {
        // Back/forward are moves through history, not new visits. If back()
        // recorded a visit, pressing it twice would oscillate instead of
        // walking backwards.
        let mut h = History::default();
        h.visit(p("/a"));
        h.visit(p("/b"));
        h.visit(p("/c"));
        assert_eq!(h.back(), Some(p("/b")));
        assert_eq!(h.back(), Some(p("/a")), "second back keeps walking backwards");
        assert_eq!(h.back(), None);
    }

    #[test]
    fn can_flags_track_the_ends() {
        let mut h = History::default();
        assert!(!h.can_back() && !h.can_forward());
        h.visit(p("/a"));
        h.visit(p("/b"));
        assert!(h.can_back() && !h.can_forward());
        h.back();
        assert!(h.can_forward());
    }
}
