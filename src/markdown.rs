//! CommonMark source → block model.
//!
//! The plain-text file is the source of truth; this module turns it into a
//! tree of blocks the view layer can render. In later phases this parser will
//! be replaced by an incremental tree-sitter pass over the editing buffer, but
//! the block model is designed to survive that swap.

use std::ops::Range;
use std::path::{Path, PathBuf};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Where a Markdown image destination points, resolved against the
/// document that wrote it.
///
/// The editor and the reading view both draw images, and both used to
/// need this: the editor grew it inline in `render_image` and the
/// reading view never grew it at all, so one drew a picture where the
/// other wrote the picture's name (#57). One implementation, two
/// callers -- a second copy is how the two views drift apart again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    /// An `http://` or `https://` URL. Nothing is checked: the
    /// renderer's own loader fetches it.
    Remote(String),
    /// A file that is there.
    Local(PathBuf),
    /// A local destination with nothing behind it. Named rather than
    /// left as an `exists()` call at each site, so both views agree
    /// that a broken link is a thing you can see.
    Missing(PathBuf),
}

/// Whether the markup at `range` is the whole line it sits on, leading
/// and trailing whitespace aside.
///
/// This is the single rule that separates a picture from a word. The
/// editor's projection (`editor::blocks`) and the reading view's block
/// model both ask it, because a document that renders two ways is the
/// bug (#57) -- and two copies of one rule is how it comes back.
pub fn is_whole_line(source: &str, range: Range<usize>) -> bool {
    let start = source[..range.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = source[range.start..]
        .find('\n')
        .map(|i| range.start + i)
        .unwrap_or(source.len());
    source[start..end].trim() == &source[range]
}

/// Resolve an image destination the way the editor always has: remote
/// URLs pass through, everything else is relative to the directory of
/// the document that wrote the link.
pub fn resolve_image(dest: &str, doc: Option<&Path>) -> ImageSource {
    if dest.starts_with("http://") || dest.starts_with("https://") {
        return ImageSource::Remote(dest.to_string());
    }
    let path = doc
        .and_then(|p| p.parent())
        .map(|dir| dir.join(dest))
        .unwrap_or_else(|| PathBuf::from(dest));
    if path.exists() { ImageSource::Local(path) } else { ImageSource::Missing(path) }
}

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
    /// For a task item, which task it is: its position among every
    /// task in the document, in source order. `task_toggle` finds a
    /// task by the same number, so the reading view can hand it back
    /// without knowing any byte offsets.
    pub task: Option<usize>,
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
    /// A metadata block at the very top of the file, delimiters
    /// stripped. Kept as literal text: this is not frontmatter support
    /// (no tags, aliases or properties), only the end of a misparse.
    FrontMatter(String),
    /// A block of raw HTML, as written. Shown literally, never rendered.
    Html(String),
    /// An image whose markup is the whole line -- a picture in its own
    /// right, not a word. An image among words is not this: it stays a
    /// placeholder inside its paragraph, because a picture cannot sit
    /// inside a line of prose. `editor::blocks` draws the same line.
    Image { alt: String, dest: String },
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

    /// Drop everything pushed since `mark`, spans and links included.
    ///
    /// An image's placeholder and alt text go in optimistically at
    /// `Start(Image)`, because whether it is a picture or a word is
    /// only settled at `End(Image)` -- a nested image can take the
    /// decision away. This is the undo.
    fn truncate(&mut self, mark: usize) {
        self.out.text.truncate(mark);
        self.out.spans.retain_mut(|(r, _)| {
            r.end = r.end.min(mark);
            r.start < r.end
        });
        self.out.links.retain_mut(|(r, _)| {
            r.end = r.end.min(mark);
            r.start < r.end
        });
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
            // An empty label falls back to the target. `[[Target|]]`
            // otherwise rendered as nothing at all: eleven bytes of
            // source became zero characters, with nothing to click and
            // no way to tell the link was there.
            Some((t, l)) if !l.trim().is_empty() => (t.trim(), l.trim()),
            Some((t, _)) => (t.trim(), t.trim()),
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
    Item { checked: Option<bool>, task: Option<usize> },
}

/// Byte range of a YAML frontmatter block, if the source opens with
/// one. Only at the very start, and only when it closes -- a `---`
/// anywhere else is a thematic break and stays one.
///
/// The one definition shared by the reading path (`parse`), the
/// editor's span and block passes, and link previews. The rules follow
/// pulldown-cmark's own metadata scanner, which we cannot simply switch
/// on: it accepts a block at the start of *any* paragraph, not only the
/// file's.
///
/// - The first line is exactly `---` (trailing spaces allowed).
/// - The next line is neither blank nor a closing delimiter, so a note
///   that opens with a rule and a blank line is still that.
/// - It closes on a line that is `---` or `...`; the range runs through
///   that line's newline (or to the end of the file).
///
/// `\r\n` endings are accepted throughout and covered by the range,
/// as is a leading UTF-8 byte-order mark.
pub fn frontmatter_range(src: &str) -> Option<Range<usize>> {
    fn delimiter(line: &str) -> &str {
        line.trim_end_matches(['\r', '\n']).trim_end_matches([' ', '\t'])
    }
    // A byte-order mark is not content; the range covers it.
    let bom = if src.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 };
    let mut lines = src[bom..].split_inclusive('\n');
    let first = lines.next()?;
    if delimiter(first) != "---" || !first.ends_with('\n') {
        return None;
    }
    let mut offset = bom + first.len();
    let mut inner_lines = 0usize;
    for line in lines {
        let d = delimiter(line);
        let closes = d == "---" || d == "...";
        if inner_lines == 0 && (closes || d.trim().is_empty()) {
            return None;
        }
        offset += line.len();
        if closes {
            return Some(0..offset);
        }
        inner_lines += 1;
    }
    None
}

