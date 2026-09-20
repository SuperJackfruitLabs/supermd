//! One open document: parsed blocks, outline, and its scroll position.

use std::path::{Path, PathBuf};

use gpui::{
    actions, div, list, px, App, FocusHandle, IntoElement, ListAlignment, ListOffset, ListState,
    ParentElement, Render, SharedString, Styled, Window,
};
use gpui::prelude::*;

use crate::highlight::Languages;
use crate::markdown::{self, Block, Document};
use crate::theme::theme;
use crate::view;

actions!(
    reader,
    [ScrollUp, ScrollDown, PageUp, PageDown, ScrollTop, ScrollBottom]
);

/// One arrow-key step, in pixels. The editor pages by a fixed line
/// count because its lines are uniform height; a reader's blocks are
/// not, so the reader steps in pixels against its measured viewport.
const LINE_STEP: f32 = 72.;
/// How much of the current screen a page jump carries over.
const PAGE_OVERLAP: f32 = 48.;

/// New offset (px from the top) after moving `delta`, clamped to the
/// scrollable range. `ListState::scroll_by` clamps at the top but seeks
/// past the content bottom, so the bottom clamp has to be ours.
fn clamped_scroll(current: f32, delta: f32, max: f32) -> f32 {
    (current + delta).clamp(0., max.max(0.))
}

/// A page jump keeps a sliver of the previous screen for continuity,
/// and always advances even when the viewport is tiny.
fn page_step(viewport_height: f32) -> f32 {
    (viewport_height - PAGE_OVERLAP).max(LINE_STEP)
}

pub struct TocEntry {
    pub level: u8,
    pub text: SharedString,
    pub block_ix: usize,
}

pub struct Reader {
    pub path: Option<PathBuf>,
    pub title: SharedString,
    pub document: std::sync::Arc<Document>,
    pub toc: Vec<TocEntry>,
    pub list_state: ListState,
    /// The owning workspace's knowledge index, for resolving a link's
    /// hover preview. None when no workspace handed one over — the
    /// index is per-workspace state, never a process global.
    knowledge: Option<crate::knowledge::KnowledgeHandle>,
    focus_handle: FocusHandle,
    scroll_anim: Option<gpui::Task<()>>,
    /// The text `document` was parsed from.
    source: String,
    /// Whether `source` is a file's own Markdown, so a checkbox click
    /// can become an edit to it. Off by default: a viewer plugin's
    /// output or a wrapped code file is not the file.
    task_edits: bool,
}

/// Language token for a file. Delegates to the central mapping.
/// Wrap a non-Markdown file so the pretty preview renders it as one
/// fenced code block instead of parsing it as prose.
///
/// Without this, previewing `sample.rs` fed Rust source to the
/// CommonMark parser: doc comments became paragraphs, the source was
/// reflowed, and any 4-space-indented block turned into an indented
/// code block. The file was legible only by accident.
///
/// The fence is made longer than the longest backtick run in the file,
/// as CommonMark requires, so a file that itself contains ``` cannot
/// close the fence early.
pub fn source_as_document(text: &str, language: Option<&str>) -> String {
    let longest = text
        .as_bytes()
        .split(|&b| b != b'`')
        .map(<[u8]>::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.saturating_add(1).max(3));
    let info = language.unwrap_or("");
    // A trailing newline before the closing fence keeps a file that
    // does not end in one from gluing onto the delimiter.
    format!("{fence}{info}\n{}\n{fence}\n", text.trim_end_matches('\n'))
}

pub fn language_for_path(path: &Path) -> Option<String> {
    crate::highlight::language_for_file(path)
}

impl Reader {
    /// Build a pretty-rendered document from Markdown source (used for
    /// the ⌘E preview of an editor buffer).
    pub fn from_source(
        title: SharedString,
        source: &str,
        langs: &Languages,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::from_source_at(None, title, source, langs, cx)
    }

    /// As `from_source`, remembering which file the text came from so a
    /// relative link in it can be resolved for its hover preview.
    pub fn from_source_at(
        path: Option<PathBuf>,
        title: SharedString,
        source: &str,
        langs: &Languages,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::from_markdown(path, title, source.to_string(), langs, cx)
    }

    pub fn welcome(langs: &Languages, cx: &mut Context<Self>) -> Self {
        Self::from_markdown(None, "Welcome".into(), include_str!("../WELCOME.md").into(), langs, cx)
    }

