//! Text buffer: a rope with byte-offset addressing and a single edit
//! primitive. Byte offsets are the universal currency across supermd.

use std::ops::Range;

use ropey::Rope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Pre-edit byte range that was replaced.
    pub range: Range<usize>,
    pub old: String,
    pub new: String,
}

pub struct Buffer {
    rope: Rope,
}

impl Buffer {
    pub fn from_text(text: &str) -> Self {
        Self { rope: Rope::from_str(text) }
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_text(&self, ix: usize) -> String {
        let line = self.rope.line(ix).to_string();
        line.strip_suffix('\n').map(str::to_string).unwrap_or(line)
    }

    pub fn line_range(&self, ix: usize) -> Range<usize> {
        let start = self.rope.line_to_byte(ix);
        start..start + self.line_text(ix).len()
    }

    pub fn line_of_byte(&self, offset: usize) -> usize {
        self.rope.byte_to_line(offset.min(self.rope.len_bytes()))
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.slice(start..end).to_string()
    }

    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Edit {
        let old = self.slice(range.clone());
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.remove(start..end);
        self.rope.insert(start, text);
        Edit { range, old, new: text.to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_text_round_trips() {
        let buf = Buffer::from_text("hello\nworld\n");
        assert_eq!(buf.text(), "hello\nworld\n");
        assert_eq!(buf.len_bytes(), 12);
    }

    #[test]
    fn line_accessors() {
        let buf = Buffer::from_text("alpha\nbëta\n\ngamma");
        assert_eq!(buf.line_count(), 4);
        assert_eq!(buf.line_text(0), "alpha");
        assert_eq!(buf.line_text(1), "bëta");
        assert_eq!(buf.line_text(2), "");
        assert_eq!(buf.line_text(3), "gamma");
        assert_eq!(buf.line_range(0), 0..5);
        assert_eq!(buf.line_range(1), 6..11); // "bëta" is 5 bytes (ë = 2)
        assert_eq!(buf.line_range(2), 12..12);
        assert_eq!(buf.line_range(3), 13..18);
        assert_eq!(buf.line_of_byte(0), 0);
        assert_eq!(buf.line_of_byte(5), 0); // the newline belongs to line 0
        assert_eq!(buf.line_of_byte(6), 1);
        assert_eq!(buf.line_of_byte(18), 3); // end of buffer
    }

    #[test]
    fn replace_inserts_deletes_and_reports_edit() {
        let mut buf = Buffer::from_text("hello world");
        let edit = buf.replace(5..5, ",");
        assert_eq!(buf.text(), "hello, world");
        assert_eq!(edit, Edit { range: 5..5, old: String::new(), new: ",".into() });

        let edit = buf.replace(0..6, "");
        assert_eq!(buf.text(), " world");
        assert_eq!(edit, Edit { range: 0..6, old: "hello,".into(), new: String::new() });

        let edit = buf.replace(1..6, "moon");
        assert_eq!(buf.text(), " moon");
        assert_eq!(edit.old, "world");
    }

    #[test]
    fn slice_returns_byte_range() {
        let buf = Buffer::from_text("héllo");
        assert_eq!(buf.slice(0..3), "hé");
        assert_eq!(buf.slice(3..6), "llo");
    }
}