/// Where the Markdown body starts: after the frontmatter, else 0.
pub fn body_start(src: &str) -> usize {
    frontmatter_range(src).map_or(0, |r| r.end)
}

/// Whether an HTML block is nothing but closed `<!-- -->` comments and
/// whitespace.
fn only_comments(html: &str) -> bool {
    let mut rest = html.trim();
    while let Some(after) = rest.strip_prefix("<!--") {
        let Some(end) = after.find("-->") else {
            return false;
        };
        rest = after[end + 3..].trim_start();
    }
    rest.is_empty()
}

/// The parser options for the reading path. `task_toggle` must see
/// exactly the tasks `parse` does, so both take them from here.
fn options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options
}

/// Byte range of the Nth task marker's state character, and what it
/// should become. `None` when the index is past the last task.
///
/// The range is the single character between the brackets -- ` `, `x`
/// or `X` -- so applying the edit changes one byte of the file and
/// nothing else: not the brackets, not the line ending. Tasks are
/// counted exactly as `parse` numbers `ListItem::task`: the body after
/// any frontmatter, in source order, nested and quoted ones included,
/// look-alikes in code excluded.
pub fn task_toggle(src: &str, nth: usize) -> Option<(Range<usize>, &'static str)> {
    let body = body_start(src);
    let (_, marker) = Parser::new_ext(&src[body..], options())
        .into_offset_iter()
        .filter(|(event, _)| matches!(event, Event::TaskListMarker(_)))
        .nth(nth)?;
    let state = body + marker.start + 1;
    let with = if src.as_bytes().get(state) == Some(&b' ') { "x" } else { " " };
    Some((state..state + 1, with))
}

