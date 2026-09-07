//! CommonMark source → block model.
//!
//! The plain-text file is the source of truth; this module turns it into a
//! tree of blocks the view layer can render. In later phases this parser will
//! be replaced by an incremental tree-sitter pass over the editing buffer, but
//! the block model is designed to survive that swap.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Inline style flags for a span of text within a block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: bool,
}

impl SpanStyle {
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }
}

/// A run of inline content: the text plus styled byte ranges.
/// Ranges are non-overlapping and sorted; unlisted bytes are plain.
#[derive(Debug, Default, Clone)]
pub struct InlineText {
    pub text: String,
    pub spans: Vec<(Range<usize>, SpanStyle)>,
    /// Where each link in `text` points, by byte range. A wiki link's
    /// destination is prefixed `[[` so a consumer can resolve it by
    /// stem rather than as a path — without it the reading view
    /// classified `[[Editing]]` as a relative path, found no such file,
    /// and every wiki link in the preview was silently dead.
    ///
    /// `SpanStyle::link` records only *that* a run is a link; the
    /// destination was dropped on the floor, so the rendered view could
    /// draw a link but never follow one. Kept beside the spans rather
    /// than inside `SpanStyle`, which is `Copy` and compared by value.
    pub links: Vec<(Range<usize>, String)>,
}

#[derive(Debug, Clone)]
pub struct ListItem {
    /// Some(done) for task-list items, None for plain items.
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone)]
pub enum Block {
    Heading { level: u8, content: InlineText },
    Paragraph(InlineText),
    Code {
        lang: Option<String>,
        code: String,
        /// Syntax highlight spans: (byte range, capture index into
        /// `highlight::CAPTURE_NAMES`). Filled in after parsing.
        spans: Vec<(Range<usize>, u8)>,
    },
    Quote(Vec<Block>),
    List { start: Option<u64>, items: Vec<ListItem> },
    Table { head: Vec<InlineText>, rows: Vec<Vec<InlineText>> },
    Rule,
}

#[derive(Debug, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
}

/// Accumulates inline events into an `InlineText`.
#[derive(Default)]
struct InlineBuilder {
    out: InlineText,
    /// Byte offset where the currently-open link began, and where it
    /// points. Nested links are not a thing in CommonMark.
    open_link: Option<(usize, String)>,
}

impl InlineBuilder {
    fn push(&mut self, s: &str, style: SpanStyle) {
        let start = self.out.text.len();
        self.out.text.push_str(s);
        if !style.is_plain() {
            self.out.spans.push((start..self.out.text.len(), style));
        }
    }

    fn is_empty(&self) -> bool {
        self.out.text.is_empty()
    }

    fn begin_link(&mut self, dest: String) {
        self.open_link = Some((self.out.text.len(), dest));
    }

    fn end_link(&mut self) {
        if let Some((start, dest)) = self.open_link.take() {
            let end = self.out.text.len();
            if start < end {
                self.out.links.push((start..end, dest));
            }
        }
    }

    fn finish(mut self) -> InlineText {
        resolve_wiki_links(&mut self.out);
        self.out
    }
}

/// Mark `range` as a link, splitting any span it partially overlaps so
/// the result stays non-overlapping and sorted.
fn apply_link_style(spans: &mut Vec<(Range<usize>, SpanStyle)>, range: Range<usize>) {
    let mut out: Vec<(Range<usize>, SpanStyle)> = Vec::with_capacity(spans.len() + 2);
    let mut covered: Vec<Range<usize>> = Vec::new();
    for (r, st) in spans.iter() {
        // No overlap: keep as is.
        if r.end <= range.start || range.end <= r.start {
            out.push((r.clone(), *st));
            continue;
        }
        // The part before the link keeps the original style.
        if r.start < range.start {
            out.push((r.start..range.start, *st));
        }
        // The overlapping part gains `link` on top of what it had.
        let mid = r.start.max(range.start)..r.end.min(range.end);
        if mid.start < mid.end {
            out.push((mid.clone(), SpanStyle { link: true, ..*st }));
            covered.push(mid);
        }
        // And the part after keeps the original.
        if range.end < r.end {
            out.push((range.end..r.end, *st));
        }
    }
    // Whatever the link covers that no existing span did.
    covered.sort_by_key(|r| r.start);
    let mut at = range.start;
    for c in covered {
        if at < c.start {
            out.push((at..c.start, SpanStyle { link: true, ..Default::default() }));
        }
        at = at.max(c.end);
    }
    if at < range.end {
        out.push((at..range.end, SpanStyle { link: true, ..Default::default() }));
    }
    *spans = out;
}

