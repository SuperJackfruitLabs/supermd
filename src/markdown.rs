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

    fn finish(self) -> InlineText {
        self.out
    }
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
            Event::Start(Tag::Link { .. }) => styles.link += 1,
            Event::End(TagEnd::Link) => styles.link -= 1,
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
