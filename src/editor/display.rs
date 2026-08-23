//! The hybrid-WYSIWYG display transform: the ONE place supermd's
//! "buffer offset == rendered offset" invariant breaks, on purpose.
//!
//! Per line, given the style spans and the current selection, this
//! produces the display text (markers hidden or replaced when their
//! span is not touched by the selection) plus a segment map that the
//! shell uses to convert between source and display offsets. Render-
//! only: the buffer is never touched from here.

use std::ops::Range;

use crate::editor::spans::{StyleKind, StyleSpan};

/// Which source edge an ambiguous display boundary resolves to: hidden
/// opening markers bias Right (cursor lands after the marker, at content
/// start), closing markers bias Left (cursor lands before the marker,
/// at content end). Either way the resolved position touches the span,
/// so it reveals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bias {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegKind {
    Verbatim,
    Hidden(Bias),
    Replacement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    /// Absolute source byte range.
    pub src: Range<usize>,
    /// Byte range within `DisplayLine::text`.
    pub disp: Range<usize>,
    pub kind: SegKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayLine {
    pub text: String,
    pub segs: Vec<Seg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Hide(Bias),
    Replace(&'static str),
}

fn revealed(span: &Range<usize>, sel: &Range<usize>) -> bool {
    span.start <= sel.end && sel.start <= span.end
}

/// Bytes of opening delimiter at local offset `l`, read from the source.
fn leading_delim(kind: &StyleKind, line: &str, l: usize) -> usize {
    let rest = &line[l..];
    match kind {
        StyleKind::Strong => {
            if rest.starts_with("**") || rest.starts_with("__") {
                2
            } else {
                0
            }
        }
        StyleKind::Emphasis => {
            if rest.starts_with('*') || rest.starts_with('_') {
                1
            } else {
                0
            }
        }
        StyleKind::Strikethrough => {
            if rest.starts_with("~~") {
                2
            } else {
                0
            }
        }
        StyleKind::InlineCode => rest.bytes().take_while(|b| *b == b'`').count(),
        _ => 0,
    }
}

/// Bytes of closing delimiter ending at local offset `e`.
fn trailing_delim(kind: &StyleKind, line: &str, e: usize) -> usize {
    let head = &line[..e];
    match kind {
        StyleKind::Strong => {
            if head.ends_with("**") || head.ends_with("__") {
                2
            } else {
                0
            }
        }
        StyleKind::Emphasis => {
            if head.ends_with('*') || head.ends_with('_') {
                1
            } else {
                0
            }
        }
        StyleKind::Strikethrough => {
            if head.ends_with("~~") {
                2
            } else {
                0
            }
        }
        StyleKind::InlineCode => head.bytes().rev().take_while(|b| *b == b'`').count(),
        _ => 0,
    }
}

fn collect_directives(
    line: &str,
    line_start: usize,
    spans: &[StyleSpan],
    selection: &Range<usize>,
) -> Vec<(Range<usize>, Action)> {
    let line_end = line_start + line.len();
    let mut out: Vec<(Range<usize>, Action)> = Vec::new();

    for span in spans {
        if span.range.end < line_start || span.range.start > line_end {
            continue;
        }
        if revealed(&span.range, selection) {
            continue;
        }
        let start_on_line =
            span.range.start >= line_start && span.range.start < line_end;
        let end_on_line = span.range.end > line_start && span.range.end <= line_end;

        match &span.kind {
            StyleKind::Strong
            | StyleKind::Emphasis
            | StyleKind::Strikethrough
            | StyleKind::InlineCode => {
                if start_on_line {
                    let l = span.range.start - line_start;
                    let n = leading_delim(&span.kind, line, l);
                    if n > 0 {
                        out.push((
                            span.range.start..span.range.start + n,
                            Action::Hide(Bias::Right),
                        ));
                    }
                }
                if end_on_line {
                    let e = span.range.end - line_start;
                    let n = trailing_delim(&span.kind, line, e);
                    if n > 0 {
                        out.push((
                            span.range.end - n..span.range.end,
                            Action::Hide(Bias::Left),
                        ));
                    }
                }
            }
            StyleKind::Link => {
                if start_on_line && end_on_line {
                    let l = span.range.start - line_start;
                    let e = span.range.end - line_start;
                    if let Some(text_end) = scan_link(&line[l..e]) {
                        // Hide "[" and "](dest)"; keep the inner text.
                        out.push((
                            span.range.start..span.range.start + 1,
                            Action::Hide(Bias::Right),
                        ));
                        out.push((
                            span.range.start + text_end..span.range.end,
                            Action::Hide(Bias::Left),
                        ));
                    }
                }
            }
            StyleKind::Heading(_) => {
                if start_on_line {
                    let l = span.range.start - line_start;
                    let hashes = line[l..].bytes().take_while(|b| *b == b'#').count();
                    if (1..=6).contains(&hashes) {
                        let mut n = hashes;
                        if line.as_bytes().get(l + hashes).copied() == Some(b' ') {
                            n += 1;
                        }
                        out.push((
                            span.range.start..span.range.start + n,
                            Action::Hide(Bias::Right),
                        ));
                    }
                }
            }
            StyleKind::ListMarker => {
                if start_on_line && end_on_line {
                    let l = span.range.start - line_start;
                    if matches!(line.as_bytes().get(l), Some(b'-' | b'*' | b'+')) {
                        out.push((span.range.clone(), Action::Replace("• ")));
                    }
                    // Ordered markers (digits) are never transformed.
                }
            }
            StyleKind::QuoteMarker => {
                if start_on_line && end_on_line {
                    out.push((span.range.clone(), Action::Replace("▍")));
                }
            }
            _ => {}
        }
    }

    // Sort (outer directive first at equal starts), clamp to the line,
    // drop empties and overlaps (first wins).
    out.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut result: Vec<(Range<usize>, Action)> = Vec::new();
    let mut last_end = line_start;
    for (range, action) in out {
        let start = range.start.max(line_start);
        let end = range.end.min(line_end);
        if start >= end || start < last_end {
            continue;
        }
        last_end = end;
        result.push((start..end, action));
    }
    result
}

/// If `s` is exactly `[text](dest)`, return the byte offset of the `]`
/// closing the text part (so `s[1..ret]` is the inner text and
/// `s[ret..]` is the `](dest)` tail to hide). None if it doesn't scan.
fn scan_link(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }
    let mut depth = 1usize;
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if depth != 0 || i + 1 >= bytes.len() || bytes[i + 1] != b'(' {
        return None;
    }
    let text_end = i; // index of ']'
    let mut paren_depth = 1usize;
    let mut j = i + 2;
    while j < bytes.len() {
        match bytes[j] {
            b'(' => paren_depth += 1,
            b')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if paren_depth != 0 || j + 1 != bytes.len() {
        return None;
    }
    Some(text_end)
}

pub fn display_line(
    line: &str,
    line_start: usize,
    spans: &[StyleSpan],
    selection: Range<usize>,
) -> DisplayLine {
    let line_end = line_start + line.len();
    let directives = collect_directives(line, line_start, spans, &selection);

    let mut text = String::new();
    let mut segs: Vec<Seg> = Vec::new();
    let mut cursor = line_start;

    let push_verbatim = |from: usize, to: usize, text: &mut String, segs: &mut Vec<Seg>| {
        if from < to {
            let disp_start = text.len();
            text.push_str(&line[from - line_start..to - line_start]);
            segs.push(Seg {
                src: from..to,
                disp: disp_start..text.len(),
                kind: SegKind::Verbatim,
            });
        }
    };

    for (range, action) in directives {
        push_verbatim(cursor, range.start, &mut text, &mut segs);
        let disp_start = text.len();
        match action {
            Action::Hide(bias) => segs.push(Seg {
                src: range.clone(),
                disp: disp_start..disp_start,
                kind: SegKind::Hidden(bias),
            }),
            Action::Replace(replacement) => {
                text.push_str(replacement);
                segs.push(Seg {
                    src: range.clone(),
                    disp: disp_start..text.len(),
                    kind: SegKind::Replacement,
                });
            }
        }
        cursor = range.end;
    }
    push_verbatim(cursor, line_end, &mut text, &mut segs);

    if segs.is_empty() {
        // Empty line: one empty verbatim segment anchors the mapping.
        segs.push(Seg {
            src: line_start..line_start,
            disp: 0..0,
            kind: SegKind::Verbatim,
        });
    }

    DisplayLine { text, segs }
}

pub fn src_to_disp(dl: &DisplayLine, src: usize) -> usize {
    for seg in &dl.segs {
        if src < seg.src.start {
            return seg.disp.start;
        }
        if src <= seg.src.end {
            return match seg.kind {
                SegKind::Verbatim => seg.disp.start + (src - seg.src.start),
                SegKind::Hidden(_) => seg.disp.start,
                SegKind::Replacement => {
                    if src == seg.src.start {
                        seg.disp.start
                    } else {
                        seg.disp.end
                    }
                }
            };
        }
    }
    dl.text.len()
}

pub fn disp_to_src(dl: &DisplayLine, disp: usize) -> usize {
    // A verbatim segment ending exactly at `disp` is only a candidate:
    // a following zero-width hidden segment at the same display point
    // wins and resolves by its bias (to the content side of the marker).
    let mut candidate: Option<usize> = None;
    for seg in &dl.segs {
        if disp < seg.disp.start {
            return candidate.unwrap_or(seg.src.start);
        }
        match seg.kind {
            SegKind::Verbatim => {
                if disp < seg.disp.end {
                    return seg.src.start + (disp - seg.disp.start);
                }
                if disp == seg.disp.end {
                    candidate = Some(seg.src.end);
                }
            }
            SegKind::Hidden(bias) => {
                if disp == seg.disp.start {
                    return match bias {
                        Bias::Right => seg.src.end,
                        Bias::Left => seg.src.start,
                    };
                }
            }
            SegKind::Replacement => {
                if disp == seg.disp.start {
                    return seg.src.start;
                }
                if disp <= seg.disp.end {
                    return seg.src.end;
                }
            }
        }
    }
    candidate
        .or_else(|| dl.segs.last().map(|seg| seg.src.end))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::spans::{StyleKind, StyleSpan};

    fn span(range: Range<usize>, kind: StyleKind) -> StyleSpan {
        StyleSpan { range, kind }
    }

    #[test]
    fn strong_markers_hide_when_cursor_elsewhere() {
        let dl = display_line("a **bold** b", 0, &[span(2..10, StyleKind::Strong)], 0..0);
        assert_eq!(dl.text, "a bold b");
    }

    #[test]
    fn strong_reveals_when_selection_touches() {
        for sel in [2..2, 5..5, 10..10, 1..3] {
            let dl = display_line("a **bold** b", 0, &[span(2..10, StyleKind::Strong)], sel);
            assert_eq!(dl.text, "a **bold** b");
        }
    }

    #[test]
    fn cursor_one_past_end_hides() {
        let dl = display_line("a **bold** b", 0, &[span(2..10, StyleKind::Strong)], 11..11);
        assert_eq!(dl.text, "a bold b");
    }

    #[test]
    fn underscore_emphasis_and_strike_and_code() {
        let dl = display_line(
            "_it_ ~~no~~ ``x``",
            0,
            &[
                span(0..4, StyleKind::Emphasis),
                span(5..11, StyleKind::Strikethrough),
                span(12..17, StyleKind::InlineCode),
            ],
            100..100,
        );
        assert_eq!(dl.text, "it no x");
    }

    #[test]
    fn mapping_around_hidden_segments() {
        // "a **bold** b" -> "a bold b"
        let dl = display_line("a **bold** b", 0, &[span(2..10, StyleKind::Strong)], 0..0);
        assert_eq!(src_to_disp(&dl, 0), 0);
        assert_eq!(src_to_disp(&dl, 2), 2); // hidden ** start snaps
        assert_eq!(src_to_disp(&dl, 3), 2); // inside hidden
        assert_eq!(src_to_disp(&dl, 4), 2); // 'b' of bold
        assert_eq!(src_to_disp(&dl, 8), 6); // hidden closing
        assert_eq!(src_to_disp(&dl, 11), 7); // ' ' after
        assert_eq!(disp_to_src(&dl, 2), 4); // 'b' of bold, exact verbatim
        assert_eq!(disp_to_src(&dl, 6), 8);
        assert_eq!(disp_to_src(&dl, 7), 11);
    }

    #[test]
    fn round_trip_for_visible_bytes() {
        let line = "x **b** `c` y";
        let spans = [
            span(2..7, StyleKind::Strong),
            span(8..11, StyleKind::InlineCode),
        ];
        let dl = display_line(line, 0, &spans, 100..100);
        // Round trip holds for every verbatim byte EXCEPT the single byte
        // immediately following a hidden/replaced segment: that display
        // boundary is shared with the marker and resolves to the marker's
        // content side by bias (one display offset cannot invert to two
        // source offsets).
        let mut prev_kind: Option<SegKind> = None;
        for seg in &dl.segs {
            if seg.kind == SegKind::Verbatim {
                for src in seg.src.clone() {
                    let ambiguous = src == seg.src.start
                        && !matches!(prev_kind, None | Some(SegKind::Verbatim));
                    if !ambiguous {
                        assert_eq!(
                            disp_to_src(&dl, src_to_disp(&dl, src)),
                            src,
                            "byte {src}"
                        );
                    }
                }
            }
            prev_kind = Some(seg.kind);
        }
    }

    #[test]
    fn cross_line_span_hides_only_local_delimiter() {
        // Simulates line 2 of "**bold\ntext**": span extends before this line.
        let dl = display_line("text**", 100, &[span(90..106, StyleKind::Strong)], 0..0);
        assert_eq!(dl.text, "text");
        // And line 1: only the leading delimiter is local.
        let dl = display_line("**bold", 84, &[span(84..100, StyleKind::Strong)], 0..0);
        assert_eq!(dl.text, "bold");
    }

    #[test]
    fn bullet_replacement_and_mapping() {
        let dl = display_line("- item", 0, &[span(0..2, StyleKind::ListMarker)], 100..100);
        assert_eq!(dl.text, "• item"); // "• " is 4 bytes
        assert_eq!(src_to_disp(&dl, 0), 0);
        assert_eq!(src_to_disp(&dl, 1), 4); // interior of replacement -> disp end
        assert_eq!(src_to_disp(&dl, 2), 4);
        assert_eq!(disp_to_src(&dl, 0), 0);
        assert_eq!(disp_to_src(&dl, 2), 2); // inside bullet glyph -> content start
        assert_eq!(disp_to_src(&dl, 4), 2);
    }

    #[test]
    fn bullet_reveals_with_cursor_in_marker() {
        let dl = display_line("- item", 0, &[span(0..2, StyleKind::ListMarker)], 1..1);
        assert_eq!(dl.text, "- item");
    }

    #[test]
    fn ordered_marker_never_transforms() {
        let dl = display_line("1. item", 0, &[span(0..3, StyleKind::ListMarker)], 100..100);
        assert_eq!(dl.text, "1. item");
    }

    #[test]
    fn quote_marker_becomes_bar() {
        let dl = display_line("> quoted", 0, &[span(0..1, StyleKind::QuoteMarker)], 100..100);
        assert_eq!(dl.text, "▍ quoted");
    }

    #[test]
    fn heading_hashes_hide_when_cursor_off_line() {
        let dl = display_line("## Title", 0, &[span(0..8, StyleKind::Heading(2))], 100..100);
        assert_eq!(dl.text, "Title");
        // Cursor anywhere on the line (span covers it) reveals.
        let dl = display_line("## Title", 0, &[span(0..8, StyleKind::Heading(2))], 5..5);
        assert_eq!(dl.text, "## Title");
    }

    #[test]
    fn link_collapses_to_inner_text() {
        // "see [zed](https://zed.dev) now" — Link span 4..26
        let dl = display_line(
            "see [zed](https://zed.dev) now",
            0,
            &[span(4..26, StyleKind::Link)],
            100..100,
        );
        assert_eq!(dl.text, "see zed now");
    }

    #[test]
    fn link_reveals_on_touch() {
        let dl = display_line(
            "see [zed](https://zed.dev) now",
            0,
            &[span(4..26, StyleKind::Link)],
            8..8,
        );
        assert_eq!(dl.text, "see [zed](https://zed.dev) now");
    }

    #[test]
    fn nested_brackets_in_link_text() {
        // "[a[b]c](u)" span 0..10 -> inner "a[b]c"
        let dl = display_line("[a[b]c](u)", 0, &[span(0..10, StyleKind::Link)], 100..100);
        assert_eq!(dl.text, "a[b]c");
    }

    #[test]
    fn unscannable_link_stays_visible() {
        // Span deliberately not matching [text](dest) shape.
        let dl = display_line("<https://x.y>", 0, &[span(0..13, StyleKind::Link)], 100..100);
        assert_eq!(dl.text, "<https://x.y>");
    }

    #[test]
    fn overlapping_directives_first_wins() {
        // Two spans claiming overlapping delimiter bytes: the second's
        // conflicting directive is dropped, output stays consistent.
        // (Synthetic — real pulldown spans nest without delimiter overlap.)
        let spans = [
            span(0..7, StyleKind::Strong),
            span(1..6, StyleKind::Emphasis),
        ];
        let dl = display_line("***it***", 0, &spans, 100..100);
        assert_eq!(dl.text, "*it*");
        for seg in &dl.segs {
            if seg.kind == SegKind::Verbatim {
                for src in seg.src.clone() {
                    if src != seg.src.start {
                        assert_eq!(disp_to_src(&dl, src_to_disp(&dl, src)), src);
                    }
                }
            }
        }
    }

    #[test]
    fn no_spans_passthrough_identity() {
        let dl = display_line("plain text", 50, &[], 0..0);
        assert_eq!(dl.text, "plain text");
        assert_eq!(src_to_disp(&dl, 53), 3);
        assert_eq!(disp_to_src(&dl, 3), 53);
    }
}
