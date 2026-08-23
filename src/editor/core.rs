//! EditorCore: the tested facade the GPUI shell drives. Owns the buffer,
//! one selection, and the undo history.

use std::ops::Range;
use std::time::{Duration, Instant};

use super::buffer::{Buffer, Edit};
use super::movement;

pub const GROUP_WINDOW: Duration = Duration::from_millis(700);

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
}

struct UndoGroup {
    /// Edits in application order.
    edits: Vec<Edit>,
    selection_before: Selection,
    selection_after: Selection,
}

#[derive(Default)]
struct History {
    undo: Vec<UndoGroup>,
    redo: Vec<UndoGroup>,
    last_kind: Option<EditKind>,
    last_at: Option<Instant>,
    broken: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn cursor(offset: usize) -> Self {
        Self { anchor: offset, head: offset }
    }

    pub fn is_cursor(&self) -> bool {
        self.anchor == self.head
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

pub struct EditorCore {
    pub buffer: Buffer,
    pub selection: Selection,
    history: History,
}

impl EditorCore {
    pub fn new(text: &str) -> Self {
        Self {
            buffer: Buffer::from_text(text),
            selection: Selection::cursor(0),
            history: History::default(),
        }
    }

    pub fn break_undo_group(&mut self) {
        self.history.broken = true;
    }

    pub fn undo(&mut self) -> bool {
        let Some(group) = self.history.undo.pop() else {
            return false;
        };
        for edit in group.edits.iter().rev() {
            let range = edit.range.start..edit.range.start + edit.new.len();
            self.buffer.replace(range, &edit.old);
        }
        self.selection = group.selection_before;
        self.history.redo.push(group);
        self.history.broken = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(group) = self.history.redo.pop() else {
            return false;
        };
        for edit in &group.edits {
            self.buffer.replace(edit.range.clone(), &edit.new);
        }
        self.selection = group.selection_after;
        self.history.undo.push(group);
        self.history.broken = true;
        true
    }

    pub fn set_cursor(&mut self, offset: usize) {
        let offset = offset.min(self.buffer.len_bytes());
        self.selection = Selection::cursor(offset);
    }

    pub fn select_to(&mut self, offset: usize) {
        self.selection.head = offset.min(self.buffer.len_bytes());
    }

    pub fn select_all(&mut self) {
        self.selection = Selection { anchor: 0, head: self.buffer.len_bytes() };
    }

    pub fn selected_text(&self) -> String {
        self.buffer.slice(self.selection.range())
    }

    fn apply(&mut self, range: Range<usize>, text: &str, now: Instant) {
        let selection_before = self.selection;
        let kind = if text.is_empty() { EditKind::Delete } else { EditKind::Insert };
        let edit = self.buffer.replace(range.clone(), text);
        self.selection = Selection::cursor(range.start + text.len());

        let coalesce = !self.history.broken
            && self.history.last_kind == Some(kind)
            && self
                .history
                .last_at
                .is_some_and(|at| now.duration_since(at) <= GROUP_WINDOW)
            && !self.history.undo.is_empty();
        if coalesce {
            let group = self.history.undo.last_mut().unwrap();
            group.edits.push(edit);
            group.selection_after = self.selection;
        } else {
            self.history.undo.push(UndoGroup {
                edits: vec![edit],
                selection_before,
                selection_after: self.selection,
            });
        }
        self.history.redo.clear();
        self.history.last_kind = Some(kind);
        self.history.last_at = Some(now);
        self.history.broken = false;
    }

    pub fn insert(&mut self, text: &str, now: Instant) {
        self.apply(self.selection.range(), text, now);
    }

    /// Apply an arbitrary replacement through the normal history path.
    /// The cursor lands at the end of the inserted text.
    pub fn replace_range(&mut self, range: Range<usize>, text: &str, now: Instant) {
        self.apply(range, text, now);
    }

    /// Newline that copies the current line's leading whitespace (only
    /// the part left of the cursor). Used by the editor in code mode.
    pub fn insert_newline_auto_indent(&mut self, now: Instant) {
        let head = self.selection.range().start;
        let line_ix = self.buffer.line_of_byte(head);
        let line_start = self.buffer.line_range(line_ix).start;
        let line = self.buffer.line_text(line_ix);
        let upto = head.saturating_sub(line_start).min(line.len());
        let ws_len = line
            .bytes()
            .take_while(|b| *b == b' ' || *b == b'\t')
            .count()
            .min(upto);
        let mut text = String::with_capacity(1 + ws_len);
        text.push('\n');
        text.push_str(&line[..ws_len]);
        self.insert(&text, now);
    }

    pub fn backspace(&mut self, now: Instant) {
        let range = if self.selection.is_cursor() {
            movement::prev_grapheme(&self.buffer, self.selection.head)..self.selection.head
        } else {
            self.selection.range()
        };
        if !range.is_empty() {
            self.apply(range, "", now);
        }
    }

    pub fn delete_forward(&mut self, now: Instant) {
        let range = if self.selection.is_cursor() {
            self.selection.head..movement::next_grapheme(&self.buffer, self.selection.head)
        } else {
            self.selection.range()
        };
        if !range.is_empty() {
            self.apply(range, "", now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn insert_at_cursor_advances_cursor() {
        let mut ed = EditorCore::new("world");
        ed.insert("hello ", now());
        assert_eq!(ed.buffer.text(), "hello world");
        assert_eq!(ed.selection, Selection::cursor(6));
    }

    #[test]
    fn insert_replaces_selection() {
        let mut ed = EditorCore::new("hello world");
        ed.set_cursor(0);
        ed.select_to(5);
        ed.insert("goodbye", now());
        assert_eq!(ed.buffer.text(), "goodbye world");
        assert_eq!(ed.selection, Selection::cursor(7));
    }

    #[test]
    fn backspace_deletes_grapheme_or_selection() {
        let mut ed = EditorCore::new("ab👍");
        ed.set_cursor(6);
        ed.backspace(now());
        assert_eq!(ed.buffer.text(), "ab");
        assert_eq!(ed.selection, Selection::cursor(2));

        let mut ed = EditorCore::new("hello world");
        ed.set_cursor(5);
        ed.select_to(11);
        ed.backspace(now());
        assert_eq!(ed.buffer.text(), "hello");
    }

    #[test]
    fn delete_forward_deletes_next_grapheme() {
        let mut ed = EditorCore::new("a👍b");
        ed.set_cursor(1);
        ed.delete_forward(now());
        assert_eq!(ed.buffer.text(), "ab");
        assert_eq!(ed.selection, Selection::cursor(1));
    }

    #[test]
    fn select_all_and_selected_text() {
        let mut ed = EditorCore::new("héllo");
        ed.select_all();
        assert_eq!(ed.selected_text(), "héllo");
        assert_eq!(ed.selection.range(), 0..6);
    }

    #[test]
    fn selection_range_normalizes_reversed() {
        let sel = Selection { anchor: 9, head: 2 };
        assert_eq!(sel.range(), 2..9);
        assert!(!sel.is_cursor());
        assert!(Selection::cursor(4).is_cursor());
    }

    use std::time::Duration;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn undo_reverts_and_redo_reapplies() {
        let mut ed = EditorCore::new("abc");
        ed.set_cursor(3);
        ed.insert("d", t0());
        assert!(ed.undo());
        assert_eq!(ed.buffer.text(), "abc");
        assert_eq!(ed.selection, Selection::cursor(3));
        assert!(ed.redo());
        assert_eq!(ed.buffer.text(), "abcd");
        assert_eq!(ed.selection, Selection::cursor(4));
        assert!(!ed.redo());
    }

    #[test]
    fn rapid_typing_coalesces_into_one_group() {
        let mut ed = EditorCore::new("");
        let start = t0();
        ed.insert("h", start);
        ed.insert("e", start + Duration::from_millis(100));
        ed.insert("y", start + Duration::from_millis(200));
        assert!(ed.undo());
        assert_eq!(ed.buffer.text(), "");
        assert!(!ed.undo());
    }

    #[test]
    fn pause_breaks_group() {
        let mut ed = EditorCore::new("");
        let start = t0();
        ed.insert("hi", start);
        ed.insert(" there", start + Duration::from_millis(1500));
        ed.undo();
        assert_eq!(ed.buffer.text(), "hi");
        ed.undo();
        assert_eq!(ed.buffer.text(), "");
    }

    #[test]
    fn kind_change_breaks_group() {
        let mut ed = EditorCore::new("");
        let start = t0();
        ed.insert("hey", start);
        ed.backspace(start + Duration::from_millis(50));
        ed.insert("y", start + Duration::from_millis(100));
        assert_eq!(ed.buffer.text(), "hey");
        ed.undo(); // undoes the trailing "y" insert
        assert_eq!(ed.buffer.text(), "he");
        ed.undo(); // undoes the backspace
        assert_eq!(ed.buffer.text(), "hey");
        ed.undo(); // undoes the initial insert
        assert_eq!(ed.buffer.text(), "");
    }

    #[test]
    fn cursor_jump_breaks_group() {
        let mut ed = EditorCore::new("xx");
        let start = t0();
        ed.set_cursor(2);
        ed.insert("a", start);
        ed.set_cursor(0);
        ed.break_undo_group();
        ed.insert("b", start + Duration::from_millis(50));
        ed.undo();
        assert_eq!(ed.buffer.text(), "xxa");
    }

    #[test]
    fn replace_range_edits_through_history() {
        let mut ed = EditorCore::new("- [x] done");
        ed.set_cursor(8);
        ed.replace_range(2..5, "[ ]", t0());
        assert_eq!(ed.buffer.text(), "- [ ] done");
        assert_eq!(ed.selection, Selection::cursor(5));
        assert!(ed.undo());
        assert_eq!(ed.buffer.text(), "- [x] done");
    }

    #[test]
    fn newline_copies_leading_whitespace() {
        let mut ed = EditorCore::new("    let x = 1;");
        ed.set_cursor(14);
        ed.insert_newline_auto_indent(t0());
        assert_eq!(ed.buffer.text(), "    let x = 1;\n    ");
        assert_eq!(ed.selection, Selection::cursor(19));
    }

    #[test]
    fn newline_mid_line_indents_and_carries_tail() {
        let mut ed = EditorCore::new("\tfoo(bar)");
        ed.set_cursor(5);
        ed.insert_newline_auto_indent(t0());
        assert_eq!(ed.buffer.text(), "\tfoo(\n\tbar)");
    }

    #[test]
    fn newline_without_indent_is_plain() {
        let mut ed = EditorCore::new("plain");
        ed.set_cursor(5);
        ed.insert_newline_auto_indent(t0());
        assert_eq!(ed.buffer.text(), "plain\n");
    }

    #[test]
    fn newline_indent_stops_at_cursor_column() {
        // Cursor inside the indentation: only the part left of the
        // cursor carries over.
        let mut ed = EditorCore::new("        x");
        ed.set_cursor(4);
        ed.insert_newline_auto_indent(t0());
        assert_eq!(ed.buffer.text(), "    \n        x");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut ed = EditorCore::new("");
        ed.insert("a", t0());
        ed.undo();
        ed.insert("b", t0());
        assert!(!ed.redo());
        assert_eq!(ed.buffer.text(), "b");
    }
}