pub fn parse(source: &str) -> Document {
    let options = options();

    // Open containers. The bottom Vec is the document itself; quotes and list
    // items push a new Vec and fold it into a Block when they close.
    let mut containers: Vec<Vec<Block>> = vec![Vec::new()];
    let mut frames: Vec<Frame> = Vec::new();

    // Stack of open lists (they contain items, not blocks directly).
    let mut lists: Vec<(Option<u64>, Vec<ListItem>)> = Vec::new();

    let mut inline: Option<InlineBuilder> = None;
    let mut styles = StyleStack::default();

    let mut code: Option<(Option<String>, String)> = None;

    // How many task markers have been seen, for `ListItem::task`.
    let mut tasks_seen = 0usize;

    // An HTML block in progress: its raw source, line by line.
    let mut html: Option<String> = None;

    // Table state: header cells, body rows, row in progress.
    let mut table: Option<(Vec<InlineText>, Vec<Vec<InlineText>>)> = None;
    let mut table_row: Vec<InlineText> = Vec::new();

    // Metadata first, then the body parsed on its own: parsing the
    // whole file is what made the block a setext heading, and a fence
    // marker inside it would otherwise swallow the document.
    let body = body_start(source);
    if body > 0 {
        // Everything between the opening and closing delimiter lines.
        let lines: Vec<&str> = source[..body].split_inclusive('\n').collect();
        let inner: String = lines[1..lines.len() - 1].concat();
        let inner = inner.replace("\r\n", "\n");
        containers[0].push(Block::FrontMatter(inner.trim_end_matches('\n').to_string()));
    }
    let source = &source[body..];

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

    // The image currently open, if any: its source range, the alt text
    // gathered so far, its destination, and where the placeholder
    // started in the inline builder. The slot is single and a nested
    // image overwrites it, exactly as in `editor::blocks` -- that is
    // what makes `![a ![b](c)](d)` a block in neither view.
    let mut open_image: Option<(Range<usize>, String, String, Option<usize>)> = None;

    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            // ── Leaf blocks with inline content ─────────────────────────
            Event::Start(Tag::Paragraph | Tag::Heading { .. }) => {
                flush_inline(&mut inline, &mut containers);
                inline = Some(InlineBuilder::default());
            }
            Event::End(TagEnd::Paragraph) => {
                // A paragraph whose only content was a block image has
                // nothing left in it; an empty one would render as a
                // blank gap under the picture.
                if let Some(builder) = inline.take() {
                    if !builder.is_empty() {
                        containers.last_mut().unwrap().push(Block::Paragraph(builder.finish()));
                    }
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
                frames.push(Frame::Item { checked: None, task: None });
                containers.push(Vec::new());
                // Tight list items carry inline content with no Paragraph tag.
                inline = Some(InlineBuilder::default());
            }
            Event::End(TagEnd::Item) => {
                flush_inline(&mut inline, &mut containers);
                let blocks = containers.pop().unwrap();
                let (checked, task) = match frames.pop() {
                    Some(Frame::Item { checked, task }) => (checked, task),
                    _ => (None, None),
                };
                lists
                    .last_mut()
                    .expect("item outside list")
                    .1
                    .push(ListItem { checked, task, blocks });
            }
            Event::TaskListMarker(done) => {
                // Counted for every marker the parser reports, so the
                // numbering cannot drift from `task_toggle`'s.
                if let Some(Frame::Item { checked, task }) = frames.last_mut() {
                    *checked = Some(done);
                    *task = Some(tasks_seen);
                }
                tasks_seen += 1;
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
            Event::Start(Tag::Image { dest_url, .. }) => {
                // An image among words is a placeholder of its alt
                // text: a picture cannot sit inside a line of prose.
                // Whether this one is that or a block of its own is
                // settled at `End(Image)`, so the placeholder goes in
                // now and comes back out there if it was a block.
                styles.image += 1;
                let mark = inline.as_ref().map(|b| b.out.text.len());
                if let Some(builder) = inline.as_mut() {
                    builder.push("🖼 ", styles.current());
                }
                open_image = Some((range, String::new(), dest_url.to_string(), mark));
            }
            Event::End(TagEnd::Image) => {
                styles.image -= 1;
                if let Some((range, alt, dest, mark)) = open_image.take() {
                    if is_whole_line(source, range) {
                        if let (Some(builder), Some(mark)) = (inline.as_mut(), mark) {
                            builder.truncate(mark);
                        }
                        let resume = inline.is_some();
                        flush_inline(&mut inline, &mut containers);
                        containers
                            .last_mut()
                            .expect("container stack is never empty")
                            .push(Block::Image { alt, dest });
                        if resume {
                            inline = Some(InlineBuilder::default());
                        }
                    }
                }
            }

            // ── Inline content ──────────────────────────────────────────
            Event::Text(text) => {
                // Alt text is gathered from `Text` alone, the way
                // `editor::blocks` gathers it, so a block image reads
                // the same in both views.
                if let Some((_, alt, _, _)) = open_image.as_mut() {
                    alt.push_str(&text);
                }
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
                // A line break joins two runs of words. With nothing
                // before it there is nothing to join -- which is what a
                // paragraph looks like once a block image has been
                // lifted out of its first line.
                if let Some(builder) = inline.as_mut() {
                    if !builder.is_empty() {
                        builder.push(" ", SpanStyle::default());
                    }
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

            // ── HTML blocks ─────────────────────────────────────────────
            // Not rendered, but kept as the literal source. This arm used
            // to be the catch-all below, and a pasted snippet vanished
            // from the preview without a trace.
            Event::Start(Tag::HtmlBlock) => {
                flush_inline(&mut inline, &mut containers);
                html = Some(String::new());
            }
            Event::Html(text) => {
                if let Some(buffer) = html.as_mut() {
                    buffer.push_str(&text);
                }
            }
            Event::End(TagEnd::HtmlBlock) => {
                if let Some(text) = html.take() {
                    let text = text.replace("\r\n", "\n");
                    if only_comments(&text) {
                        // Invisible in every renderer, so hiding it
                        // erases nothing (the `toc` plugin's markers).
                        continue;
                    }
                    containers
                        .last_mut()
                        .unwrap()
                        .push(Block::Html(text.trim_end_matches('\n').to_string()));
                }
            }

            // Inline HTML (a separate question from the block-level issue
            // above), footnotes, math: out of scope.
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

    #[test]
    fn an_empty_label_falls_back_to_the_target() {
        let doc = parse("[[Roadmap|]] and [[A| ]] end\n");
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("paragraph") };
        assert_eq!(inline.text, "Roadmap and A end", "neither renders as nothing");
        assert_eq!(inline.links.len(), 2, "both are still links");
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

    /// The placeholder is what an image among words looks like. It
    /// used to be what *every* image looked like, standalone ones
    /// included -- this test read `![alt text](img.png)` on its own
    /// line, which is now the picture itself (see
    /// `a_standalone_image_is_its_own_block`). The alt text stays
    /// italic either way.
    #[test]
    fn image_renders_placeholder_with_italic_alt() {
        let Block::Paragraph(inline) = parse_one("see ![alt text](img.png)") else {
            panic!("expected paragraph")
        };
        assert_eq!(inline.text, "see \u{1f5bc} alt text");
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

    /// Apply what `task_toggle` asks for, the way a caller would.
    fn toggled(src: &str, nth: usize) -> Option<String> {
        let (range, with) = task_toggle(src, nth)?;
        let mut out = src.to_string();
        out.replace_range(range, with);
        Some(out)
    }

    /// A toggle is one byte: the state character between the brackets.
    /// Nothing else in the file moves.
    #[test]
    fn task_toggle_flips_exactly_the_state_character() {
        let src = "- [ ] one\n- [x] two\n";
        assert_eq!(task_toggle(src, 0), Some((3..4, "x")));
        assert_eq!(task_toggle(src, 1), Some((13..14, " ")));
        assert_eq!(task_toggle(src, 2), None, "past the last task");
        assert_eq!(toggled(src, 0).as_deref(), Some("- [x] one\n- [x] two\n"));
        assert_eq!(toggled(src, 1).as_deref(), Some("- [ ] one\n- [ ] two\n"));
        // An upper-case X is checked too, and unchecks the same way.
        assert_eq!(toggled("* [X] up\n", 0).as_deref(), Some("* [ ] up\n"));
        // Windows line endings are left exactly as they were.
        assert_eq!(
            toggled("- [ ] a\r\n- [ ] b\r\n", 1).as_deref(),
            Some("- [ ] a\r\n- [x] b\r\n")
        );
        assert_eq!(task_toggle("no tasks here\n", 0), None);
    }

    /// Only real tasks count: not a look-alike in a fence, not one in
    /// the metadata block, and nested or quoted ones in document order.
    #[test]
    fn task_toggle_counts_what_the_parser_calls_a_task() {
        let src = "---\n- [ ] meta\n---\n```\n- [ ] code\n```\n- [ ] a\n  - [x] nested\n> - [ ] quoted\n1. [ ] ordered\n";
        let at = |needle: &str| src.find(needle).unwrap() + 1;
        assert_eq!(task_toggle(src, 0), Some((at("[ ] a"), "x")).map(|(s, w)| (s..s + 1, w)));
        assert_eq!(task_toggle(src, 1), Some((at("[x] nested")..at("[x] nested") + 1, " ")));
        assert_eq!(task_toggle(src, 2), Some((at("[ ] quoted")..at("[ ] quoted") + 1, "x")));
        assert_eq!(task_toggle(src, 3), Some((at("[ ] ordered")..at("[ ] ordered") + 1, "x")));
        assert_eq!(task_toggle(src, 4), None);
    }

    /// The reading view numbers each task it draws from `ListItem::task`,
    /// and the toggle finds the Nth task on its own. They must agree, or
    /// a click flips a different box than the one under the pointer.
    #[test]
    fn parsed_task_indices_match_the_toggle_targets() {
        let src = "---\na: 1\n---\n- [ ] a\n  - [x] b\n    - plain\n    - [ ] c\n\n> - [X] d\n\n- e\n- [ ] f\n";
        fn walk(blocks: &[Block], out: &mut Vec<(Option<usize>, Option<bool>)>) {
            for b in blocks {
                match b {
                    Block::List { items, .. } => {
                        for item in items {
                            out.push((item.task, item.checked));
                            walk(&item.blocks, out);
                        }
                    }
                    Block::Quote(inner) => walk(inner, out),
                    _ => {}
                }
            }
        }
        let mut seen = Vec::new();
        walk(&parse(src).blocks, &mut seen);
        let tasks: Vec<_> = seen.iter().filter(|(t, _)| t.is_some()).collect();
        assert_eq!(tasks.len(), 5, "{seen:?}");
        for (i, (task, checked)) in tasks.iter().enumerate() {
            assert_eq!(*task, Some(i), "numbered in the order they are drawn");
            let (range, with) = task_toggle(src, i).expect("the toggle finds it");
            let state = &src[range];
            assert_eq!(*checked == Some(true), state != " ", "task {i} is the same box");
            assert_eq!(with == " ", *checked == Some(true));
        }
        assert!(seen.iter().all(|(t, c)| t.is_some() == c.is_some()), "plain items have no index");
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

    /// The delimited block at the top of a file is metadata, not a
    /// heading. CommonMark's setext rule turns it into one, which is
    /// why a note's title line used to render as the loudest thing on
    /// the page.
    #[test]
    fn frontmatter_is_its_own_block_not_a_heading() {
        let src = "---\ntitle: Weekly Review\ntags: [planning]\n---\n\n# Real Heading\n\nBody.\n";
        let doc = parse(src);
        let Some(Block::FrontMatter(inner)) = doc.blocks.first() else {
            panic!("first block is frontmatter, got {:?}", doc.blocks.first())
        };
        assert_eq!(inner, "title: Weekly Review\ntags: [planning]", "the metadata, delimiters gone");
        let headings: Vec<_> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading { level, content } => Some((*level, content.text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(headings, vec![(1, "Real Heading".to_string())], "only the real heading");
        assert!(
            !doc.blocks.iter().any(|b| matches!(b, Block::Rule)),
            "neither delimiter survives as a rule"
        );
    }

    /// Only at the very start, and only when it closes. A --- further
    /// down is a thematic break and must stay one.
    #[test]
    fn frontmatter_is_recognised_only_at_the_top_and_only_when_closed() {
        assert_eq!(frontmatter_range("---\na: 1\n---\nbody\n"), Some(0..13));
        assert_eq!(frontmatter_range("---\na: 1\n..."), Some(0..12), "`...` closes, EOF ends");
        assert!(frontmatter_range("\n---\na: 1\n---\n").is_none(), "not at the top");
        assert!(frontmatter_range("---\na: 1\nnever closes\n").is_none(), "unclosed");
        assert!(frontmatter_range("body\n\n---\n\nmore\n").is_none(), "a real rule");
        assert!(frontmatter_range("").is_none());
        // A document that opens with a break, a blank line, and later a
        // setext underline is ordinary Markdown, not metadata: the
        // first line inside must be neither blank nor the close.
        assert!(frontmatter_range("---\n\nPara\n---\n").is_none(), "blank first line");
        assert!(frontmatter_range("---\n---\n").is_none(), "two rules, not an empty block");
        assert!(frontmatter_range("----\na: 1\n---\n").is_none(), "four hyphens is a rule");
        assert!(frontmatter_range("   ---\na: 1\n---\n").is_none(), "indented");
        // A byte-order mark before the opening line is not content; the
        // range covers it so the body still starts after the block.
        assert_eq!(frontmatter_range("\u{feff}---\na: 1\n---\nbody\n"), Some(0..16));
        let Some(Block::FrontMatter(inner)) =
            parse("\u{feff}---\na: 1\n---\n# H\n").blocks.first().cloned()
        else {
            panic!("BOM frontmatter")
        };
        assert_eq!(inner, "a: 1");
    }

    /// Windows line endings and trailing spaces on a delimiter are the
    /// same block; the range still covers every byte of it.
    #[test]
    fn frontmatter_tolerates_crlf_and_trailing_spaces() {
        let src = "--- \r\na: 1\r\n---\r\nbody\r\n";
        assert_eq!(frontmatter_range(src), Some(0..17));
        let Some(Block::FrontMatter(inner)) = parse(src).blocks.first().cloned() else {
            panic!("crlf frontmatter")
        };
        assert_eq!(inner, "a: 1");
    }

    /// A fence opened inside the metadata must not run on into the
    /// body: the body is parsed on its own.
    #[test]
    fn a_fence_marker_inside_frontmatter_does_not_swallow_the_body() {
        let doc = parse("---\nnote: ```\n```\n---\n# Body\n");
        assert!(matches!(doc.blocks.first(), Some(Block::FrontMatter(_))));
        assert!(
            doc.blocks.iter().any(|b| matches!(b, Block::Heading { level: 1, .. })),
            "the body heading survives: {:?}",
            doc.blocks
        );
    }

    /// Unclosed, the opening --- is what CommonMark says it is.
    #[test]
    fn an_unclosed_opening_stays_a_rule() {
        let doc = parse("---\n\nbody\n");
        assert!(matches!(doc.blocks.first(), Some(Block::Rule)), "{:?}", doc.blocks);
    }

    /// HTML is not rendered, but it is never erased. The old behaviour
    /// dropped the block entirely, so a pasted snippet vanished from
    /// the preview with nothing to show the user it had gone.
    #[test]
    fn html_blocks_are_kept_as_literal_text() {
        let doc = parse("<div>raw</div>\n");
        let Some(Block::Html(s)) = doc.blocks.first() else {
            panic!("got {:?}", doc.blocks.first())
        };
        assert_eq!(s, "<div>raw</div>", "the source survives, trailing newline dropped");
    }

    /// A multi-line block keeps every line, in order, and the Markdown
    /// around it still parses as Markdown.
    #[test]
    fn a_multi_line_html_block_keeps_every_line() {
        let doc = parse("intro\n\n<details>\n<summary>More</summary>\nhidden\n</details>\n\n# After\n");
        let kinds: Vec<_> = doc.blocks.iter().map(|b| match b {
            Block::Paragraph(_) => "p",
            Block::Html(_) => "html",
            Block::Heading { .. } => "h",
            _ => "other",
        }).collect();
        assert_eq!(kinds, ["p", "html", "h"]);
        let Block::Html(s) = &doc.blocks[1] else { unreachable!() };
        assert_eq!(s, "<details>\n<summary>More</summary>\nhidden\n</details>");
    }

    /// Containers hold HTML blocks too; they must not vanish there either.
    #[test]
    fn html_inside_a_quote_or_list_item_survives() {
        let doc = parse("> <div>q</div>\n");
        let Block::Quote(inner) = &doc.blocks[0] else { panic!("{:?}", doc.blocks) };
        assert!(matches!(&inner[0], Block::Html(s) if s == "<div>q</div>"), "{inner:?}");

        let doc = parse("- item\n\n  <div>l</div>\n");
        let Block::List { items, .. } = &doc.blocks[0] else { panic!("{:?}", doc.blocks) };
        assert!(
            items[0].blocks.iter().any(|b| matches!(b, Block::Html(s) if s == "<div>l</div>")),
            "{:?}",
            items[0].blocks
        );

        // A tight item's text has no paragraph tag of its own; the HTML
        // that interrupts it must land after it, not before.
        let doc = parse("- a\n  <div>t</div>\n");
        let Block::List { items, .. } = &doc.blocks[0] else { panic!("{:?}", doc.blocks) };
        assert!(
            matches!(
                items[0].blocks.as_slice(),
                [Block::Paragraph(p), Block::Html(h)] if p.text == "a" && h == "<div>t</div>"
            ),
            "{:?}",
            items[0].blocks
        );
    }

    /// A comment is invisible in every Markdown renderer, so hiding one
    /// erases nothing -- and the seeded `toc` plugin writes a pair of
    /// them around every table of contents, which drew as two empty
    /// code boxes. Only a block that is nothing *but* comments hides:
    /// real HTML beside a comment is still content.
    #[test]
    fn html_comments_stay_invisible_but_their_neighbours_do_not() {
        let doc = parse("<!-- toc -->\n- [A](#a)\n<!-- /toc -->\n\n# A\n");
        assert!(!doc.blocks.iter().any(|b| matches!(b, Block::Html(_))), "{:?}", doc.blocks);
        assert!(matches!(doc.blocks.first(), Some(Block::List { .. })), "the TOC itself stays");

        let doc = parse("<!--\nmulti\nline\n-->\n\n<!-- a --> <!-- b -->\n");
        assert!(doc.blocks.is_empty(), "{:?}", doc.blocks);

        // The parser ends a comment block at its `-->` line, so HTML on
        // the next line is its own block, and stays.
        let doc = parse("<!-- note -->\n<div>kept</div>\n");
        assert!(
            matches!(doc.blocks.as_slice(), [Block::Html(s)] if s == "<div>kept</div>"),
            "{:?}",
            doc.blocks
        );
        // HTML after a comment in the same block keeps the whole block.
        let doc = parse("<!-- note --><div>kept</div>\n");
        assert!(
            matches!(doc.blocks.as_slice(), [Block::Html(s)] if s == "<!-- note --><div>kept</div>"),
            "{:?}",
            doc.blocks
        );
        // Comments on both sides do not make the middle invisible.
        let doc = parse("<!-- a --><div>x</div><!-- b -->\n");
        assert!(matches!(doc.blocks.as_slice(), [Block::Html(_)]), "{:?}", doc.blocks);
        // The shipped guide the toc plugin maintains renders no boxes.
        let guide = parse(include_str!("../examples/vault/Guide/Plugins.md"));
        assert!(!guide.blocks.iter().any(|b| matches!(b, Block::Html(_))));
        // An unclosed comment is not provably invisible text; keep it.
        let doc = parse("<!-- never closed\n");
        assert!(matches!(doc.blocks.as_slice(), [Block::Html(_)]), "{:?}", doc.blocks);
    }

    /// Inline HTML is a different issue and keeps its old behaviour: the
    /// paragraph around it is intact and no block appears.
    #[test]
    fn inline_html_is_not_a_block() {
        let doc = parse("a <b>bold</b> c\n");
        assert_eq!(doc.blocks.len(), 1);
        assert!(matches!(&doc.blocks[0], Block::Paragraph(_)));
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
                Block::FrontMatter(_) => "frontmatter",
                Block::Html(_) => "html",
                Block::Image { .. } => "image",
            })
            .collect();
        assert_eq!(kinds, ["heading", "paragraph", "list", "quote", "code"]);
    }

    /// A standalone image is a block, not a run of text. The editor has
    /// drawn the picture since images became a claimed block; the
    /// reading view answered `🖼 ` and the alt text for the same
    /// document (#57).
    #[test]
    fn a_standalone_image_is_its_own_block() {
        let doc = parse("Before\n\n![A city](city.png)\n\nAfter\n");
        assert!(
            doc.blocks
                .iter()
                .any(|b| matches!(b, Block::Image { dest, alt } if dest == "city.png" && alt == "A city")),
            "standalone image did not become a block: {:?}",
            doc.blocks
        );
    }

    /// An image among words keeps the inline placeholder -- a picture
    /// cannot sit inside a line of prose.
    #[test]
    fn an_inline_image_keeps_its_placeholder() {
        let doc = parse("Text with ![a pic](p.png) inside.\n");
        assert!(
            !doc.blocks.iter().any(|b| matches!(b, Block::Image { .. })),
            "inline image became a block: {:?}",
            doc.blocks
        );
        let Block::Paragraph(inline) = &doc.blocks[0] else { panic!("{:?}", doc.blocks) };
        assert!(inline.text.contains('🖼'), "inline placeholder lost: {}", inline.text);
    }

    /// The rule ignores the whitespace around the markup. An image
    /// indented under the paragraph above it, or one a stray trailing
    /// space follows, is still the only thing on its line -- and both
    /// views ask this one question, so a change here moves them
    /// together and the agreement test above cannot see it.
    #[test]
    fn the_whole_line_rule_ignores_surrounding_whitespace() {
        assert!(is_whole_line("  ![a](b.png)  ", 2..13), "indented and trailed");
        assert!(is_whole_line("![a](b.png)", 0..11), "bare");
        assert!(!is_whole_line("see ![a](b.png)", 4..15), "among words");
        let doc = parse("words\n\n  ![indented](i.png)\n");
        assert!(
            doc.blocks.iter().any(|b| matches!(b, Block::Image { .. })),
            "an indented image is still alone on its line: {:?}",
            doc.blocks
        );
    }

    /// A picture lifted out of the middle of a paragraph leaves the
    /// words on either side of it intact: no blank paragraph where it
    /// used to be, and no stray indent on what followed it, which is
    /// what the line break between them would otherwise become.
    #[test]
    fn words_around_a_lifted_picture_survive_it() {
        let doc = parse("words\n![pic](p.png)\nmore\n");
        let kinds: Vec<&str> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(_) => "paragraph",
                Block::Image { .. } => "image",
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(kinds, ["paragraph", "image", "paragraph"], "{:?}", doc.blocks);
        let Block::Paragraph(after) = &doc.blocks[2] else { unreachable!() };
        assert_eq!(after.text, "more");
        // A picture alone in its paragraph leaves nothing at all behind.
        assert_eq!(parse("![only](o.png)\n").blocks.len(), 1);
    }

    /// Whether an image is a block is decided in `editor::blocks` for
    /// the editor; the reading view has to reach the same verdict on
    /// the same source or one view draws a picture where the other
    /// writes its name. This is the same rule, not a second one.
    #[test]
    fn block_images_agree_with_the_editors_rule() {
        fn reading(blocks: &[Block], out: &mut Vec<(String, String)>) {
            for b in blocks {
                match b {
                    Block::Image { alt, dest } => out.push((alt.clone(), dest.clone())),
                    Block::Quote(inner) => reading(inner, out),
                    Block::List { items, .. } => {
                        for item in items {
                            reading(&item.blocks, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        for src in [
            "![alone](a.png)\n",
            "see ![a](b.png) here\n",
            "words\n![after](c.png)\n",
            "![before](c.png)\nwords\n",
            "![a ![b](c)](d)\n",
            "- ![in a list](l.png)\n",
            "> ![quoted](q.png)\n",
            "  ![indented](i.png)\n",
            "---\ntitle: t\n---\n\n![past frontmatter](f.png)\n",
            "```\n![fenced](x.png)\n```\n",
            "![one](1.png)\n![two](2.png)\n",
            "![titled](t.png \"a title\")\n",
            "# ![in a heading](h.png)\n",
        ] {
            let mut mine = Vec::new();
            reading(&parse(src).blocks, &mut mine);
            let editors: Vec<(String, String)> = crate::editor::blocks::blocks(src)
                .into_iter()
                .filter_map(|b| match b.kind {
                    crate::editor::blocks::BlockKind::Image { alt, dest } => Some((alt, dest)),
                    _ => None,
                })
                .collect();
            assert_eq!(mine, editors, "the two views disagreed on {src:?}");
        }
    }

    #[test]
    fn a_remote_image_resolves_without_touching_the_disk() {
        assert_eq!(
            resolve_image("https://example.com/a.png", Some(Path::new("/nowhere/doc.md"))),
            ImageSource::Remote("https://example.com/a.png".to_string())
        );
        assert_eq!(
            resolve_image("http://example.com/a.png", None),
            ImageSource::Remote("http://example.com/a.png".to_string())
        );
    }

    #[test]
    fn a_local_image_resolves_against_the_documents_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir(&assets).unwrap();
        std::fs::write(assets.join("pic.png"), b"not really a png").unwrap();
        let doc = dir.path().join("notes").join("note.md");
        std::fs::create_dir(dir.path().join("notes")).unwrap();
        assert_eq!(
            resolve_image("../assets/pic.png", Some(&doc)),
            ImageSource::Local(dir.path().join("notes").join("../assets/pic.png"))
        );
    }

    /// A broken link has to look deliberate. Rendering nothing reads as
    /// a broken app; the editor says `— file not found` and the reading
    /// view needs the same answer, so the missing case is named here
    /// rather than left to each caller's `exists()` check.
    #[test]
    fn a_missing_local_image_resolves_to_missing() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("note.md");
        assert_eq!(
            resolve_image("gone.png", Some(&doc)),
            ImageSource::Missing(dir.path().join("gone.png"))
        );
    }
}
