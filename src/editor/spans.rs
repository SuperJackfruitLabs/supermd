//! Source text → style spans for the styled-source editor. Byte ranges
//! over the raw source; markers are included in their spans (they stay
//! visible in Phase 2).

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleKind {
    Heading(u8),
    Strong,
    Emphasis,
    Strikethrough,
    InlineCode,
    Link,
    ListMarker,
    QuoteMarker,
    FenceContent,
    Rule,
    /// Tree-sitter capture index into `highlight::CAPTURE_NAMES`.
    Syntax(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub kind: StyleKind,
}

fn markdown_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options
}

pub fn markdown_spans(source: &str) -> Vec<StyleSpan> {
    let mut spans = Vec::new();
    let mut fence_body: Option<Range<usize>> = None;

    for (event, range) in Parser::new_ext(source, markdown_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let mut r = range.clone();
                trim_trailing_newline(source, &mut r);
                spans.push(StyleSpan { range: r, kind: StyleKind::Heading(level as u8) });
            }
            Event::Start(Tag::Strong) => spans.push(StyleSpan { range, kind: StyleKind::Strong }),
            Event::Start(Tag::Emphasis) => {
                spans.push(StyleSpan { range, kind: StyleKind::Emphasis })
            }
            Event::Start(Tag::Strikethrough) => {
                spans.push(StyleSpan { range, kind: StyleKind::Strikethrough })
            }
            Event::Code(_) => spans.push(StyleSpan { range, kind: StyleKind::InlineCode }),
            Event::Start(Tag::Link { .. }) => {
                spans.push(StyleSpan { range, kind: StyleKind::Link })
            }
            Event::Start(Tag::Item) => {
                if let Some(len) = list_marker_len(&source[range.clone()]) {
                    spans.push(StyleSpan {
                        range: range.start..range.start + len,
                        kind: StyleKind::ListMarker,
                    });
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                for (line_start, line) in line_offsets(&source[range.clone()]) {
                    let abs = range.start + line_start;
                    let marks = line.bytes().take_while(|b| *b == b'>').count();
                    if marks > 0 {
                        spans.push(StyleSpan {
                            range: abs..abs + marks,
                            kind: StyleKind::QuoteMarker,
                        });
                    }
                }
            }
            Event::Start(Tag::CodeBlock(_)) => fence_body = Some(range.start..range.start),
            Event::Text(_) if fence_body.is_some() => {
                let body = fence_body.as_mut().unwrap();
                if body.is_empty() {
                    *body = range;
                } else {
                    body.end = range.end;
                }
            }
            Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                if let Some(body) = fence_body.take() {
                    if !body.is_empty() {
                        spans.push(StyleSpan { range: body, kind: StyleKind::FenceContent });
                    }
                }
            }
            Event::Rule => spans.push(StyleSpan { range, kind: StyleKind::Rule }),
            _ => {}
        }
    }

    spans.sort_by_key(|s| (s.range.start, s.range.end));
    spans
}

fn trim_trailing_newline(source: &str, range: &mut Range<usize>) {
    while range.end > range.start && source.as_bytes()[range.end - 1] == b'\n' {
        range.end -= 1;
    }
}

/// Byte length of a list marker ("- ", "* ", "12. ", "3) ") at the start
/// of an item, including one trailing space.
fn list_marker_len(item: &str) -> Option<usize> {
    let bytes = item.as_bytes();
    let mut i = 0;
    if matches!(bytes.first(), Some(b'-' | b'*' | b'+')) {
        i = 1;
    } else {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == 0 || i >= bytes.len() || !(bytes[i] == b'.' || bytes[i] == b')') {
            return None;
        }
        i += 1;
    }
    if bytes.get(i).copied() == Some(b' ') {
        i += 1;
    }
    Some(i)
}

/// (byte offset, line) pairs over a str, offsets relative to the input.
fn line_offsets(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for line in s.split_inclusive('\n') {
        out.push((start, line.trim_end_matches('\n')));
        start += line.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans_of_kind(source: &str, kind_match: fn(&StyleKind) -> bool) -> Vec<Range<usize>> {
        markdown_spans(source)
            .into_iter()
            .filter(|s| kind_match(&s.kind))
            .map(|s| s.range)
            .collect()
    }

    #[test]
    fn heading_span_covers_marks_and_text() {
        let src = "# Title\n\nbody\n";
        let spans = markdown_spans(src);
        assert!(spans.contains(&StyleSpan { range: 0..7, kind: StyleKind::Heading(1) }));
    }

    #[test]
    fn heading_levels() {
        let src = "## Two\n### Three\n";
        let spans = markdown_spans(src);
        assert!(spans.contains(&StyleSpan { range: 0..6, kind: StyleKind::Heading(2) }));
        assert!(spans.contains(&StyleSpan { range: 7..16, kind: StyleKind::Heading(3) }));
    }

    #[test]
    fn strong_and_emphasis_include_markers() {
        let src = "a **bold** and *it*\n";
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::Strong), vec![2..10]);
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::Emphasis), vec![15..19]);
    }

    #[test]
    fn inline_code_includes_backticks() {
        let src = "use `foo()` here\n";
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::InlineCode), vec![4..11]);
    }

    #[test]
    fn fence_content_covers_body_not_delimiters() {
        let src = "```rust\nlet x = 1;\n```\n";
        // body is "let x = 1;\n" at bytes 8..19
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::FenceContent), vec![8..19]);
    }

    #[test]
    fn list_markers_are_spanned() {
        let src = "- one\n- two\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::ListMarker),
            vec![0..2, 6..8]
        );
    }

    #[test]
    fn ordered_list_markers() {
        let src = "1. one\n2. two\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::ListMarker),
            vec![0..3, 7..10]
        );
    }

    #[test]
    fn quote_markers_per_line() {
        let src = "> a\n> b\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::QuoteMarker),
            vec![0..1, 4..5]
        );
    }

    #[test]
    fn link_span() {
        let src = "see [zed](https://zed.dev) now\n";
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::Link), vec![4..26]);
    }

    #[test]
    fn rule_span() {
        let src = "a\n\n---\n\nb\n";
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::Rule), vec![3..7]);
    }
}
