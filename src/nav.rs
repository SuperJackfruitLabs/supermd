//! Back/forward history for followed links. Pure: the workspace drives
//! it and owns the tabs, this owns only the order things were visited.

use std::path::{Path, PathBuf};

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

    /// Forget everything. A workspace switch invalidates the whole
    /// stack: its entries name files in a folder that is no longer open,
    /// and under the App Store sandbox they are outside the active
    /// security-scoped bookmark, so opening one fails silently.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.at = 0;
    }

    /// Rewrite entries after something moved on disk. `moved` answers
    /// "where did this path go?" — `None` leaves the entry alone. The
    /// filesystem rule lives with the caller (`fileops::retarget`), so
    /// this stays a pure list operation.
    pub fn rewrite(&mut self, mut moved: impl FnMut(&Path) -> Option<PathBuf>) {
        for entry in &mut self.entries {
            if let Some(new) = moved(entry) {
                *entry = new;
            }
        }
    }

    /// Step back to the nearest entry `keep` accepts, skipping over the
    /// ones it rejects (files that have since been deleted, or that a
    /// sandbox can no longer reach). The cursor does not move when
    /// nothing behind it qualifies, so a dead tail cannot swallow the
    /// stack: `back` stays available once the entries come back.
    pub fn back_matching(&mut self, keep: impl Fn(&Path) -> bool) -> Option<PathBuf> {
        let mut ix = self.at;
        while ix > 0 {
            ix -= 1;
            if keep(&self.entries[ix]) {
                self.at = ix;
                return Some(self.entries[ix].clone());
            }
        }
        None
    }

    /// `back_matching`, forwards.
    pub fn forward_matching(&mut self, keep: impl Fn(&Path) -> bool) -> Option<PathBuf> {
        let mut ix = self.at;
        while ix + 1 < self.entries.len() {
            ix += 1;
            if keep(&self.entries[ix]) {
                self.at = ix;
                return Some(self.entries[ix].clone());
            }
        }
        None
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
    fn clearing_forgets_everything() {
        let mut h = History::default();
        h.visit(p("/old/a"));
        h.visit(p("/old/b"));
        h.clear();
        assert!(!h.can_back() && !h.can_forward(), "a cleared stack has no moves");
        h.visit(p("/new/a"));
        assert_eq!(h.back(), None, "the new workspace starts from scratch");
    }

    #[test]
    fn rewriting_follows_moved_paths_and_leaves_the_rest() {
        let mut h = History::default();
        h.visit(p("/w/a"));
        h.visit(p("/w/docs/n"));
        h.visit(p("/w/b"));
        h.rewrite(|entry| {
            entry.strip_prefix("/w/docs").ok().map(|rest| p("/w/notes").join(rest))
        });
        assert_eq!(h.back(), Some(p("/w/notes/n")), "the moved entry follows");
        assert_eq!(h.back(), Some(p("/w/a")), "untouched entries are untouched");
    }

    #[test]
    fn matching_steps_skip_entries_that_are_gone() {
        let mut h = History::default();
        for path in ["/a", "/gone", "/c"] {
            h.visit(p(path));
        }
        let alive = |path: &Path| path != Path::new("/gone");
        assert_eq!(
            h.back_matching(alive), Some(p("/a")),
            "a dead entry is stepped over, not stepped onto"
        );
        assert_eq!(
            h.forward_matching(alive), Some(p("/c")),
            "and forward skips it too"
        );
        // Nothing alive behind us: the cursor stays put rather than
        // walking off the end of the stack.
        let mut h = History::default();
        h.visit(p("/gone"));
        h.visit(p("/c"));
        assert_eq!(h.back_matching(alive), None);
        assert_eq!(h.forward_matching(alive), None, "still sitting on /c");
        assert_eq!(h.back(), Some(p("/gone")), "the cursor never moved");
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