/// Turn `[[Target]]` and `[[Target|label]]` into real links.
///
/// CommonMark has no wiki-link syntax, so pulldown-cmark hands these
/// back as ordinary text and the rendered view drew them as literal
/// `[[Editing]]` — unstyled, unclickable, brackets showing. The editor
/// learned about wiki links through its own span pass; this is the
/// same idea for the reading view, which is a separate pipeline.
///
/// The brackets (and any `Target|` prefix) are removed from the text,
/// so every existing span offset after a match has to move with it.
fn resolve_wiki_links(inline: &mut InlineText) {
    if !inline.text.contains("[[") {
        return;
    }
    // Code spans are literal: `[[not a link]]` stays as written.
    let code: Vec<Range<usize>> = inline
        .spans
        .iter()
        .filter(|(_, st)| st.code)
        .map(|(r, _)| r.clone())
        .collect();

    let src = std::mem::take(&mut inline.text);
    let mut out = String::with_capacity(src.len());
    // old byte offset -> new byte offset, for remapping the spans.
    let mut map = vec![0usize; src.len() + 1];
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    let mut link_spans: Vec<Range<usize>> = Vec::new();

    let bytes = src.as_bytes();
    let mut i = 0usize;
    while i < src.len() {
        map[i] = out.len();
        let starts_wiki = bytes[i] == b'['
            && bytes.get(i + 1) == Some(&b'[')
            && !code.iter().any(|r| r.contains(&i));
        let close = starts_wiki.then(|| src[i + 2..].find("]]")).flatten();
        let Some(rel) = close else {
            let ch_len = src[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&src[i..i + ch_len]);
            for k in i..i + ch_len {
                map[k] = map[i];
            }
            i += ch_len;
            continue;
        };
        let inner = &src[i + 2..i + 2 + rel];
        let (target, label) = match inner.split_once('|') {
            Some((t, l)) => (t.trim(), l.trim()),
            None => (inner.trim(), inner.trim()),
        };
        if target.is_empty() {
            out.push_str("[[");
            map[i + 1] = map[i];
            i += 2;
            continue;
        }
        let start = out.len();
        out.push_str(label);
        // Every byte of the source match maps to the start of the
        // label: a style span that covered part of it now covers the
        // label instead, which is the only sensible answer.
        for k in i..(i + 2 + rel + 2).min(map.len()) {
            map[k] = start;
        }
        link_spans.push(start..out.len());
        links.push((start..out.len(), format!("[[{target}")));
        i += 2 + rel + 2;
    }
    map[src.len()] = out.len();

    let remap = |o: usize| map.get(o).copied().unwrap_or(out.len());
    inline.spans = inline
        .spans
        .iter()
        .map(|(r, st)| (remap(r.start)..remap(r.end), *st))
        .filter(|(r, _)| r.start < r.end)
        .collect();
    // Markdown links found before the rewrite carry pre-rewrite
    // offsets, and removing brackets moved everything after each match.
    // Leaving them stale pointed a link at the wrong words -- or past
    // the end of the text -- while it still rendered as a link.
    inline.links = inline
        .links
        .iter()
        .map(|(r, dest)| (remap(r.start)..remap(r.end), dest.clone()))
        .filter(|(r, _)| r.start < r.end)
        .collect();
    // Merge `link` into the styles already covering those bytes rather
    // than appending a second span. `InlineText` documents its ranges
    // as non-overlapping, and `view::runs_for` relies on it: two spans
    // over the same bytes emit two runs, so the painted runs outrun the
    // text and a wiki link inside `**bold**` underlined the wrong four
    // characters and lost the tail's weight.
    for r in link_spans {
        apply_link_style(&mut inline.spans, r);
    }
    inline.spans.sort_by_key(|(r, _)| (r.start, r.end));
    // Wiki targets are appended after any markdown links already found,
    // then sorted so the ranges stay in document order.
    inline.links.extend(links);
    inline.links.sort_by_key(|(r, _)| r.start);
    inline.text = out;
}