    fn from_markdown(
        path: Option<PathBuf>,
        title: SharedString,
        source: String,
        langs: &Languages,
        cx: &mut Context<Self>,
    ) -> Self {
        let (document, toc) = Self::build(&source, langs);
        let list_state = ListState::new(document.blocks.len(), ListAlignment::Top, px(512.));
        Self {
            path,
            title,
            document,
            toc,
            list_state,
            knowledge: None,
            focus_handle: cx.focus_handle(),
            scroll_anim: None,
            source,
            task_edits: false,
        }
    }

    /// Parse, highlight, and outline one source text.
    fn build(source: &str, langs: &Languages) -> (std::sync::Arc<Document>, Vec<TocEntry>) {
        let mut document = markdown::parse(source);
        langs.highlight_document(&mut document);
        let toc = document
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(ix, block)| match block {
                Block::Heading { level, content } => Some(TocEntry {
                    level: *level,
                    text: SharedString::from(content.text.clone()),
                    block_ix: ix,
                }),
                _ => None,
            })
            .collect();
        (std::sync::Arc::new(document), toc)
    }

    /// The text the document was parsed from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Let checkbox clicks edit the source. Only for a reader showing a
    /// Markdown file's own text; the owner applies the emitted edit.
    pub fn allow_task_toggles(&mut self) {
        self.task_edits = true;
    }

    /// Re-render from new source text, keeping the scroll position.
    pub fn set_source(&mut self, source: String, cx: &mut Context<Self>) {
        let langs = cx
            .try_global::<crate::highlight::SyntaxLanguages>()
            .map(|l| l.0.clone())
            .unwrap_or_else(|| std::sync::Arc::new(Languages::new()));
        let (document, toc) = Self::build(&source, &langs);
        if document.blocks.len() != self.document.blocks.len() {
            let top = self.list_state.logical_scroll_top();
            self.list_state.reset(document.blocks.len());
            self.list_state.scroll_to(ListOffset {
                item_ix: top.item_ix.min(document.blocks.len().saturating_sub(1)),
                offset_in_item: top.offset_in_item,
            });
        }
        self.document = document;
        self.toc = toc;
        self.source = source;
        cx.notify();
    }

    /// Flip the Nth task's checkbox (`ListItem::task`). The view shows
    /// the new state at once, and the one-byte edit is emitted for the
    /// owner of the file to apply and save -- this never writes a file.
    pub fn toggle_task(&mut self, nth: usize, cx: &mut Context<Self>) {
        if !self.task_edits {
            return;
        }
        let Some((range, replacement)) = markdown::task_toggle(&self.source, nth) else {
            return;
        };
        let before = self.source.clone();
        let mut after = before.clone();
        after.replace_range(range.clone(), replacement);
        self.set_source(after, cx);
        cx.emit(ReaderEvent::EditSource { before, range, replacement });
    }

    /// Hand the reader its workspace's index (the workspace does this
    /// right after building it).
    pub fn set_knowledge(&mut self, knowledge: crate::knowledge::KnowledgeHandle) {
        self.knowledge = Some(knowledge);
    }

    /// The hover preview for a link destination in this document --
    /// exactly what the rendered view shows for it. Resolving needs
    /// both halves the workspace hands over: the file this text came
    /// from, and the index. Without the index every wiki link and
    /// every relative link previews as a note that does not exist.
    pub fn describe(&self, dest: &str, cx: &App) -> Option<crate::preview::Preview> {
        describe_link(self.path.as_deref(), dest, self.knowledge.as_ref(), cx)
    }

    /// The hover-preview callback the rendered view hands to
    /// `view::list_item`: every tooltip asks the reader itself, so it
    /// resolves with both halves the workspace handed over -- the file
    /// and the index.
    ///
    /// Named rather than written inline in `render` so a test can hold
    /// the same callback the view gets. Built any other way -- calling
    /// `describe_link` here with `None` for the index, say -- every wiki
    /// link and every relative link in the reading view previews as a
    /// note that does not exist, and a test that calls `describe`
    /// directly cannot see the difference.
    fn describe_callback(reader: &gpui::Entity<Self>) -> view::Describe {
        let reader = reader.clone();
        std::rc::Rc::new(move |dest: &str, cx: &mut App| reader.read(cx).describe(dest, cx))
    }

    pub fn scroll_to_block(&mut self, block_ix: usize, cx: &mut Context<Self>) {
        let state = self.list_state.clone();
        let current = -state.scroll_px_offset_for_scrollbar().y;
        state.scroll_to(ListOffset { item_ix: block_ix, offset_in_item: px(0.) });
        let target_px = -state.scroll_px_offset_for_scrollbar().y;
        if (target_px - current).abs() < px(24.) {
            cx.notify();
            return;
        }
        state.set_offset_from_scrollbar(gpui::point(px(0.), -current));
        self.scroll_anim = Some(cx.spawn(async move |this, cx| {
            const FRAMES: u32 = 22;
            for frame in 1..=FRAMES {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(11))
                    .await;
                let t = frame as f32 / FRAMES as f32;
                let eased = 1.0 - (1.0 - t).powi(3);
                let y = current + (target_px - current) * eased;
                if this
                    .update(cx, |reader, cx| {
                        reader
                            .list_state
                            .set_offset_from_scrollbar(gpui::point(px(0.), -y));
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            this.update(cx, |reader, cx| {
                reader
                    .list_state
                    .scroll_to(ListOffset { item_ix: block_ix, offset_in_item: px(0.) });
                cx.notify();
            })
            .ok();
        }));
    }

    /// Move the viewport by `delta` px, clamped to the document. A key
    /// press cancels any in-flight outline animation: the key wins.
    fn scroll_px(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.scroll_anim = None;
        let current = f32::from(-self.list_state.scroll_px_offset_for_scrollbar().y);
        let max = f32::from(self.list_state.max_offset_for_scrollbar().height);
        let target = clamped_scroll(current, delta, max);
        self.list_state
            .set_offset_from_scrollbar(gpui::point(px(0.), -px(target)));
        cx.notify();
    }

    /// A page is the measured viewport, less the overlap kept for context.
    fn page(&self) -> f32 {
        page_step(f32::from(self.list_state.viewport_bounds().size.height))
    }

    fn scroll_up(&mut self, _: &ScrollUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_px(-LINE_STEP, cx);
    }

    fn scroll_down(&mut self, _: &ScrollDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_px(LINE_STEP, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_px(-self.page(), cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_px(self.page(), cx);
    }

    fn scroll_top(&mut self, _: &ScrollTop, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_anim = None;
        self.list_state
            .scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
        cx.notify();
    }

    fn scroll_bottom(&mut self, _: &ScrollBottom, _: &mut Window, cx: &mut Context<Self>) {
        // The list measures items lazily, so content height is only known
        // for what has already been rendered — a px jump to `max_offset`
        // stops short. Scrolling past the last item is measurement-free:
        // layout finds nothing below, walks back up measuring until the
        // viewport is full, and settles at the true bottom.
        self.scroll_anim = None;
        self.list_state.scroll_to(ListOffset {
            item_ix: self.list_state.item_count(),
            offset_in_item: px(0.),
        });
        cx.notify();
    }
}

impl gpui::Focusable for Reader {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// A link was clicked in the rendered view. Carries the destination
/// exactly as written; the workspace classifies and resolves it,
/// because only it knows which file this reader is showing.
#[derive(Debug, Clone)]
pub enum ReaderEvent {
    Follow(String),
    /// A checkbox was clicked: replace `range` of `before` -- the text
    /// the reader was showing -- with `replacement`.
    EditSource { before: String, range: std::ops::Range<usize>, replacement: &'static str },
}

impl gpui::EventEmitter<ReaderEvent> for Reader {}

/// What a rendered-view tooltip should show for a link destination in
/// the note at `base`. Pulled out of the render closure that builds it
/// so it can be tested directly, the way `Editor::preview_for` is —
/// including that it reads `PreviewState`'s cached grants rather than
/// settings off disk (see the module doc on `PreviewState::grants`).
fn describe_link(
    base: Option<&Path>,
    dest: &str,
    knowledge: Option<&crate::knowledge::KnowledgeHandle>,
    cx: &App,
) -> Option<crate::preview::Preview> {
    let base = base?;
    // Same marker the click path reads: a wiki destination resolves
    // by stem, not as a path, or every `[[Wiki]]` previewed as "does
    // not exist".
    let (wiki, target) = match dest.strip_prefix("[[") {
        Some(stem) => (true, stem.to_string()),
        None => (false, dest.to_string()),
    };
    let link = crate::knowledge::RawLink { target, wiki, range: 0..0, context: String::new() };
    let grants = cx
        .try_global::<crate::preview::PreviewState>()
        .map(|s| s.grants())
        .unwrap_or_default();
    Some(crate::preview::preview_for_link(&link, dest, &grants, |l| {
        knowledge.and_then(|k| k.lock().unwrap().resolve(base, l))
    }))
}

impl Render for Reader {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.weak_entity();
        let t = theme(cx);
        div()
            .size_full()
            .bg(t.page_bg)
            // GPUI content masks are rectangular -- `ContentMask` has
            // bounds and no radii -- so this square fill would
            // otherwise overpaint the page's rounded bottom corners.
            // Rounding here keeps the common case honest; the page
            // masks its own corners for everything deeper than this
            // root (see `Workspace::page_corner_masks`).
            .rounded_b(crate::elevation::radius(crate::elevation::Surface::Page))
            .key_context("Reader")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::scroll_up))
            .on_action(cx.listener(Self::scroll_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::scroll_top))
            .on_action(cx.listener(Self::scroll_bottom))
            .child(
                list(self.list_state.clone(), move |ix, _window, cx| {
                    let Some(reader) = entity.upgrade() else {
                        return div().into_any_element();
                    };
                    let t = theme(cx);
                    let document = reader.read(cx).document.clone();
                    // Clicking a link in the rendered view emits; the
                    // workspace resolves it against the open file.
                    let follow: view::Follow = {
                        let reader = reader.clone();
                        std::rc::Rc::new(move |dest: &str, _window: &mut Window, cx: &mut App| {
                            let dest = dest.to_string();
                            reader.update(cx, |_, cx| cx.emit(ReaderEvent::Follow(dest)));
                        })
                    };
                    let describe = Self::describe_callback(&reader);
                    let toggle: Option<view::ToggleTask> =
                        reader.read(cx).task_edits.then(|| {
                            let reader = reader.clone();
                            std::rc::Rc::new(move |task: usize, _: &mut Window, cx: &mut App| {
                                reader.update(cx, |reader, cx| reader.toggle_task(task, cx));
                            }) as view::ToggleTask
                        });
                    let base = reader.read(cx).path.clone();
                    view::list_item(
                        &document,
                        ix,
                        &t,
                        cx,
                        view::Links {
                            follow: Some(&follow),
                            describe: Some(&describe),
                            toggle: toggle.as_ref(),
                            base: base.as_deref(),
                        },
                    )
                })
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Entity, TestAppContext, VisualTestContext};
    use std::sync::Arc;

    const DOC: &str = "# Alpha\n\nintro\n\n## Beta\n\n```rust\nfn main() {}\n```\n\n### Gamma\n\ntail\n";

    // ── pure construction ──────────────────────────────────────────────

    #[test]
    fn source_becomes_one_fenced_block_not_prose() {
        let doc = source_as_document("fn main() {}\n", Some("rust"));
        assert_eq!(doc, "```rust\nfn main() {}\n```\n");
        // No language is still a fence: monospace, never reflowed prose.
        assert_eq!(source_as_document("plain", None), "```\nplain\n```\n");
    }

    /// A file containing a fence must not close the wrapper early, or
    /// the tail of it renders as prose — the very bug being fixed.
    #[test]
    fn the_fence_outgrows_any_backtick_run_in_the_file() {
        let doc = source_as_document("a\n```\nb\n", Some("md"));
        assert!(doc.starts_with("````md\n"), "fence longer than the run: {doc}");
        assert!(doc.ends_with("\n````\n"));
        let doc = source_as_document("`````\n", Some("md"));
        assert!(doc.starts_with("``````md\n"), "{doc}");
    }

    /// The rendered block must contain the source verbatim: a preview
    /// that reflows or re-indents code is not a preview of that file.
    #[test]
    fn the_source_survives_verbatim() {
        let src = "//! doc\n\n    indented\n\ttabbed\n#[derive(Debug)]\nstruct S;";
        let doc = source_as_document(src, Some("rust"));
        let body = doc
            .trim_start_matches("```rust\n")
            .trim_end_matches("```\n")
            .trim_end_matches('\n');
        assert_eq!(body, src);
    }

    #[test]
    fn language_for_path_delegates_to_central_mapping() {
        assert_eq!(language_for_path(Path::new("main.rs")).as_deref(), Some("rust"));
        assert_eq!(language_for_path(Path::new("Dockerfile")).as_deref(), Some("dockerfile"));
        assert_eq!(language_for_path(Path::new("noext")), None);
    }

    #[gpui::test]
    fn from_source_builds_outline_and_highlights_code(cx: &mut TestAppContext) {
        let langs = Languages::new();
        let entity = cx.new(|cx| Reader::from_source("Preview".into(), DOC, &langs, cx));
        entity.read_with(cx, |reader, _| {
        assert_eq!(reader.title.as_ref(), "Preview");
        assert!(reader.path.is_none());
        assert!(reader.scroll_anim.is_none());

        let toc: Vec<(u8, &str)> =
            reader.toc.iter().map(|e| (e.level, e.text.as_ref())).collect();
        assert_eq!(toc, [(1, "Alpha"), (2, "Beta"), (3, "Gamma")]);

        // Each outline entry points at the heading block it was built from.
        for entry in &reader.toc {
            let Block::Heading { level, content } = &reader.document.blocks[entry.block_ix]
            else {
                panic!("toc entry {} does not point at a heading", entry.text)
            };
            assert_eq!(*level, entry.level);
            assert_eq!(content.text.as_str(), entry.text.as_ref());
        }

        // The rust fence got real highlight spans during construction.
        let highlighted = reader.document.blocks.iter().any(|b| {
            matches!(b, Block::Code { lang: Some(l), spans, .. } if l == "rust" && !spans.is_empty())
        });
        assert!(highlighted, "code block should be highlighted");
        });
    }

    /// The outline lists the document's headings. A metadata block is
    /// not one, and it used to appear there as a garbled row.
    #[gpui::test]
    fn the_outline_skips_frontmatter(cx: &mut TestAppContext) {
        let src = "---\ntitle: x\ntags: [a]\n---\n\n# Only Me\n";
        let langs = Languages::new();
        let entity = cx.new(|cx| Reader::from_source("fm".into(), src, &langs, cx));
        entity.read_with(cx, |reader, _| {
            let toc: Vec<(u8, &str)> =
                reader.toc.iter().map(|e| (e.level, e.text.as_ref())).collect();
            assert_eq!(toc, [(1, "Only Me")]);
            assert!(matches!(reader.document.blocks[0], Block::FrontMatter(_)));
            assert_eq!(reader.toc[0].block_ix, 1, "the entry points past the metadata");
        });
    }

    #[gpui::test]
    fn welcome_document_carries_the_bundled_tour(cx: &mut TestAppContext) {
        let langs = Languages::new();
        let entity = cx.new(|cx| Reader::welcome(&langs, cx));
        entity.read_with(cx, |reader, _| {
        assert_eq!(reader.title.as_ref(), "Welcome");
        assert!(reader.path.is_none());
        assert_eq!(reader.toc[0].level, 1);
        assert_eq!(reader.toc[0].text.as_ref(), "Welcome to SuperMD");
        assert!(
            reader.toc.iter().any(|e| e.text.as_ref() == "Start here"),
            "expected the Start here section in the welcome outline"
        );
        assert!(reader.document.blocks.len() > 5, "welcome tour lost its body");
        });
    }

    // ── task toggles ───────────────────────────────────────────────────

    /// Everything the reader emitted, in order.
    fn record_events(
        reader: &Entity<Reader>,
        cx: &mut VisualTestContext,
    ) -> std::rc::Rc<std::cell::RefCell<Vec<ReaderEvent>>> {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = seen.clone();
        cx.update(|_, app| {
            app.subscribe(reader, move |_, event: &ReaderEvent, _| {
                sink.borrow_mut().push(event.clone())
            })
            .detach()
        });
        seen
    }

    fn checked_states(reader: &Entity<Reader>, cx: &mut VisualTestContext) -> Vec<Option<bool>> {
        cx.update(|_, app| {
            let doc = &reader.read(app).document;
            let Block::List { items, .. } = &doc.blocks[0] else { panic!("{:?}", doc.blocks) };
            items.iter().map(|i| i.checked).collect()
        })
    }

    /// A toggle in the reading view is an edit to the source: the
    /// reader shows it at once and hands exactly that one-byte edit to
    /// whoever owns the file. It never writes the file itself -- the
    /// editor's buffer is the file's in-memory truth, and writing past
    /// it would leave that buffer stale and let its next save undo the
    /// click.
    #[gpui::test]
    fn toggling_a_task_updates_the_view_and_emits_the_edit(cx: &mut TestAppContext) {
        let src = "- [ ] one\n- [x] two\n";
        let (reader, cx) = open_reader(cx, src);
        reader.update(cx, |r, _| r.allow_task_toggles());
        let events = record_events(&reader, cx);

        reader.update(cx, |r, cx| r.toggle_task(0, cx));
        cx.run_until_parked();
        assert_eq!(checked_states(&reader, cx), [Some(true), Some(true)]);
        let seen = events.borrow().clone();
        assert!(
            matches!(
                seen.as_slice(),
                [ReaderEvent::EditSource { before, range, replacement }]
                    if before == src && *range == (3..4) && *replacement == "x"
            ),
            "{seen:?}"
        );

        // A second click flips it back, from the updated source.
        reader.update(cx, |r, cx| r.toggle_task(0, cx));
        cx.run_until_parked();
        assert_eq!(checked_states(&reader, cx), [Some(false), Some(true)]);
        assert!(
            matches!(&events.borrow()[1], ReaderEvent::EditSource { before, .. } if before == "- [x] one\n- [x] two\n")
        );
    }

    /// A reader whose text is not the file's own -- a viewer plugin's
    /// rendering, a wrapped code file, the bundled welcome tour -- has
    /// nothing a toggle could be written back to, so the box stays
    /// decoration rather than pretending.
    #[gpui::test]
    fn task_toggles_are_off_unless_the_text_is_the_file(cx: &mut TestAppContext) {
        let (reader, cx) = open_reader(cx, "- [ ] one\n");
        let events = record_events(&reader, cx);
        reader.update(cx, |r, cx| r.toggle_task(0, cx));
        cx.run_until_parked();
        assert_eq!(checked_states(&reader, cx), [Some(false)]);
        assert!(events.borrow().is_empty());
        // Past the last task is nothing, too.
        reader.update(cx, |r, cx| {
            r.allow_task_toggles();
            r.toggle_task(7, cx)
        });
        assert!(events.borrow().is_empty());
    }

    /// The glyph is the control: a real click on the drawn marker, not
    /// a call, reaches the toggle. The marker used to have no id and no
    /// handler at all.
    #[gpui::test]
    fn clicking_the_drawn_marker_toggles_that_task(cx: &mut TestAppContext) {
        // Two separate lists: the second box is the first row of its own
        // list but the document's second task, and it is the second
        // task that must flip.
        let src = "- [ ] one\n\nbetween\n\n- [ ] two\n- [ ] three\n";
        let (reader, cx) = open_reader(cx, src);
        reader.update(cx, |r, cx| {
            r.allow_task_toggles();
            cx.notify();
        });
        cx.run_until_parked();
        let second = cx.debug_bounds("task-1").expect("the second marker drew");
        cx.simulate_click(second.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let source = cx.update(|_, app| reader.read(app).source.clone());
        assert_eq!(source, "- [ ] one\n\nbetween\n\n- [x] two\n- [ ] three\n", "only the clicked box");
    }

    // ── window rendering and scrolling ─────────────────────────────────

    /// Enough paragraphs that the far end sits well past one viewport.
    fn long_source() -> String {
        let mut s = String::from("# Top\n\n");
        for i in 0..120 {
            s.push_str(&format!("Paragraph number {i} with a little bit of text.\n\n"));
        }
        s.push_str("## Bottom\n");
        s
    }

    fn open_reader<'a>(
        cx: &'a mut TestAppContext,
        source: &str,
    ) -> (Entity<Reader>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(crate::theme::Theme::dark())));
        });
        let langs = Languages::new();
        let (reader, cx) =
            cx.add_window_view(|_, cx| Reader::from_source("doc".into(), source, &langs, cx));
        cx.run_until_parked();
        (reader, cx)
    }

    #[gpui::test]
    fn scrolling_to_a_nearby_block_snaps_without_animation(cx: &mut TestAppContext) {
        let (reader, cx) = open_reader(cx, &long_source());
        reader.update_in(cx, |reader, _, cx| {
            reader.scroll_to_block(0, cx);
            assert!(reader.scroll_anim.is_none(), "top-to-top scroll must not animate");
            assert_eq!(reader.list_state.logical_scroll_top().item_ix, 0);
        });
    }

    #[gpui::test]
    fn scrolling_to_a_far_block_animates_to_its_offset(cx: &mut TestAppContext) {
        let (reader, cx) = open_reader(cx, &long_source());
        let last = cx.update(|_, app| reader.read(app).document.blocks.len() - 1);

        // Where does a direct jump to the last block settle once the list
        // clamps at the content bottom? That is the animation's target.
        reader.update_in(cx, |reader, _, cx| {
            reader
                .list_state
                .scroll_to(ListOffset { item_ix: last, offset_in_item: px(0.) });
            cx.notify();
        });
        cx.run_until_parked();
        let expected = cx.update(|_, app| reader.read(app).list_state.logical_scroll_top());
        assert!(expected.item_ix > 0, "window draw did not measure list items");

        // Back to the top, then take the animated path.
        reader.update_in(cx, |reader, _, cx| {
            reader
                .list_state
                .scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
            cx.notify();
        });
        cx.run_until_parked();
        reader.update_in(cx, |reader, _, cx| {
            reader.scroll_to_block(last, cx);
            assert!(reader.scroll_anim.is_some(), "far scroll should animate");
        });

        // Let all 22 animation frames (11ms apart) play out.
        cx.executor().advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();

        reader.update_in(cx, |reader, _, _| {
            let landed = reader.list_state.logical_scroll_top();
            assert_eq!(landed.item_ix, expected.item_ix, "animation should land where a direct jump does");
            assert_eq!(landed.offset_in_item, expected.offset_in_item);
        });
    }

    // ── keyboard scrolling ─────────────────────────────────────────────

    #[test]
    fn scrolling_up_from_the_top_stays_at_the_top() {
        assert_eq!(clamped_scroll(0., -LINE_STEP, 900.), 0.);
        assert_eq!(clamped_scroll(30., -LINE_STEP, 900.), 0.);
    }

    #[test]
    fn scrolling_down_stops_at_the_content_bottom() {
        assert_eq!(clamped_scroll(880., 200., 900.), 900.);
        // A document shorter than its viewport has nowhere to go.
        assert_eq!(clamped_scroll(0., 200., 0.), 0.);
    }

    #[test]
    fn scrolling_within_the_range_moves_by_the_full_delta() {
        assert_eq!(clamped_scroll(100., 72., 900.), 172.);
        assert_eq!(clamped_scroll(100., -72., 900.), 28.);
    }

    #[test]
    fn a_page_step_keeps_some_context_from_the_previous_screen() {
        assert_eq!(page_step(600.), 600. - PAGE_OVERLAP);
    }

    #[test]
    fn a_page_step_on_a_short_viewport_still_advances() {
        // Never zero, or PageDown would appear dead on a tiny window.
        assert!(page_step(20.) > 0.);
        assert!(page_step(0.) > 0.);
    }

    // ── the keys, through a real window ────────────────────────────────

    /// Scroll position in px from the top of the document.
    fn offset_px(reader: &Entity<Reader>, cx: &mut VisualTestContext) -> f32 {
        cx.update(|_, app| {
            f32::from(-reader.read(app).list_state.scroll_px_offset_for_scrollbar().y)
        })
    }

    fn open_focused_reader<'a>(
        cx: &'a mut TestAppContext,
        source: &str,
    ) -> (Entity<Reader>, &'a mut VisualTestContext) {
        let (reader, cx) = open_reader(cx, source);
        reader.update_in(cx, |reader, window, _| window.focus(&reader.focus_handle));
        cx.run_until_parked();
        (reader, cx)
    }

    #[gpui::test]
    fn arrow_keys_scroll_the_rendered_document(cx: &mut TestAppContext) {
        let (reader, cx) = open_focused_reader(cx, &long_source());
        assert_eq!(offset_px(&reader, cx), 0.);

        cx.dispatch_action(ScrollDown);
        cx.run_until_parked();
        let down = offset_px(&reader, cx);
        assert!(down > 0., "down arrow should scroll the preview, got {down}");

        cx.dispatch_action(ScrollUp);
        cx.run_until_parked();
        assert_eq!(offset_px(&reader, cx), 0., "up arrow should scroll back");
    }

    #[gpui::test]
    fn page_down_moves_further_than_an_arrow_key(cx: &mut TestAppContext) {
        let (reader, cx) = open_focused_reader(cx, &long_source());
        cx.dispatch_action(ScrollDown);
        cx.run_until_parked();
        let line = offset_px(&reader, cx);

        cx.dispatch_action(ScrollTop);
        cx.run_until_parked();
        cx.dispatch_action(PageDown);
        cx.run_until_parked();
        let page = offset_px(&reader, cx);

        assert!(page > line, "PageDown ({page}) should outrun one arrow ({line})");

        cx.dispatch_action(PageUp);
        cx.run_until_parked();
        assert_eq!(offset_px(&reader, cx), 0., "PageUp should return to the top");
    }

    #[gpui::test]
    fn end_and_home_jump_to_the_document_ends(cx: &mut TestAppContext) {
        let (reader, cx) = open_focused_reader(cx, &long_source());
        let last = cx.update(|_, app| reader.read(app).document.blocks.len() - 1);
        // The end of a long document starts off screen.
        let offscreen = cx
            .update(|_, app| reader.read(app).list_state.bounds_for_item(last).is_none());
        assert!(offscreen, "test document must overflow its viewport");

        cx.dispatch_action(ScrollBottom);
        cx.run_until_parked();
        let onscreen = cx
            .update(|_, app| reader.read(app).list_state.bounds_for_item(last).is_some());
        assert!(onscreen, "End should bring the last block on screen");

        cx.dispatch_action(ScrollTop);
        cx.run_until_parked();
        assert_eq!(offset_px(&reader, cx), 0., "Home should land at the top");
    }

    /// The callback the rendered view actually hands to `view::list_item`
    /// has to carry the reader's own index. Only `describe` was tested,
    /// so building that callback any other way -- `describe_link` with
    /// `None` for the index, say -- left the suite green while every
    /// wiki link and every relative link in the reading view previewed
    /// as a note that does not exist.
    #[gpui::test]
    fn the_rendered_views_callback_previews_through_the_index(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Target.md"), "# Target\n\nthe body\n").unwrap();
        let note = dir.path().join("Note.md");
        let source = "see [[Target]] and [here](Target.md)\n";
        std::fs::write(&note, source).unwrap();
        let index = Arc::new(std::sync::Mutex::new(crate::knowledge::Index::scan(dir.path())));

        let langs = Languages::new();
        let reader = cx.new(|cx| {
            let mut reader =
                Reader::from_source_at(Some(note.clone()), "Note".into(), source, &langs, cx);
            reader.set_knowledge(index);
            reader
        });
        // The very callback `render` builds, not `describe` behind it.
        let describe = Reader::describe_callback(&reader);
        cx.update(|app| {
            for dest in ["[[Target", "Target.md"] {
                assert!(
                    matches!(describe(dest, app), Some(crate::preview::Preview::Note { .. })),
                    "{dest} must preview the note it resolves to, not a missing one: {:?}",
                    describe(dest, app),
                );
            }
        });
    }

    /// The tooltip's preview must answer from `PreviewState`'s cached
    /// grants -- the same cache `Editor::preview_for` reads -- and must
    /// see a newly written grant once told to refresh, not only at
    /// startup.
    fn consent_for_dest(
        cx: &mut TestAppContext,
        base: &Path,
        dest: &str,
    ) -> crate::preview::Consent {
        cx.update(|app| {
            let Some(crate::preview::Preview::External { consent, .. }) =
                describe_link(Some(base), dest, None, app)
            else {
                panic!("expected an external preview")
            };
            consent
        })
    }

    #[gpui::test]
    fn describe_link_reads_the_cached_grants_and_sees_a_refresh(cx: &mut TestAppContext) {
        let _home = crate::workspace::tests::temp_home();
        cx.update(|app| {
            app.set_global(crate::preview::PreviewState::new(Arc::new(|_: &str| Ok(Vec::new()))));
        });

        let base = PathBuf::from("/vault/note.md");
        let dest = "https://example.test/a";
        assert_eq!(
            consent_for_dest(cx, &base, dest),
            crate::preview::Consent::Ungranted,
            "nothing is granted before any grant is written"
        );

        // A grant lands on disk exactly the way
        // `enable_previews_for_hovered_site` writes one -- through
        // `settings::save`, never through this cx.
        let dir = crate::settings::config_dir();
        let mut settings = crate::settings::load(&dir);
        settings.plugin_grants.insert("supermd".into(), vec!["net:example.test".into()]);
        crate::settings::save(&dir, &settings).unwrap();

        assert_eq!(
            consent_for_dest(cx, &base, dest),
            crate::preview::Consent::Ungranted,
            "a stale cache must not see the new grant on its own"
        );

        cx.update(|app| app.global::<crate::preview::PreviewState>().refresh_grants());
        assert_eq!(
            consent_for_dest(cx, &base, dest),
            crate::preview::Consent::Granted,
            "refresh_grants must pick up what is on disk now"
        );
    }
}
