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
    /// The `[x]` / `[ ]` of a task-list item (checked state carried).
    TaskMarker(bool),
    QuoteMarker,
    FenceContent,
    /// A ``` or ~~~ fence line (never hidden; rendered faded).
    FenceDelimiter,
    Rule,
    /// Tree-sitter capture index into `highlight::CAPTURE_NAMES`.
    Syntax(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub kind: StyleKind,
}

pub(crate) fn markdown_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options
}

pub fn markdown_spans(source: &str) -> Vec<StyleSpan> {
    let mut spans = Vec::new();

    for fence in fence_infos(source) {
        if fence.fenced {
            let mut open = fence.block.start..fence.body.start;
            let mut close = fence.body.end..fence.block.end;
            trim_trailing_newline(source, &mut open);
            trim_trailing_newline(source, &mut close);
            for delim in [open, close] {
                if delim.start < delim.end {
                    spans.push(StyleSpan { range: delim, kind: StyleKind::FenceDelimiter });
                }
            }
        }
        spans.push(StyleSpan { range: fence.body, kind: StyleKind::FenceContent });
    }

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
            Event::TaskListMarker(done) => {
                spans.push(StyleSpan { range, kind: StyleKind::TaskMarker(done) })
            }
            Event::Rule => spans.push(StyleSpan { range, kind: StyleKind::Rule }),
            _ => {}
        }
    }

    spans.sort_by_key(|s| (s.range.start, s.range.end));
    spans
}

