//! Cursor movement over a Buffer. All functions take and return byte
//! offsets, clamped to valid positions. Vertical (up/down) movement is
//! the view layer's job — it needs wrapped-line geometry.

use unicode_segmentation::UnicodeSegmentation;

use super::buffer::Buffer;

pub fn next_grapheme(buf: &Buffer, offset: usize) -> usize {
    let len = buf.len_bytes();
    if offset >= len {
        return len;
    }
    let line_ix = buf.line_of_byte(offset);
    let range = buf.line_range(line_ix);
    if offset >= range.end {
        // Sitting on the newline: step onto the next line's start.
        return (offset + 1).min(len);
    }
    let line = buf.line_text(line_ix);
    let local = offset - range.start;
    line[local..]
        .grapheme_indices(true)
        .nth(1)
        .map(|(i, _)| range.start + local + i)
        .unwrap_or(range.end)
}

pub fn prev_grapheme(buf: &Buffer, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let line_ix = buf.line_of_byte(offset);
    let range = buf.line_range(line_ix);
    if offset <= range.start {
        // At a line start: step back over the newline.
        return offset - 1;
    }
    let line = buf.line_text(line_ix);
    let local = offset - range.start;
    line[..local]
        .grapheme_indices(true)
        .last()
        .map(|(i, _)| range.start + i)
        .unwrap_or(range.start)
}

fn is_word(s: &str) -> bool {
    s.chars().any(|c| c.is_alphanumeric() || c == '_')
}

pub fn next_word(buf: &Buffer, offset: usize) -> usize {
    let text = buf.text();
    for (start, segment) in text[offset..].split_word_bound_indices() {
        if is_word(segment) {
            return offset + start + segment.len();
        }
    }
    text.len()
}

pub fn prev_word(buf: &Buffer, offset: usize) -> usize {
    let text = buf.text();
    let mut result = 0;
    for (start, segment) in text[..offset].split_word_bound_indices() {
        if is_word(segment) {
            result = start;
        }
    }
    result
}

pub fn line_start(buf: &Buffer, offset: usize) -> usize {
    buf.line_range(buf.line_of_byte(offset)).start
}

pub fn line_end(buf: &Buffer, offset: usize) -> usize {
    buf.line_range(buf.line_of_byte(offset)).end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> Buffer {
        Buffer::from_text(s)
    }

    #[test]
    fn grapheme_steps_within_line() {
        let b = buf("aé👍b");
        // bytes: a=1, é=2, 👍=4, b=1
        assert_eq!(next_grapheme(&b, 0), 1);
        assert_eq!(next_grapheme(&b, 1), 3);
        assert_eq!(next_grapheme(&b, 3), 7);
        assert_eq!(next_grapheme(&b, 7), 8);
        assert_eq!(next_grapheme(&b, 8), 8); // clamp at end
        assert_eq!(prev_grapheme(&b, 8), 7);
        assert_eq!(prev_grapheme(&b, 7), 3);
        assert_eq!(prev_grapheme(&b, 0), 0); // clamp at start
    }

    #[test]
    fn grapheme_steps_across_newline() {
        let b = buf("ab\ncd");
        assert_eq!(next_grapheme(&b, 2), 3); // from end of line 0 onto line 1
        assert_eq!(prev_grapheme(&b, 3), 2);
    }

    #[test]
    fn word_steps() {
        let b = buf("foo bar_baz  qux");
        assert_eq!(next_word(&b, 0), 3); // end of "foo"
        assert_eq!(next_word(&b, 3), 11); // end of "bar_baz"
        assert_eq!(next_word(&b, 11), 16); // end of "qux"
        assert_eq!(prev_word(&b, 16), 13); // start of "qux"
        assert_eq!(prev_word(&b, 13), 4); // start of "bar_baz"
        assert_eq!(prev_word(&b, 2), 0);
    }

    #[test]
    fn next_word_without_following_word_clamps_to_end() {
        let b = buf("foo   ");
        assert_eq!(next_word(&b, 3), 6); // only whitespace remains
        assert_eq!(next_word(&b, 6), 6); // already at end
    }

    #[test]
    fn word_steps_cross_lines() {
        let b = buf("foo\nbar");
        assert_eq!(next_word(&b, 3), 7); // from line end into next word
        assert_eq!(prev_word(&b, 4), 0); // from line start into previous word
    }

    #[test]
    fn line_edges() {
        let b = buf("hello\nworld");
        assert_eq!(line_start(&b, 8), 6);
        assert_eq!(line_end(&b, 8), 11);
        assert_eq!(line_start(&b, 3), 0);
        assert_eq!(line_end(&b, 3), 5);
    }
}