/// Nesting depth counters for the inline styles currently open.
#[derive(Default)]
struct StyleStack {
    bold: u32,
    italic: u32,
    strike: u32,
    link: u32,
    image: u32,
}

impl StyleStack {
    fn current(&self) -> SpanStyle {
        SpanStyle {
            bold: self.bold > 0,
            italic: self.italic > 0 || self.image > 0,
            code: false,
            strike: self.strike > 0,
            link: self.link > 0,
        }
    }
}

/// What produced the currently-open container of blocks.
enum Frame {
    Quote,
    Item { checked: Option<bool> },
}

pub fn parse(source: &str) -> Document {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    // Open containers. The bottom Vec is the document itself; quotes and list
    // items push a new Vec and fold it into a Block when they close.
    let mut containers: Vec<Vec<Block>> = vec![Vec::new()];
    let mut frames: Vec<Frame> = Vec::new();

    // Stack of open lists (they contain items, not blocks directly).
    let mut lists: Vec<(Option<u64>, Vec<ListItem>)> = Vec::new();

    let mut inline: Option<InlineBuilder> = None;
    let mut styles = StyleStack::default();

    let mut code: Option<(Option<String>, String)> = None;

    // Table state: header cells, body rows, row in progress.
    let mut table: Option<(Vec<InlineText>, Vec<Vec<InlineText>>)> = None;
    let mut table_row: Vec<InlineText> = Vec::new();

    // Flush any loose inline content (tight list items have no Paragraph tag).
    fn flush_inline(inline: &mut Option<InlineBuilder>, containers: &mut [Vec<Block>]) {
        if let Some(builder) = inline.take() {
            if !builder.is_empty() {
                containers
                    .last_mut()
                    .expect("container stack is never empty")
                    .push(Block::Paragraph(builder.finish()));
            }
        }
    }

    for event in Parser::new_ext(source, options) {
        match event {
            // ── Leaf blocks with inline content ─────────────────────────
            Event::Start(Tag::Paragraph | Tag::Heading { .. }) => {
                flush_inline(&mut inline, &mut containers);
                inline = Some(InlineBuilder::default());
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some(builder) = inline.take() {
                    containers.last_mut().unwrap().push(Block::Paragraph(builder.finish()));
                }
            }
            Event::End(TagEnd::Heading(level)) => {
                if let Some(builder) = inline.take() {
                    containers.last_mut().unwrap().push(Block::Heading {
                        level: level as u8,
                        content: builder.finish(),
                    });
                }
            }

            // ── Code blocks ─────────────────────────────────────────────
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_inline(&mut inline, &mut containers);
                let lang = match kind {
                    CodeBlockKind::Fenced(info) if !info.is_empty() => {
                        Some(info.split_whitespace().next().unwrap_or("").to_string())
                    }
                    _ => None,
                };
                code = Some((lang, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, mut text)) = code.take() {
                    // Fenced blocks end with a trailing newline we don't render.
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    containers.last_mut().unwrap().push(Block::Code {
                        lang,
                        code: text,
                        spans: Vec::new(),
                    });
                }
            }

            // ── Containers ──────────────────────────────────────────────
            Event::Start(Tag::BlockQuote(_)) => {
                flush_inline(&mut inline, &mut containers);
                frames.push(Frame::Quote);
                containers.push(Vec::new());
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush_inline(&mut inline, &mut containers);
                let blocks = containers.pop().unwrap();
                frames.pop();
                containers.last_mut().unwrap().push(Block::Quote(blocks));
            }
            Event::Start(Tag::List(start)) => {
                flush_inline(&mut inline, &mut containers);
                lists.push((start, Vec::new()));
            }
            Event::End(TagEnd::List(_)) => {
                let (start, items) = lists.pop().unwrap();
                containers.last_mut().unwrap().push(Block::List { start, items });
            }
            Event::Start(Tag::Item) => {
                frames.push(Frame::Item { checked: None });
                containers.push(Vec::new());
                // Tight list items carry inline content with no Paragraph tag.
                inline = Some(InlineBuilder::default());
            }
            Event::End(TagEnd::Item) => {
                flush_inline(&mut inline, &mut containers);
                let blocks = containers.pop().unwrap();
                let checked = match frames.pop() {
                    Some(Frame::Item { checked }) => checked,
                    _ => None,
                };
                lists
                    .last_mut()
                    .expect("item outside list")
                    .1
                    .push(ListItem { checked, blocks });
            }
            Event::TaskListMarker(done) => {
                if let Some(Frame::Item { checked }) = frames.last_mut() {
                    *checked = Some(done);
                }
            }

            // ── Tables ──────────────────────────────────────────────────
            Event::Start(Tag::Table(_)) => {
                flush_inline(&mut inline, &mut containers);
                table = Some((Vec::new(), Vec::new()));
            }
            Event::Start(Tag::TableHead) => table_row = Vec::new(),
            Event::End(TagEnd::TableHead) => {
                if let Some((head, _)) = table.as_mut() {
                    *head = std::mem::take(&mut table_row);
                }
            }
            Event::Start(Tag::TableRow) => table_row = Vec::new(),
            Event::End(TagEnd::TableRow) => {
                if let Some((_, rows)) = table.as_mut() {
                    rows.push(std::mem::take(&mut table_row));
                }
            }
            Event::Start(Tag::TableCell) => inline = Some(InlineBuilder::default()),
            Event::End(TagEnd::TableCell) => {
                if let Some(builder) = inline.take() {
                    table_row.push(builder.finish());
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some((head, rows)) = table.take() {
                    containers.last_mut().unwrap().push(Block::Table { head, rows });
                }
            }

            // ── Inline styles ───────────────────────────────────────────
            Event::Start(Tag::Strong) => styles.bold += 1,
            Event::End(TagEnd::Strong) => styles.bold -= 1,
            Event::Start(Tag::Emphasis) => styles.italic += 1,
            Event::End(TagEnd::Emphasis) => styles.italic -= 1,
            Event::Start(Tag::Strikethrough) => styles.strike += 1,
            Event::End(TagEnd::Strikethrough) => styles.strike -= 1,
            Event::Start(Tag::Link { dest_url, .. }) => {
                styles.link += 1;
                if let Some(builder) = inline.as_mut() {
                    builder.begin_link(dest_url.to_string());
                }
            }
            Event::End(TagEnd::Link) => {
                styles.link -= 1;
                if let Some(builder) = inline.as_mut() {
                    builder.end_link();
                }
            }
            Event::Start(Tag::Image { .. }) => {
                // Phase 0: render images as a labeled placeholder of their alt text.
                styles.image += 1;
                if let Some(builder) = inline.as_mut() {
                    builder.push("🖼 ", styles.current());
                }
            }
            Event::End(TagEnd::Image) => styles.image -= 1,

            // ── Inline content ──────────────────────────────────────────
            Event::Text(text) => {
                if let Some((_, buffer)) = code.as_mut() {
                    buffer.push_str(&text);
                } else if let Some(builder) = inline.as_mut() {
                    builder.push(&text, styles.current());
                }
            }
            Event::Code(text) => {
                if let Some(builder) = inline.as_mut() {
                    let style = SpanStyle { code: true, ..styles.current() };
                    builder.push(&text, style);
                }
            }
            Event::SoftBreak => {
                if let Some(builder) = inline.as_mut() {
                    builder.push(" ", SpanStyle::default());
                }
            }
            Event::HardBreak => {
                if let Some(builder) = inline.as_mut() {
                    builder.push("\n", SpanStyle::default());
                }
            }

            Event::Rule => {
                flush_inline(&mut inline, &mut containers);
                containers.last_mut().unwrap().push(Block::Rule);
            }

            // HTML, footnotes, math: out of scope for Phase 0.
            _ => {}
        }
    }

    flush_inline(&mut inline, &mut containers);
    Document {
        blocks: containers.pop().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_one(source: &str) -> Block {
        let mut doc = parse(source);
        assert_eq!(doc.blocks.len(), 1, "expected one block from {source:?}");
        doc.blocks.pop().unwrap()
    }

    #[test]
    fn empty_source_is_empty_document() {
        assert!(parse("").blocks.is_empty());
    }

    #[test]
    fn heading_levels_and_text() {
        for level in 1..=6u8 {
            let src = format!("{} title", "#".repeat(level as usize));
            let Block::Heading { level: l, content } = parse_one(&src) else { panic!("expected heading") };
            assert_eq!(l, level);
            assert_eq!(content.text, "title");
            assert!(content.spans.is_empty());
        }
    }

    #[test]
    fn paragraph_plain_text_has_no_spans() {
        let Block::Paragraph(inline) = parse_one("just words") else { panic!("expected paragraph") };
        assert_eq!(inline.text, "just words");
        assert!(inline.spans.is_empty());
    }

    #[test]
    fn bold_italic_and_nested_styles() {
        let Block::Paragraph(inline) = parse_one("a **b *c*** d") else { panic!("expected paragraph") };
        assert_eq!(inline.text, "a b c d");
        let bold = SpanStyle { bold: true, ..Default::default() };
        let bold_italic = SpanStyle { bold: true, italic: true, ..Default::default() };
        assert_eq!(inline.spans, vec![(2..4, bold), (4..5, bold_italic)]);
    }

    /// The reading view drew `[[Editing]]` as literal grey text with
    /// its brackets showing: unstyled, unclickable, and the default
    /// view once a single click started opening previews. CommonMark
    /// has no wiki syntax, so the reader's parser has to add it.
    #[test]
    fn wiki_links_become_links_in_the_rendered_view() {
        let doc = parse("see [[Editing]] and [[Links and notes]] here\n");
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("paragraph") };
        assert_eq!(
            inline.text, "see Editing and Links and notes here",
            "the brackets are gone from the rendered text"
        );
        let targets: Vec<(&str, &str)> = inline
            .links
            .iter()
            .map(|(r, d)| (&inline.text[r.clone()], d.as_str()))
            .collect();
        assert_eq!(
            targets,
            vec![("Editing", "[[Editing"), ("Links and notes", "[[Links and notes")]
        );
        assert!(
            inline.spans.iter().any(|(r, st)| st.link && &inline.text[r.clone()] == "Editing"),
            "and they are styled as links: {:?}",
            inline.spans
        );
    }

    /// `[[Target|label]]` shows the label and points at the target.
    #[test]
    fn a_labelled_wiki_link_shows_its_label() {
        let doc = parse("[[Tables|the table guide]] follows\n");
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("paragraph") };
        assert_eq!(inline.text, "the table guide follows");
        assert_eq!(inline.links[0].1, "[[Tables");
        assert_eq!(&inline.text[inline.links[0].0.clone()], "the table guide");
    }

    /// Code is literal: a wiki link inside backticks stays as written.
    #[test]
    fn a_wiki_link_inside_code_is_left_alone() {
        let doc = parse("`[[not a link]]` but [[Real]] is\n");
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("paragraph") };
        assert!(inline.text.contains("[[not a link]]"), "code kept: {}", inline.text);
        assert_eq!(inline.links.len(), 1, "only the real one is a link");
        assert_eq!(inline.links[0].1, "[[Real");
    }

    /// Styling around a rewritten link must not end up pointing at the
    /// wrong bytes — every offset after a match moves when the brackets
    /// are removed.
    #[test]
    fn spans_after_a_wiki_link_still_line_up() {
        let doc = parse("[[A]] then **bold** after\n");
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("paragraph") };
        assert_eq!(inline.text, "A then bold after");
        let bold = inline
            .spans
            .iter()
            .find(|(_, st)| st.bold)
            .map(|(r, _)| inline.text[r.clone()].to_string());
        assert_eq!(bold.as_deref(), Some("bold"), "spans: {:?}", inline.spans);
    }

    /// Malformed shapes must not panic or eat the rest of the line.
    #[test]
    fn unterminated_and_empty_wiki_links_are_left_as_text() {
        assert_eq!(
            match &parse("an [[unterminated link\n").blocks[0] {
                Block::Paragraph(i) => i.text.clone(),
                _ => panic!(),
            },
            "an [[unterminated link"
        );
        assert_eq!(
            match &parse("empty [[]] here\n").blocks[0] {
                Block::Paragraph(i) => i.text.clone(),
                _ => panic!(),
            },
            "empty [[]] here"
        );
    }

    /// A rendered link must know where it points. The destination used
    /// to be dropped at parse time — `SpanStyle` recorded only that a
    /// run *was* a link — so the reading view could draw links it could
    /// never follow.
    #[test]
    fn link_destinations_survive_parsing() {
        let doc = parse("see [the spec](https://commonmark.org) and [a note](Notes/a.md)\n");
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("paragraph") };
        let targets: Vec<(&str, &str)> = inline
            .links
            .iter()
            .map(|(r, dest)| (&inline.text[r.clone()], dest.as_str()))
            .collect();
        assert_eq!(
            targets,
            vec![
                ("the spec", "https://commonmark.org"),
                ("a note", "Notes/a.md"),
            ]
        );
    }

    /// `InlineText` documents its spans as non-overlapping, and
    /// `view::runs_for` relies on it: two spans over the same bytes emit
    /// two runs, so the painted runs outran the text and a wiki link
    /// inside `**bold**` underlined the wrong characters and lost the
    /// tail's weight.
    #[test]
    fn a_wiki_link_inside_emphasis_keeps_spans_disjoint() {
        let doc = parse("**bold [[Wiki]] more**\n");
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("paragraph") };
        assert_eq!(inline.text, "bold Wiki more");

        // Sorted, disjoint, in bounds.
        let mut last = 0usize;
        for (r, _) in &inline.spans {
            assert!(r.start >= last, "spans overlap or are unsorted: {:?}", inline.spans);
            assert!(r.end <= inline.text.len(), "span past the end: {r:?}");
            last = r.end;
        }
        // The link's own bytes carry both styles.
        let at = inline.text.find("Wiki").unwrap();
        let (_, st) = inline
            .spans
            .iter()
            .find(|(r, _)| r.start <= at && at < r.end)
            .expect("a span covers the link");
        assert!(st.link, "the link text is a link");
        assert!(st.bold, "and keeps the emphasis it sits inside");
    }

    /// A wiki link and a markdown link in one paragraph: the rewrite
    /// shortens the text, so every markdown-link offset after it moves.
    /// Leaving them stale pointed a link at the wrong words, or past the
    /// end of the text, while it still rendered as a link.
    #[test]
    fn markdown_links_after_a_wiki_link_still_point_at_their_own_text() {
        let doc = parse("[[Setup]] see [guide](g.md) and [FAQ](f.md)\n");
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("paragraph") };
        assert_eq!(inline.text, "Setup see guide and FAQ");
        let got: Vec<(&str, &str)> = inline
            .links
            .iter()
            .map(|(r, d)| (&inline.text[r.clone()], d.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![("Setup", "[[Setup"), ("guide", "g.md"), ("FAQ", "f.md")],
            "every link covers its own words"
        );
        // And none of them can index outside the text.
        for (r, _) in &inline.links {
            assert!(r.end <= inline.text.len(), "range {r:?} past the end");
        }
    }

    /// A link whose text is styled still records one range covering the
    /// whole of it, not one per style run.
    #[test]
    fn a_styled_link_is_still_one_destination() {
        let doc = parse("[**bold** and *italic*](x.md)\n");
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("paragraph") };
        assert_eq!(inline.links.len(), 1);
        let (range, dest) = &inline.links[0];
        assert_eq!(&inline.text[range.clone()], "bold and italic");
        assert_eq!(dest, "x.md");
    }

    #[test]
    fn inline_code_strike_and_link() {
        let Block::Paragraph(inline) = parse_one("`x` ~~y~~ [z](https://example.com)") else { panic!("expected paragraph") };
        assert_eq!(inline.text, "x y z");
        let code = SpanStyle { code: true, ..Default::default() };
        let strike = SpanStyle { strike: true, ..Default::default() };
        let link = SpanStyle { link: true, ..Default::default() };
        assert_eq!(inline.spans, vec![(0..1, code), (2..3, strike), (4..5, link)]);
    }

    #[test]
    fn image_renders_placeholder_with_italic_alt() {
        let Block::Paragraph(inline) = parse_one("![alt text](img.png)") else { panic!("expected paragraph") };
        assert_eq!(inline.text, "\u{1f5bc} alt text");
        assert!(inline.spans.iter().all(|(_, s)| s.italic));
        assert_eq!(inline.spans.last().unwrap().0.end, inline.text.len());
    }

    #[test]
    fn soft_break_is_space_hard_break_is_newline() {
        let Block::Paragraph(soft) = parse_one("a\nb") else { panic!("expected paragraph") };
        assert_eq!(soft.text, "a b");
        let Block::Paragraph(hard) = parse_one("a  \nb") else { panic!("expected paragraph") };
        assert_eq!(hard.text, "a\nb");
    }

    #[test]
    fn fenced_code_keeps_lang_and_trims_trailing_newline() {
        let Block::Code { lang, code, spans } = parse_one("```rust\nfn main() {}\n```") else { panic!("expected code") };
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(code, "fn main() {}");
        assert!(spans.is_empty());
    }

    #[test]
    fn fence_info_string_keeps_first_word_only() {
        let Block::Code { lang, .. } = parse_one("```mermaid theme=dark\nA-->B\n```") else { panic!("expected code") };
        assert_eq!(lang.as_deref(), Some("mermaid"));
    }

    #[test]
    fn bare_fence_and_indented_code_have_no_lang() {
        let Block::Code { lang, code, .. } = parse_one("```\nplain\n```") else { panic!("expected code") };
        assert_eq!(lang, None);
        assert_eq!(code, "plain");
        let Block::Code { lang, code, .. } = parse_one("    indented\n") else { panic!("expected code") };
        assert_eq!(lang, None);
        assert_eq!(code, "indented");
    }

    #[test]
    fn quote_wraps_inner_blocks_and_nests() {
        let Block::Quote(blocks) = parse_one("> outer\n>\n> > inner") else { panic!("expected quote") };
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], Block::Paragraph(p) if p.text == "outer"));
        let Block::Quote(inner) = &blocks[1] else { panic!("expected nested quote") };
        assert!(matches!(&inner[0], Block::Paragraph(p) if p.text == "inner"));
    }

    #[test]
    fn unordered_list_tight_items() {
        let Block::List { start, items } = parse_one("- a\n- b") else { panic!("expected list") };
        assert_eq!(start, None);
        assert_eq!(items.len(), 2);
        for (item, text) in items.iter().zip(["a", "b"]) {
            assert_eq!(item.checked, None);
            assert!(matches!(&item.blocks[0], Block::Paragraph(p) if p.text == text));
        }
    }

    #[test]
    fn ordered_list_keeps_start_number() {
        let Block::List { start, items } = parse_one("3. c\n4. d") else { panic!("expected list") };
        assert_eq!(start, Some(3));
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn task_list_markers_set_checked_state() {
        let Block::List { items, .. } = parse_one("- [x] done\n- [ ] todo\n- plain") else { panic!("expected list") };
        assert_eq!(items[0].checked, Some(true));
        assert_eq!(items[1].checked, Some(false));
        assert_eq!(items[2].checked, None);
    }

    #[test]
    fn nested_list_lives_inside_parent_item() {
        let Block::List { items, .. } = parse_one("- outer\n  - inner") else { panic!("expected list") };
        assert_eq!(items.len(), 1);
        let inner = items[0]
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::List { items, .. } => Some(items),
                _ => None,
            })
            .expect("inner list");
        assert!(matches!(&inner[0].blocks[0], Block::Paragraph(p) if p.text == "inner"));
    }

    #[test]
    fn loose_item_paragraphs_survive() {
        let Block::List { items, .. } = parse_one("- a\n\n- b") else { panic!("expected list") };
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0].blocks[0], Block::Paragraph(p) if p.text == "a"));
    }

    #[test]
    fn table_head_rows_and_styled_cells() {
        let Block::Table { head, rows } = parse_one("| A | B |\n| - | - |\n| **x** | y |\n| p | q |") else { panic!("expected table") };
        assert_eq!(head.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].text, "x");
        assert_eq!(rows[0][0].spans, vec![(0..1, SpanStyle { bold: true, ..Default::default() })]);
        assert_eq!(rows[1][1].text, "q");
    }

    #[test]
    fn rule_between_paragraphs() {
        let doc = parse("a\n\n---\n\nb");
        assert_eq!(doc.blocks.len(), 3);
        assert!(matches!(doc.blocks[1], Block::Rule));
    }

    #[test]
    fn html_is_ignored() {
        let doc = parse("<div>raw</div>");
        assert!(doc.blocks.is_empty());
    }

    #[test]
    fn mixed_document_block_order() {
        let doc = parse("# h\n\ntext\n\n- item\n\n> q\n\n```\nc\n```");
        let kinds: Vec<&str> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Heading { .. } => "heading",
                Block::Paragraph(_) => "paragraph",
                Block::List { .. } => "list",
                Block::Quote(_) => "quote",
                Block::Code { .. } => "code",
                Block::Table { .. } => "table",
                Block::Rule => "rule",
            })
            .collect();
        assert_eq!(kinds, ["heading", "paragraph", "list", "quote", "code"]);
    }
}