pub(crate) fn trim_trailing_newline(source: &str, range: &mut Range<usize>) {
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

use crate::highlight::Languages;

pub const MAX_STYLED_BYTES: usize = 1_000_000;

pub fn code_spans(source: &str, lang: &str, langs: &Languages) -> Vec<StyleSpan> {
    if source.len() > MAX_STYLED_BYTES {
        return Vec::new();
    }
    langs
        .highlight(lang, source)
        .into_iter()
        .map(|(range, capture)| StyleSpan { range, kind: StyleKind::Syntax(capture) })
        .collect()
}

pub fn markdown_spans_highlighted(source: &str, langs: &Languages) -> Vec<StyleSpan> {
    if source.len() > MAX_STYLED_BYTES {
        return Vec::new();
    }
    let mut spans = markdown_spans(source);
    for fence in fence_infos(source) {
        if let Some(lang) = fence.lang {
            let body = fence.body;
            for (range, capture) in langs.highlight(&lang, &source[body.clone()]) {
                spans.push(StyleSpan {
                    range: body.start + range.start..body.start + range.end,
                    kind: StyleKind::Syntax(capture),
                });
            }
        }
    }
    spans.sort_by_key(|s| (s.range.start, s.range.end));
    spans
}

/// One code block: whole-block range, body range, info-string language,
/// and whether it is fenced (indented blocks have no delimiters).
pub(crate) struct FenceInfo {
    pub(crate) block: Range<usize>,
    pub(crate) body: Range<usize>,
    pub(crate) lang: Option<String>,
    pub(crate) fenced: bool,
}

pub(crate) fn fence_infos(source: &str) -> Vec<FenceInfo> {
    use pulldown_cmark::{CodeBlockKind, TagEnd};
    let mut out = Vec::new();
    let mut current: Option<FenceInfo> = None;
    for (event, range) in Parser::new_ext(source, markdown_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let (lang, fenced) = match kind {
                    CodeBlockKind::Fenced(info) if !info.is_empty() => (
                        Some(info.split_whitespace().next().unwrap_or("").to_string()),
                        true,
                    ),
                    CodeBlockKind::Fenced(_) => (None, true),
                    CodeBlockKind::Indented => (None, false),
                };
                current = Some(FenceInfo {
                    block: range.clone(),
                    body: range.start..range.start,
                    lang,
                    fenced,
                });
            }
            Event::Text(_) => {
                if let Some(fence) = current.as_mut() {
                    if fence.body.start == fence.body.end {
                        fence.body = range;
                    } else {
                        fence.body.end = range.end;
                    }
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(fence) = current.take() {
                    if fence.body.start != fence.body.end {
                        out.push(fence);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Body,
    Heading(u8),
    Code,
}

pub fn line_kinds(source: &str, spans: &[StyleSpan]) -> Vec<LineKind> {
    let mut kinds = Vec::new();
    let mut offset = 0;
    for line in source.split('\n') {
        let range = offset..offset + line.len();
        let mut kind = LineKind::Body;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            kind = LineKind::Code;
        }
        // A span intersects this line if it overlaps [start, end); empty
        // lines count as one position wide so spans covering them match.
        let effective_end = range.end.max(range.start + 1);
        for span in spans {
            if span.range.end <= range.start || span.range.start >= effective_end {
                continue;
            }
            match span.kind {
                StyleKind::Heading(n) => kind = LineKind::Heading(n),
                StyleKind::FenceContent => kind = LineKind::Code,
                _ => {}
            }
        }
        kinds.push(kind);
        offset = range.end + 1;
    }
    kinds
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

    use crate::highlight::Languages;

    #[test]
    fn code_spans_highlight_rust() {
        let langs = Languages::new();
        let spans = code_spans("fn main() {}\n", "rust", &langs);
        // "fn" must be captured as something (keyword) => a Syntax span at 0..2
        assert!(spans
            .iter()
            .any(|s| s.range == (0..2) && matches!(s.kind, StyleKind::Syntax(_))));
    }

    #[test]
    fn all_new_grammars_produce_spans() {
        let langs = Languages::new();
        let cases: &[(&str, &str)] = &[
            ("yaml", "key: value\n"),
            ("toml", "[section]\nkey = \"v\"\n"),
            ("ruby", "def foo\n  1\nend\n"),
            ("java", "class A { int x = 1; }\n"),
            ("php", "<?php function f() { return 1; } ?>\n"),
            ("cpp", "int main() { return 0; }\n"),
            ("csharp", "class A { int x = 1; }\n"),
            ("lua", "local x = 1\n"),
            ("elixir", "defmodule A do\n  def f, do: 1\nend\n"),
            ("haskell", "main = print 1\n"),
            ("ocaml", "let x = 1\n"),
            ("scala", "object A { val x = 1 }\n"),
            ("zig", "const x = 1;\n"),
            ("swift", "func f() -> Int { return 1 }\n"),
            ("elm", "x = 1\n"),
            ("erlang", "-module(a).\n"),
            ("sql", "SELECT * FROM t;\n"),
            ("regex", "[a-z]+|foo\n"),
            ("kotlin", "fun main() { val x = 1 }\n"),
            ("julia", "function f()\n    1\nend\n"),
            ("dockerfile", "FROM alpine\nRUN echo hi\n"),
            ("nix", "{ x = 1; }\n"),
            ("r", "x <- 1\n"),
            ("gleam", "pub fn main() { 1 }\n"),
            ("svelte", "<div class=\"a\">{name}</div>\n"),
            ("dart", "void main() {}\n"),
            ("d", "void main() {}\n"),
        ];
        for (lang, src) in cases {
            assert!(
                !langs.highlight(lang, src).is_empty(),
                "no spans for {lang}"
            );
        }
    }

    #[test]
    fn code_spans_unknown_language_is_empty() {
        let langs = Languages::new();
        assert!(code_spans("hello\n", "klingon", &langs).is_empty());
    }

    #[test]
    fn fence_bodies_get_syntax_spans_at_absolute_offsets() {
        let langs = Languages::new();
        let src = "pre\n\n```rust\nfn main() {}\n```\n";
        // fence body "fn main() {}\n" starts at byte 13
        let spans = markdown_spans_highlighted(src, &langs);
        assert!(spans
            .iter()
            .any(|s| s.range == (13..15) && matches!(s.kind, StyleKind::Syntax(_))));
        // structural spans still present
        assert!(spans.iter().any(|s| s.kind == StyleKind::FenceContent));
    }

    #[test]
    fn oversized_source_returns_no_spans() {
        let langs = Languages::new();
        let big = "x".repeat(MAX_STYLED_BYTES + 1);
        assert!(markdown_spans_highlighted(&big, &langs).is_empty());
        assert!(code_spans(&big, "rust", &langs).is_empty());
    }

    #[test]
    fn task_markers_spanned_with_checked_state() {
        let src = "- [x] done\n- [ ] todo\n";
        let spans = markdown_spans(src);
        assert!(spans.contains(&StyleSpan { range: 2..5, kind: StyleKind::TaskMarker(true) }));
        assert!(spans.contains(&StyleSpan { range: 13..16, kind: StyleKind::TaskMarker(false) }));
    }

    #[test]
    fn fence_delimiters_spanned() {
        let src = "```rust\nlet x = 1;\n```\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::FenceDelimiter),
            vec![0..7, 19..22]
        );
    }

    #[test]
    fn tilde_fence_delimiters() {
        let src = "~~~\ncode\n~~~\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::FenceDelimiter),
            vec![0..3, 9..12]
        );
    }

    #[test]
    fn indented_code_has_no_delimiters() {
        let src = "para\n\n    indented code\n";
        assert!(spans_of_kind(src, |k| *k == StyleKind::FenceDelimiter).is_empty());
    }

    #[test]
    fn line_kinds_classify_heading_code_body() {
        let src = "# Title\nbody\n```rust\nlet x = 1;\n```\ntail";
        let langs = Languages::new();
        let spans = markdown_spans_highlighted(src, &langs);
        let kinds = line_kinds(src, &spans);
        assert_eq!(kinds.len(), 6);
        assert_eq!(kinds[0], LineKind::Heading(1));
        assert_eq!(kinds[1], LineKind::Body);
        assert_eq!(kinds[2], LineKind::Code); // ``` delimiter line renders mono
        assert_eq!(kinds[3], LineKind::Code);
        assert_eq!(kinds[4], LineKind::Code);
        assert_eq!(kinds[5], LineKind::Body);
    }
}
