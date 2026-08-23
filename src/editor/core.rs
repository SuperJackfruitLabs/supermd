//! EditorCore: the tested facade the GPUI shell drives. Owns the buffer,
//! one selection, and the undo history.

use std::ops::Range;
use std::time::Instant;

use super::buffer::{Buffer, Edit};
use super::movement;

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
}

impl EditorCore {
    pub fn new(text: &str) -> Self {
        Self {
            buffer: Buffer::from_text(text),
            selection: Selection::cursor(0),
        }
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

    fn apply(&mut self, range: Range<usize>, text: &str, _now: Instant) -> Edit {
        let edit = self.buffer.replace(range.clone(), text);
        self.selection = Selection::cursor(range.start + text.len());
        edit
    }

    pub fn insert(&mut self, text: &str, now: Instant) {
        self.apply(self.selection.range(), text, now);
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
}
