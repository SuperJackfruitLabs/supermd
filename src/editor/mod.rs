//! The Editor: GPUI shell around the tested core. Renders one logical
//! line per virtualized list item with styled-source typography; input
//! flows through EntityInputHandler (IME-correct) and editor actions.

pub mod autosave;
pub mod blocks;
pub mod buffer;
pub mod core;
pub mod display;
pub mod find;
pub mod formatting;
pub mod lists;
pub mod paste_image;
pub mod table_edit;
pub mod table_ops;
pub mod movement;
pub mod projection;
pub mod projector;
pub mod replace;
pub mod spans;

use std::collections::HashMap;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use gpui::prelude::*;
use gpui::{
    actions, anchored, deferred, div, fill, list, point, px, relative, size, App, AvailableSpace,
    Bounds, Corner,
    ClipboardItem, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    Focusable, Font, FontFeatures, FontStyle, FontWeight, GlobalElementId, Hsla, IntoElement,
    LayoutId, ListAlignment, ListOffset, ListState, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, Render, SharedString, StrikethroughStyle, Style,
    TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine,
};

use crate::elevation::Elevated as _;
use crate::highlight::Languages;
use crate::reader::language_for_path;
use crate::theme::{theme, Theme};
use autosave::SavePolicy;
use core::{EditorCore, Selection};
use spans::{LineKind, StyleKind, StyleSpan};

actions!(
    editor,
    [
        MoveLeft, MoveRight, MoveUp, MoveDown, SelectLeft, SelectRight, SelectUp, SelectDown,
        MoveWordLeft, MoveWordRight, SelectWordLeft, SelectWordRight, LineStart, LineEnd,
        SelectLineStart, SelectLineEnd, DocStart, DocEnd, PageUp, PageDown, Backspace, Delete,
        DeleteWordLeft, Newline, InsertTab, Undo, Redo, SelectAll, Copy, Cut, Paste, SaveNow,
        OpenFind, FindNext, FindPrev, CloseFind, ToggleBold, ToggleItalic, ToggleCode,
        ToggleStrike, InsertLink, CycleHeading, ToggleQuote, Outdent, FollowLink,
        DismissCompletion, ReplaceNext, ReplaceAll, TableInsertRow, TableDeleteRow,
        TableInsertColumn, TableDeleteColumn, RenumberList
    ]
);

struct FindState {
    input: Entity<crate::input::TextInput>,
    /// The replacement field, shown only once the user asks for it.
    replace_input: Entity<crate::input::TextInput>,
    replacing: bool,
    matches: Vec<Range<usize>>,
    active: usize,
    _watch: gpui::Subscription,
}

const PAGE_LINES: usize = 40;

enum Provider {
    Markdown,
    Code(String),
    Plain,
}

/// Digits needed for the last line number (gutter width).
fn gutter_cols(line_count: usize) -> usize {
    line_count.max(1).to_string().len()
}

#[cfg(test)]
mod gutter_tests {
    #[test]
    fn gutter_cols_counts_digits_of_last_line() {
        assert_eq!(super::gutter_cols(1), 1);
        assert_eq!(super::gutter_cols(9), 1);
        assert_eq!(super::gutter_cols(10), 2);
        assert_eq!(super::gutter_cols(9999), 4);
        assert_eq!(super::gutter_cols(0), 1);
    }
}

/// Geometry of a painted line, kept for mouse hit-testing, IME rects,
/// and vertical cursor movement.
struct CachedLine {
    line: WrappedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    display: display::DisplayLine,
}

/// Read-only "Show Changes" state: the merged old+new document, its
/// styling, and the change wash map. Lives beside the buffer — the
/// buffer itself is never touched by diff mode.
pub struct DiffState {
    core: EditorCore,
    spans: Vec<StyleSpan>,
    line_kinds: Vec<LineKind>,
    changes: Vec<crate::diff::Change>,
    /// Code-mode gutter labels (new-file numbers, `-` on deleted lines).
    gutter: Vec<String>,
    missing: Option<crate::git::Baseline>,
    adds: usize,
    dels: usize,
}

pub struct Editor {
    /// Test seams standing in for a format-on-save plugin and a save
    /// hook plugin, consulted at the exact points the plugins are, so
    /// tests can make a save change the buffer without building wasm.
    #[cfg(test)]
    pub(crate) test_formatter: Option<fn(&str) -> String>,
    #[cfg(test)]
    pub(crate) test_save_hook: Option<fn(&str) -> String>,
    core: EditorCore,
    provider: Provider,
    diff: Option<DiffState>,
    spans: Vec<StyleSpan>,
    line_kinds: Vec<LineKind>,
    blocks: Vec<blocks::BlockInfo>,
    /// Every followable link in the document, in document order.
    ///
    /// Recomputed in `restyle` — on open and on edit, never per frame —
    /// because `extract_all_links` costs ~7.65 ms on a 1 MB document.
    /// Hover hit-testing runs on every pointer move and could not
    /// afford that; neither, really, could the click path, which paid
    /// it once per press.
    links: Vec<crate::knowledge::RawLink>,
    claims: Vec<(usize, projector::Claim)>,
    /// Inline-cache generation this editor last styled against.
    inline_gen: u64,
    projection: Vec<projection::Item>,
    path: PathBuf,
    pub save: SavePolicy,
    pub disk_mtime: Option<SystemTime>,
    list_state: ListState,
    focus_handle: FocusHandle,
    layout_cache: HashMap<usize, CachedLine>,
    marked_range: Option<Range<usize>>,
    dragging: bool,
    /// A left press that landed on a followable link. Navigation waits
    /// for the release, so a drag can still begin inside link text; the
    /// link found at press time rides along rather than being extracted
    /// a second time.
    pending_link: Option<PendingLink>,
    preferred_x: Option<Pixels>,
    save_task: Option<gpui::Task<()>>,
    find: Option<FindState>,
    scrollbar_dragging: bool,
    scroll_anim: Option<gpui::Task<()>>,
    /// A paste awaiting (or retrying) net enrichment.
    pending_enrich: Option<PendingEnrich>,
    /// Latest widget-plugin status line ("1,234 words · 6 min read").
    status_text: Option<SharedString>,
    /// Debounce handle: replacing it cancels the pending refresh.
    status_task: Option<gpui::Task<()>>,
    /// `[[` wiki-link completion, refreshed on every edit.
    completion: Option<CompletionState>,
    /// The floating format toolbar has been armed by a settled mouse
    /// selection. It only paints while the selection is still live.
    toolbar_visible: bool,
    /// Settle timer; replacing it cancels the pending reveal.
    toolbar_task: Option<gpui::Task<()>>,
    /// The link the pointer is resting on, the dwell task that will
    /// open its popover, and what to draw once it does.
    hover_link: Option<crate::knowledge::RawLink>,
    hover_task: Option<gpui::Task<()>>,
    hover_at: Option<gpui::Point<Pixels>>,
    hover_preview: Option<crate::preview::Preview>,
    /// The pointer is inside the popover itself, so it must not close.
    hover_held: bool,
    hover_close_task: Option<gpui::Task<()>>,
    /// The owning workspace's knowledge index and plugin host, handed
    /// over when the workspace builds the editor. An editor does not
    /// know *which* workspace owns it, so it is given what it needs
    /// rather than reaching for a process global — two windows on two
    /// folders have two indexes and two plugin sandbox roots.
    knowledge: Option<crate::knowledge::KnowledgeHandle>,
    host: Option<crate::extensions::HostHandle>,
    /// The host's preopen root, on a cell shared with the host. Block
    /// widgets read it every frame to key the diagram cache, and
    /// locking the host for that would stall the UI behind a running
    /// plugin call.
    ///
    /// Shared, not copied — but only because every path that replaces
    /// the host keeps the cell: `set_workspace_root` writes through it,
    /// and Reload Plugins hands the replacement host the old cell via
    /// `ExtensionHost::adopt_root_handle`. A host swap that minted a
    /// fresh cell instead would orphan this one silently, and the
    /// editor would key its renders under a root that had stopped
    /// tracking the window. `reloading_plugins_keeps_the_root_cell_editors_already_hold`
    /// is what holds that up.
    host_root: Option<crate::extensions::RootHandle>,
    /// Registered once, lazily, from the first render: a press belongs
    /// to the document that was on screen when it happened, and
    /// neither it nor the hover it started should outlive this editor
    /// losing focus — a tab switch moves focus to the newly active
    /// document before this one is ever rendered again, so the moment
    /// it happens is the only reliable place to catch it.
    blur_subscription: Option<gpui::Subscription>,
}

/// Snapshot taken right after a paste lands, so a background enricher
/// can replace the pasted range iff the document has not moved.
struct PendingEnrich {
    range: Range<usize>,
    snapshot: String,
    pasted: String,
}

/// A left press on a link, waiting for its release to decide whether it
/// was a click (navigate) or the start of a drag (select).
struct PendingLink {
    /// Buffer offset of the press — where the caret goes if the link
    /// turns out not to be followable, and the anchor a drag starts from.
    offset: usize,
    link: crate::knowledge::RawLink,
}

pub enum EditorEvent {
    /// A net-capable enricher needs a per-domain grant
    /// (cap is "net:<domain>").
    ConsentNeeded { plugin: String, cap: String },
    /// A followed link wants this file opened in a tab.
    OpenPath(PathBuf),
    /// A right-click landed in the document. The workspace owns the
    /// one context-menu overlay (sidebar, tabs and graph nodes already
    /// raise it), so the editor reports where the press was and what it
    /// knew at the caret; `menus::items_for` turns that into rows.
    ContextMenu { position: gpui::Point<Pixels>, ctx: crate::menus::EditorContext },
    /// A command declined and has to say why. A refusal nobody can see
    /// is indistinguishable from a broken command, and the workspace
    /// owns the one transient message strip (`show_command_error`).
    CommandError(String),
}

/// The `[[` completion popup: doc offset of the opener, the filtered
/// candidates, and the highlighted row.
struct CompletionState {
    open: usize,
    matches: Vec<(String, PathBuf)>,
    selected: usize,
}

impl gpui::EventEmitter<EditorEvent> for Editor {}

/// Run the save-hook chain: each plugin sees the previous result;
/// Err/None leave the text unchanged for the next.
fn chain_save_hooks(
    text: String,
    path: &str,
    plugins: &[String],
    mut call: impl FnMut(&str, &str, &str) -> Result<Option<String>, String>,
) -> String {
    plugins.iter().fold(text, |acc, plugin| match call(plugin, path, &acc) {
        Ok(Some(next)) => next,
        _ => acc,
    })
}

/// Compute the enriched document's replacement range, or None when the
/// document changed since the paste snapshot (the enrichment is then
/// forfeited — recorded honest limit).
fn enrich_plan(
    current: &str,
    pasted: Range<usize>,
    snapshot: &str,
    replacement: &str,
) -> Option<(String, Range<usize>)> {
    if current != snapshot {
        return None;
    }
    let mut out = String::with_capacity(current.len());
    out.push_str(&current[..pasted.start]);
    out.push_str(replacement);
    out.push_str(&current[pasted.end..]);
    Some((out, pasted.start..pasted.start + replacement.len()))
}

/// One backup registry per app session, shared by all editors.
pub struct SessionBackups(pub std::sync::Arc<std::sync::Mutex<autosave::BackupRegistry>>);

impl gpui::Global for SessionBackups {}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "markdown" | "mdown" | "mdx")
    )
}

/// Whether a click should navigate rather than place the caret.
///
/// A rendered link follows on a plain click, matching every note-focused
/// editor. A link whose syntax is revealed is one the cursor is already
/// inside, so a plain click there edits it — otherwise the link could
/// never be corrected. ⌘-click always follows, as before.
pub fn click_follows_link(has_modifier: bool, on_link: bool, revealed: bool) -> bool {
    on_link && (has_modifier || !revealed)
}

/// Whether ⌘⇧G belongs to the editor's find bar or should fall through
/// to the global Graph View binding. Pure so the collision rule is
/// recorded in a test rather than in a comment.
pub fn find_prev_should_consume(find_open: bool) -> bool {
    find_open
}

impl Editor {
    /// Read a file's text. Call `from_text` inside `cx.new` (which cannot
    /// be fallible) with the result.
    pub fn read_file(path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub fn from_text(path: &Path, text: String, langs: &Languages, cx: &mut Context<Self>) -> Self {
        Self::from_text_in(path, text, langs, None, None, cx)
    }

    /// The workspace's constructor: the same editor, plus the handles
    /// to the index and plugin host that own it.
    pub fn from_text_in(
        path: &Path,
        text: String,
        langs: &Languages,
        knowledge: Option<crate::knowledge::KnowledgeHandle>,
        host: Option<crate::extensions::HostHandle>,
        cx: &mut Context<Self>,
    ) -> Self {
        let provider = if is_markdown(path) {
            Provider::Markdown
        } else if let Some(lang) = language_for_path(path) {
            Provider::Code(lang)
        } else {
            Provider::Plain
        };
        let core = EditorCore::new(&text);
        let line_count = core.buffer.line_count();
        let list_state = ListState::new(line_count, ListAlignment::Top, px(512.));
        {
            // Keep the scrollbar thumb in sync with wheel scrolling.
            let entity = cx.weak_entity();
            list_state.set_scroll_handler(move |_, _, cx| {
                entity.update(cx, |_, cx| cx.notify()).ok();
            });
        }
        let mut editor = Self {
            core,
            provider,
            diff: None,
            spans: Vec::new(),
            line_kinds: Vec::new(),
            blocks: Vec::new(),
            links: Vec::new(),
            claims: Vec::new(),
            inline_gen: 0,
            projection: Vec::new(),
            path: path.to_path_buf(),
            save: SavePolicy::default(),
            disk_mtime: autosave::disk_mtime(path),
            list_state,
            focus_handle: cx.focus_handle(),
            layout_cache: HashMap::new(),
            marked_range: None,
            dragging: false,
            pending_link: None,
            preferred_x: None,
            save_task: None,
            find: None,
            scrollbar_dragging: false,
            scroll_anim: None,
            pending_enrich: None,
            status_text: None,
            #[cfg(test)]
            test_formatter: None,
            #[cfg(test)]
            test_save_hook: None,
            status_task: None,
            completion: None,
            toolbar_visible: false,
            toolbar_task: None,
            hover_link: None,
            hover_task: None,
            hover_at: None,
            hover_preview: None,
            hover_held: false,
            hover_close_task: None,
            knowledge,
            host_root: host
                .as_ref()
                .map(|h| h.lock().unwrap_or_else(|e| e.into_inner()).root_handle()),
            host,
            blur_subscription: None,
        };
        editor.restyle(langs);
        editor.schedule_status(cx);
        editor
    }

    pub fn text(&self) -> String {
        self.core.buffer.text()
    }

    /// The cell naming the folder this editor's plugin renders may
    /// read. The block-widget path forwards it straight to
    /// `diagram::plugin_diagram_state`, which does the read itself —
    /// nothing in between computes a root that could be wrong.
    pub(crate) fn plugin_root_handle(&self) -> Option<crate::extensions::RootHandle> {
        self.host_root.clone()
    }

    /// Point the editor at a new path after a rename or move; buffer
    /// and history stay put.
    pub fn retarget(&mut self, path: PathBuf) {
        self.disk_mtime = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());
        self.path = path;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where an image destination in this document points.
    ///
    /// Anchored to the document that wrote the link, never to the
    /// process's working directory -- and the reading view asks the
    /// same function with the same anchor, so one link cannot resolve
    /// two ways (#57).
    pub fn image_source(&self, dest: &str) -> crate::markdown::ImageSource {
        crate::markdown::resolve_image(dest, Some(&self.path))
    }

    pub fn title(&self) -> SharedString {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
            .into()
    }

    /// The followable link containing `offset`, from the cache.
    ///
    /// `extract_all_links` returns links in document order, so the
    /// ranges are sorted and disjoint and a binary search answers in
    /// log time — cheap enough for a pointer-move handler.
    pub(crate) fn link_at_offset(&self, offset: usize) -> Option<&crate::knowledge::RawLink> {
        let ix = self
            .links
            .binary_search_by(|l| {
                if l.range.end <= offset {
                    std::cmp::Ordering::Less
                } else if offset < l.range.start {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()?;
        self.links.get(ix)
    }

    fn restyle(&mut self, langs: &Languages) {
        let text = self.core.buffer.text();
        // Links only mean anything in a Markdown document; a code file's
        // click path is gated on `can_format()` and never follows one.
        self.links = if matches!(self.provider, Provider::Markdown) {
            crate::knowledge::extract_all_links(&text)
        } else {
            Vec::new()
        };
        self.spans = match &self.provider {
            Provider::Markdown => spans::markdown_spans_highlighted(&text, langs),
            Provider::Code(lang) => spans::code_spans(&text, lang.as_str(), langs),
            Provider::Plain => Vec::new(),
        };
        // Plugin inline pass: cache hits become replacement spans;
        // misses go to the background drainer (never wasm here).
        if matches!(self.provider, Provider::Markdown) {
            let (extra, misses) = crate::extensions::with_inline_table(|table| {
                let lookup = |p: &str, i: &str, m: &str| crate::extensions::inline_lookup(p, i, m);
                spans::inline_pass(&text, &self.spans, table, &lookup)
            });
            {
                if !extra.is_empty() {
                    self.spans.extend(extra);
                    self.spans.sort_by_key(|s| (s.range.start, s.range.end));
                }
                crate::extensions::enqueue_inline(misses);
            }
        }
        self.inline_gen = crate::extensions::inline_generation();
        self.line_kinds = spans::line_kinds(&text, &self.spans);
        self.blocks = match self.provider {
            Provider::Markdown => blocks::blocks(&text),
            _ => Vec::new(),
        };
        self.claims = {
            let line_ranges: Vec<Range<usize>> = (0..self.core.buffer.line_count())
                .map(|ix| self.core.buffer.line_range(ix))
                .collect();
            projector::discover_all(&text, &self.blocks, &line_ranges)
        };
        self.layout_cache.clear();
        self.projection = self.compute_projection();
        self.list_state.reset(self.projection.len());
    }

    fn compute_projection(&self) -> Vec<projection::Item> {
        let line_ranges: Vec<Range<usize>> = (0..self.core.buffer.line_count())
            .map(|ix| self.core.buffer.line_range(ix))
            .collect();
        projection::project(
            &line_ranges,
            &self.blocks,
            &self.claims,
            self.core.selection.range(),
        )
    }

    // ── diff mode ("Show Changes") ─────────────────────────────────────

    pub fn diff_active(&self) -> bool {
        self.diff.is_some()
    }

    /// The "repository is out of scope" hint for the empty-diff message,
    /// asked of the open workspace root. No workspace, no hint.
    fn git_scope_hint(&self) -> Option<&'static str> {
        let root = self
            .knowledge
            .as_ref()
            .and_then(|k| k.lock().ok().map(|ix| ix.root.clone()))
            .filter(|root| !root.as_os_str().is_empty())?;
        crate::workspace::git_scope_hint(
            false,
            crate::workspace::repo_root_may_be_out_of_scope(&root),
        )
    }

    /// Enter (or recompute) the read-only diff-vs-HEAD view.
    pub fn enter_diff(&mut self, langs: &Languages, cx: &mut Context<Self>) {
        let (doc, missing) = match crate::git::head_text(&self.path) {
            crate::git::Baseline::Text(old) => {
                (crate::diff::diff_doc(&old, &self.core.buffer.text()), None)
            }
            other => (crate::diff::DiffDoc::default(), Some(other)),
        };
        let (adds, dels) = crate::diff::counts(&doc);
        let spans = match &self.provider {
            Provider::Markdown => spans::markdown_spans_highlighted(&doc.text, langs),
            Provider::Code(lang) => spans::code_spans(&doc.text, lang.as_str(), langs),
            Provider::Plain => Vec::new(),
        };
        let line_kinds = spans::line_kinds(&doc.text, &spans);
        let core = EditorCore::new(&doc.text);
        let line_count = core.buffer.line_count();
        let gutter = crate::diff::diff_gutter_labels(&doc);
        self.diff = Some(DiffState {
            core,
            spans,
            line_kinds,
            changes: doc.changes,
            gutter,
            missing,
            adds,
            dels,
        });
        self.layout_cache.clear();
        self.list_state.reset(line_count);
        cx.notify();
    }

    pub fn exit_diff(&mut self, cx: &mut Context<Self>) {
        if self.diff.take().is_some() {
            self.layout_cache.clear();
            self.list_state.reset(self.projection.len());
            cx.notify();
        }
    }

    /// Recompute the diff if it is showing (buffer reloaded from disk).
    pub fn refresh_diff(&mut self, langs: &Languages, cx: &mut Context<Self>) {
        if self.diff.is_some() {
            self.enter_diff(langs, cx);
        }
    }

    /// Buffer the view renders from: the merged diff doc in diff mode,
    /// the real buffer otherwise.
    fn view_buffer(&self) -> &buffer::Buffer {
        match &self.diff {
            Some(d) => &d.core.buffer,
            None => &self.core.buffer,
        }
    }

    fn view_spans(&self) -> &[StyleSpan] {
        match &self.diff {
            Some(d) => &d.spans,
            None => &self.spans,
        }
    }

    fn view_line_kinds(&self) -> &[LineKind] {
        match &self.diff {
            Some(d) => &d.line_kinds,
            None => &self.line_kinds,
        }
    }

    /// Code-mode gutter label for a line (diff-aware).
    fn gutter_label(&self, ix: usize) -> String {
        match &self.diff {
            Some(d) => d.gutter.get(ix).cloned().unwrap_or_default(),
            None => (ix + 1).to_string(),
        }
    }

    /// Recompute the projection for the current selection; reset the
    /// list only when the item structure actually changed.
    fn reproject(&mut self) {
        if self.diff.is_some() {
            return; // list is showing the diff doc, not the projection
        }
        let items = self.compute_projection();
        if items != self.projection {
            // reset() clears logical_scroll_top AND discards every
            // measured height, so a following scroll_to_reveal_item
            // computes goal_top = 0 and pins to item 0. Capture the
            // anchor first and restore it with scroll_to, which sets the
            // anchor directly and needs no measurements.
            let anchor = self.list_state.logical_scroll_top();
            self.projection = items;
            self.list_state.reset(self.projection.len());
            // The anchor is a raw item index, not a document position:
            // if the projection change happened *above* the viewport
            // (a widget up there collapsed or expanded, changing how
            // many items precede this one), the same index now names a
            // different line and the view shifts by that difference.
            // Only clamping is done here. Fixing it properly means
            // anchoring on a buffer offset and mapping it back through
            // the new projection; the drift is bounded by the size of
            // one claim and is strictly better than the pin-to-item-0
            // top-jump this replaced.
            let clamped = ListOffset {
                item_ix: anchor.item_ix.min(self.projection.len().saturating_sub(1)),
                offset_in_item: anchor.offset_in_item,
            };
            self.list_state.scroll_to(clamped);
            // Known limitation: reveal_cursor()'s downward case cannot
            // actually scroll further right after this reset. Every item
            // was just spliced back in as Unmeasured (height 0), and
            // ListState::scroll_to_reveal_item's below-anchor branch derives
            // goal_top from summed measured heights (vendor/gpui/src/
            // elements/list.rs:360) — with all heights zero it always
            // computes goal_top = 0, so start_ix comes back 0 and the
            // `start_ix >= scroll_top.item_ix` guard fails for any nonzero
            // anchor, silently no-opping. The above-or-at-anchor branch
            // sets the index directly and needs no heights, so upward
            // reveals still work. Net effect here: a cursor that lands
            // below the just-restored anchor (e.g. a click deep inside a
            // widget taller than the viewport) may not be scrolled into
            // view by this call. This is strictly no worse than before —
            // the call was equally unable to reveal pre-fix, it just
            // happened to pin to the top — so it's left as a known gap
            // rather than a regression. Closing it for real means either
            // deferring this call until a paint has measured items near
            // the target (window.on_next_frame, and even then only
            // measures outward from the anchor incrementally, so it isn't
            // guaranteed for far-off targets) or computing the restored
            // anchor to include the cursor's item directly instead of
            // going through gpui's height-based reveal — both are timing/
            // behavior changes to render and measurement, out of scope
            // for this bug fix.
            self.reveal_cursor();
        }
    }

    pub fn heading_lines(&self) -> Vec<(u8, String, usize)> {
        self.spans
            .iter()
            .filter_map(|s| match s.kind {
                StyleKind::Heading(level) => {
                    let line = self.core.buffer.line_of_byte(s.range.start);
                    let text = self
                        .core
                        .buffer
                        .slice(s.range.clone())
                        .trim_start_matches('#')
                        .trim()
                        .to_string();
                    Some((level, text, line))
                }
                _ => None,
            })
            .collect()
    }

    pub fn scroll_to_line(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.diff.is_some() {
            return; // outline indices don't map to the merged diff doc
        }
        let item = projection::item_of_line(&self.projection, ix);
        self.animate_scroll_to_item(item, cx);
    }

    /// Eased pixel-space scroll (~250 ms). The list's height tree lets us
    /// read the target's pixel offset synchronously (jump, read, restore
    /// — no frame is painted in between), then interpolate real pixels.
    fn animate_scroll_to_item(&mut self, target: usize, cx: &mut Context<Self>) {
        let state = self.list_state.clone();
        let current = -state.scroll_px_offset_for_scrollbar().y;
        state.scroll_to(ListOffset { item_ix: target, offset_in_item: px(0.) });
        let target_px = -state.scroll_px_offset_for_scrollbar().y;
        if (target_px - current).abs() < px(24.) {
            cx.notify(); // stay on the (tiny) jump
            return;
        }
        state.set_offset_from_scrollbar(point(px(0.), -current));
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
                    .update(cx, |editor, cx| {
                        editor
                            .list_state
                            .set_offset_from_scrollbar(point(px(0.), -y));
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            // Heights near the target may have re-measured mid-flight;
            // land exactly on the item.
            this.update(cx, |editor, cx| {
                editor
                    .list_state
                    .scroll_to(ListOffset { item_ix: target, offset_in_item: px(0.) });
                cx.notify();
            })
            .ok();
        }));
    }

    // ── editing plumbing ───────────────────────────────────────────────

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        let langs = crate::highlight::languages(cx);
        self.restyle(&langs);
        self.save.record_edit(Instant::now());
        // Debounced autosave: replacing save_task drops (cancels) the
        // previous timer; should_flush re-checks in case of races.
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(autosave::DEBOUNCE).await;
            this.update(cx, |editor, cx| {
                if editor.save.should_flush(Instant::now()) {
                    editor.flush(cx);
                }
            })
            .ok();
        }));
        self.preferred_x = None;
        if self.find.is_some() {
            let query = self
                .find
                .as_ref()
                .map(|s| s.input.read(cx).content.to_string())
                .unwrap_or_default();
            self.recompute_matches(&query);
        }
        self.reveal_cursor();
        self.schedule_status(cx);
        self.refresh_completion();
        cx.notify();
    }

    /// Rebuild the `[[` completion for the text left of the cursor.
    fn refresh_completion(&mut self) {
        self.completion = None;
        if !self.can_format() || !self.core.selection.is_cursor() {
            return;
        }
        let head = self.core.selection.head;
        let line_ix = self.core.buffer.line_of_byte(head);
        let line_start = self.core.buffer.line_range(line_ix).start;
        let line = self.core.buffer.line_text(line_ix);
        let upto = &line[..(head - line_start).min(line.len())];
        let Some(open_rel) = upto.rfind("[[") else {
            return;
        };
        let query = &upto[open_rel + 2..];
        if query.contains(']') || query.contains('[') || query.contains('|') {
            return;
        }
        let Some(state) = self.knowledge.clone() else {
            return;
        };
        let q = query.to_lowercase();
        let mut matches: Vec<(String, PathBuf)> = state
            .lock()
            .unwrap()
            .note_names()
            .into_iter()
            .filter(|(name, _)| name.to_lowercase().contains(&q))
            .collect();
        matches.sort_by_key(|(name, _)| (!name.to_lowercase().starts_with(&q), name.clone()));
        matches.truncate(8);
        if matches.is_empty() {
            return;
        }
        let open = line_start + open_rel;
        let selected = match &self.completion {
            Some(prev) if prev.open == open => prev.selected.min(matches.len() - 1),
            _ => 0,
        };
        self.completion = Some(CompletionState { open, matches, selected });
    }

    /// Steer the completion popup; true when the keystroke was ours.
    fn completion_step(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(comp) = &mut self.completion else {
            return false;
        };
        let n = comp.matches.len() as isize;
        comp.selected = ((comp.selected as isize + delta + n) % n) as usize;
        cx.notify();
        true
    }

    /// Replace `[[query` with the chosen `[[Name]]`.
    fn confirm_completion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(comp) = &self.completion else {
            return false;
        };
        let Some((name, _)) = comp.matches.get(comp.selected).cloned() else {
            return false;
        };
        let range = comp.open..self.core.selection.head;
        self.completion = None;
        self.core.break_undo_group();
        self.core
            .replace_range(range, &format!("[[{name}]]"), Instant::now());
        self.core.break_undo_group();
        self.after_edit(cx);
        true
    }

    fn dismiss_completion(&mut self, _: &DismissCompletion, _: &mut Window, cx: &mut Context<Self>) {
        if self.completion.take().is_some() {
            cx.notify();
        }
    }

    fn follow_link(&mut self, _: &FollowLink, _: &mut Window, cx: &mut Context<Self>) {
        self.follow_link_at(self.core.selection.head, cx);
    }

    /// Open (or create, for unresolved wiki targets) the link under
    /// `offset`. Returns true when a link was followed.
    fn follow_link_at(&mut self, offset: usize, cx: &mut Context<Self>) -> bool {
        if !self.can_format() {
            return false;
        }
        let text = self.core.buffer.text();
        let Some(link) = crate::knowledge::Index::link_at(&text, offset) else {
            return false;
        };
        self.open_link(&link, cx)
    }

    /// Open an already-located link. Split out from `follow_link_at` so
    /// a caller that has just found the link (the mouse handler) does not
    /// pay for a second `link_at` over the whole document.
    fn open_link(&mut self, link: &crate::knowledge::RawLink, cx: &mut Context<Self>) -> bool {
        if !self.can_format() {
            return false;
        }
        // Neither an external link nor an anchor touches the index —
        // classify first.
        match crate::knowledge::classify(link) {
            crate::knowledge::LinkTarget::External(url) => {
                cx.open_url(&url);
                return true;
            }
            // `#heading` is a position in the document already open, not
            // a path. The `toc` plugin writes a page of these, and until
            // now every one of them was joined onto a directory, failed
            // to resolve, and did nothing when clicked.
            crate::knowledge::LinkTarget::Anchor(anchor) => {
                let text = self.core.buffer.text();
                let Some(offset) = crate::knowledge::heading_offset(&text, &anchor) else {
                    return false;
                };
                self.core.selection = crate::editor::core::Selection::cursor(offset);
                self.reveal_cursor();
                cx.notify();
                return true;
            }
            _ => {}
        }
        let Some(state) = self.knowledge.clone() else {
            return false;
        };
        let resolved = state.lock().unwrap().resolve(&self.path, link);
        let target = match resolved {
            Some(path) => path,
            None if link.wiki => {
                // Create the missing note beside this file.
                let Some(dir) = self.path.parent() else {
                    return false;
                };
                let root = state.lock().unwrap().root.clone();
                // `link.target` is unsanitised text from between the
                // brackets: contain it before anything touches the disk.
                let Some(path) = crate::knowledge::creatable_note_path(&root, dir, &link.target)
                else {
                    eprintln!(
                        "supermd: refusing [[{}]]: it resolves outside the workspace",
                        link.target
                    );
                    return false;
                };
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                // `create_new`, never `write`: an existing file must be
                // OPENED, and `fs::write(path, "")` silently zeroes it
                // instead — with no undo, because it is not the open
                // buffer. `AlreadyExists` (a file we did not see, or one
                // that appeared in between) is the same answer: open it.
                match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(_) => state.lock().unwrap().update_file(&path, ""),
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                        if std::fs::symlink_metadata(&path)
                            .is_ok_and(|m| m.file_type().is_symlink())
                        {
                            eprintln!(
                                "supermd: refusing [[{}]]: it is a symlink out of the workspace",
                                link.target
                            );
                            return false;
                        }
                        // O_EXCL refuses a symlink, dangling ones
                        // included — which is the only reason a target
                        // pointing outside the workspace was not written
                        // here. Opening it anyway would hand the editor
                        // that escaping path, and the next save would
                        // write through it. Containment cannot catch
                        // this case: a dangling link has no canonical
                        // leaf, so the check anchors at the parent and
                        // approves it.
                    }
                    Err(err) => {
                        eprintln!("supermd: cannot create {}: {err}", path.display());
                        return false;
                    }
                }
                path
            }
            None => return false,
        };
        cx.emit(EditorEvent::OpenPath(target));
        true
    }

    /// Latest widget status line, if any plugin produced one.
    pub fn status(&self) -> Option<SharedString> {
        self.status_text.clone()
    }

    /// Debounced status-widget refresh (500ms after the last edit).
    /// Zero cost when no widget plugins are loaded.
    pub fn schedule_status(&mut self, cx: &mut Context<Self>) {
        if crate::extensions::widget_plugins().is_empty() {
            return;
        }
        let Some(host) = self.host.clone() else {
            return;
        };
        self.status_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            let Ok(document) = this.update(cx, |this, _| this.core.buffer.text()) else {
                return;
            };
            let text = cx
                .background_executor()
                .spawn(async move {
                    let mut host = host.lock().unwrap();
                    let parts: Vec<String> = crate::extensions::widget_plugins()
                        .iter()
                        .filter_map(|p| host.status_text(p, &document).ok())
                        .collect();
                    parts.join(" · ")
                })
                .await;
            this.update(cx, |this, cx| {
                this.status_text = (!text.is_empty()).then(|| text.into());
                cx.notify();
            })
            .ok();
        }));
    }

    /// Replace the buffer with the on-disk content (clean buffers only —
    /// callers gate on `autosave::should_reload`). History resets.
    pub fn reload_from_disk(&mut self, cx: &mut Context<Self>) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return;
        };
        let mut head = self.core.selection.head.min(text.len());
        while head > 0 && !text.is_char_boundary(head) {
            head -= 1;
        }
        self.core = EditorCore::new(&text);
        self.core.set_cursor(head);
        self.save = SavePolicy::default();
        self.disk_mtime = autosave::disk_mtime(&self.path);
        self.marked_range = None;
        // The buffer just got swapped out from under any in-flight
        // press or hover: their offsets belong to text that no longer
        // exists, so a release now must not navigate and a stale
        // popover must not linger.
        self.pending_link = None;
        self.hover_link = None;
        self.hover_task = None;
        self.hover_at = None;
        self.hover_preview = None;
        self.hover_held = false;
        self.hover_close_task = None;
        let langs = crate::highlight::languages(cx);
        self.restyle(&langs);
        if self.find.is_some() {
            let query = self
                .find
                .as_ref()
                .map(|s| s.input.read(cx).content.to_string())
                .unwrap_or_default();
            self.recompute_matches(&query);
        }
        cx.notify();
    }

    /// The one save path: conflict check → backup → atomic write.
    /// Format-on-save (opt-in): synchronous under the plugin epoch cap;
    /// any failure saves the original unformatted text.
    fn maybe_format_before_save(&mut self, cx: &mut Context<Self>) {
        if !crate::settings::load(&crate::settings::config_dir()).format_on_save {
            return;
        }
        #[cfg(test)]
        if let Some(format) = self.test_formatter {
            let snapshot = self.core.buffer.text();
            let formatted = format(&snapshot);
            if formatted != snapshot {
                self.apply_command_output(
                    &crate::extensions::CommandOutput::ReplaceDocument(formatted),
                    cx,
                );
            }
            return;
        }
        let plugins = crate::extensions::format_plugins();
        let Some(plugin) = plugins.first() else {
            return;
        };
        let Some(host) = self.host.clone() else {
            return;
        };
        let snapshot = self.core.buffer.text();
        let result = host.lock().unwrap().format_document(plugin, &snapshot);
        if let Ok(formatted) = result {
            if formatted != snapshot {
                self.apply_command_output(
                    &crate::extensions::CommandOutput::ReplaceDocument(formatted),
                    cx,
                );
            }
        }
    }

    /// Always-on pre-save transforms (hooks = ["save"]), after the
    /// optional formatter. The flush path is synchronous on the main
    /// thread, so the buffer cannot move between snapshot and apply —
    /// the same guarantee the formatter relies on.
    fn run_save_hooks(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        if let Some(hook) = self.test_save_hook {
            let snapshot = self.core.buffer.text();
            let result = hook(&snapshot);
            if result != snapshot {
                self.apply_command_output(
                    &crate::extensions::CommandOutput::ReplaceDocument(result),
                    cx,
                );
            }
            return;
        }
        let plugins = crate::extensions::hook_plugins();
        if plugins.is_empty() {
            return;
        }
        let Some(host) = self.host.clone() else {
            return;
        };
        let snapshot = self.core.buffer.text();
        let path = self.path.to_string_lossy().into_owned();
        let result = chain_save_hooks(snapshot.clone(), &path, &plugins, |p, path, doc| {
            host.lock().unwrap().on_save(p, path, doc)
        });
        if result != snapshot {
            self.apply_command_output(
                &crate::extensions::CommandOutput::ReplaceDocument(result),
                cx,
            );
        }
    }

    /// Apply an edit that came from outside the editor (a checkbox
    /// clicked in the reading view) and save it now, through the one
    /// save path. One undo step; the selection stays where it was.
    ///
    /// The opt-in formatter is skipped (decided with the user): a click
    /// is a one-byte change, and a formatter run on it rewrote the file
    /// out of sight, added a second undo step and moved the caret. Save
    /// hooks still run -- they are always on -- so the caller must
    /// expect the buffer to differ from its own edit afterwards.
    pub fn replace_and_save(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let saved = self.core.selection;
        self.core.break_undo_group();
        self.core.replace_range(range, text, Instant::now());
        self.core.break_undo_group();
        let len = self.core.buffer.len_bytes();
        self.core.selection = saved;
        self.core.selection.anchor = self.core.selection.anchor.min(len);
        self.core.selection.head = self.core.selection.head.min(len);
        self.after_edit(cx);
        self.flush_with(false, cx);
    }

    pub fn flush(&mut self, cx: &mut Context<Self>) {
        self.flush_with(true, cx);
    }

    fn flush_with(&mut self, format: bool, cx: &mut Context<Self>) {
        if format {
            self.maybe_format_before_save(cx);
        }
        self.run_save_hooks(cx);
        if !self.save.take_flush_now() {
            return;
        }
        let text = self.core.buffer.text();
        let backups = cx.global::<SessionBackups>().0.clone();
        {
            let mut backups = backups.lock().unwrap();
            if autosave::has_conflict(self.disk_mtime, &self.path) {
                // Never silently clobber external edits: keep the disk copy.
                match backups.force_backup(&self.path) {
                    Ok(_) => eprintln!(
                        "supermd: {} changed on disk; disk version backed up before overwrite",
                        self.path.display()
                    ),
                    Err(err) => eprintln!(
                        "supermd: conflict backup failed for {}: {err}",
                        self.path.display()
                    ),
                }
            } else if let Err(err) = backups.backup_if_needed(&self.path) {
                eprintln!("supermd: backup failed for {}: {err}", self.path.display());
            }
        }
        match autosave::atomic_write(&self.path, &text) {
            Ok(()) => {
                self.disk_mtime = autosave::disk_mtime(&self.path);
                self.save.mark_saved();
            }
            Err(err) => {
                // Stay dirty; the next edit or flush point retries.
                eprintln!("supermd: save failed for {}: {err}", self.path.display());
            }
        }
    }

    fn reveal_cursor(&mut self) {
        let line = self.core.buffer.line_of_byte(self.core.selection.head);
        let item = projection::item_of_line(&self.projection, line);
        self.list_state.scroll_to_reveal_item(item);
    }

    fn move_head(&mut self, target: usize, extend: bool, cx: &mut Context<Self>) {
        if extend {
            self.core.select_to(target);
        } else {
            self.core.set_cursor(target);
        }
        self.core.break_undo_group();
        self.preferred_x = None;
        self.reveal_cursor();
        cx.notify();
    }

    fn insert_str(&mut self, text: &str, cx: &mut Context<Self>) {
        self.core.insert(text, Instant::now());
        self.after_edit(cx);
    }

    /// Apply a plugin command result as one undo group.
    pub fn apply_command_output(
        &mut self,
        out: &crate::extensions::CommandOutput,
        cx: &mut Context<Self>,
    ) {
        use crate::extensions::CommandOutput as O;
        self.core.break_undo_group();
        match out {
            O::ReplaceDocument(s) => {
                self.core.selection = Selection { anchor: 0, head: self.core.buffer.text().len() };
                self.core.insert(s, Instant::now());
            }
            O::ReplaceSelection(s) => {
                self.core.insert(s, Instant::now());
            }
            O::InsertAtCursor(s) => {
                self.core.set_cursor(self.core.selection.head);
                self.core.insert(s, Instant::now());
            }
        }
        self.core.break_undo_group();
        self.after_edit(cx);
    }

    /// Snapshot for building a plugin command-input.
    pub fn command_snapshot(&self) -> (String, std::ops::Range<usize>) {
        (self.core.buffer.text(), self.core.selection.range())
    }

    // ── action handlers ────────────────────────────────────────────────

    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.core.selection.is_cursor() {
            movement::prev_grapheme(&self.core.buffer, self.core.selection.head)
        } else {
            self.core.selection.range().start
        };
        self.move_head(target, false, cx);
    }

    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.core.selection.is_cursor() {
            movement::next_grapheme(&self.core.buffer, self.core.selection.head)
        } else {
            self.core.selection.range().end
        };
        self.move_head(target, false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::prev_grapheme(&self.core.buffer, self.core.selection.head);
        self.move_head(target, true, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::next_grapheme(&self.core.buffer, self.core.selection.head);
        self.move_head(target, true, cx);
    }

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.completion_step(-1, cx) {
            return;
        }
        self.vertical_move(-1, false, cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.completion_step(1, cx) {
            return;
        }
        self.vertical_move(1, false, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical_move(-1, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical_move(1, true, cx);
    }

    fn move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::prev_word(&self.core.buffer, self.core.selection.head);
        self.move_head(target, false, cx);
    }

    fn move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::next_word(&self.core.buffer, self.core.selection.head);
        self.move_head(target, false, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::prev_word(&self.core.buffer, self.core.selection.head);
        self.move_head(target, true, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::next_word(&self.core.buffer, self.core.selection.head);
        self.move_head(target, true, cx);
    }

    fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::line_start(&self.core.buffer, self.core.selection.head);
        self.move_head(target, false, cx);
    }

    fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::line_end(&self.core.buffer, self.core.selection.head);
        self.move_head(target, false, cx);
    }

    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::line_start(&self.core.buffer, self.core.selection.head);
        self.move_head(target, true, cx);
    }

    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let target = movement::line_end(&self.core.buffer, self.core.selection.head);
        self.move_head(target, true, cx);
    }

    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(0, false, cx);
    }

    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.core.buffer.len_bytes(), false, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page_move(-(PAGE_LINES as isize), cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page_move(PAGE_LINES as isize, cx);
    }

    fn page_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let line = self.core.buffer.line_of_byte(self.core.selection.head) as isize;
        let target_line = (line + delta)
            .clamp(0, self.core.buffer.line_count() as isize - 1) as usize;
        let target = self.core.buffer.line_range(target_line).start;
        self.move_head(target, false, cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.core.backspace(Instant::now());
        self.after_edit(cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.core.delete_forward(Instant::now());
        self.after_edit(cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.core.selection.is_cursor() {
            let start = movement::prev_word(&self.core.buffer, self.core.selection.head);
            self.core.selection = Selection { anchor: start, head: self.core.selection.head };
        }
        self.core.backspace(Instant::now());
        self.after_edit(cx);
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_completion(cx) {
            return;
        }
        if self.is_code_mode() {
            self.core.insert_newline_auto_indent(Instant::now());
            self.after_edit(cx);
            return;
        }
        // Enter always starts a fresh undo group, independent of
        // whatever coalescing window the preceding typing left open —
        // whether the cursor is collapsed or Enter is replacing a
        // selection. Without this, typing right up against Enter (no
        // pause, no explicit break) could merge into the same group as
        // the newline — and, once the ordered-list renumber stopped
        // breaking the group on its own side (so one Enter costs one
        // Undo, not two), that merge would reach all the way back into
        // the user's typing on Undo.
        self.core.break_undo_group();
        if self.core.selection.is_cursor() {
            let head = self.core.selection.head;
            // Enter inside a table: tidy the block first, keeping the
            // cursor at its row boundary so the row stays whole.
            let text = self.core.buffer.text();
            if let Some(br) = table_edit::table_block(&text, head) {
                let block = &text[br.clone()];
                let aligned = table_edit::align(block);
                if aligned != block {
                    let mapped =
                        br.start + table_edit::map_offset(block, &aligned, head - br.start);
                    self.core.break_undo_group();
                    self.core.replace_range(br.clone(), &aligned, Instant::now());
                    self.core.set_cursor(mapped);
                }
                self.insert_str("\n", cx);
                return;
            }
            let line_ix = self.core.buffer.line_of_byte(head);
            let line_start = self.core.buffer.line_range(line_ix).start;
            if let Some(item) = lists::list_item(&self.core.buffer.line_text(line_ix)) {
                if item.content_empty {
                    // Enter on a bare marker ends the list instead of
                    // spawning another empty item.
                    self.core.replace_range(
                        line_start..line_start + item.indent + item.marker_len,
                        "",
                        Instant::now(),
                    );
                    self.core.break_undo_group();
                    self.after_edit(cx);
                    return;
                }
                let line = self.core.buffer.line_text(line_ix);
                let indent = &line[..item.indent];
                let ordered = item.next_marker.as_bytes().first().is_some_and(u8::is_ascii_digit);
                self.insert_str(&format!("\n{indent}{}", item.next_marker), cx);
                if ordered {
                    self.renumber_current_list(cx);
                }
                return;
            }
        }
        self.insert_str("\n", cx);
    }

    fn insert_tab(&mut self, _: &InsertTab, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_code_mode() {
            // Selections included: a previous Tab leaves the next cell
            // selected, and Tab again must keep hopping.
            if self.table_tab(false, cx) {
                return;
            }
            if self.core.selection.is_cursor() {
                let line_ix = self.core.buffer.line_of_byte(self.core.selection.head);
                if let Some(item) = lists::list_item(&self.core.buffer.line_text(line_ix)) {
                    self.reindent_line(line_ix, item.indent_step as isize, cx);
                    return;
                }
            }
        }
        self.insert_str("\t", cx);
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_code_mode() {
            return;
        }
        if self.table_tab(true, cx) {
            return;
        }
        if self.core.selection.is_cursor() {
            let line_ix = self.core.buffer.line_of_byte(self.core.selection.head);
            if let Some(item) = lists::list_item(&self.core.buffer.line_text(line_ix)) {
                let step = (item.indent_step.min(item.indent)) as isize;
                if step > 0 {
                    self.reindent_line(line_ix, -step, cx);
                }
            }
        }
    }

    /// Tab inside a table: align the block, then select the next (or
    /// previous) cell; Tab off the last cell appends an empty row.
    /// One undo group per press.
    fn table_tab(&mut self, backward: bool, cx: &mut Context<Self>) -> bool {
        let head = self.core.selection.head;
        let text = self.core.buffer.text();
        let Some(br) = table_edit::table_block(&text, head) else {
            return false;
        };
        let block = text[br.clone()].to_string();
        let Some(pos) = table_edit::cell_at(&block, head - br.start) else {
            return false;
        };
        let aligned = table_edit::align(&block);
        self.core.break_undo_group();
        if aligned != block {
            self.core.replace_range(br.clone(), &aligned, Instant::now());
        }
        let base = br.start;
        let select = |range: Range<usize>| Selection {
            anchor: base + range.start,
            head: base + range.end,
        };
        match table_edit::next_pos(&aligned, pos, backward) {
            Some(p) => {
                if let Some(r) = table_edit::cell_range(&aligned, p) {
                    self.core.selection = select(r);
                }
            }
            // Backward off the first cell: stay in place.
            None if backward => {
                if let Some(r) = table_edit::cell_range(&aligned, pos) {
                    self.core.selection = select(r);
                }
            }
            None => {
                let row = table_edit::new_row(&aligned);
                let insert_at = base + aligned.len();
                self.core
                    .replace_range(insert_at..insert_at, &format!("\n{row}"), Instant::now());
                let combined = format!("{aligned}\n{row}");
                let last = table_edit::rows(&combined).len() - 1;
                if let Some(r) =
                    table_edit::cell_range(&combined, table_edit::CellPos { row: last, cell: 0 })
                {
                    self.core.selection = select(r);
                }
            }
        }
        self.core.break_undo_group();
        self.after_edit(cx);
        true
    }

    /// A click that moves the cursor out of a table tidies the table
    /// it left. Returns the click offset adjusted for the alignment.
    fn tidy_table_on_leave(&mut self, target: usize, cx: &mut Context<Self>) -> usize {
        if self.is_code_mode() {
            return target;
        }
        let head = self.core.selection.head;
        let text = self.core.buffer.text();
        let Some(br) = table_edit::table_block(&text, head) else {
            return target;
        };
        if target >= br.start && target <= br.end {
            return target; // still inside the table
        }
        let block = &text[br.clone()];
        let aligned = table_edit::align(block);
        if aligned == block {
            return target;
        }
        self.core.break_undo_group();
        self.core.replace_range(br.clone(), &aligned, Instant::now());
        self.core.break_undo_group();
        self.after_edit(cx);
        if target > br.end {
            (target as isize + aligned.len() as isize - block.len() as isize) as usize
        } else {
            target
        }
    }

    /// The table block, cell-relative cursor position, and block text
    /// at the cursor, or `None` outside a table.
    fn table_cursor(&self) -> Option<(Range<usize>, String, table_edit::CellPos)> {
        let head = self.core.selection.head;
        let text = self.core.buffer.text();
        let br = table_edit::table_block(&text, head)?;
        let block = text[br.clone()].to_string();
        let pos = table_edit::cell_at(&block, head - br.start)?;
        Some((br, block, pos))
    }

    /// Replace the table block with `new_block` as one undo group, and
    /// place the cursor collapsed at the start of `pos`'s cell.
    fn apply_table_edit(
        &mut self,
        br: Range<usize>,
        new_block: &str,
        pos: table_edit::CellPos,
        cx: &mut Context<Self>,
    ) {
        self.core.break_undo_group();
        self.core.replace_range(br.clone(), new_block, Instant::now());
        if let Some(r) = table_edit::cell_range(new_block, pos) {
            self.core.set_cursor(br.start + r.start);
        }
        self.core.break_undo_group();
        self.after_edit(cx);
    }

    /// Report a command that declined. The editor has no message
    /// surface of its own; the workspace owns the transient strip and
    /// turns this into `show_command_error`.
    fn refuse(&mut self, why: &str, cx: &mut Context<Self>) {
        cx.emit(EditorEvent::CommandError(why.to_string()));
    }

    /// "Put the caret in a table first" -- the four table commands all
    /// share this precondition, and all four used to fail it in silence.
    const NOT_IN_A_TABLE: &'static str = "Put the cursor in a table first";

    fn table_insert_row(&mut self, _: &TableInsertRow, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_format() {
            cx.propagate();
            return;
        }
        let Some((br, block, pos)) = self.table_cursor() else {
            self.refuse(Self::NOT_IN_A_TABLE, cx);
            return;
        };
        // Not `pos.row + 1`: a row asked for from the header lands
        // below the separator, and the cursor has to follow it there.
        let at = table_ops::insert_row_index(&block, pos.row);
        let new_block = table_ops::insert_row(&block, pos.row);
        self.apply_table_edit(br, &new_block, table_edit::CellPos { row: at + 1, cell: 0 }, cx);
    }

    fn table_delete_row(&mut self, _: &TableDeleteRow, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_format() {
            cx.propagate();
            return;
        }
        let Some((br, block, pos)) = self.table_cursor() else {
            self.refuse(Self::NOT_IN_A_TABLE, cx);
            return;
        };
        let Some(new_block) = table_ops::delete_row(&block, pos.row) else {
            self.refuse(
                "The header and the dashed line under it are the table's structure, not rows",
                cx,
            );
            return;
        };
        // Row 1 is the delimiter; a caret there turns the next
        // keystroke into `| z--- | --- |`. Clamp to a body row, and
        // fall back to the header when the body is now empty.
        let rows = table_edit::rows(&new_block).len();
        let row = if rows > 2 { pos.row.clamp(2, rows - 1) } else { 0 };
        self.apply_table_edit(br, &new_block, table_edit::CellPos { row, cell: pos.cell }, cx);
    }

    fn table_insert_column(
        &mut self,
        _: &TableInsertColumn,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_format() {
            cx.propagate();
            return;
        }
        let Some((br, block, pos)) = self.table_cursor() else {
            self.refuse(Self::NOT_IN_A_TABLE, cx);
            return;
        };
        let new_block = table_ops::insert_column(&block, pos.cell);
        self.apply_table_edit(br, &new_block, table_edit::CellPos { row: pos.row, cell: pos.cell + 1 }, cx);
    }

    fn table_delete_column(
        &mut self,
        _: &TableDeleteColumn,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_format() {
            cx.propagate();
            return;
        }
        let Some((br, block, pos)) = self.table_cursor() else {
            self.refuse(Self::NOT_IN_A_TABLE, cx);
            return;
        };
        let Some(new_block) = table_ops::delete_column(&block, pos.cell) else {
            self.refuse("A table needs at least one column", cx);
            return;
        };
        let cell = pos.cell.min(table_edit::rows(&new_block)[pos.row].cells.len().saturating_sub(1));
        self.apply_table_edit(br, &new_block, table_edit::CellPos { row: pos.row, cell }, cx);
    }

    /// The contiguous run of non-blank lines the cursor sits in — the
    /// block Renumber List rewrites, and so the block that decides
    /// whether the right-click menu offers it.
    fn list_run_around_cursor(&self) -> Range<usize> {
        let cur_line = self.core.buffer.line_of_byte(self.core.selection.head);
        let mut start_line = cur_line;
        while start_line > 0 && !self.core.buffer.line_text(start_line - 1).trim().is_empty() {
            start_line -= 1;
        }
        let last_line = self.core.buffer.line_count().saturating_sub(1);
        let mut end_line = cur_line;
        while end_line < last_line && !self.core.buffer.line_text(end_line + 1).trim().is_empty() {
            end_line += 1;
        }
        self.core.buffer.line_range(start_line).start..self.core.buffer.line_range(end_line).end
    }

    /// What a right-click at `offset` knows, for `menus::items_for`.
    /// Each fact is the *command's own* precondition, read at the caret
    /// after the click has placed it: `table_cursor()` for the four
    /// table commands, `renumber_block` for Renumber List (both skip a
    /// fenced code block, so a fence offers neither), and a cached-link
    /// hit for Follow Link. Menu availability and command applicability
    /// are then the same test, not two that can drift apart.
    fn menu_context(&self, offset: usize) -> crate::menus::EditorContext {
        let can_format = self.can_format();
        if !can_format {
            return crate::menus::EditorContext::default();
        }
        let text = self.core.buffer.text();
        crate::menus::EditorContext {
            in_table: self.table_cursor().is_some(),
            in_ordered_list: lists::renumber_block(&text, self.list_run_around_cursor()).is_some(),
            on_link: self.link_at_offset(offset).is_some(),
            can_format,
        }
    }

    /// Renumber the ordered-list run around the cursor (the contiguous
    /// non-blank lines it sits in). A no-op outside an ordered list.
    ///
    /// Deliberately does not call `break_undo_group()` before its own
    /// edit: called right after `newline()`'s Enter-continuation insert,
    /// it must coalesce into that same undo group so one Enter costs one
    /// Undo. A caller that needs this isolated as its own undo step
    /// (`renumber_list` below) breaks the group itself first.
    fn renumber_current_list(&mut self, cx: &mut Context<Self>) {
        let head = self.core.selection.head;
        let cur_line = self.core.buffer.line_of_byte(head);
        let col = head - self.core.buffer.line_range(cur_line).start;
        let block = self.list_run_around_cursor();

        let text = self.core.buffer.text();
        // The list, not the file: a whole-document replacement pushed an
        // undo entry holding two full copies of it on every Enter, and
        // the helper already knows the block's own range.
        let Some(new_block) = lists::renumber_block(&text, block.clone()) else {
            return;
        };
        if new_block == text[block.clone()] {
            return;
        }
        self.core.replace_range(block, &new_block, Instant::now());
        let new_head = (self.core.buffer.line_range(cur_line).start + col).min(self.core.buffer.len_bytes());
        self.core.set_cursor(new_head);
        self.core.break_undo_group();
        self.after_edit(cx);
    }

    fn renumber_list(&mut self, _: &RenumberList, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_format() {
            cx.propagate();
            return;
        }
        self.core.break_undo_group();
        self.renumber_current_list(cx);
    }

    /// Add (or remove, when negative) leading spaces on a line while
    /// keeping the cursor over the same character.
    fn reindent_line(&mut self, line_ix: usize, delta: isize, cx: &mut Context<Self>) {
        let line_start = self.core.buffer.line_range(line_ix).start;
        let saved = self.core.selection;
        if delta >= 0 {
            self.core
                .replace_range(line_start..line_start, &" ".repeat(delta as usize), Instant::now());
        } else {
            self.core
                .replace_range(line_start..line_start + (-delta) as usize, "", Instant::now());
        }
        self.core.break_undo_group();
        let shift = |offset: usize| {
            (offset as isize + delta).max(line_start as isize) as usize
        };
        self.core.selection = Selection { anchor: shift(saved.anchor), head: shift(saved.head) };
        self.after_edit(cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.core.undo() {
            self.after_edit(cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.core.redo() {
            self.after_edit(cx);
        }
    }

    // ── formatting toggles (floating toolbar + ⌘B/⌘I) ──

    /// Formatting only makes sense on an editable markdown buffer.
    fn can_format(&self) -> bool {
        matches!(self.provider, Provider::Markdown) && self.diff.is_none()
    }

    /// Apply a formatting edit as its own undo group and keep the
    /// content selected so toggles can repeat.
    fn apply_fmt(&mut self, edit: formatting::FmtEdit, cx: &mut Context<Self>) {
        self.core.break_undo_group();
        self.core
            .replace_range(edit.range, &edit.replacement, Instant::now());
        self.core.selection = Selection { anchor: edit.select.start, head: edit.select.end };
        self.core.break_undo_group();
        self.after_edit(cx);
    }

    fn inline_fmt(&mut self, marker: &str, cx: &mut Context<Self>) {
        if !self.can_format() {
            cx.propagate();
            return;
        }
        let text = self.core.buffer.text();
        self.apply_fmt(
            formatting::toggle_inline(&text, self.core.selection.range(), marker),
            cx,
        );
    }

    fn toggle_bold(&mut self, _: &ToggleBold, _: &mut Window, cx: &mut Context<Self>) {
        // Cursor-only ⌘B falls through to the app's next binding
        // (sidebar toggle) — bold needs a selection.
        if self.core.selection.is_cursor() {
            cx.propagate();
            return;
        }
        self.inline_fmt("**", cx);
    }

    fn toggle_italic(&mut self, _: &ToggleItalic, _: &mut Window, cx: &mut Context<Self>) {
        self.inline_fmt("*", cx);
    }

    fn toggle_code(&mut self, _: &ToggleCode, _: &mut Window, cx: &mut Context<Self>) {
        self.inline_fmt("`", cx);
    }

    fn toggle_strike(&mut self, _: &ToggleStrike, _: &mut Window, cx: &mut Context<Self>) {
        self.inline_fmt("~~", cx);
    }

    fn insert_link(&mut self, _: &InsertLink, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_format() {
            cx.propagate();
            return;
        }
        let text = self.core.buffer.text();
        self.apply_fmt(formatting::toggle_link(&text, self.core.selection.range()), cx);
    }

    fn cycle_heading(&mut self, _: &CycleHeading, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_format() {
            cx.propagate();
            return;
        }
        let text = self.core.buffer.text();
        self.apply_fmt(formatting::cycle_heading(&text, self.core.selection.range()), cx);
    }

    fn toggle_quote(&mut self, _: &ToggleQuote, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_format() {
            cx.propagate();
            return;
        }
        let text = self.core.buffer.text();
        self.apply_fmt(formatting::toggle_quote(&text, self.core.selection.range()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.core.select_all();
        self.core.break_undo_group();
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.core.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.core.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.core.insert("", Instant::now());
            self.after_edit(cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        // Image on the clipboard: save it beside the document and
        // insert the link. Markdown buffers only.
        if self.can_format() {
            let image = item.entries().iter().find_map(|entry| match entry {
                gpui::ClipboardEntry::Image(image) => Some(image),
                _ => None,
            });
            if let Some(image) = image {
                if self.paste_image(&image.clone(), cx) {
                    return;
                }
            }
        }
        if let Some(text) = item.text() {
            // Paste processors: first Some wins; errors/None pass the
            // original through. Synchronous under the epoch deadline —
            // paste is an explicit action.
            let mut out = text.clone();
            let paste_plugins = crate::extensions::paste_plugins();
            if !paste_plugins.is_empty() {
                if let Some(state) = self.host.clone() {
                    let mut host = state.lock().unwrap();
                    for plugin in &paste_plugins {
                        if let Ok(Some(replaced)) = host.process_paste(plugin, &text) {
                            out = replaced;
                            break;
                        }
                    }
                }
            }
            self.insert_str(&out, cx);
            // Net-capable paste plugins run asynchronously after the
            // paste lands — a network call must never block the UI.
            if !crate::extensions::enrich_plugins().is_empty() {
                let head = self.core.selection.head;
                self.pending_enrich = Some(PendingEnrich {
                    range: head - out.len()..head,
                    snapshot: self.core.buffer.text(),
                    pasted: out.clone(),
                });
                self.start_enrich(cx);
            }
        }
    }

    /// Write a pasted image into `assets/` beside the document and
    /// insert its markdown link. False on any I/O failure — the paste
    /// then falls back to whatever text the clipboard held.
    fn paste_image(&mut self, image: &gpui::Image, cx: &mut Context<Self>) -> bool {
        let Some(dir) = self.path.parent() else {
            return false;
        };
        let assets = dir.join("assets");
        if let Err(err) = std::fs::create_dir_all(&assets) {
            eprintln!("supermd: cannot create {}: {err}", assets.display());
            return false;
        }
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let name = paste_image::pick_name(
            |candidate| assets.join(candidate).exists(),
            &stamp,
            paste_image::extension(image.format()),
        );
        if let Err(err) = std::fs::write(assets.join(&name), image.bytes()) {
            eprintln!("supermd: cannot write pasted image: {err}");
            return false;
        }
        self.insert_str(&paste_image::markdown_link(&name), cx);
        true
    }

    /// Run net-capable paste plugins in the background; first Some
    /// wins. A consent-shaped failure keeps `pending_enrich` so the
    /// workspace can retry after the grant.
    fn start_enrich(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_enrich.as_ref() else {
            return;
        };
        let Some(host) = self.host.clone() else {
            return;
        };
        let text = pending.pasted.clone();
        let task = cx.background_executor().spawn(async move {
            let mut consent: Option<(String, String)> = None;
            for plugin in crate::extensions::enrich_plugins() {
                match host.lock().unwrap().process_paste(&plugin, &text) {
                    Ok(Some(replacement)) => return Ok(Some(replacement)),
                    Ok(None) => {}
                    Err(e) => {
                        if let Some(domain) = e.split("consent required: ").nth(1) {
                            consent = Some((plugin, format!("net:{}", domain.trim())));
                        }
                        // other errors: enrichment is best-effort
                    }
                }
            }
            match consent {
                Some(c) => Err(c),
                None => Ok(None),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| match result {
                Ok(Some(replacement)) => this.apply_enrichment(&replacement, cx),
                Ok(None) => this.pending_enrich = None,
                Err((plugin, cap)) => {
                    cx.emit(EditorEvent::ConsentNeeded { plugin, cap });
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply_enrichment(&mut self, replacement: &str, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_enrich.take() else {
            return;
        };
        let current = self.core.buffer.text();
        if enrich_plan(&current, pending.range.clone(), &pending.snapshot, replacement)
            .is_none()
        {
            return; // document moved; forfeit
        }
        self.core.break_undo_group();
        self.core.selection =
            Selection { anchor: pending.range.start, head: pending.range.end };
        self.core.insert(replacement, Instant::now());
        self.core.break_undo_group();
        self.after_edit(cx);
    }

    /// Called by the workspace after a net grant lands.
    pub fn retry_enrich(&mut self, cx: &mut Context<Self>) {
        self.start_enrich(cx);
    }

    fn save_now(&mut self, _: &SaveNow, _: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
    }

    // ── find in file ───────────────────────────────────────────────────

    fn recompute_matches(&mut self, query: &str) {
        let Some(state) = &mut self.find else {
            return;
        };
        state.matches = find::find_matches(&self.core.buffer.text(), query);
        let head = self.core.selection.head;
        state.active = state
            .matches
            .iter()
            .position(|m| m.start >= head)
            .unwrap_or(0);
    }

    /// Create the find bar if it isn't already open. Shared by
    /// `open_find` and the replace commands, which may need the bar
    /// (and its query field) open before they can show their own.
    fn ensure_find(&mut self, cx: &mut Context<Self>) {
        if self.find.is_some() {
            return;
        }
        let input = cx.new(|cx| crate::input::TextInput::new("Find…", cx));
        let replace_input = cx.new(|cx| crate::input::TextInput::new("Replace…", cx));
        let watch = cx.observe(&input, |this: &mut Editor, input, cx| {
            let query = input.read(cx).content.to_string();
            this.recompute_matches(&query);
            cx.notify();
        });
        self.find = Some(FindState {
            input,
            replace_input,
            replacing: false,
            matches: Vec::new(),
            active: 0,
            _watch: watch,
        });
    }

    fn open_find(&mut self, _: &OpenFind, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_find(cx);
        let state = self.find.as_ref().expect("just ensured");
        window.focus(&state.input.read(cx).focus_handle);
        cx.notify();
    }

    /// Apply a `ReplaceEdit` as its own undo group, same path
    /// `formatting.rs` edits already use: one replace, one undo group,
    /// cursor left at the end of the new text.
    fn apply_replace(&mut self, edit: replace::ReplaceEdit, cx: &mut Context<Self>) {
        self.core.break_undo_group();
        self.core.replace_range(edit.range, &edit.replacement, Instant::now());
        self.core.selection = Selection::cursor(edit.select.end);
        self.core.break_undo_group();
        self.after_edit(cx);
    }

    /// First press reveals the replace field (and focuses it) without
    /// touching the buffer; the field is "shown only once the user
    /// asks for it".
    ///
    /// Returns `true` when the field was just revealed, so the caller
    /// stops there instead of also replacing. This is visibility only
    /// -- it says nothing about which action asked, or what is in the
    /// field -- so callers must not treat a `false` return as
    /// permission to fire; that was the bug (see `replace_next` /
    /// `replace_all`'s own content gate below).
    fn reveal_replace_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.ensure_find(cx);
        let state = self.find.as_mut().expect("just ensured");
        if state.replacing {
            return false;
        }
        state.replacing = true;
        window.focus(&state.replace_input.read(cx).focus_handle);
        cx.notify();
        true
    }

    fn replace_next(&mut self, _: &ReplaceNext, window: &mut Window, cx: &mut Context<Self>) {
        if self.reveal_replace_field(window, cx) {
            return;
        }
        let Some(state) = self.find.as_ref() else {
            return;
        };
        let with = state.replace_input.read(cx).content.to_string();
        // An empty replacement field never fires, no matter how many
        // times a replace shortcut is pressed, and regardless of
        // which one revealed the field. Silently wiping a match
        // because a *different* shortcut was pressed while the field
        // sat empty is exactly the bug this guard exists to prevent;
        // requiring the field to be non-empty removes the ambiguity
        // instead of trying to track "which action asked" across
        // presses. A deliberate "delete every match" is not supported
        // this way -- type a replacement, don't rely on repetition.
        if with.is_empty() {
            return;
        }
        let Some(at) = state.matches.get(state.active).cloned() else {
            return;
        };
        let text = self.core.buffer.text();
        let edit = replace::replace_one(&text, at, &with);
        self.apply_replace(edit, cx);
    }

    fn replace_all(&mut self, _: &ReplaceAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.reveal_replace_field(window, cx) {
            return;
        }
        let Some(state) = self.find.as_ref() else {
            return;
        };
        let query = state.input.read(cx).content.to_string();
        let with = state.replace_input.read(cx).content.to_string();
        // Same guard as `replace_next`, and just as load-bearing here:
        // an empty replacement must never fire Replace All, or one
        // stray press (from either replace shortcut) wipes every
        // match in the document. See `replace_next` for the full
        // rationale.
        if with.is_empty() {
            return;
        }
        let text = self.core.buffer.text();
        let Some(edit) = replace::replace_all(&text, &query, &with) else {
            return;
        };
        self.apply_replace(edit, cx);
    }

    fn cycle_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        let target = {
            let Some(state) = &mut self.find else {
                return;
            };
            if state.matches.is_empty() {
                return;
            }
            let len = state.matches.len();
            state.active = if forward {
                (state.active + 1) % len
            } else {
                (state.active + len - 1) % len
            };
            state.matches[state.active].clone()
        };
        self.core.selection = Selection { anchor: target.start, head: target.end };
        self.core.break_undo_group();
        self.preferred_x = None;
        self.reveal_cursor();
        cx.notify();
    }

    fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.cycle_find(true, cx);
    }

    fn find_prev(&mut self, _: &FindPrev, _: &mut Window, cx: &mut Context<Self>) {
        if !find_prev_should_consume(self.find.is_some()) {
            cx.propagate(); // ⌘⇧G belongs to Graph View when find is closed
            return;
        }
        self.cycle_find(false, cx);
    }

    fn close_find(&mut self, _: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            window.focus(&self.focus_handle);
            cx.notify();
        }
    }

    // ── geometry: vertical movement + mouse ────────────────────────────

    fn vertical_move(&mut self, dir: isize, extend: bool, cx: &mut Context<Self>) {
        let head = self.core.selection.head;
        let line_ix = self.core.buffer.line_of_byte(head);
        let line_count = self.core.buffer.line_count();
        let len_bytes = self.core.buffer.len_bytes();

        let mut preferred_x = self.preferred_x;
        let target = if let Some(entry) = self.layout_cache.get(&line_ix) {
            let lh = entry.line_height;
            let local = display::src_to_disp(&entry.display, head);
            let pos = entry
                .line
                .position_for_index(local, lh)
                .unwrap_or(point(px(0.), px(0.)));
            let x = *preferred_x.get_or_insert(pos.x);
            let target_y = pos.y + lh * (dir as f32);
            let total_h = entry.line.size(lh).height;
            if target_y >= px(0.) && target_y < total_h {
                // Stay within this (wrapped) line.
                let ix = match entry.line.closest_index_for_position(point(x, target_y), lh) {
                    Ok(i) | Err(i) => i,
                };
                display::disp_to_src(&entry.display, ix)
            } else {
                let neighbor = line_ix as isize + dir;
                if neighbor < 0 {
                    0
                } else if neighbor as usize >= line_count {
                    len_bytes
                } else {
                    let neighbor = neighbor as usize;
                    match self.layout_cache.get(&neighbor) {
                        Some(n) => {
                            let nh = n.line_height;
                            let ny = if dir > 0 {
                                px(0.)
                            } else {
                                n.line.size(nh).height - nh
                            };
                            let ix = match n.line.closest_index_for_position(point(x, ny), nh) {
                                Ok(i) | Err(i) => i,
                            };
                            display::disp_to_src(&n.display, ix)
                        }
                        None => {
                            let r = self.core.buffer.line_range(neighbor);
                            if dir > 0 { r.start } else { r.end }
                        }
                    }
                }
            }
        } else {
            // Line not laid out (off-screen): logical line movement.
            let neighbor = (line_ix as isize + dir).clamp(0, line_count as isize - 1) as usize;
            let r = self.core.buffer.line_range(neighbor);
            if dir > 0 { r.start } else { r.end }
        };

        if extend {
            self.core.select_to(target);
        } else {
            self.core.set_cursor(target);
        }
        self.core.break_undo_group();
        self.preferred_x = preferred_x;
        self.reveal_cursor();
        cx.notify();
    }

    fn offset_at_point(&self, position: Point<Pixels>) -> Option<usize> {
        let mut best: Option<(Pixels, usize)> = None;
        for entry in self.layout_cache.values() {
            let height = entry.line.size(entry.line_height).height;
            let local_y = (position.y - entry.origin.y)
                .clamp(px(0.), (height - px(1.)).max(px(0.)));
            let local = point(position.x - entry.origin.x, local_y);
            let ix = match entry.line.closest_index_for_position(local, entry.line_height) {
                Ok(i) | Err(i) => i,
            };
            let offset = display::disp_to_src(&entry.display, ix);
            if position.y >= entry.origin.y && position.y < entry.origin.y + height {
                return Some(offset);
            }
            let dist = if position.y < entry.origin.y {
                entry.origin.y - position.y
            } else {
                position.y - (entry.origin.y + height)
            };
            if best.map_or(true, |(d, _)| dist < d) {
                best = Some((dist, offset));
            }
        }
        best.map(|(_, offset)| offset)
    }

    fn on_line_mouse_down(
        &mut self,
        line_ix: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.diff.is_some() {
            return; // diff view is read-only; merged offsets never touch the buffer
        }
        // A fresh press starts a new interaction: drop the toolbar and
        // any pending reveal.
        self.toolbar_visible = false;
        self.toolbar_task = None;
        // A plain click follows a rendered link; ⌘-click always follows,
        // even a revealed one being edited. The decision is made here,
        // where the press happened, but it is *acted on* in
        // on_root_mouse_up: navigating on mouse-down makes it impossible
        // to start a drag-selection inside link text, which is why every
        // browser (and Obsidian) navigates on release.
        self.pending_link = None;
        if !event.modifiers.shift {
            if let Some(offset) = self.offset_at_point(event.position) {
                // From the cache built in `restyle`, not a fresh scan:
                // this runs on every press, and extracting links costs
                // ~7.65 ms on a 1 MB document.
                let link = self.link_at_offset(offset).cloned();
                let on_link = link.is_some();
                // "Revealed" is span overlap, not a shared line — the
                // same rule display::revealed uses. A link merely on the
                // cursor's line is still rendered and must still follow.
                let sel = self.core.selection.range();
                let revealed = link
                    .as_ref()
                    .is_some_and(|l| l.range.start <= sel.end && sel.start <= l.range.end);
                if self.can_format() && click_follows_link(event.modifiers.platform, on_link, revealed) {
                    // The caret deliberately does not move yet: a click
                    // that navigates should not reveal the link's markers
                    // on its way out, and a drag sets its anchor from
                    // `offset` when the first move arrives.
                    self.pending_link =
                        Some(PendingLink { offset, link: link.expect("on_link") });
                    self.dragging = true;
                    self.core.break_undo_group();
                    window.focus(&self.focus_handle);
                    return;
                }
            }
        }
        // Checkbox toggle: a plain click on a ✓/○ glyph flips the source
        // without moving the cursor into the line.
        if !event.modifiers.shift {
            let hit = self.layout_cache.get(&line_ix).and_then(|entry| {
                let height = entry.line.size(entry.line_height).height;
                let local = point(
                    event.position.x - entry.origin.x,
                    (event.position.y - entry.origin.y)
                        .clamp(px(0.), (height - px(1.)).max(px(0.))),
                );
                let ix = match entry.line.closest_index_for_position(local, entry.line_height)
                {
                    Ok(i) | Err(i) => i,
                };
                entry
                    .display
                    .segs
                    .iter()
                    .find(|seg| {
                        seg.toggle.is_some()
                            && seg.disp.start <= ix
                            && ix < seg.disp.end.max(seg.disp.start + 1)
                    })
                    .map(|seg| (seg.src.clone(), seg.toggle.unwrap()))
            });
            if let Some((src, checked)) = hit {
                let saved = self.core.selection;
                self.core.replace_range(
                    src,
                    if checked { "[ ]" } else { "[x]" },
                    Instant::now(),
                );
                self.core.break_undo_group();
                self.core.selection = saved;
                self.after_edit(cx);
                return;
            }
        }

        let offset = self
            .offset_at_point(event.position)
            .unwrap_or_else(|| self.core.buffer.line_range(line_ix).start);
        let offset = self.tidy_table_on_leave(offset, cx);
        if event.modifiers.shift {
            self.core.select_to(offset);
        } else {
            self.core.set_cursor(offset);
        }
        self.dragging = true;
        self.core.break_undo_group();
        self.preferred_x = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// A right press: place the caret (or keep the selection), then ask
    /// the workspace to raise the context menu here.
    ///
    /// Deliberately *not* a call into `on_line_mouse_down`. That path
    /// arms `pending_link`, which the next left release follows — a
    /// right-click must never navigate, and must not leave a primed
    /// link behind for a later click to trip over either, so it clears
    /// one rather than setting one. It also does not start a drag: the
    /// root's `on_mouse_up` only listens for the left button, so a
    /// `dragging` flag set here would stay set and turn every later
    /// pointer move into a selection drag.
    fn on_line_right_mouse_down(
        &mut self,
        line_ix: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.diff.is_some() {
            return; // read-only, same as the left path
        }
        self.pending_link = None;
        self.dragging = false;
        self.toolbar_visible = false;
        self.toolbar_task = None;

        let clicked = self
            .offset_at_point(event.position)
            .unwrap_or_else(|| self.core.buffer.line_range(line_ix).start);
        // A right-click *inside* the selection keeps it. Collapsing the
        // caret here is the classic way this feature breaks: the user
        // selects a phrase, right-clicks it, picks Bold — and the
        // selection the command was for is gone.
        let sel = self.core.selection.range();
        let caret = if sel.start < sel.end && sel.start <= clicked && clicked < sel.end {
            self.core.selection.head
        } else {
            let offset = self.tidy_table_on_leave(clicked, cx);
            self.core.set_cursor(offset);
            self.core.break_undo_group();
            self.preferred_x = None;
            offset
        };
        // Redundant on paper — gpui's `track_focus` focuses any div it
        // is on when a mouse button goes down over it, whichever button
        // — and so untestable in isolation; kept because the left path
        // states it too and this one must not quietly depend on that.
        window.focus(&self.focus_handle);

        // An empty menu is worse than none: a code file or the diff
        // view takes none of these commands, so no overlay opens.
        let ctx = self.menu_context(caret);
        if !crate::menus::items_for(crate::menus::Surface::Editor, ctx).is_empty() {
            cx.emit(EditorEvent::ContextMenu { position: event.position, ctx });
        }
        cx.notify();
    }

    fn on_root_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.scrollbar_dragging {
            self.scrollbar_scrub(event.position, cx);
            return;
        }
        self.hover_moved(event.position, window, cx);
        if self.dragging {
            if let Some(offset) = self.offset_at_point(event.position) {
                // A press on a link left the caret alone. The moment the
                // pointer actually moves off that offset it is a drag,
                // not a click: drop the navigation and plant the anchor
                // where the press landed.
                if let Some(pending) = &self.pending_link {
                    if offset == pending.offset {
                        return; // jitter within one offset is still a click
                    }
                    let anchor = pending.offset;
                    self.pending_link = None;
                    self.core.set_cursor(anchor);
                }
                self.core.select_to(offset);
                cx.notify();
            }
        }
    }

    fn on_root_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_mouse_up(true, cx);
    }

    /// The release landed outside the editor's own bounds. Browsers
    /// treat that as a cancelled click, not a completed one: a
    /// pending link must be dropped, not followed.
    fn on_root_mouse_up_out(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_mouse_up(false, cx);
    }

    fn finish_mouse_up(&mut self, in_bounds: bool, cx: &mut Context<Self>) {
        let selection_drag_ended = self.dragging && !self.scrollbar_dragging;
        self.dragging = false;
        // The press landed on a followable link and nothing dragged it
        // away: an in-bounds release is the click, so navigate now.
        if let Some(pending) = self.pending_link.take() {
            if !in_bounds {
                // Cancelled: the press still belongs to nothing, and
                // no caret placement follows a click that never
                // completed.
                cx.notify();
                return;
            }
            if !self.open_link(&pending.link, cx) {
                // Refused (an escaping wiki target, say) — the click
                // still belongs to the document, so place the caret.
                self.core.set_cursor(pending.offset);
                self.preferred_x = None;
                cx.notify();
            }
            return;
        }
        if self.scrollbar_dragging {
            self.scrollbar_dragging = false;
            self.list_state.scrollbar_drag_ended();
            cx.notify();
        }
        if selection_drag_ended && !self.core.selection.is_cursor() && self.can_format() {
            // Let the selection settle before the toolbar pops in.
            self.toolbar_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(150))
                    .await;
                this.update(cx, |editor, cx| {
                    editor.toolbar_visible = true;
                    cx.notify();
                })
                .ok();
            }));
        }
    }

    /// The pointer moved. Starts, keeps, or cancels a link's dwell.
    ///
    /// Cheap by construction: the hit test is a binary search over the
    /// link cache, and the common case — moving over prose, or moving
    /// within the same link — does no work beyond that and starts no
    /// task.
    fn hover_moved(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_format() || self.dragging {
            self.hover_left(cx);
            return;
        }
        // The pointer is over the document, so it is not inside the
        // popover. Clearing here is what stops `hover_held` latching:
        // if the popover disappears while hovered (a tab switch removes
        // its hitbox, so no `on_hover(false)` ever arrives) the flag
        // would otherwise stay set, and both opening a new preview and
        // closing the stale one early-return on it forever.
        self.hover_held = false;
        let link = self
            .offset_at_point(position)
            .and_then(|offset| self.link_at_offset(offset))
            .cloned();
        let Some(link) = link else {
            self.hover_left(cx);
            return;
        };
        // Still the same link: keep its dwell running rather than
        // restarting it on every pixel, so sliding within a link opens
        // the popover on time.
        if self.hover_link.as_ref().is_some_and(|l| l.range == link.range) {
            // Back on the link it belongs to: call off any pending close.
            self.hover_close_task = None;
            return;
        }
        // Moving to a different link does NOT tear the popover down.
        // Reaching a popover means crossing whatever lies between, and
        // in a list of links that is another link — closing on the
        // first one crossed made the popover unreachable. The showing
        // preview stays until the new link's dwell elapses and replaces
        // it, so travel is free but a genuine pause still swaps.
        //
        // `hover_preview` is what the popover renders from, so it must
        // survive here; only `hover_at` moves with the new dwell.
        self.hover_close_task = None;
        self.hover_link = Some(link);
        self.hover_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(crate::preview::DWELL).await;
            this.update(cx, |editor, cx| {
                editor.open_hover_preview(cx);
            })
            .ok();
        }));
    }

    /// The pointer left the link. The popover does not close at once:
    /// reaching its button means moving off the link, and closing on
    /// that movement made the button unclickable. It survives a short
    /// grace period, and indefinitely while the pointer is inside it.
    fn hover_left(&mut self, cx: &mut Context<Self>) {
        if self.hover_link.is_none() && self.hover_preview.is_none() {
            return;
        }
        // Nothing shown yet — just a dwell in progress. Drop it now;
        // there is nothing on screen to walk to.
        if self.hover_preview.is_none() {
            self.hover_link = None;
            self.hover_task = None;
            self.hover_at = None;
            cx.notify();
            return;
        }
        if self.hover_close_task.is_some() || self.hover_held {
            return;
        }
        self.hover_close_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(crate::preview::CLOSE_GRACE).await;
            this.update(cx, |editor, cx| {
                if !editor.hover_held {
                    editor.close_hover(cx);
                }
                editor.hover_close_task = None;
            })
            .ok();
        }));
    }

    /// Drop the popover and everything behind it.
    fn close_hover(&mut self, cx: &mut Context<Self>) {
        self.hover_link = None;
        self.hover_task = None;
        self.hover_at = None;
        self.hover_preview = None;
        self.hover_held = false;
        cx.notify();
    }

    /// The dwell elapsed: work out what to show, and if the site is
    /// one the user has enabled, go and read its title.
    /// The dwell elapsed. Leaves a popover the pointer is inside alone.
    fn open_hover_preview(&mut self, cx: &mut Context<Self>) {
        self.open_hover_preview_inner(false, cx);
    }

    fn open_hover_preview_inner(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(link) = self.hover_link.clone() else {
            return;
        };
        // The pointer is inside the popover, not on the link any more:
        // swapping its contents out from under the cursor would move
        // the button the user is reaching for. `force` is set when the
        // user has just acted on the popover (granting a site), where
        // refreshing it is the whole point.
        if self.hover_held && !force {
            return;
        }
        self.hover_at = self.link_anchor(link.range.start).or(self.hover_at);
        let preview = self.preview_for(&link, cx);
        // A granted domain we have not read yet: fetch once, in the
        // background. The task lives in `hover_task`, so moving the
        // pointer away drops it and the answer is discarded.
        if let crate::preview::Preview::External { url, consent, fetched: None, .. } = &preview {
            if *consent == crate::preview::Consent::Granted {
                let url = url.clone();
                if let Some(state) = cx.try_global::<crate::preview::PreviewState>().cloned() {
                    self.hover_task = Some(cx.spawn(async move |this, cx| {
                        let fetched = cx
                            .background_executor()
                            .spawn({
                                let url = url.clone();
                                let state = state.clone();
                                async move {
                                    match state.cached(&url) {
                                        Some(m) => Some(m),
                                        None => (state.fetcher)(&url).ok().and_then(|bytes| {
                                            let meta = crate::preview::parse_meta(
                                                &String::from_utf8_lossy(&bytes),
                                            )?;
                                            state.remember(&url, meta.clone());
                                            Some(meta)
                                        }),
                                    }
                                }
                            })
                            .await;
                        this.update(cx, |editor, cx| {
                            // Only if the pointer is still on the link
                            // this answer belongs to.
                            let still_here = matches!(
                                &editor.hover_preview,
                                Some(crate::preview::Preview::External { url: u, .. }) if *u == url
                            );
                            if still_here {
                                if let Some(crate::preview::Preview::External { fetched: f, .. }) =
                                    editor.hover_preview.as_mut()
                                {
                                    *f = fetched;
                                    cx.notify();
                                }
                            }
                        })
                        .ok();
                    }));
                }
            }
        }
        self.hover_preview = Some(preview);
        cx.notify();
    }

    /// Enable previews for the site the popover is showing, and read it
    /// straight away so the click has a visible result.
    fn enable_previews_for_hovered_site(&mut self, cx: &mut Context<Self>) {
        let Some(crate::preview::Preview::External { domain, .. }) = self.hover_preview.clone()
        else {
            return;
        };
        if domain.is_empty() {
            return;
        }
        let dir = crate::settings::config_dir();
        let mut settings = crate::settings::load(&dir);
        let grants = settings.plugin_grants.entry("supermd".to_string()).or_default();
        *grants = crate::preview::grant_domain(&domain, grants);
        if let Err(err) = crate::settings::save(&dir, &settings) {
            eprintln!("supermd: cannot record the preview grant: {err}");
            return;
        }
        // The grant just landed on disk, but `preview_for` never reads
        // disk itself: without this the cache `PreviewState` holds
        // stays exactly as stale as it was before the click, and the
        // popover below would still show `Ungranted`.
        if let Some(state) = cx.try_global::<crate::preview::PreviewState>() {
            state.refresh_grants();
        }
        // Forced: clicking the button requires the pointer inside the
        // popover, which is exactly the state the dwell path refuses to
        // touch. Without this the grant was written to settings and
        // nothing happened on screen — no refresh, no fetch.
        self.open_hover_preview_inner(true, cx);
    }

    /// What a link's popover should contain. Pure decisions live in
    /// `crate::preview`; this supplies the filesystem and the index.
    fn preview_for(
        &self,
        link: &crate::knowledge::RawLink,
        cx: &App,
    ) -> crate::preview::Preview {
        use crate::knowledge::LinkTarget;
        use crate::preview::{self as pv, Preview};

        match crate::knowledge::classify(link) {
            LinkTarget::External(url) => {
                // Cached on `PreviewState`, not read from disk here:
                // this runs on every dwell, and `settings::load` is a
                // file read plus a TOML parse the UI thread should
                // never do that often.
                let grants = cx
                    .try_global::<crate::preview::PreviewState>()
                    .map(|s| s.grants())
                    .unwrap_or_default();
                // The *visible* text, not `context` — that is the
                // whole line, which never looks like a hostname and so
                // would silently disable the mismatch warning.
                let text = self.core.buffer.text();
                let shown = pv::display_text(
                    text.get(link.range.clone()).unwrap_or_default(),
                );
                pv::external_preview(&url, shown, &grants)
            }
            LinkTarget::Anchor(a) => {
                let text = self.core.buffer.text();
                match crate::knowledge::heading_offset(&text, &a) {
                    Some(off) => Preview::Anchor {
                        heading: text[off..].lines().next().unwrap_or("").trim_start_matches('#')
                            .trim().to_string(),
                        excerpt: pv::excerpt(&text[off..], 6),
                    },
                    None => Preview::Missing { name: format!("#{a}") },
                }
            }
            LinkTarget::Wiki(name) | LinkTarget::Relative(name) => {
                let resolved = self
                    .knowledge
                    .as_ref()
                    .and_then(|k| k.lock().unwrap().resolve(&self.path, link));
                let Some(path) = resolved else {
                    return Preview::Missing { name };
                };
                if crate::files::is_image_path(&path) {
                    return Preview::Image { path };
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    return Preview::Missing { name };
                };
                let is_md = matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("md" | "markdown" | "mdown" | "mdx")
                );
                if is_md {
                    Preview::Note {
                        title: pv::title_of(&text, &path),
                        excerpt: pv::excerpt(&text, 8),
                    }
                } else {
                    Preview::Code {
                        language: crate::reader::language_for_path(&path),
                        excerpt: pv::excerpt(&text, 10),
                    }
                }
            }
        }
    }

    /// Whether the floating format toolbar should paint right now.
    fn toolbar_showing(&self) -> bool {
        self.toolbar_visible && self.can_format() && !self.core.selection.is_cursor()
    }

    /// Window point just above the selection start, if that line is
    /// currently laid out.
    /// Where a link's popover should sit: just under the start of the
    /// link itself, from the laid-out glyphs.
    ///
    /// Not the pointer position. Anchoring to the pointer put the
    /// popover wherever the pointer happened to enter the link, which
    /// in a tight list landed it below the *next* item — so reaching it
    /// meant crossing another link.
    fn link_anchor(&self, byte: usize) -> Option<Point<Pixels>> {
        let line_ix = self.core.buffer.line_of_byte(byte);
        let entry = self.layout_cache.get(&line_ix)?;
        let disp = display::src_to_disp(&entry.display, byte);
        let pos = entry.line.position_for_index(disp, entry.line_height)?;
        Some(point(
            entry.origin.x + pos.x,
            entry.origin.y + pos.y + entry.line_height + px(4.),
        ))
    }

    fn toolbar_anchor(&self) -> Option<Point<Pixels>> {
        let start = self.core.selection.range().start;
        let line_ix = self.core.buffer.line_of_byte(start);
        let entry = self.layout_cache.get(&line_ix)?;
        let disp = display::src_to_disp(&entry.display, start);
        let pos = entry.line.position_for_index(disp, entry.line_height)?;
        Some(point(entry.origin.x + pos.x, entry.origin.y + pos.y - px(6.)))
    }

    /// Map a window-space y position on the scrollbar track to a scroll
    /// offset and apply it.
    fn scrollbar_scrub(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let viewport = self.list_state.viewport_bounds();
        let max = self.list_state.max_offset_for_scrollbar().height;
        if max <= px(0.) {
            return;
        }
        let vh = viewport.size.height;
        let total = vh + max;
        let thumb_h = (vh * (vh / total)).max(px(30.)).min(vh);
        let denom = vh - thumb_h;
        let y_rel = position.y - viewport.origin.y - thumb_h * 0.5;
        let frac = if denom > px(0.) { (y_rel / denom).clamp(0., 1.) } else { 0. };
        self.list_state
            .set_offset_from_scrollbar(point(px(0.), -(max * frac)));
        cx.notify();
    }

    // ── UTF-16 helpers (IME protocol speaks UTF-16 offsets) ────────────

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let text = self.core.buffer.text();
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in text.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let text = self.core.buffer.text();
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in text.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    // ── styling ────────────────────────────────────────────────────────

    /// Code and plain files render mono, full width, with a gutter.
    pub fn is_code_mode(&self) -> bool {
        matches!(self.provider, Provider::Code(_) | Provider::Plain)
    }

    /// (font size, base weight, family, line height multiple) for a line.
    fn line_typography(&self, ix: usize, t: &Theme) -> (f32, FontWeight, SharedString, f32) {
        if self.is_code_mode() {
            return (t.code_size, FontWeight::NORMAL, t.mono_family.clone(), 1.55);
        }
        match self.view_line_kinds().get(ix) {
            Some(LineKind::Heading(n)) => {
                let weight = if *n <= 2 { FontWeight::BOLD } else { FontWeight::SEMIBOLD };
                (t.heading_size(*n), weight, t.body_family.clone(), 1.35)
            }
            Some(LineKind::Code) => (t.code_size, FontWeight::NORMAL, t.mono_family.clone(), 1.55),
            Some(LineKind::FrontMatter) => {
                let s = crate::view::frontmatter_style(t);
                (s.size, FontWeight::NORMAL, s.family, 1.55)
            }
            Some(LineKind::Body) | None => {
                (t.body_size, FontWeight::NORMAL, t.body_family.clone(), 1.65)
            }
        }
    }

    fn syntax_color(capture: u8, t: &Theme) -> Option<Hsla> {
        let name = crate::highlight::CAPTURE_NAMES.get(capture as usize)?;
        let root = name.split('.').next().unwrap_or(name);
        let s = &t.syntax;
        Some(match root {
            "attribute" => s.attribute,
            "comment" => s.comment,
            "constant" | "number" => s.constant,
            "constructor" | "type" => s.kind,
            "function" => s.function,
            "keyword" => s.keyword,
            "operator" | "punctuation" => s.operator,
            "property" => s.property,
            "string" => s.string,
            "tag" => s.tag,
            "namespace" => s.kind,
            "label" => s.constant,
            "special" => s.string,
            _ => return None,
        })
    }

    /// Source-space style attributes for one line, one entry per byte.
    fn line_attrs(&self, ix: usize, t: &Theme) -> (String, Vec<Attr>) {
        // (decoration overlay applied below, after style spans)
        let range = self.view_buffer().line_range(ix);
        let text = self.view_buffer().line_text(ix);
        let (_, base_weight, family, _) = self.line_typography(ix, t);

        let default_attr = Attr {
            color: t.fg,
            weight: base_weight,
            italic: false,
            family: family.clone(),
            bg: None,
            underline: false,
            strike: false,
        };
        let mut attrs: Vec<Attr> = vec![default_attr; text.len()];
        for span in self.view_spans() {
            let start = span.range.start.max(range.start);
            let end = span.range.end.min(range.end);
            if start >= end {
                continue;
            }
            for a in &mut attrs[start - range.start..end - range.start] {
                match &span.kind {
                    StyleKind::Heading(_) => a.color = t.fg_strong,
                    StyleKind::Strong => a.weight = FontWeight::BOLD,
                    StyleKind::Emphasis => a.italic = true,
                    StyleKind::Strikethrough => a.strike = true,
                    StyleKind::InlineCode => {
                        a.family = t.mono_family.clone();
                        a.bg = Some(t.code_bg);
                        a.color = t.code_fg;
                    }
                    StyleKind::Link => {
                        a.color = t.link;
                        a.underline = true;
                    }
                    StyleKind::ListMarker | StyleKind::QuoteMarker => a.color = t.accent,
                    StyleKind::TaskMarker(checked) => {
                        a.color = if *checked { t.accent } else { t.fg_muted };
                    }
                    StyleKind::Rule => a.color = t.fg_muted,
                    StyleKind::FenceContent => a.color = t.code_fg,
                    StyleKind::FenceDelimiter => {
                        a.color = Hsla { a: 0.55, ..t.fg_muted };
                    }
                    StyleKind::FrontMatter => a.color = crate::view::frontmatter_style(t).ink,
                    StyleKind::InlineReplace(_) => {
                        // Rendering handled by the display transform;
                        // source text (when revealed) keeps base style.
                    }
                    StyleKind::Syntax(capture) => {
                        if let Some(c) = Self::syntax_color(*capture, t) {
                            a.color = c;
                        }
                        if crate::highlight::CAPTURE_NAMES
                            .get(*capture as usize)
                            .is_some_and(|n| n.starts_with("comment"))
                        {
                            a.italic = true;
                        }
                    }
                }
            }
        }

        // Plugin decoration overlays (prose lines only).
        if !self.is_code_mode()
            && !matches!(
                self.view_line_kinds().get(ix),
                Some(LineKind::Code | LineKind::FrontMatter)
            )
        {
            for (deco, color, is_bg) in crate::extensions::with_decoration_table(|table| {
                decoration_overlay(&text, &range, table, t)
            }) {
                let start = deco.start.max(range.start) - range.start;
                let end = deco.end.min(range.end) - range.start;
                for a in &mut attrs[start..end] {
                    if is_bg {
                        a.bg = Some(color);
                    } else {
                        a.color = color;
                    }
                }
            }
        }

        // Diff washes paint over the style spans; deleted text also
        // strikes through. (Find/IME overlays below use buffer offsets,
        // so they are skipped in diff mode.)
        if let Some(d) = &self.diff {
            for c in &d.changes {
                let start = c.range.start.max(range.start);
                let end = c.range.end.min(range.end);
                if start >= end {
                    continue;
                }
                for a in &mut attrs[start - range.start..end - range.start] {
                    match c.kind {
                        crate::diff::ChangeKind::Added => {
                            a.bg = Some(t.diff_added_bg);
                            a.color = t.diff_added_fg;
                        }
                        crate::diff::ChangeKind::Deleted => {
                            a.bg = Some(t.diff_deleted_bg);
                            a.color = t.diff_deleted_fg;
                            a.strike = true;
                        }
                    }
                }
            }
            return (text, attrs);
        }

        // Find matches get a background highlight; the active one stronger.
        if let Some(state) = &self.find {
            for (mi, m) in state.matches.iter().enumerate() {
                let start = m.start.max(range.start);
                let end = m.end.min(range.end);
                if start < end {
                    let bg = if mi == state.active { t.find_active_bg } else { t.find_match_bg };
                    for a in &mut attrs[start - range.start..end - range.start] {
                        a.bg = Some(bg);
                    }
                }
            }
        }

        // IME composition text renders underlined.
        if let Some(marked) = &self.marked_range {
            let start = marked.start.max(range.start);
            let end = marked.end.min(range.end);
            if start < end {
                for a in &mut attrs[start - range.start..end - range.start] {
                    a.underline = true;
                }
            }
        }

        (text, attrs)
    }

    /// The colour of the divider drawn over a hidden thematic break.
    /// In diff mode a change covering the break is painted into it:
    /// the per-character wash lands on hyphens that are not drawn.
    fn rule_color(&self, ix: usize, t: &Theme) -> Hsla {
        let (_, plain) = crate::view::rule_style(t);
        let Some(d) = &self.diff else {
            return plain;
        };
        let range = self.view_buffer().line_range(ix);
        d.changes
            .iter()
            .find(|c| c.range.start < range.end && range.start < c.range.end)
            .map_or(plain, |c| match c.kind {
                crate::diff::ChangeKind::Added => t.diff_added_fg,
                crate::diff::ChangeKind::Deleted => t.diff_deleted_fg,
            })
    }

    /// Display text, styled runs, and the source↔display map for a line.
    fn display_for_line(
        &self,
        ix: usize,
        t: &Theme,
    ) -> (SharedString, Vec<TextRun>, display::DisplayLine) {
        let range = self.view_buffer().line_range(ix);
        let (text, attrs) = self.line_attrs(ix, t);
        // In diff mode nothing is "touched", so all syntax markers stay
        // hidden — clean styled prose with the washes woven in.
        let selection = if self.diff.is_some() {
            usize::MAX..usize::MAX
        } else {
            self.core.selection.range()
        };
        let dl = display::display_line(&text, range.start, self.view_spans(), selection);

        let mut disp_attrs: Vec<Attr> = Vec::with_capacity(dl.text.len());
        for seg in &dl.segs {
            match seg.kind {
                display::SegKind::Verbatim => {
                    let s = seg.src.start - range.start;
                    let e = seg.src.end - range.start;
                    disp_attrs.extend_from_slice(&attrs[s..e]);
                }
                display::SegKind::Replacement => {
                    if let Some(attr) = attrs.get(seg.src.start - range.start) {
                        for _ in 0..seg.disp.len() {
                            disp_attrs.push(attr.clone());
                        }
                    }
                }
                display::SegKind::Hidden(_) => {}
            }
        }

        (
            SharedString::from(dl.text.clone()),
            runs_from_attrs(&disp_attrs, t),
            dl,
        )
    }
}

#[derive(Clone, PartialEq)]
struct Attr {
    color: Hsla,
    weight: FontWeight,
    italic: bool,
    family: SharedString,
    bg: Option<Hsla>,
    underline: bool,
    strike: bool,
}


/// Decoration overlay for one line: absolute byte ranges + color, from
/// host-compiled plugin decoration rules. Pure — table injected.
fn decoration_overlay(
    line_text: &str,
    line_range: &Range<usize>,
    table: &[crate::extensions::CompiledDecoration],
    t: &Theme,
) -> Vec<(Range<usize>, Hsla, bool)> {
    // (range, color, is_background)
    let mut out = Vec::new();
    for rule in table {
        let (color, is_bg) = match rule.style.as_str() {
            "accent" => (t.accent, false),
            "muted" => (t.fg_muted, false),
            "strong" => (t.fg_strong, false),
            "highlight" => (t.find_match_bg, true),
            _ => continue,
        };
        for m in rule.regex.find_iter(line_text) {
            out.push((
                line_range.start + m.start()..line_range.start + m.end(),
                color,
                is_bg,
            ));
        }
    }
    out
}

#[cfg(test)]
mod hook_tests {
    use super::chain_save_hooks;

    #[test]
    fn hooks_chain_in_order_and_skip_failures() {
        let plugins = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let out = chain_save_hooks("x".into(), "f.md", &plugins, |p, _, doc| match p {
            "a" => Ok(Some(format!("{doc}a"))),
            "b" => Err("boom".into()),
            _ => Ok(Some(format!("{doc}c"))),
        });
        assert_eq!(out, "xac");
        let none = chain_save_hooks("x".into(), "f.md", &plugins, |_, _, _| Ok(None));
        assert_eq!(none, "x");
    }
}

#[cfg(test)]
mod enrich_tests {
    use super::enrich_plan;

    #[test]
    fn enrichment_applies_only_when_snapshot_matches() {
        assert_eq!(
            enrich_plan("abc URL def", 4..7, "abc URL def", "[T](URL)"),
            Some(("abc [T](URL) def".to_string(), 4..12))
        );
        // document moved since the paste → discard
        assert_eq!(enrich_plan("abc URL defX", 4..7, "abc URL def", "[T](URL)"), None);
    }
}

#[cfg(test)]
mod decoration_tests {
    use super::*;

    #[test]
    fn decorations_match_and_map_styles() {
        let table = vec![crate::extensions::CompiledDecoration {
            regex: regex::Regex::new(r"\b(TODO|FIXME)\b").unwrap(),
            style: "accent".into(),
        }];
        let t = Theme::light();
        let hits = decoration_overlay("a TODO here", &(100..111), &table, &t);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 102..106);
        assert_eq!(hits[0].1, t.accent);
        assert!(!hits[0].2);
    }

    #[test]
    fn unknown_style_skipped_and_highlight_is_bg() {
        let table = vec![
            crate::extensions::CompiledDecoration {
                regex: regex::Regex::new("x").unwrap(),
                style: "sparkle".into(),
            },
            crate::extensions::CompiledDecoration {
                regex: regex::Regex::new("y").unwrap(),
                style: "highlight".into(),
            },
        ];
        let t = Theme::light();
        let hits = decoration_overlay("xy", &(0..2), &table, &t);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].2, "highlight is a background style");
    }
}

/// Compress per-byte attributes into TextRuns.
fn runs_from_attrs(attrs: &[Attr], t: &Theme) -> Vec<TextRun> {
    let font_of = |a: &Attr| Font {
        family: a.family.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: a.weight,
        style: if a.italic { FontStyle::Italic } else { FontStyle::Normal },
    };

    let mut runs: Vec<TextRun> = Vec::new();
    let mut i = 0;
    while i < attrs.len() {
        let mut j = i + 1;
        while j < attrs.len() && attrs[j] == attrs[i] {
            j += 1;
        }
        let a = &attrs[i];
        runs.push(TextRun {
            len: j - i,
            font: font_of(a),
            color: a.color,
            background_color: a.bg,
            underline: a.underline.then_some(UnderlineStyle {
                thickness: px(1.),
                color: Some(a.color),
                wavy: false,
            }),
            strikethrough: a.strike.then_some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(t.fg_muted),
            }),
        });
        i = j;
    }
    runs
}

// ── IME / text input protocol ──────────────────────────────────────────

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.core.buffer.slice(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.core.selection.range()),
            reversed: self.core.selection.head < self.core.selection.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Typing a marker over a selection wraps it instead of
        // replacing; repeated presses stack (`*` then `*` → `**`).
        if range_utf16.is_none()
            && self.marked_range.is_none()
            && !self.core.selection.is_cursor()
            && self.can_format()
            && matches!(new_text, "*" | "_" | "`" | "~")
        {
            let range = self.core.selection.range();
            let content = self.core.selected_text();
            self.core.break_undo_group();
            self.core.replace_range(
                range.clone(),
                &format!("{new_text}{content}{new_text}"),
                Instant::now(),
            );
            self.core.selection = Selection {
                anchor: range.start + new_text.len(),
                head: range.start + new_text.len() + content.len(),
            };
            self.core.break_undo_group();
            self.after_edit(cx);
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.core.selection.range());
        self.core.selection = Selection { anchor: range.start, head: range.end };
        self.core.insert(new_text, Instant::now());
        self.marked_range = None;
        self.after_edit(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.core.selection.range());
        self.core.selection = Selection { anchor: range.start, head: range.end };
        self.core.insert(new_text, Instant::now());
        self.marked_range = if new_text.is_empty() {
            None
        } else {
            Some(range.start..range.start + new_text.len())
        };
        if let Some(sel) = new_selected_range_utf16.as_ref() {
            let sel = self.range_from_utf16(sel);
            self.core.selection = Selection {
                anchor: range.start + sel.start,
                head: range.start + sel.end,
            };
        }
        self.after_edit(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let line_ix = self.core.buffer.line_of_byte(range.start);
        let entry = self.layout_cache.get(&line_ix)?;
        let local = display::src_to_disp(&entry.display, range.start);
        let pos = entry.line.position_for_index(local, entry.line_height)?;
        Some(Bounds::new(
            point(entry.origin.x + pos.x, entry.origin.y + pos.y),
            size(px(2.), entry.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point_: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_at_point(point_)?;
        Some(self.offset_to_utf16(offset))
    }
}

// ── per-line element: shaping, selection/caret painting, hit geometry ──

struct LineElement {
    editor: Entity<Editor>,
    line_ix: usize,
    range: Range<usize>,
    text: SharedString,
    runs: Vec<TextRun>,
    display: display::DisplayLine,
    font_size: Pixels,
    line_height: Pixels,
    caret_color: Hsla,
    selection_color: Hsla,
}

struct LinePrepaint {
    line: Option<WrappedLine>,
    selection_quads: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for LineElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for LineElement {
    type RequestLayoutState = ();
    type PrepaintState = LinePrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let text = self.text.clone();
        let runs = self.runs.clone();
        let font_size = self.font_size;
        let line_height = self.line_height;
        let layout_id = window.request_measured_layout(
            style,
            move |_known, available, window, _cx| {
                let wrap_width = match available.width {
                    AvailableSpace::Definite(w) => Some(w),
                    _ => None,
                };
                let Ok(lines) =
                    window
                        .text_system()
                        .shape_text(text.clone(), font_size, &runs, wrap_width, None)
                else {
                    return size(wrap_width.unwrap_or(px(0.)), line_height);
                };
                let height: Pixels = lines
                    .first()
                    .map(|l| l.size(line_height).height)
                    .unwrap_or(line_height);
                let width = wrap_width
                    .or_else(|| lines.first().map(|l| l.size(line_height).width))
                    .unwrap_or(px(0.));
                size(width, height.max(line_height))
            },
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let lh = self.line_height;
        let Ok(lines) = window.text_system().shape_text(
            self.text.clone(),
            self.font_size,
            &self.runs,
            Some(bounds.size.width),
            None,
        ) else {
            return LinePrepaint { line: None, selection_quads: Vec::new(), caret: None };
        };
        let Some(line) = lines.into_iter().next() else {
            return LinePrepaint { line: None, selection_quads: Vec::new(), caret: None };
        };

        let (selection, head_in_line) = {
            let editor = self.editor.read(cx);
            if editor.diff.is_some() {
                // Read-only diff view: buffer selection offsets don't
                // apply to the merged doc — no selection, no caret.
                (usize::MAX..usize::MAX, None)
            } else {
                let sel = editor.core.selection;
                let head = sel.head;
                (
                    sel.range(),
                    (head >= self.range.start && head <= self.range.end).then_some(head),
                )
            }
        };

        // Selection quads, one per wrapped row the selection touches.
        let mut selection_quads = Vec::new();
        let sel_start = selection.start.max(self.range.start);
        let sel_end = selection.end.min(self.range.end);
        if sel_start < sel_end || (selection.start < self.range.start
            && selection.end > self.range.end)
        {
            let local_start = display::src_to_disp(&self.display, sel_start);
            let local_end = display::src_to_disp(&self.display, sel_end);
            if let (Some(p1), Some(p2)) = (
                line.position_for_index(local_start, lh),
                line.position_for_index(local_end, lh),
            ) {
                let full_width = bounds.size.width;
                if p1.y == p2.y {
                    selection_quads.push(fill(
                        Bounds::new(
                            point(bounds.origin.x + p1.x, bounds.origin.y + p1.y),
                            size(p2.x - p1.x, lh),
                        ),
                        self.selection_color,
                    ));
                } else {
                    // First row: from start to the row's end.
                    selection_quads.push(fill(
                        Bounds::new(
                            point(bounds.origin.x + p1.x, bounds.origin.y + p1.y),
                            size(full_width - p1.x, lh),
                        ),
                        self.selection_color,
                    ));
                    // Middle rows: full width.
                    let mut y = p1.y + lh;
                    while y < p2.y {
                        selection_quads.push(fill(
                            Bounds::new(
                                point(bounds.origin.x, bounds.origin.y + y),
                                size(full_width, lh),
                            ),
                            self.selection_color,
                        ));
                        y += lh;
                    }
                    // Last row: from row start to end position.
                    selection_quads.push(fill(
                        Bounds::new(
                            point(bounds.origin.x, bounds.origin.y + p2.y),
                            size(p2.x, lh),
                        ),
                        self.selection_color,
                    ));
                }
            }
        }

        // Caret.
        let caret = head_in_line.and_then(|head| {
            let editor = self.editor.read(cx);
            if !editor.core.selection.is_cursor() {
                return None;
            }
            let local = display::src_to_disp(&self.display, head);
            let pos = line.position_for_index(local, lh)?;
            Some(fill(
                Bounds::new(
                    point(bounds.origin.x + pos.x, bounds.origin.y + pos.y),
                    size(px(2.), lh),
                ),
                self.caret_color,
            ))
        });

        LinePrepaint { line: Some(line), selection_quads, caret }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (is_cursor_line, focus_handle) = {
            let editor = self.editor.read(cx);
            let cursor_line = editor.core.buffer.line_of_byte(editor.core.selection.head);
            (
                cursor_line == self.line_ix && editor.diff.is_none(),
                editor.focus_handle.clone(),
            )
        };
        if is_cursor_line {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.editor.clone()),
                cx,
            );
        }

        for quad in prepaint.selection_quads.drain(..) {
            window.paint_quad(quad);
        }

        let Some(line) = prepaint.line.take() else {
            return;
        };
        line.paint(bounds.origin, self.line_height, TextAlign::Left, None, window, cx)
            .ok();

        if focus_handle.is_focused(window) {
            if let Some(caret) = prepaint.caret.take() {
                window.paint_quad(caret);
            }
        }

        let line_height = self.line_height;
        let line_ix = self.line_ix;
        let display = self.display.clone();
        self.editor.update(cx, |editor, _| {
            editor.layout_cache.insert(
                line_ix,
                CachedLine { line, origin: bounds.origin, line_height, display },
            );
        });
    }
}

/// A real table for an untouched table block. Clicking a row drops the
/// cursor onto that row's source line, dissolving the widget.
fn render_table(
    editor: &Entity<Editor>,
    item_ix: usize,
    lines: Range<usize>,
    t: &Theme,
    cx: &mut App,
) -> gpui::AnyElement {
    let mut rows: Vec<(usize, Vec<String>)> = Vec::new();
    // The delimiter row is not drawn, but it is the only place a
    // table's alignment is recorded — read it on the way past.
    let mut aligns: Vec<Option<blocks::ColumnAlign>> = Vec::new();
    {
        let ed = editor.read(cx);
        for line in lines {
            let text = ed.core.buffer.line_text(line);
            if blocks::is_separator_row(&text) {
                aligns = blocks::column_alignments(&text);
                continue;
            }
            rows.push((line, blocks::parse_row(&text)));
        }
    }
    let ncols = rows.iter().map(|(_, cells)| cells.len()).max().unwrap_or(1);
    let borders = crate::view::table_borders(t);

    let mut container = div()
        .my_1()
        .rounded_lg()
        .border_1()
        .border_color(borders.outer)
        .font_family(t.body_family.clone())
        .flex()
        .flex_col()
        .overflow_hidden();

    for (row_ix, (line, cells)) in rows.into_iter().enumerate() {
        let is_header = row_ix == 0;
        // The rule under the header keeps full weight; every other row
        // separates from its neighbour with a hairline. The first body
        // row's separator IS the header rule, drawn as the header's own
        // bottom border -- giving it a second, hairline top border here
        // would double the line.
        let is_first_body_row = row_ix == 1;
        let handle = editor.clone();
        let mut row = div()
            .id(("trow", item_ix * 1024 + row_ix))
            .flex()
            .flex_row()
            .w_full()
            .cursor_pointer()
            .when(is_header, |d| {
                d.bg(t.panel_bg)
                    .rounded_t_lg()
                    .border_b_1()
                    .border_color(borders.header)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.fg_strong)
            })
            .when(!is_header && !is_first_body_row, |d| {
                d.border_t_1().border_color(borders.row)
            })
            .when(!is_header, |d| {
                d.text_color(t.fg).hover(|s| s.bg(t.hover_bg))
            })
            .on_click(move |_, window, cx| {
                handle.update(cx, |editor, cx| {
                    let start = editor.core.buffer.line_range(line).start;
                    editor.core.set_cursor(start);
                    editor.core.break_undo_group();
                    window.focus(&editor.focus_handle);
                    cx.notify();
                });
            });
        for c in 0..ncols {
            let cell = cells.get(c).cloned().unwrap_or_default();
            let mut cell_el = div()
                .flex_1()
                .min_w_0()
                .px_3()
                .py_2()
                .text_size(px(t.body_size - 1.))
                .line_height(relative(1.45));
            match aligns.get(c).copied().flatten() {
                Some(blocks::ColumnAlign::Center) => cell_el = cell_el.text_center(),
                Some(blocks::ColumnAlign::Right) => cell_el = cell_el.text_right(),
                // Left is the default flow direction; a bare `---`
                // column has no opinion and is left alone too.
                Some(blocks::ColumnAlign::Left) | None => {}
            }
            row = row.child(cell_el.child(SharedString::from(cell)));
        }
        container = container.child(row);
    }
    container.into_any_element()
}

/// The rendered image for an untouched whole-line image block. Local
/// paths resolve against the file's directory; missing files fall back
/// to the raw markup with a warning tint. Click dissolves to source.
fn render_image(
    editor: &Entity<Editor>,
    item_ix: usize,
    line: usize,
    alt: &str,
    dest: &str,
    t: &Theme,
    cx: &mut App,
) -> gpui::AnyElement {
    // Shared with the reading view: one answer about where an image
    // lives, so the same document cannot resolve two ways (#57).
    let source = editor.read(cx).image_source(dest);

    let handle = editor.clone();
    let on_click = move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
        handle.update(cx, |editor, cx| {
            let start = editor.core.buffer.line_range(line).start;
            editor.core.set_cursor(start);
            editor.core.break_undo_group();
            window.focus(&editor.focus_handle);
            cx.notify();
        });
    };

    let image = match source {
        crate::markdown::ImageSource::Remote(url) => gpui::img(url),
        crate::markdown::ImageSource::Local(path) => gpui::img(path),
        crate::markdown::ImageSource::Missing(_) => {
            return div()
                .id(("img", item_ix))
                .my_1()
                .cursor_pointer()
                .on_click(on_click)
                .font_family(t.mono_family.clone())
                .text_size(px(t.code_size))
                .text_color(Hsla { a: 0.8, ..t.accent })
                .child(SharedString::from(format!("![{alt}]({dest}) — file not found")))
                .into_any_element();
        }
    };
    // The same corner as the reading view draws, on the element that
    // actually paints: gpui's `ContentMask` carries no radii, so a
    // rounded parent cannot clip a square child.
    let radius = crate::elevation::radius(crate::elevation::Surface::Page);
    div()
        .id(("img", item_ix))
        .my_1()
        .w_full()
        .cursor_pointer()
        .on_click(on_click)
        .child(image.w_full().max_h(px(420.)).rounded(radius))
        .into_any_element()
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.inline_gen != crate::extensions::inline_generation()
            && matches!(self.provider, Provider::Markdown)
        {
            let langs = crate::highlight::languages(cx);
            self.restyle(&langs);
        }
        if self.blur_subscription.is_none() {
            // A window is only available from render, so the
            // subscription cannot be set up any earlier than the
            // first paint. Idempotent: every later render sees
            // `Some` and skips this.
            self.blur_subscription =
                Some(cx.on_blur(&self.focus_handle, window, |editor, _window, cx| {
                    editor.pending_link = None;
                    editor.hover_link = None;
                    editor.hover_task = None;
                    editor.hover_at = None;
                    editor.hover_preview = None;
                    editor.hover_held = false;
                    editor.hover_close_task = None;
                    cx.notify();
                }));
        }
        self.reproject();
        let entity = cx.weak_entity();
        let t = theme(cx);

        let scrollbar = {
            let max = self.list_state.max_offset_for_scrollbar().height;
            let vh = self.list_state.viewport_bounds().size.height;
            if max > px(0.) && vh > px(0.) {
                let offset = -self.list_state.scroll_px_offset_for_scrollbar().y;
                let total = vh + max;
                let thumb_h = (vh * (vh / total)).max(px(30.)).min(vh);
                let frac = (offset / max).clamp(0., 1.);
                let top = (vh - thumb_h) * frac;
                let dragging = self.scrollbar_dragging;
                let handle = cx.entity();
                Some(
                    div()
                        .id("scrollbar-track")
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .w(px(12.))
                        .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                            cx.stop_propagation();
                            handle.update(cx, |editor, cx| {
                                editor.scrollbar_dragging = true;
                                editor.list_state.scrollbar_drag_started();
                                editor.scrollbar_scrub(event.position, cx);
                            });
                        })
                        .child(
                            div()
                                .absolute()
                                .right(px(3.))
                                .w(px(6.))
                                .top(top)
                                .h(thumb_h)
                                .rounded_full()
                                .bg(Hsla {
                                    a: if dragging { 0.55 } else { 0.28 },
                                    ..t.fg_muted
                                }),
                        ),
                )
            } else {
                None
            }
        };

        let toolbar = self
            .toolbar_showing()
            .then(|| self.toolbar_anchor())
            .flatten()
            .map(|pos| {
                let button = |ix: usize, label: &'static str| {
                    div()
                        .id(("fmt-btn", ix))
                        .px(px(7.))
                        .py(px(3.))
                        .rounded_md()
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(t.fg)
                        .hover(|d| d.bg(t.hover_bg))
                        .child(label)
                };
                let bar = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.))
                    .p(px(3.))
                    .border_1()
                    .border_color(t.border)
                    .elevated(crate::elevation::Overlay::FormatToolbar, &t)
                    .child(button(0, "B").font_weight(FontWeight::BOLD).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ed, _, w, cx| {
                            cx.stop_propagation();
                            ed.toggle_bold(&ToggleBold, w, cx);
                        }),
                    ))
                    .child(button(1, "I").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ed, _, w, cx| {
                            cx.stop_propagation();
                            ed.toggle_italic(&ToggleItalic, w, cx);
                        }),
                    ))
                    .child(button(2, "<>").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ed, _, w, cx| {
                            cx.stop_propagation();
                            ed.toggle_code(&ToggleCode, w, cx);
                        }),
                    ))
                    .child(button(3, "S̶").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ed, _, w, cx| {
                            cx.stop_propagation();
                            ed.toggle_strike(&ToggleStrike, w, cx);
                        }),
                    ))
                    .child(button(4, "[⌁]").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ed, _, w, cx| {
                            cx.stop_propagation();
                            ed.insert_link(&InsertLink, w, cx);
                        }),
                    ))
                    .child(button(5, "H").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ed, _, w, cx| {
                            cx.stop_propagation();
                            ed.cycle_heading(&CycleHeading, w, cx);
                        }),
                    ))
                    .child(button(6, "❝").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ed, _, w, cx| {
                            cx.stop_propagation();
                            ed.toggle_quote(&ToggleQuote, w, cx);
                        }),
                    ));
                deferred(
                    anchored()
                        .position(pos)
                        .anchor(Corner::BottomLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(bar),
                )
            });

        // `[[` completion popup, anchored under the cursor.
        let completion_el = self.completion.as_ref().and_then(|comp| {
            let head = self.core.selection.head;
            let line_ix = self.core.buffer.line_of_byte(head);
            let entry = self.layout_cache.get(&line_ix)?;
            let disp = display::src_to_disp(&entry.display, head);
            let pos = entry.line.position_for_index(disp, entry.line_height)?;
            let anchor = point(
                entry.origin.x + pos.x,
                entry.origin.y + pos.y + entry.line_height + px(4.),
            );
            let selected = comp.selected;
            let rows = comp
                .matches
                .iter()
                .enumerate()
                .map(|(ix, (name, path))| {
                    let rel = path
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|d| format!("{}/", d.to_string_lossy()))
                        .unwrap_or_default();
                    div()
                        .id(("completion-row", ix))
                        .px_2()
                        .py(px(3.))
                        .flex()
                        .flex_row()
                        .gap_2()
                        .cursor_pointer()
                        .when(ix == selected, |d| d.bg(t.selected_bg))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(t.fg_strong)
                                .child(SharedString::from(name.clone())),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(t.fg_muted)
                                .child(SharedString::from(rel)),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |editor, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(comp) = &mut editor.completion {
                                    comp.selected = ix;
                                }
                                editor.confirm_completion(cx);
                            }),
                        )
                })
                .collect::<Vec<_>>();
            Some(deferred(
                anchored()
                    .position(anchor)
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .w(px(280.))
                            .border_1()
                            .border_color(t.border)
                            .elevated(crate::elevation::Overlay::LinkCompletion, &t)
                            .overflow_hidden()
                            // The first row is selected by default and
                            // its fill is square: flush with the rounded
                            // edge it would paint over the corner arcs.
                            .py(crate::elevation::corner_inset(
                                crate::elevation::Overlay::LinkCompletion,
                            ))
                            .flex()
                            .flex_col()
                            .children(rows),
                    ),
            ))
        });

        // The link hover popover. Anchored where the pointer rested,
        // and snapped into the window so a link near an edge still
        // shows its preview rather than half of one.
        let hover_popover = self.hover_preview.as_ref().zip(self.hover_at).map(|(pv, at)| {
            use crate::preview::{Consent, Preview};
            let body = |title: String, sub: Option<String>, text: String, t: &Theme| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(t.ui_size))
                            .text_color(t.fg_strong)
                            .child(SharedString::from(title)),
                    )
                    .children(sub.map(|s| {
                        div()
                            .text_size(px(t.ui_size - 1.))
                            .text_color(t.fg_muted)
                            .child(SharedString::from(s))
                    }))
                    .when(!text.is_empty(), |d| {
                        d.child(
                            div()
                                .mt_1()
                                .text_size(px(t.ui_size - 1.))
                                .text_color(t.fg)
                                .child(SharedString::from(text)),
                        )
                    })
            };
            let inner = match pv {
                Preview::Note { title, excerpt } => {
                    body(title.clone(), None, excerpt.clone(), &t)
                }
                Preview::Code { language, excerpt } => body(
                    language.clone().unwrap_or_else(|| "Text".into()),
                    None,
                    excerpt.clone(),
                    &t,
                ),
                Preview::Anchor { heading, excerpt } => {
                    body(heading.clone(), None, excerpt.clone(), &t)
                }
                Preview::Image { path } => body(
                    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    Some("Image".into()),
                    String::new(),
                    &t,
                ),
                Preview::Missing { name } => body(
                    name.clone(),
                    Some("Does not exist — click to create".into()),
                    String::new(),
                    &t,
                ),
                Preview::External { url, domain, mismatch, consent, fetched } => {
                    let sub = match (consent, fetched) {
                        (Consent::Granted, Some(m)) => m.description.clone(),
                        (Consent::Ungranted, _) => {
                            Some("Previews are off for this site".into())
                        }
                        (Consent::Denied, _) => Some("Previews refused for this site".into()),
                        _ => None,
                    };
                    let title = match (consent, fetched) {
                        (Consent::Granted, Some(m)) => m.title.clone(),
                        _ if domain.is_empty() => url.clone(),
                        _ => domain.clone(),
                    };
                    let ungranted = *consent == Consent::Ungranted;
                    body(title, sub, url.clone(), &t)
                        .when(ungranted, |d| {
                            // The consent prompt is a click, never the
                            // hover: the pointer passing over a link
                            // must not be able to reach a server.
                            d.child(
                                div()
                                    .mt_2()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(t.hover_bg)
                                    .text_size(px(t.ui_size - 1.))
                                    .text_color(t.accent)
                                    .cursor_pointer()
                                    .child("Enable previews for this site")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|editor, _, _, cx| {
                                            editor.enable_previews_for_hovered_site(cx);
                                        }),
                                    ),
                            )
                        })
                        .when(*mismatch, |d| {
                        d.child(
                            div()
                                .mt_1()
                                .text_size(px(t.ui_size - 1.))
                                // The palette's red. A dedicated
                                // `warning` colour would have to be
                                // threaded through `Theme::map_colors`
                                // and every theme TOML, or flux warming
                                // would miss it — not worth it for one
                                // line.
                                .text_color(t.diff_deleted_fg)
                                .child(SharedString::from(
                                    "⚠ the link text names a different site",
                                )),
                        )
                    })
                }
            };
            deferred(
                anchored()
                    .position(at)
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            // `on_hover` needs a stateful element.
                            .id("link-hover-popover")
                            .on_hover(cx.listener(|editor, hovered: &bool, _, cx| {
                                // Inside the popover the pointer is not
                                // on the link, but the popover must stay:
                                // its button is the whole point.
                                editor.hover_held = *hovered;
                                if *hovered {
                                    editor.hover_close_task = None;
                                } else {
                                    editor.hover_left(cx);
                                }
                            }))
                            .max_w(px(360.))
                            .border_1()
                            .border_color(t.border)
                            .elevated(crate::elevation::Overlay::LinkHover, &t)
                            .overflow_hidden()
                            .p_3()
                            .child(inner),
                    ),
            )
        });

        let diffing = self.diff.is_some();
        let diff_header = self.diff.as_ref().map(|d| {
            div()
                .h(px(34.))
                .w_full()
                .flex_none()
                .bg(t.panel_bg)
                .border_b_1()
                .border_color(t.border)
                .flex()
                .flex_row()
                .items_center()
                .px_3()
                .text_size(px(12.))
                .child(
                    div()
                        .flex_1()
                        .text_color(t.fg)
                        .child(SharedString::from(format!(
                            "Changes vs HEAD · +{} −{}",
                            d.adds, d.dels
                        ))),
                )
                .child(div().text_color(t.fg_muted).child("esc to close"))
        });
        let scope_hint = self.git_scope_hint();
        let diff_empty: Option<String> = self.diff.as_ref().and_then(|d| {
            use crate::git::Baseline;
            match &d.missing {
                Some(Baseline::NotInRepo) => Some(match scope_hint {
                    Some(hint) => format!("Not in a git repository — or {hint}."),
                    None => "Not in a git repository.".into(),
                }),
                Some(Baseline::Untracked) => Some("Not tracked in git yet.".into()),
                Some(Baseline::Binary) => Some("No text baseline at HEAD.".into()),
                _ => d
                    .changes
                    .is_empty()
                    .then(|| "No uncommitted changes.".to_string()),
            }
        });

        let find_bar = self.find.as_ref().filter(|_| !diffing).map(|state| {
            let total = state.matches.len();
            let current = if total == 0 { 0 } else { state.active + 1 };
            div()
                .h(px(38.))
                .w_full()
                .flex_none()
                .bg(t.panel_bg)
                .border_b_1()
                .border_color(t.border)
                .key_context("FindBar")
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .child(div().flex_1().child(state.input.clone()))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.fg_muted)
                        .child(SharedString::from(format!("{current}/{total}"))),
                )
                .when(state.replacing, |d| {
                    d.child(div().flex_1().child(state.replace_input.clone()))
                })
        });

        div()
            .size_full()
            .bg(t.page_bg)
            // GPUI content masks are rectangular -- `ContentMask` has
            // bounds and no radii -- so this square fill would
            // otherwise overpaint the page's rounded bottom corners.
            // Rounding here keeps the common case honest; the page
            // masks its own corners for everything deeper than this
            // root (see `Workspace::page_corner_masks`), because no
            // test can see a corner and nothing else can enforce it.
            .rounded_b(crate::elevation::radius(crate::elevation::Surface::Page))
            .debug_selector(|| "editor-root".into())
            .key_context(if diffing { "DiffView" } else { "Editor" })
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::move_word_left))
            .on_action(cx.listener(Self::move_word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::insert_tab))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::follow_link))
            .on_action(cx.listener(Self::dismiss_completion))
            .on_action(cx.listener(Self::toggle_bold))
            .on_action(cx.listener(Self::toggle_italic))
            .on_action(cx.listener(Self::toggle_code))
            .on_action(cx.listener(Self::toggle_strike))
            .on_action(cx.listener(Self::insert_link))
            .on_action(cx.listener(Self::cycle_heading))
            .on_action(cx.listener(Self::toggle_quote))
            .on_action(cx.listener(Self::save_now))
            .on_action(cx.listener(Self::open_find))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_prev))
            .on_action(cx.listener(Self::close_find))
            .on_action(cx.listener(Self::replace_next))
            .on_action(cx.listener(Self::replace_all))
            .on_action(cx.listener(Self::table_insert_row))
            .on_action(cx.listener(Self::table_delete_row))
            .on_action(cx.listener(Self::table_insert_column))
            .on_action(cx.listener(Self::table_delete_column))
            .on_action(cx.listener(Self::renumber_list))
            .on_mouse_move(cx.listener(Self::on_root_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_root_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_root_mouse_up_out))
            .flex()
            .flex_col()
            .children(diff_header)
            .children(find_bar)
            .child(if let Some(msg) = diff_empty {
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(t.fg_muted)
                    .child(msg)
                    .into_any_element()
            } else {
                div().flex_1().min_h_0().relative().child(
                list(self.list_state.clone(), move |ix, _window, cx| {
                    let Some(editor_entity) = entity.upgrade() else {
                        return div().into_any_element();
                    };
                    let t = theme(cx);
                    let (item, item_count) = {
                        let editor = editor_entity.read(cx);
                        if editor.diff.is_some() {
                            let n = editor.view_buffer().line_count();
                            ((ix < n).then_some(projection::Item::Line(ix)), n)
                        } else {
                            (editor.projection.get(ix).cloned(), editor.projection.len())
                        }
                    };
                    let column = |inner: gpui::AnyElement| {
                        div()
                            .w_full()
                            .flex()
                            .flex_row()
                            .justify_center()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(760.))
                                    .px(px(48.))
                                    .when(ix == 0, |d| d.pt(px(40.)))
                                    .when(ix + 1 == item_count, |d| d.pb(px(96.)))
                                    .child(inner),
                            )
                            .into_any_element()
                    };
                    match item {
                        Some(projection::Item::Line(line_ix)) => {
                            let (
                                line_range,
                                text,
                                runs,
                                dl,
                                font_size,
                                line_height_px,
                                is_code,
                                code_mode,
                                line_count,
                                rule_color,
                            ) = {
                                let editor = editor_entity.read(cx);
                                let (size_f, _, _, mult) = editor.line_typography(line_ix, &t);
                                let (text, runs, dl) = editor.display_for_line(line_ix, &t);
                                let rule_color = (!editor.is_code_mode()
                                    && display::draws_rule(&dl, editor.view_spans()))
                                .then(|| editor.rule_color(line_ix, &t));
                                (
                                    editor.view_buffer().line_range(line_ix),
                                    text,
                                    runs,
                                    dl,
                                    px(size_f),
                                    px(size_f * mult),
                                    matches!(
                                        editor.view_line_kinds().get(line_ix),
                                        Some(LineKind::Code)
                                    ),
                                    editor.is_code_mode(),
                                    editor.view_buffer().line_count(),
                                    rule_color,
                                )
                            };
                            let mouse_editor = editor_entity.clone();
                            let menu_editor = editor_entity.clone();
                            let line_el = LineElement {
                                editor: editor_entity.clone(),
                                line_ix,
                                range: line_range,
                                text,
                                runs,
                                display: dl,
                                font_size,
                                line_height: line_height_px,
                                caret_color: t.accent,
                                selection_color: Hsla { a: 0.25, ..t.accent },
                            };
                            let on_down = move |event: &MouseDownEvent,
                                                window: &mut Window,
                                                cx: &mut App| {
                                mouse_editor.update(cx, |editor, cx| {
                                    editor.on_line_mouse_down(line_ix, event, window, cx);
                                });
                            };
                            let on_right = move |event: &MouseDownEvent,
                                                 window: &mut Window,
                                                 cx: &mut App| {
                                menu_editor.update(cx, |editor, cx| {
                                    editor.on_line_right_mouse_down(line_ix, event, window, cx);
                                });
                            };
                            if code_mode {
                                let cols = gutter_cols(line_count);
                                let gutter_w = px(cols as f32 * 8.0 + 24.0);
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_row()
                                    .when(ix == 0, |d| d.pt(px(16.)))
                                    .when(ix + 1 == item_count, |d| d.pb(px(64.)))
                                    .child(
                                        div()
                                            .w(gutter_w)
                                            .flex_none()
                                            .pr(px(10.))
                                            .flex()
                                            .justify_end()
                                            .font_family(t.mono_family.clone())
                                            .text_size(px(t.code_size - 2.))
                                            .line_height(relative(1.55 * t.code_size / (t.code_size - 2.)))
                                            .text_color(Hsla { a: 0.5, ..t.fg_muted })
                                            .child(SharedString::from(
                                                editor_entity.read(cx).gutter_label(line_ix),
                                            )),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .pr(px(16.))
                                            .on_mouse_down(MouseButton::Left, on_down)
                                            .on_mouse_down(MouseButton::Right, on_right)
                                            .child(line_el),
                                    )
                                    .into_any_element()
                            } else {
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_row()
                                    .justify_center()
                                    .child(
                                        div()
                                            .w_full()
                                            .max_w(px(760.))
                                            .px(px(48.))
                                            .when(ix == 0, |d| d.pt(px(40.)))
                                            .when(ix + 1 == item_count, |d| d.pb(px(96.)))
                                            .when(is_code, |d| d.bg(t.code_bg))
                                            .on_mouse_down(MouseButton::Left, on_down)
                                            .on_mouse_down(MouseButton::Right, on_right)
                                            .child(if let Some(color) = rule_color {
                                                // A hidden thematic break: the
                                                // line keeps its height (and
                                                // its hit-testing) and a divider
                                                // is drawn across its middle,
                                                // styled as the reading view's
                                                // (or in its diff colour).
                                                let (thick, _) = crate::view::rule_style(&t);
                                                div()
                                                    .relative()
                                                    .child(line_el)
                                                    .child(
                                                        div()
                                                            .debug_selector(move || {
                                                                format!("rule-line-{line_ix}")
                                                            })
                                                            .absolute()
                                                            .left_0()
                                                            .right_0()
                                                            .top(line_height_px / 2. - px(thick / 2.))
                                                            .h(px(thick))
                                                            .bg(color),
                                                    )
                                                    .into_any_element()
                                            } else {
                                                line_el.into_any_element()
                                            }),
                                    )
                                    .into_any_element()
                            }
                        }
                        Some(projection::Item::Widget { projector, lines, payload }) => {
                            let mut wctx = projector::WidgetCtx {
                                editor: &editor_entity,
                                item_ix: ix,
                                lines,
                                payload: &payload,
                                theme: &t,
                                cx,
                            };
                            column(projector::projectors()[projector].render(&mut wctx))
                        }
                        None => div().into_any_element(),
                    }
                })
                .size_full(),
            )
                .children(scrollbar)
                .children(toolbar)
                .children(hover_popover)
                .children(completion_el)
                .into_any_element()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{KeyBinding, TestAppContext, VisualTestContext};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    #[test]
    fn find_prev_only_consumes_the_key_when_the_find_bar_is_open() {
        // Mirrors cmd_b_is_shared_across_contexts_on_purpose: a shared
        // chord must fall through when this handler has nothing to do,
        // or the global binding is unreachable.
        assert!(!find_prev_should_consume(false), "closed find bar must propagate");
        assert!(find_prev_should_consume(true), "open find bar consumes the key");
    }

    /// Everything an editor test touches on disk, rooted in tempdirs:
    /// the edited file and the session backup registry. Nothing under
    /// the real HOME is read or written.
    struct Fixture {
        _files: tempfile::TempDir,
        backups: tempfile::TempDir,
        path: PathBuf,
    }

    impl Fixture {
        fn backup_contents(&self) -> Vec<String> {
            let mut out = Vec::new();
            if let Ok(entries) = std::fs::read_dir(self.backups.path().join("backups")) {
                for entry in entries.flatten() {
                    out.push(std::fs::read_to_string(entry.path()).unwrap());
                }
            }
            out
        }
    }

    fn open_editor<'a>(
        cx: &'a mut TestAppContext,
        name: &str,
        text: &str,
    ) -> (Fixture, Entity<Editor>, &'a mut VisualTestContext) {
        let files = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let path = files.path().join(name);
        std::fs::write(&path, text).unwrap();
        let langs = Arc::new(Languages::new());
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(
                crate::theme::Theme::dark(),
            )));
            cx.set_global(crate::highlight::SyntaxLanguages(langs.clone()));
            cx.set_global(SessionBackups(Arc::new(Mutex::new(
                autosave::BackupRegistry::new(backups.path().join("backups")),
            ))));
        });
        let contents = text.to_string();
        let file = path.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| Editor::from_text(&file, contents, &langs, cx));
        cx.update(|window, app| {
            let handle = editor.read(app).focus_handle.clone();
            window.focus(&handle);
        });
        attach_workspace_handles(&editor, cx);
        cx.run_until_parked();
        (Fixture { _files: files, backups, path }, editor, cx)
    }

    /// Hand the editor the index and host a workspace would give it.
    /// The test globals stand in for the workspace that does not exist
    /// here; production wiring is `Workspace::make_editor`.
    fn attach_workspace_handles(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
        cx.update(|_, app| {
            let knowledge = app
                .try_global::<crate::knowledge::KnowledgeState>()
                .map(|s| s.0.clone());
            let host = app
                .try_global::<crate::extensions::ExtensionState>()
                .map(|s| s.0.clone());
            editor.update(app, |editor, cx| {
                if knowledge.is_some() {
                    editor.knowledge = knowledge;
                }
                if host.is_some() {
                    editor.host_root = host
                        .as_ref()
                        .map(|h| h.lock().unwrap_or_else(|e| e.into_inner()).root_handle());
                    editor.host = host;
                    // A workspace-built editor has its host at
                    // construction, so `from_text_in` already scheduled
                    // the status widgets against it; re-run that here.
                    editor.schedule_status(cx);
                }
            });
        });
    }

    fn buffer_text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
        cx.update(|_, app| editor.read(app).text())
    }

    fn head(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
        cx.update(|_, app| editor.read(app).core.selection.head)
    }

    fn widget_count(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
        cx.update(|_, app| {
            editor
                .read(app)
                .projection
                .iter()
                .filter(|item| matches!(item, projection::Item::Widget { .. }))
                .count()
        })
    }

    /// Clicking a table far down a document must not scroll to the top.
    #[gpui::test]
    fn revealing_a_widget_keeps_the_scroll_position(cx: &mut TestAppContext) {
        // A long document with a table near the end.
        let mut text = String::new();
        for i in 0..300 {
            text.push_str(&format!("line {i}\n\n"));
        }
        text.push_str("| a | b |\n| - | - |\n| 1 | 2 |\n");
        let (_fx, editor, cx) = open_editor(cx, "long.md", &text);
        cx.run_until_parked();

        // Scroll to the table and let the projection settle.
        editor.update(cx, |ed, _| {
            let last = ed.projection.len().saturating_sub(1);
            ed.list_state.scroll_to_reveal_item(last);
        });
        cx.run_until_parked();
        let before = editor.read_with(cx, |ed, _| ed.list_state.logical_scroll_top().item_ix);
        assert!(before > 0, "precondition: we are not at the top");

        // Put the cursor in the table, which reveals it and changes the
        // projection — the path that used to reset the scroll.
        editor.update(cx, |ed, cx| {
            let offset = ed.core.buffer.text().find("| a |").unwrap();
            ed.core.set_cursor(offset);
            cx.notify();
        });
        cx.run_until_parked();

        // Not just "somewhere below the top": `after > 0` alone would
        // pass for a fix that landed on item 1 of 300. The anchor must
        // still be where the reader left it.
        let after = editor.read_with(cx, |ed, _| ed.list_state.logical_scroll_top().item_ix);
        assert!(
            after.abs_diff(before) <= 1,
            "revealing a widget must keep the scroll position: was item {before}, now {after}"
        );
    }

    /// Widgets (table, image, diagrams) render through the projector
    /// registry: the initial frame draws each claim's widget arm, and
    /// diagram results (ready or failed) land after the background
    /// render settles.
    #[gpui::test]
    fn projector_widgets_render_in_the_window(cx: &mut TestAppContext) {
        let doc = "intro line\n\n\
                   | h1 | h2 |\n| --- | --- |\n| a | b |\n\n\
                   ![pic](missing.png)\n\n\
                   ```mermaid\nflowchart TD\n  A --> B\n```\n\n\
                   ```mermaid\nthis is not a diagram\n```\n\n\
                   tail\n";
        let (_fx, editor, cx) = open_editor(cx, "widgets.md", doc);
        // Cursor sits in the intro line: every claim is untouched.
        assert_eq!(widget_count(&editor, cx), 4, "table, image, two diagrams");
        // Let the diagram renders finish and redraw (ready + failed arms).
        for _ in 0..20 {
            cx.executor().advance_clock(std::time::Duration::from_millis(100));
            cx.run_until_parked();
        }
        cx.update(|_, app| {
            editor.update(app, |_, cx| cx.notify());
        });
        cx.run_until_parked();
        assert_eq!(widget_count(&editor, cx), 4, "widgets survive the redraw");
        // Touching a widget's range dissolves it back to source lines.
        editor.update_in(cx, |editor, _, cx| {
            let table_start = doc.find('|').unwrap();
            editor.core.set_cursor(table_start);
            editor.after_edit(cx);
        });
        cx.run_until_parked();
        assert_eq!(widget_count(&editor, cx), 3, "touched table dissolves");
    }

    /// A fenced block claimed by the echo plugin renders through the
    /// PluginBlock projector (pending spinner, then the rasterized SVG
    /// or the failure arm).
    #[gpui::test]
    fn plugin_fence_renders_as_a_widget(cx: &mut TestAppContext) {
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        let doc = "intro\n\n```echo-fixture\nhello widget\n```\n\ntail\n";
        let (_fx, editor, cx) = open_editor(cx, "plugin-widget.md", doc);
        assert_eq!(widget_count(&editor, cx), 1, "echo fence claimed");
        // Let the background plugin render + rasterize land, then
        // redraw so the Ready/Failed arm executes.
        for _ in 0..30 {
            cx.executor().advance_clock(std::time::Duration::from_millis(100));
            cx.run_until_parked();
        }
        editor.update_in(cx, |_, _, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(widget_count(&editor, cx), 1, "widget survives the redraw");
        crate::extensions::set_surface_tables(&[]);
        crate::extensions::set_fence_table(Vec::new());
    }

    /// Load the fixture plugins into the tables + global. The returned
    /// guard serializes table-mutating tests; None = fixtures absent.
    fn with_plugins(
        cx: &mut TestAppContext,
    ) -> Option<std::sync::MutexGuard<'static, ()>> {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/plugins");
        if !dir.join("probe/plugin.wasm").exists() {
            eprintln!("SKIP: fixtures not built");
            return None;
        }
        let guard = crate::extensions::table_test_guard();
        let mut host = crate::extensions::ExtensionHost::load(&dir);
        crate::extensions::refresh_tables(&mut host);
        cx.update(|cx| {
            cx.set_global(crate::extensions::ExtensionState(Arc::new(Mutex::new(host))));
        });
        Some(guard)
    }

    #[gpui::test]
    fn save_hooks_transform_on_flush(cx: &mut TestAppContext) {
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        // probe's on-save appends a marker when the doc says "hookme".
        let (_fx, editor, cx) = open_editor(cx, "hooked.md", "hookme");
        editor.update_in(cx, |editor, _, cx| {
            editor.save.record_edit(Instant::now());
            editor.flush(cx);
        });
        cx.run_until_parked();
        let text = buffer_text(&editor, cx);
        assert!(text.contains("<!-- saved -->"), "{text}");
        crate::extensions::set_surface_tables(&[]);
    }

    #[gpui::test]
    fn net_paste_plugins_enrich_after_the_paste(cx: &mut TestAppContext) {
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        let (_fx, editor, cx) = open_editor(cx, "enrich.md", "");
        cx.update(|_, app| {
            app.write_to_clipboard(ClipboardItem::new_string("enrichme".into()))
        });
        cx.dispatch_action(Paste);
        // The paste lands synchronously; the async enrich pass then
        // replaces the pasted range (often within the first park).
        for _ in 0..20 {
            cx.executor().advance_clock(std::time::Duration::from_millis(100));
            cx.run_until_parked();
            if buffer_text(&editor, cx) == "[enriched]" {
                break;
            }
        }
        assert_eq!(buffer_text(&editor, cx), "[enriched]");
        crate::extensions::set_surface_tables(&[]);
    }

    #[gpui::test]
    fn status_widgets_fill_the_editor_status(cx: &mut TestAppContext) {
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        let (_fx, editor, cx) = open_editor(cx, "status.md", "12345");
        for _ in 0..10 {
            cx.executor().advance_clock(std::time::Duration::from_millis(200));
            cx.run_until_parked();
            if cx.update(|_, app| editor.read(app).status().is_some()) {
                break;
            }
        }
        let status = cx.update(|_, app| editor.read(app).status());
        assert_eq!(status.map(|s| s.to_string()), Some("status:5".to_string()));
        crate::extensions::set_surface_tables(&[]);
    }

    #[gpui::test]
    fn typing_flows_through_the_window_input_handler(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "");
        cx.simulate_input("hello world");
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.text(), "hello world");
            assert_eq!(ed.core.selection.head, 11);
            assert!(ed.core.selection.is_cursor());
            assert!(ed.save.is_dirty(), "typing marks the buffer dirty");
        });
        cx.dispatch_action(InsertTab);
        assert_eq!(buffer_text(&editor, cx), "hello world\t");
    }

    #[gpui::test]
    fn movement_actions_place_the_cursor(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "alpha beta\ngamma\n");
        cx.dispatch_action(DocEnd);
        assert_eq!(head(&editor, cx), 17);
        cx.dispatch_action(DocStart);
        assert_eq!(head(&editor, cx), 0);
        cx.dispatch_action(MoveRight);
        assert_eq!(head(&editor, cx), 1);
        cx.dispatch_action(LineEnd);
        assert_eq!(head(&editor, cx), 10, "line end stops before the newline");
        cx.dispatch_action(MoveWordLeft);
        assert_eq!(head(&editor, cx), 6, "word-left lands on the start of beta");
        cx.dispatch_action(LineStart);
        assert_eq!(head(&editor, cx), 0);
        // Vertical movement goes through the painted-line geometry cache.
        cx.dispatch_action(MoveDown);
        assert_eq!(head(&editor, cx), 11, "down lands on the start of gamma");
        cx.dispatch_action(MoveUp);
        assert_eq!(head(&editor, cx), 0);
    }

    #[gpui::test]
    fn selection_extends_and_typing_replaces_it(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "alpha beta");
        cx.dispatch_action(SelectWordRight);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.core.selection.range(), 0..5);
            assert_eq!(ed.core.selected_text(), "alpha");
        });
        cx.dispatch_action(SelectAll);
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selection.range(), 0..10)
        });
        cx.simulate_input("x");
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.text(), "x", "typing replaces the whole selection");
            assert_eq!(ed.core.selection.head, 1);
        });
    }

    #[gpui::test]
    fn backspace_delete_and_word_backspace_edit_the_buffer(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "foo bar\n");
        cx.dispatch_action(DocEnd);
        cx.dispatch_action(Backspace);
        assert_eq!(buffer_text(&editor, cx), "foo bar");
        cx.dispatch_action(DeleteWordLeft);
        assert_eq!(buffer_text(&editor, cx), "foo ");
        cx.dispatch_action(DocStart);
        cx.dispatch_action(Delete);
        assert_eq!(buffer_text(&editor, cx), "oo ");
        assert_eq!(head(&editor, cx), 0);
    }

    #[gpui::test]
    fn undo_and_redo_roundtrip_a_typed_group(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "");
        cx.simulate_input("abc");
        assert_eq!(buffer_text(&editor, cx), "abc");
        cx.dispatch_action(Undo);
        assert_eq!(buffer_text(&editor, cx), "", "quick keystrokes undo as one group");
        cx.dispatch_action(Redo);
        assert_eq!(buffer_text(&editor, cx), "abc");
        assert_eq!(head(&editor, cx), 3);
        cx.dispatch_action(Redo);
        assert_eq!(buffer_text(&editor, cx), "abc", "redo past history is a no-op");
    }

    #[gpui::test]
    fn copy_cut_and_paste_go_through_the_clipboard(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "hello world");
        cx.dispatch_action(SelectAll);
        cx.dispatch_action(Copy);
        assert_eq!(buffer_text(&editor, cx), "hello world", "copy leaves the buffer alone");
        let clip = cx.update(|_, app| app.read_from_clipboard().and_then(|i| i.text()));
        assert_eq!(clip.as_deref(), Some("hello world"));

        cx.dispatch_action(Cut);
        assert_eq!(buffer_text(&editor, cx), "");
        cx.dispatch_action(Paste);
        assert_eq!(buffer_text(&editor, cx), "hello world");
        assert_eq!(head(&editor, cx), 11);
    }

    #[gpui::test]
    fn bound_keystrokes_trigger_editor_actions(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "");
        cx.update(|_, app| {
            app.bind_keys([
                KeyBinding::new("enter", Newline, Some("Editor")),
                KeyBinding::new("backspace", Backspace, Some("Editor")),
            ]);
        });
        cx.simulate_input("hi");
        cx.simulate_keystrokes("enter");
        assert_eq!(buffer_text(&editor, cx), "hi\n");
        cx.simulate_keystrokes("backspace backspace");
        assert_eq!(buffer_text(&editor, cx), "h");
    }

    fn select(editor: &Entity<Editor>, cx: &mut VisualTestContext, range: Range<usize>) {
        editor.update_in(cx, |editor, _, cx| {
            editor.core.set_cursor(range.start);
            editor.core.select_to(range.end);
            cx.notify();
        });
    }

    #[gpui::test]
    fn bold_toggles_round_trip_and_undo_as_one_group(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "fmt.md", "say hello now");
        select(&editor, cx, 4..9);
        cx.dispatch_action(ToggleBold);
        assert_eq!(buffer_text(&editor, cx), "say **hello** now");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selection.range(), 6..11, "word stays selected");
        });
        cx.dispatch_action(ToggleBold);
        assert_eq!(buffer_text(&editor, cx), "say hello now");
        cx.dispatch_action(Undo);
        assert_eq!(buffer_text(&editor, cx), "say **hello** now", "one group per toggle");
        cx.dispatch_action(Undo);
        assert_eq!(buffer_text(&editor, cx), "say hello now");
    }

    #[gpui::test]
    fn every_toolbar_action_edits_the_document(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "fmt.md", "alpha beta");
        let cases: Vec<(Box<dyn Fn(&mut VisualTestContext)>, &str)> = vec![
            (Box::new(|cx| cx.dispatch_action(ToggleItalic)), "*alpha* beta"),
            (Box::new(|cx| cx.dispatch_action(ToggleCode)), "`alpha` beta"),
            (Box::new(|cx| cx.dispatch_action(ToggleStrike)), "~~alpha~~ beta"),
            (Box::new(|cx| cx.dispatch_action(InsertLink)), "[alpha](url) beta"),
            (Box::new(|cx| cx.dispatch_action(CycleHeading)), "# alpha beta"),
            (Box::new(|cx| cx.dispatch_action(ToggleQuote)), "> alpha beta"),
        ];
        for (fire, expect) in cases {
            select(&editor, cx, 0..5);
            fire(cx);
            assert_eq!(buffer_text(&editor, cx), expect);
            cx.dispatch_action(Undo);
            assert_eq!(buffer_text(&editor, cx), "alpha beta");
        }
    }

    #[gpui::test]
    fn bold_needs_a_selection_and_falls_through_to_the_next_binding(cx: &mut TestAppContext) {
        // Newline stands in for the app's global cmd-b (sidebar toggle):
        // with no selection the keystroke must reach the next binding.
        let (_fx, editor, cx) = open_editor(cx, "fmt.md", "hello");
        cx.update(|_, app| {
            app.bind_keys([
                KeyBinding::new("cmd-b", Newline, None),
                KeyBinding::new("cmd-b", ToggleBold, Some("Editor")),
            ]);
        });
        cx.simulate_keystrokes("cmd-b");
        assert_eq!(buffer_text(&editor, cx), "\nhello", "cursor-only cmd-b fell through");
        select(&editor, cx, 1..6);
        cx.simulate_keystrokes("cmd-b");
        assert_eq!(buffer_text(&editor, cx), "\n**hello**", "selection cmd-b bolds");
    }

    #[gpui::test]
    fn formatting_is_inert_outside_markdown(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "code.rs", "let x = 1;");
        select(&editor, cx, 0..3);
        cx.dispatch_action(ToggleBold);
        cx.dispatch_action(CycleHeading);
        cx.dispatch_action(ToggleQuote);
        assert_eq!(buffer_text(&editor, cx), "let x = 1;");
    }

    #[gpui::test]
    fn save_now_writes_the_file_and_backs_up_the_original(cx: &mut TestAppContext) {
        let (fx, editor, cx) = open_editor(cx, "save.md", "v1\n");
        // A clean buffer has nothing to flush: no write, no backup.
        cx.dispatch_action(SaveNow);
        assert!(fx.backup_contents().is_empty());
        assert_eq!(std::fs::read_to_string(&fx.path).unwrap(), "v1\n");

        cx.simulate_input("new ");
        cx.dispatch_action(SaveNow);
        assert_eq!(std::fs::read_to_string(&fx.path).unwrap(), "new v1\n");
        assert_eq!(fx.backup_contents(), vec!["v1\n".to_string()]);
        cx.update(|_, app| assert!(!editor.read(app).save.is_dirty()));

        // Second save in the same session: no second backup of the file.
        cx.simulate_input("more ");
        cx.dispatch_action(SaveNow);
        assert_eq!(std::fs::read_to_string(&fx.path).unwrap(), "new more v1\n");
        assert_eq!(fx.backup_contents().len(), 1);
    }

    #[gpui::test]
    fn external_disk_change_is_backed_up_before_overwrite(cx: &mut TestAppContext) {
        let (fx, editor, cx) = open_editor(cx, "conflict.md", "ours\n");
        cx.simulate_input("A");
        assert_eq!(buffer_text(&editor, cx), "Aours\n");

        // Simulate an external edit; push mtime clearly forward so the
        // conflict check never races sub-second timestamp granularity.
        std::fs::write(&fx.path, "theirs\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        let f = std::fs::File::options().write(true).open(&fx.path).unwrap();
        f.set_modified(later).unwrap();

        cx.dispatch_action(SaveNow);
        assert_eq!(
            std::fs::read_to_string(&fx.path).unwrap(),
            "Aours\n",
            "our buffer wins the write"
        );
        assert_eq!(
            fx.backup_contents(),
            vec!["theirs\n".to_string()],
            "the clobbered disk version is backed up first"
        );
    }

    #[gpui::test]
    fn debounce_timer_rechecks_before_flushing(cx: &mut TestAppContext) {
        let (fx, editor, cx) = open_editor(cx, "note.md", "");
        cx.simulate_input("z");
        // Fire the debounce timer (test clock) while the wall clock says
        // the last edit was a moment ago: should_flush's re-check must
        // decline, keeping the buffer dirty and the disk untouched.
        cx.background_executor
            .advance_clock(autosave::DEBOUNCE + std::time::Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|_, app| assert!(editor.read(app).save.is_dirty()));
        assert_eq!(std::fs::read_to_string(&fx.path).unwrap(), "");
    }

    #[gpui::test]
    fn find_opens_matches_cycles_and_closes(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "one two one\nstone\n");
        cx.dispatch_action(OpenFind);
        cx.update(|_, app| {
            let state = editor.read(app).find.as_ref().expect("find bar open");
            assert!(state.matches.is_empty(), "empty query matches nothing");
        });

        editor.update_in(cx, |ed, _, cx| {
            let input = ed.find.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| {
                input.content = "one".into();
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            let state = ed.find.as_ref().unwrap();
            assert_eq!(state.matches, vec![0..3, 8..11, 14..17]);
            assert_eq!(state.active, 0);
        });

        cx.dispatch_action(FindNext);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.core.selection.range(), 8..11);
            assert_eq!(ed.core.selected_text(), "one");
        });
        cx.dispatch_action(FindNext);
        cx.dispatch_action(FindNext);
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selection.range(), 0..3, "next wraps around")
        });
        cx.dispatch_action(FindPrev);
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selection.range(), 14..17, "prev wraps back")
        });

        cx.dispatch_action(CloseFind);
        cx.update(|window, app| {
            let ed = editor.read(app);
            assert!(ed.find.is_none());
            assert!(ed.focus_handle.is_focused(window), "close refocuses the editor");
        });
    }

    /// Replace All is one undo entry. Stepping back through a hundred
    /// replacements one at a time is not undo.
    #[gpui::test]
    fn replace_all_is_a_single_undo_entry(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "a cat b cat c cat\n");
        editor.update(cx, |ed, cx| {
            let text = ed.core.buffer.text();
            let e = crate::editor::replace::replace_all(&text, "cat", "dog").expect("matches");
            ed.core.replace_range(e.range.clone(), &e.replacement, std::time::Instant::now());
            cx.notify();
        });
        editor.update(cx, |ed, _| {
            assert_eq!(ed.core.buffer.text(), "a dog b dog c dog\n");
            ed.core.undo();
            assert_eq!(
                ed.core.buffer.text(),
                "a cat b cat c cat\n",
                "one undo takes back the whole Replace All"
            );
        });
    }

    /// First press of ⌘⌥E only reveals the replace field; the second
    /// press, once it has text, replaces the active match.
    #[gpui::test]
    fn replace_next_reveals_the_field_then_replaces_the_active_match(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "a cat b cat\n");
        cx.dispatch_action(OpenFind);
        editor.update_in(cx, |ed, _, cx| {
            let input = ed.find.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| {
                input.content = "cat".into();
                cx.notify();
            });
        });
        cx.run_until_parked();

        cx.dispatch_action(ReplaceNext);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.find.as_ref().unwrap().replacing, "field revealed");
            assert_eq!(ed.core.buffer.text(), "a cat b cat\n", "no edit on the reveal press");
        });

        editor.update_in(cx, |ed, _, cx| {
            let replace_input = ed.find.as_ref().unwrap().replace_input.clone();
            replace_input.update(cx, |input, cx| {
                input.content = "dog".into();
                cx.notify();
            });
        });

        cx.dispatch_action(ReplaceNext);
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).core.buffer.text(),
                "a dog b cat\n",
                "only the active match is replaced"
            );
        });
    }

    /// ⌘⌥⇧E rewrites every match through the same reveal-then-act flow,
    /// and the whole thing is one undo entry.
    #[gpui::test]
    fn replace_all_reveals_the_field_then_replaces_every_match(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "a cat b cat c cat\n");
        cx.dispatch_action(OpenFind);
        editor.update_in(cx, |ed, _, cx| {
            let input = ed.find.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| {
                input.content = "cat".into();
                cx.notify();
            });
        });
        cx.run_until_parked();

        cx.dispatch_action(ReplaceAll);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.find.as_ref().unwrap().replacing, "field revealed");
            assert_eq!(ed.core.buffer.text(), "a cat b cat c cat\n", "no edit on the reveal press");
        });

        editor.update_in(cx, |ed, _, cx| {
            let replace_input = ed.find.as_ref().unwrap().replace_input.clone();
            replace_input.update(cx, |input, cx| {
                input.content = "dog".into();
                cx.notify();
            });
        });

        cx.dispatch_action(ReplaceAll);
        editor.update(cx, |ed, _| {
            assert_eq!(ed.core.buffer.text(), "a dog b dog c dog\n");
            ed.core.undo();
            assert_eq!(
                ed.core.buffer.text(),
                "a cat b cat c cat\n",
                "one undo takes back the whole Replace All"
            );
        });
    }

    /// A different replace action must not piggyback on a field that
    /// another action revealed: ReplaceNext reveals the field, then
    /// ReplaceAll (with nothing typed) must not fire -- the shared
    /// "is the field visible" flag is not permission to act.
    #[gpui::test]
    fn replace_next_then_replace_all_with_an_empty_field_is_a_no_op(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "a cat b cat c cat\n");
        cx.dispatch_action(OpenFind);
        editor.update_in(cx, |ed, _, cx| {
            let input = ed.find.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| {
                input.content = "cat".into();
                cx.notify();
            });
        });
        cx.run_until_parked();

        cx.dispatch_action(ReplaceNext); // reveals the field, no edit
        cx.dispatch_action(ReplaceAll); // must not fire: field is empty
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).core.buffer.text(),
                "a cat b cat c cat\n",
                "an empty replace field must never wipe every match"
            );
        });
    }

    /// Same bug, other order: ReplaceAll reveals the field, then
    /// ReplaceNext (with nothing typed) must not fire.
    #[gpui::test]
    fn replace_all_then_replace_next_with_an_empty_field_is_a_no_op(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "a cat b cat c cat\n");
        cx.dispatch_action(OpenFind);
        editor.update_in(cx, |ed, _, cx| {
            let input = ed.find.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| {
                input.content = "cat".into();
                cx.notify();
            });
        });
        cx.run_until_parked();

        cx.dispatch_action(ReplaceAll); // reveals the field, no edit
        cx.dispatch_action(ReplaceNext); // must not fire: field is empty
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).core.buffer.text(),
                "a cat b cat c cat\n",
                "an empty replace field must never wipe a match"
            );
        });
    }

    /// Pressing the very same replace action twice with the field
    /// left empty is deliberately still a no-op: this codebase does
    /// not treat "asked twice" as consent to delete every match.
    /// Typing an actual (even empty-after-edit) intent is the only
    /// way to confirm a replacement -- see the comment on
    /// `replace_next`/`replace_all`'s content gate.
    #[gpui::test]
    fn the_same_replace_action_twice_with_an_empty_field_stays_a_no_op(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "a cat b cat c cat\n");
        cx.dispatch_action(OpenFind);
        editor.update_in(cx, |ed, _, cx| {
            let input = ed.find.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| {
                input.content = "cat".into();
                cx.notify();
            });
        });
        cx.run_until_parked();

        cx.dispatch_action(ReplaceAll); // reveals the field, no edit
        cx.dispatch_action(ReplaceAll); // still empty: still no edit
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).core.buffer.text(),
                "a cat b cat c cat\n",
                "repeating the same shortcut on an empty field is not consent to delete"
            );
        });
    }

    #[gpui::test]
    fn markdown_projects_widgets_that_dissolve_under_the_cursor(cx: &mut TestAppContext) {
        let src = "# Title\n\n|a|b|\n|-|-|\n|1|2|\n\n```mermaid\nflowchart LR\n a-->b\n```\n";
        let (_fx, editor, cx) = open_editor(cx, "doc.md", src);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(!ed.is_code_mode());
            assert_eq!(ed.title().as_ref(), "doc.md");
            assert_eq!(ed.heading_lines(), vec![(1, "Title".to_string(), 0)]);
        });
        assert_eq!(
            widget_count(&editor, cx),
            2,
            "table and mermaid fence each project a widget"
        );

        // Cursor inside the table dissolves that widget back to source.
        let row_start =
            cx.update(|_, app| editor.read(app).core.buffer.line_range(4).start);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(row_start);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(widget_count(&editor, cx), 1, "table dissolved, diagram remains");

        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(0);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(widget_count(&editor, cx), 2, "leaving the table re-forms it");
    }

    #[gpui::test]
    fn code_mode_newline_copies_leading_indentation(cx: &mut TestAppContext) {
        let (_fx, editor, cx) =
            open_editor(cx, "main.rs", "fn main() {\n    let x = 1;\n}");
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.is_code_mode());
            assert_eq!(ed.gutter_label(1), "2");
            assert!(ed.projection.iter().all(|i| matches!(i, projection::Item::Line(_))));
        });
        let line1_end = cx.update(|_, app| editor.read(app).core.buffer.line_range(1).end);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(line1_end);
            cx.notify();
        });
        cx.dispatch_action(Newline);
        assert_eq!(
            buffer_text(&editor, cx),
            "fn main() {\n    let x = 1;\n    \n}",
            "newline auto-indents in code mode"
        );
    }

    #[gpui::test]
    fn diff_mode_outside_a_repo_shows_placeholder_and_exits(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "diff.md", "hello\n");
        editor.update_in(cx, |ed, _, cx| {
            let langs = crate::highlight::languages(cx);
            ed.enter_diff(&langs, cx);
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.diff_active());
            let d = ed.diff.as_ref().unwrap();
            let Some(crate::git::Baseline::NotInRepo) = d.missing else {
                panic!("tempdir must not resolve a git baseline")
            };
        });
        editor.update_in(cx, |ed, _, cx| ed.exit_diff(cx));
        cx.run_until_parked();
        cx.update(|_, app| assert!(!editor.read(app).diff_active()));
    }

    #[gpui::test]
    fn reload_from_disk_replaces_buffer_and_clamps_cursor(cx: &mut TestAppContext) {
        let (fx, editor, cx) = open_editor(cx, "note.md", "one two three\n");
        cx.dispatch_action(DocEnd);
        cx.simulate_input("!");
        std::fs::write(&fx.path, "short\n").unwrap();
        editor.update_in(cx, |ed, _, cx| ed.reload_from_disk(cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.text(), "short\n");
            assert_eq!(ed.core.selection.head, 6, "cursor clamps into the new text");
            assert!(!ed.save.is_dirty(), "reload resets the save policy");
        });
    }

    /// A press belongs to the document that was on screen when it
    /// happened. Neither survives the buffer being swapped or the
    /// editor leaving the screen.
    #[gpui::test]
    fn a_reload_clears_a_pending_press(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "see [[Target]] here\n");
        editor.update(cx, |ed, _| {
            ed.pending_link =
                ed.link_at_offset(6).cloned().map(|link| PendingLink { offset: 6, link });
            assert!(ed.pending_link.is_some(), "precondition");
        });
        editor.update(cx, |ed, cx| ed.reload_from_disk(cx));
        editor.update(cx, |ed, _| {
            assert!(ed.pending_link.is_none(), "a reload drops the press");
            assert!(ed.hover_link.is_none(), "and the hover it belonged to");
        });
    }

    /// The other way a press can outlive the document it was made on:
    /// the editor loses focus (a tab switch, in the app) before the
    /// release ever reaches it. Returning and releasing over blank
    /// space must not still be holding that press.
    #[gpui::test]
    fn losing_focus_clears_a_pending_press(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "see [[Target]] here\n");
        // `on_blur` only fires for a window the platform considers
        // active -- true of any real window that has ever been shown,
        // but a test window starts inactive until told otherwise.
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        editor.update(cx, |ed, _| {
            ed.pending_link =
                ed.link_at_offset(6).cloned().map(|link| PendingLink { offset: 6, link });
            ed.hover_link = ed.link_at_offset(6).cloned();
            assert!(ed.pending_link.is_some(), "precondition");
            assert!(ed.hover_link.is_some(), "precondition");
        });
        // Something else takes focus -- in the app, a tab switch moves
        // it to the newly active document's own handle.
        let elsewhere = cx.update(|_, app| app.focus_handle());
        cx.update(|window, _| window.focus(&elsewhere));
        cx.run_until_parked();
        editor.update(cx, |ed, _| {
            assert!(ed.pending_link.is_none(), "losing focus drops the press");
            assert!(ed.hover_link.is_none(), "and the hover it belonged to");
        });
    }

    #[gpui::test]
    fn ime_marked_text_composes_and_commits(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "");
        editor.update_in(cx, |ed, window, cx| {
            ed.replace_and_mark_text_in_range(None, "ni", None, window, cx);
        });
        cx.update(|_, app| assert_eq!(editor.read(app).text(), "ni"));
        let marked = editor.update_in(cx, |ed, window, cx| ed.marked_text_range(window, cx));
        assert_eq!(marked, Some(0..2));

        // Committing replaces the composition with multibyte text; the
        // selection round-trips through UTF-16 offsets.
        editor.update_in(cx, |ed, window, cx| {
            ed.replace_text_in_range(None, "\u{4f60}", window, cx);
        });
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.text(), "\u{4f60}");
            assert_eq!(ed.core.selection.head, 3, "cursor sits after the 3-byte char");
        });
        let marked = editor.update_in(cx, |ed, window, cx| ed.marked_text_range(window, cx));
        assert_eq!(marked, None, "commit clears the composition");
        let sel = editor
            .update_in(cx, |ed, window, cx| ed.selected_text_range(false, window, cx))
            .unwrap();
        assert_eq!(sel.range, 1..1, "UTF-16 offset for a BMP CJK char is 1");
    }

    // ── helpers for path-based fixtures (git repos, bad backup roots) ──

    use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase};

    /// Open an editor on an existing file at `path`, with fresh globals
    /// whose backups live in the returned tempdir.
    fn open_editor_path<'a>(
        cx: &'a mut TestAppContext,
        path: &Path,
    ) -> (tempfile::TempDir, Entity<Editor>, &'a mut VisualTestContext) {
        let backups = tempfile::tempdir().unwrap();
        let langs = Arc::new(Languages::new());
        let backup_root = backups.path().join("backups");
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(
                crate::theme::Theme::dark(),
            )));
            cx.set_global(crate::highlight::SyntaxLanguages(langs.clone()));
            cx.set_global(SessionBackups(Arc::new(Mutex::new(
                autosave::BackupRegistry::new(backup_root),
            ))));
        });
        let contents = std::fs::read_to_string(path).unwrap();
        let file = path.to_path_buf();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| Editor::from_text(&file, contents, &langs, cx));
        cx.update(|window, app| {
            let handle = editor.read(app).focus_handle.clone();
            window.focus(&handle);
        });
        attach_workspace_handles(&editor, cx);
        cx.run_until_parked();
        (backups, editor, cx)
    }

    /// An image's destination is anchored to the document that wrote
    /// it, and a destination with nothing behind it says so rather
    /// than quietly drawing nothing. The reading view asks the same
    /// function with the same anchor (#57).
    #[gpui::test]
    fn an_images_path_is_anchored_to_its_own_document(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets").join("pic.png"), b"x").unwrap();
        let note = dir.path().join("note.md");
        std::fs::write(&note, "![p](assets/pic.png)\n\n![q](assets/gone.png)\n").unwrap();
        let (_backups, editor, cx) = open_editor_path(cx, &note);
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).image_source("assets/pic.png"),
                crate::markdown::ImageSource::Local(dir.path().join("assets").join("pic.png"))
            );
            assert!(matches!(
                editor.read(app).image_source("assets/gone.png"),
                crate::markdown::ImageSource::Missing(_)
            ));
            assert_eq!(
                editor.read(app).image_source("https://example.com/r.png"),
                crate::markdown::ImageSource::Remote("https://example.com/r.png".to_string())
            );
        });
    }

    /// Author git fixtures with the system CLI (same approach as
    /// src/git.rs tests) so no git library shows up in fixture setup.
    fn sh_git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn commit_all(dir: &Path) {
        sh_git(dir, &["add", "-A"]);
        sh_git(dir, &["commit", "-qm", "commit"]);
    }

    /// Window-space point for a display index on a painted line, biased
    /// one pixel into the glyph so hit-testing is unambiguous.
    fn point_for_index(
        editor: &Entity<Editor>,
        cx: &mut VisualTestContext,
        line_ix: usize,
        disp_ix: usize,
    ) -> Point<Pixels> {
        cx.update(|_, app| {
            let ed = editor.read(app);
            let entry = ed.layout_cache.get(&line_ix).expect("line painted");
            let lh = entry.line_height;
            let pos = entry.line.position_for_index(disp_ix, lh).expect("index in line");
            point(
                entry.origin.x + pos.x + px(1.),
                entry.origin.y + pos.y + lh * 0.5,
            )
        })
    }

    fn scroll_offset_y(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Pixels {
        cx.update(|_, app| {
            -editor
                .read(app)
                .list_state
                .scroll_px_offset_for_scrollbar()
                .y
        })
    }

    // ── plain files and remaining action handlers ──────────────────────

    #[gpui::test]
    fn plain_text_files_use_code_layout_without_highlighting(cx: &mut TestAppContext) {
        let (fx, editor, cx) = open_editor(cx, "notes.txt", "alpha\nbeta\n");
        assert_eq!(Editor::read_file(&fx.path).unwrap(), "alpha\nbeta\n");
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.is_code_mode(), "plain files render mono with a gutter");
            assert!(ed.spans.is_empty(), "no styling spans for plain text");
            assert_eq!(ed.gutter_label(0), "1");
            assert_eq!(
                Focusable::focus_handle(ed, app),
                ed.focus_handle,
                "the trait hands out the editor's own focus handle"
            );
        });
        cx.simulate_input("x");
        assert_eq!(buffer_text(&editor, cx), "xalpha\nbeta\n");
        // Scrubbing a document that does not overflow is a no-op.
        editor.update_in(cx, |ed, _, cx| {
            ed.scrollbar_scrub(point(px(0.), px(0.)), cx);
        });
        assert_eq!(scroll_offset_y(&editor, cx), px(0.));
    }

    #[test]
    fn syntax_color_maps_every_known_capture_root() {
        let t = crate::theme::Theme::dark();
        let colored = (0..crate::highlight::CAPTURE_NAMES.len())
            .filter(|ix| Editor::syntax_color(*ix as u8, &t).is_some())
            .count();
        assert!(colored > 0, "capture names resolve to syntax colors");
        // Out-of-range captures resolve to no color at all.
        assert!(Editor::syntax_color(u8::MAX, &t).is_none());
    }

    #[gpui::test]
    fn remaining_movement_and_selection_actions(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "alpha beta\ngamma delta\nomega\n");
        // Arrow over an active selection collapses to its edges.
        cx.dispatch_action(SelectWordRight);
        cx.dispatch_action(MoveLeft);
        assert_eq!(head(&editor, cx), 0, "left collapses to selection start");
        cx.dispatch_action(SelectWordRight);
        cx.dispatch_action(MoveRight);
        assert_eq!(head(&editor, cx), 5, "right collapses to selection end");
        // Plain cursor arrows.
        cx.dispatch_action(MoveLeft);
        assert_eq!(head(&editor, cx), 4);
        // Grapheme-wise selection.
        cx.dispatch_action(SelectRight);
        cx.update(|_, app| assert_eq!(editor.read(app).core.selection.range(), 4..5));
        cx.dispatch_action(SelectLeft);
        cx.update(|_, app| assert!(editor.read(app).core.selection.is_cursor()));
        // Word-wise movement and selection.
        cx.dispatch_action(MoveWordRight);
        assert_eq!(head(&editor, cx), 5, "word-right lands at the end of alpha");
        cx.dispatch_action(MoveWordRight);
        assert_eq!(head(&editor, cx), 10, "word-right lands at the end of beta");
        cx.dispatch_action(SelectWordLeft);
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selected_text(), "beta");
        });
        // Line-edge selection extends from the existing anchor (10).
        cx.dispatch_action(SelectLineEnd);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.core.selection.head, 10, "head extends to line end");
        });
        cx.dispatch_action(SelectLineStart);
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selection.range(), 0..10);
        });
        // Vertical selection through the painted geometry.
        cx.dispatch_action(DocStart);
        cx.dispatch_action(SelectDown);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.core.buffer.line_of_byte(ed.core.selection.head), 1);
            assert_eq!(ed.core.selection.anchor, 0);
        });
        cx.dispatch_action(SelectUp);
        cx.update(|_, app| assert!(editor.read(app).core.selection.is_cursor()));
        // Page movement clamps to the document.
        cx.dispatch_action(PageDown);
        cx.update(|_, app| {
            let ed = editor.read(app);
            let last = ed.core.buffer.line_count() - 1;
            assert_eq!(ed.core.buffer.line_of_byte(ed.core.selection.head), last);
        });
        cx.dispatch_action(PageUp);
        assert_eq!(head(&editor, cx), 0);
    }

    // ── mouse: click, drag-select, shift-click ─────────────────────────

    #[gpui::test]
    fn mouse_click_places_cursor_and_drag_selects(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "alpha beta gamma\nsecond line\n");
        let p2 = point_for_index(&editor, cx, 0, 2);
        let p10 = point_for_index(&editor, cx, 0, 10);
        let p14 = point_for_index(&editor, cx, 0, 14);

        cx.simulate_mouse_down(p2, MouseButton::Left, Modifiers::none());
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.dragging, "mouse down starts a drag");
            assert_eq!(ed.core.selection.head, 2);
            assert!(ed.core.selection.is_cursor());
        });
        cx.simulate_mouse_move(p10, MouseButton::Left, Modifiers::none());
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selection.range(), 2..10, "drag extends");
        });
        cx.simulate_mouse_up(p10, MouseButton::Left, Modifiers::none());
        cx.update(|_, app| assert!(!editor.read(app).dragging));
        // Moving without a pressed button changes nothing.
        cx.simulate_mouse_move(p2, None, Modifiers::none());
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selection.range(), 2..10);
        });
        // Shift-click extends from the existing anchor.
        cx.simulate_mouse_down(p14, MouseButton::Left, Modifiers::shift());
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selection.range(), 2..14);
        });
        // Dragging below every painted line falls back to the nearest
        // line and extends the selection toward the end.
        let below = cx.update(|_, app| {
            let ed = editor.read(app);
            let bottom = ed
                .layout_cache
                .values()
                .map(|e| e.origin.y + e.line.size(e.line_height).height)
                .fold(px(0.), Pixels::max);
            point(p14.x, bottom + px(50.))
        });
        cx.simulate_mouse_move(below, MouseButton::Left, Modifiers::shift());
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(
                ed.core.buffer.line_of_byte(ed.core.selection.head) >= 1,
                "the head snapped to the closest (last) line"
            );
        });
        cx.simulate_mouse_up(below, MouseButton::Left, Modifiers::shift());
    }

    #[gpui::test]
    fn enter_continues_lists_and_empty_items_end_them(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "list.md", "- milk");
        cx.dispatch_action(DocEnd);
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "- milk\n- ");
        cx.simulate_input("eggs");
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "- milk\n- eggs\n- ");
        // Enter on the empty item removes the marker and ends the list.
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "- milk\n- eggs\n");

        // Numbers increment; tasks continue unchecked; indent carries.
        let (_fx2, editor, cx) = open_editor(cx, "more.md", "  3. three");
        cx.dispatch_action(DocEnd);
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "  3. three\n  4. ");
        let (_fx3, editor, cx) = open_editor(cx, "task.md", "- [x] done");
        cx.dispatch_action(DocEnd);
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "- [x] done\n- [ ] ");
    }

    #[gpui::test]
    fn enter_off_lists_and_in_code_keeps_old_behavior(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "plain.md", "hello");
        cx.dispatch_action(DocEnd);
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "hello\n");
        // Code mode still auto-indents instead of continuing lists.
        let (_fx2, editor, cx) = open_editor(cx, "code.rs", "    - x");
        cx.dispatch_action(DocEnd);
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "    - x\n    ");
    }

    #[gpui::test]
    fn tab_indents_list_items_and_shift_tab_outdents(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "list.md", "- a\n- bike");
        cx.dispatch_action(DocEnd);
        cx.dispatch_action(InsertTab);
        assert_eq!(buffer_text(&editor, cx), "- a\n  - bike");
        assert_eq!(head(&editor, cx), 12, "cursor rides the shifted line");
        cx.dispatch_action(InsertTab);
        assert_eq!(buffer_text(&editor, cx), "- a\n    - bike");
        cx.dispatch_action(Outdent);
        cx.dispatch_action(Outdent);
        assert_eq!(buffer_text(&editor, cx), "- a\n- bike");
        cx.dispatch_action(Outdent);
        assert_eq!(buffer_text(&editor, cx), "- a\n- bike", "flat items stay put");

        // Outside a list, Tab still types a literal tab.
        let (_fx2, editor, cx) = open_editor(cx, "plain.md", "text");
        cx.dispatch_action(DocEnd);
        cx.dispatch_action(InsertTab);
        assert_eq!(buffer_text(&editor, cx), "text\t");
    }

    /// Scan `root` and register it as the global knowledge index.
    fn index_workspace(cx: &mut TestAppContext, root: &Path) {
        let index = crate::knowledge::Index::scan(root);
        cx.update(|cx| {
            cx.set_global(crate::knowledge::KnowledgeState(Arc::new(Mutex::new(index))));
        });
    }

    /// A two-note knowledge workspace registered as the global index.
    fn knowledge_fixture(cx: &mut TestAppContext) -> tempfile::TempDir {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("Roadmap.md"), "the plan\n").unwrap();
        std::fs::write(ws.path().join("Recipes.md"), "the food\n").unwrap();
        index_workspace(cx, ws.path());
        ws
    }

    /// Every path an editor emitted through `EditorEvent::OpenPath`.
    fn open_path_sink(
        cx: &mut VisualTestContext,
        editor: &Entity<Editor>,
    ) -> Rc<RefCell<Vec<PathBuf>>> {
        let opened: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
        let sink = opened.clone();
        cx.update(|_, app| {
            app.subscribe(editor, move |_, event: &EditorEvent, _| {
                if let EditorEvent::OpenPath(p) = event {
                    sink.borrow_mut().push(p.clone());
                }
            })
            .detach();
        });
        opened
    }

    /// Every refusal an editor reported through
    /// `EditorEvent::CommandError` — the workspace turns each into
    /// `show_command_error`.
    fn command_error_sink(
        cx: &mut VisualTestContext,
        editor: &Entity<Editor>,
    ) -> Rc<RefCell<Vec<String>>> {
        let said: Rc<RefCell<Vec<String>>> = Rc::default();
        let sink = said.clone();
        cx.update(|_, app| {
            app.subscribe(editor, move |_, event: &EditorEvent, _| {
                if let EditorEvent::CommandError(msg) = event {
                    sink.borrow_mut().push(msg.clone());
                }
            })
            .detach();
        });
        said
    }

    // ── right-click context menu ───────────────────────────────────────

    /// Every `EditorEvent::ContextMenu` an editor raised.
    fn menu_sink(
        cx: &mut VisualTestContext,
        editor: &Entity<Editor>,
    ) -> Rc<RefCell<Vec<(Point<Pixels>, crate::menus::EditorContext)>>> {
        let raised: Rc<RefCell<Vec<(Point<Pixels>, crate::menus::EditorContext)>>> = Rc::default();
        let sink = raised.clone();
        cx.update(|_, app| {
            app.subscribe(editor, move |_, event: &EditorEvent, _| {
                if let EditorEvent::ContextMenu { position, ctx } = event {
                    sink.borrow_mut().push((*position, *ctx));
                }
            })
            .detach();
        });
        raised
    }

    /// Put the caret at `at` and repaint, so the lines it reveals are
    /// in the layout cache and can be clicked.
    fn caret_and_draw(editor: &Entity<Editor>, cx: &mut VisualTestContext, at: usize) {
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(at);
            cx.notify();
        });
        cx.run_until_parked();
    }

    /// The rows a raised menu would actually draw.
    fn menu_ids(ctx: crate::menus::EditorContext) -> Vec<&'static str> {
        crate::menus::items_for(crate::menus::Surface::Editor, ctx)
            .into_iter()
            .map(|i| i.id)
            .collect()
    }

    /// The reported bug, end to end: the user clicks into a table, gets
    /// the raw Markdown as designed, right-clicks to add a row — and
    /// until now nothing happened, because no `MouseButton::Right`
    /// handler existed anywhere in the editor. The five commands all
    /// ship with `keys: []`, so this menu is their only pointer surface.
    #[gpui::test]
    fn a_right_click_in_a_table_raises_the_table_commands(cx: &mut TestAppContext) {
        let doc = "intro\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let (_fx, editor, cx) = open_editor(cx, "t.md", doc);
        let raised = menu_sink(cx, &editor);
        caret_and_draw(&editor, cx, doc.find('1').unwrap());

        let p = point_for_index(&editor, cx, 4, 2);
        cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());

        let raised = raised.borrow();
        let (pos, ctx) = *raised.first().expect("a right-click raises the menu");
        assert_eq!(pos, p, "the menu opens where the press landed");
        assert!(ctx.in_table, "the caret is in a table cell: {ctx:?}");
        let ids = menu_ids(ctx);
        for expected in [
            "table_insert_row",
            "table_delete_row",
            "table_insert_column",
            "table_delete_column",
        ] {
            assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
        }
    }

    /// A right-click inside an existing selection keeps it. Collapsing
    /// the caret to the press is the usual way this breaks: the user
    /// selects a phrase, right-clicks it, picks Bold, and the command
    /// runs on an empty cursor.
    #[gpui::test]
    fn a_right_click_inside_the_selection_keeps_it(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "sel.md", "alpha beta gamma\n");
        let _raised = menu_sink(cx, &editor);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.selection = Selection { anchor: 0, head: 10 };
            cx.notify();
        });
        cx.run_until_parked();

        let inside = point_for_index(&editor, cx, 0, 5);
        cx.simulate_mouse_down(inside, MouseButton::Right, Modifiers::none());
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).core.selection.range(),
                0..10,
                "the selection the menu is about survives the press that opened it",
            );
        });

        // Outside it, the caret does move — the menu is about the new
        // spot, and leaving a far-away selection standing would be just
        // as wrong.
        let outside = point_for_index(&editor, cx, 0, 14);
        cx.simulate_mouse_down(outside, MouseButton::Right, Modifiers::none());
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.core.selection.is_cursor(), "a press outside collapses");
            assert_eq!(ed.core.selection.head, 14);
        });
    }

    /// A right press must not navigate, and must not leave a primed
    /// link behind for the next left release to follow.
    #[gpui::test]
    fn a_right_click_on_a_link_neither_follows_nor_arms_it(cx: &mut TestAppContext) {
        let ws = knowledge_fixture(cx);
        let note = ws.path().join("note.md");
        std::fs::write(&note, "see [[Roadmap]] now\n").unwrap();
        index_workspace(cx, ws.path());
        let (_bk, editor, cx) = open_editor_path(cx, &note);
        let opened = open_path_sink(cx, &editor);
        let raised = menu_sink(cx, &editor);
        cx.run_until_parked();

        let p = point_for_index(&editor, cx, 0, 7);
        cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(p, MouseButton::Right, Modifiers::none());
        cx.run_until_parked();

        assert!(opened.borrow().is_empty(), "a right-click never navigates");
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.pending_link.is_none(), "no link is left primed for a later release");
            assert!(!ed.dragging, "a right press is not the start of a drag");
        });

        // And it clears one it finds: a left press arms the link and
        // starts a drag, both of which the release acts on. A right
        // press in between ends that interaction — the release that
        // follows must not navigate on a press the user abandoned.
        // (The caret goes back off the link first: a *revealed* link is
        // being edited and a plain left click does not arm it.)
        caret_and_draw(&editor, cx, 0);
        let p = point_for_index(&editor, cx, 0, 7);
        cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
        cx.update(|_, app| assert!(editor.read(app).pending_link.is_some(), "the left press armed it"));
        let elsewhere = point_for_index(&editor, cx, 0, 13);
        cx.simulate_mouse_down(elsewhere, MouseButton::Right, Modifiers::none());
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.pending_link.is_none(), "the right press cleared the primed link");
            assert!(!ed.dragging, "and ended the drag it would have extended");
        });
        cx.simulate_mouse_up(elsewhere, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        assert!(opened.borrow().is_empty(), "the abandoned press still never navigates");
        let ctx = raised.borrow().first().expect("the menu opened").1;
        assert!(ctx.on_link, "the press was on a link: {ctx:?}");
        assert!(menu_ids(ctx).contains(&"follow_link"), "{:?}", menu_ids(ctx));
    }

    /// Off a link, Follow Link is not offered — a dead row is exactly
    /// the Format-menu failure this whole menu exists to fix.
    #[gpui::test]
    fn prose_offers_the_toggles_and_nothing_dead(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "prose.md", "just some words here\n");
        let raised = menu_sink(cx, &editor);
        // The menu's rows dispatch their action through the window, so
        // they land wherever focus is. A right-click in a document the
        // user was not typing in (the sidebar had focus) has to take it.
        cx.update(|window, _| window.blur());
        cx.run_until_parked();
        cx.update(|window, app| {
            assert!(!editor.read(app).focus_handle.is_focused(window), "the premise: focus is elsewhere");
        });
        let p = point_for_index(&editor, cx, 0, 5);
        cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());

        let ctx = raised.borrow().first().expect("the menu opened").1;
        assert_eq!(menu_ids(ctx), vec!["bold", "italic"], "{ctx:?}");
        cx.update(|window, app| {
            assert!(
                editor.read(app).focus_handle.is_focused(window),
                "the press focused the editor, so the row it opens can reach it",
            );
        });
    }

    /// An ordered list offers Renumber List; a fenced code block that
    /// happens to hold numbers does not — `renumber_block` skips fence
    /// bodies, and the menu asks it rather than guessing.
    #[gpui::test]
    fn an_ordered_list_offers_renumber_but_a_fence_does_not(cx: &mut TestAppContext) {
        let doc = "1. one\n1. two\n\n```\n1. not a list\n```\n";
        let (_fx, editor, cx) = open_editor(cx, "list.md", doc);
        let raised = menu_sink(cx, &editor);

        caret_and_draw(&editor, cx, 3);
        let p = point_for_index(&editor, cx, 0, 3);
        cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());
        let ctx = raised.borrow().last().expect("the menu opened").1;
        assert!(ctx.in_ordered_list, "{ctx:?}");
        assert!(menu_ids(ctx).contains(&"renumber_list"), "{:?}", menu_ids(ctx));

        // Inside the fence: numbers there are the user's literal text.
        let inside = doc.find("not a list").unwrap();
        caret_and_draw(&editor, cx, inside);
        let fence_line = cx.update(|_, app| editor.read(app).core.buffer.line_of_byte(inside));
        let p = point_for_index(&editor, cx, fence_line, 2);
        cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());
        let ctx = raised.borrow().last().expect("the menu opened").1;
        assert!(!ctx.in_ordered_list, "a fence is not a list: {ctx:?}");
        let ids = menu_ids(ctx);
        assert!(!ids.contains(&"renumber_list"), "{ids:?}");
        assert!(!ids.contains(&"table_insert_row"), "{ids:?}");
    }

    /// A code file takes none of these commands (`can_format()` is
    /// false for every one of them), so no menu opens at all: an empty
    /// overlay is worse than none.
    #[gpui::test]
    fn a_code_file_raises_no_menu(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "main.rs", "fn main() {}\n");
        let raised = menu_sink(cx, &editor);
        let p = point_for_index(&editor, cx, 0, 3);
        cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());
        assert!(raised.borrow().is_empty(), "a code file offers no editor commands");
        cx.update(|_, app| {
            let ctx = editor.read(app).menu_context(3);
            assert!(menu_ids(ctx).is_empty(), "{ctx:?}");
        });
    }

    /// Following `[[drafts/secret]]` must never zero the file it names.
    /// `drafts/` is gitignored, so the scan never indexes it and the
    /// wiki target resolves to `None` — straight into the create branch,
    /// which used to `fs::write(path, "")` over the real file. Nothing
    /// about this needs an attacker, and there is no undo: the truncated
    /// file is not the open buffer.
    #[gpui::test]
    fn following_a_wiki_link_never_truncates_an_existing_file(cx: &mut TestAppContext) {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join(".gitignore"), "drafts/\n").unwrap();
        std::fs::create_dir_all(ws.path().join("drafts")).unwrap();
        let secret = ws.path().join("drafts").join("secret.md");
        std::fs::write(&secret, "important\n").unwrap();
        let note = ws.path().join("note.md");
        std::fs::write(&note, "see [[drafts/secret]] here").unwrap();
        index_workspace(cx, ws.path());

        let (_bk, editor, cx) = open_editor_path(cx, &note);
        let opened = open_path_sink(cx, &editor);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(10); // inside [[drafts/secret]]
            cx.notify();
        });
        cx.dispatch_action(FollowLink);
        cx.run_until_parked();

        assert_eq!(
            std::fs::read_to_string(&secret).unwrap(),
            "important\n",
            "an existing file must never be truncated by following a link"
        );
        assert!(
            opened.borrow().last().is_some_and(|p| p.ends_with("secret.md")),
            "an existing file is opened, not re-created: {opened:?}"
        );
    }

    /// A *dangling* symlink defeats the containment check: with no
    /// canonical leaf it anchors at the parent, which is inside the
    /// root, and approves the path. `create_new`'s O_EXCL refuses to
    /// write through it, but treating that refusal as "open it anyway"
    /// handed the editor a path outside the workspace — and the next
    /// save wrote through it. A vault cloned from git can carry one.
    // Symlinks are a unix concept here; every other symlink test in the
    // tree carries the same gate.
    #[cfg(unix)]
    #[gpui::test]
    fn a_dangling_symlink_target_is_refused_not_opened(cx: &mut TestAppContext) {
        let fx = tempfile::tempdir().unwrap();
        let root = fx.path().join("vault");
        std::fs::create_dir(&root).unwrap();
        let outside = fx.path().join("outside.md");
        // Points at a file that does not exist yet.
        std::os::unix::fs::symlink(&outside, root.join("Later.md")).unwrap();
        let note = root.join("n.md");
        std::fs::write(&note, "see [[Later]]\n").unwrap();
        cx.update(|cx| {
            cx.set_global(crate::knowledge::KnowledgeState(std::sync::Arc::new(
                std::sync::Mutex::new(crate::knowledge::Index::scan(&root)),
            )));
        });
        let (_b, editor, cx) = open_editor_path(cx, &note);

        let handled = editor.update(cx, |ed, cx| ed.follow_link_at(6, cx));
        assert!(!handled, "the link is refused");
        assert!(!outside.exists(), "and nothing was created outside the workspace");
    }

    /// A wiki target is unsanitised text: `..` segments and absolute
    /// paths must not reach outside the workspace root. `Path::join`
    /// with an absolute path replaces the base entirely, so `[[/tmp/x]]`
    /// escapes without a single `..`.
    #[gpui::test]
    fn a_wiki_link_cannot_create_a_note_outside_the_workspace(cx: &mut TestAppContext) {
        let base = tempfile::tempdir().unwrap();
        let victim = base.path().join("victim.md");
        std::fs::write(&victim, "precious\n").unwrap();
        let ws = base.path().join("ws");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        let note = ws.join("sub").join("note.md");
        // `..` out of the workspace, and the same file named absolutely.
        let escape = format!(
            "a [[../../victim]] b [[{}]]",
            victim.with_extension("").display(),
        );
        std::fs::write(&note, &escape).unwrap();
        index_workspace(cx, &ws);

        let (_bk, editor, cx) = open_editor_path(cx, &note);
        let opened = open_path_sink(cx, &editor);
        let relative_at = escape.find("[[").unwrap() + 3;
        let absolute_at = escape.rfind("[[").unwrap() + 3;
        for offset in [relative_at, absolute_at] {
            editor.update_in(cx, |ed, _, cx| {
                ed.core.set_cursor(offset);
                cx.notify();
            });
            cx.dispatch_action(FollowLink);
            cx.run_until_parked();
        }

        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "precious\n",
            "a link must not touch a file outside the workspace"
        );
        assert!(
            opened.borrow().is_empty(),
            "an escaping link must not be followed at all: {opened:?}"
        );
    }

    /// A press inside link text must still be able to start a
    /// drag-selection: navigation belongs on mouse *up*, and only when
    /// nothing was dragged in between — what every browser and Obsidian
    /// do, and the reason selecting link text is possible at all.
    #[gpui::test]
    fn dragging_out_of_a_link_selects_instead_of_navigating(cx: &mut TestAppContext) {
        let ws = knowledge_fixture(cx);
        let note = ws.path().join("note.md");
        std::fs::write(&note, "go [[Roadmap]] or here").unwrap();
        let (_bk, editor, cx) = open_editor_path(cx, &note);
        let opened = open_path_sink(cx, &editor);

        // Display text is "go Roadmap or here": press inside the link
        // text, drag past its end, release.
        let start = point_for_index(&editor, cx, 0, 4);
        let end = point_for_index(&editor, cx, 0, 14);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();

        assert!(
            opened.borrow().is_empty(),
            "a drag that began inside a link must not navigate: {opened:?}"
        );
        cx.update(|_, app| {
            let sel = editor.read(app).core.selection.range();
            assert!(!sel.is_empty(), "the drag must have selected text (got {sel:?})");
        });
    }

    /// A browser treats a release outside the element as a cancelled
    /// click, not a completed one -- so must this editor.
    #[gpui::test]
    fn a_release_outside_the_editor_cancels_the_pending_link(cx: &mut TestAppContext) {
        let ws = knowledge_fixture(cx);
        let note = ws.path().join("note.md");
        std::fs::write(&note, "go [[Roadmap]] or here").unwrap();
        let (_bk, editor, cx) = open_editor_path(cx, &note);
        let opened = open_path_sink(cx, &editor);

        // Display text is "go Roadmap or here": press inside the link.
        let start = point_for_index(&editor, cx, 0, 4);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        editor.update(cx, |ed, _| {
            assert!(ed.pending_link.is_some(), "precondition: the press is pending");
        });

        // The release lands well outside the editor's own bounds.
        cx.simulate_mouse_up(point(px(-500.), px(-500.)), MouseButton::Left, Modifiers::none());
        cx.run_until_parked();

        assert!(
            opened.borrow().is_empty(),
            "a release outside the editor must not navigate: {opened:?}"
        );
        editor.update(cx, |ed, _| {
            assert!(ed.pending_link.is_none(), "the press must be cancelled, not merely unactioned");
        });
    }

    #[gpui::test]
    fn follow_link_opens_resolved_and_creates_unresolved(cx: &mut TestAppContext) {
        // The note lives *inside* the indexed workspace, as it does in
        // the app: creating a note is contained to the workspace root.
        let ws = knowledge_fixture(cx);
        let note = ws.path().join("note.md");
        std::fs::write(&note, "go [[Roadmap]] or [[Ghost]] now").unwrap();
        let (_bk, editor, cx) = open_editor_path(cx, &note);
        let opened = open_path_sink(cx, &editor);

        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(7); // inside [[Roadmap]]
            cx.notify();
        });
        cx.dispatch_action(FollowLink);
        cx.run_until_parked();
        assert!(opened.borrow()[0].ends_with("Roadmap.md"), "{opened:?}");

        // Unresolved wiki target: the note is created beside this file.
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(21); // inside [[Ghost]]
            cx.notify();
        });
        cx.dispatch_action(FollowLink);
        cx.run_until_parked();
        let ghost = ws.path().join("Ghost.md");
        assert!(ghost.exists(), "unresolved link created the note");
        assert!(opened.borrow()[1].ends_with("Ghost.md"));

        // Cursor away from any link: nothing happens.
        cx.dispatch_action(DocEnd);
        cx.dispatch_action(FollowLink);
        cx.run_until_parked();
        assert_eq!(opened.borrow().len(), 2);
    }

    #[gpui::test]
    fn plain_click_navigates_a_rendered_link_but_edits_a_revealed_one(cx: &mut TestAppContext) {
        let _ws = knowledge_fixture(cx);
        let (_fx, editor, cx) = open_editor(cx, "note.md", "go [[Roadmap]] or [[Ghost]] now");
        let opened: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
        cx.update(|_, app| {
            let sink = opened.clone();
            app.subscribe(&editor, move |_, event: &EditorEvent, _| {
                if let EditorEvent::OpenPath(p) = event {
                    sink.borrow_mut().push(p.clone());
                }
            })
            .detach();
        });

        // The cursor starts at the very top of the document, well outside
        // the link — its syntax is not revealed — so a plain click on the
        // rendered link text navigates.
        let inside = point_for_index(&editor, cx, 0, 7); // "a" of "Roadmap"
        cx.simulate_click(inside, Modifiers::none());
        cx.run_until_parked();
        assert!(
            opened.borrow().last().is_some_and(|p| p.ends_with("Roadmap.md")),
            "plain click on a rendered link must navigate: {opened:?}"
        );

        // Now put the caret inside that same link, so its syntax is
        // revealed (the user is editing it), and click elsewhere within
        // it. This must move the caret and must NOT navigate again —
        // otherwise the link could never be corrected.
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(7); // inside [[Roadmap]]
            cx.notify();
        });
        cx.run_until_parked();
        let elsewhere = point_for_index(&editor, cx, 0, 10); // still inside the link
        cx.simulate_click(elsewhere, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            opened.borrow().len(),
            1,
            "a plain click on a revealed link must not navigate"
        );
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).core.selection.range(),
                10..10,
                "a plain click on a revealed link still places the caret"
            );
        });
    }

    #[test]
    fn plain_click_follows_a_rendered_link_but_not_a_revealed_one() {
        // (modifier, on a link, link syntax revealed) -> follows?
        assert!(click_follows_link(false, true, false), "plain click on rendered link");
        assert!(click_follows_link(true, true, false), "cmd-click still follows");
        // Revealed means the cursor is inside it and the user is editing.
        assert!(!click_follows_link(false, true, true), "plain click edits a revealed link");
        assert!(click_follows_link(true, true, true), "cmd-click follows even when revealed");
        assert!(!click_follows_link(false, false, false), "not on a link");
    }

    #[gpui::test]
    fn external_links_are_handled_rather_than_falling_through(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "see [apple](https://apple.com)\n");
        // Offset 12 sits inside the link text.
        let handled = editor.update(cx, |ed, cx| ed.follow_link_at(12, cx));
        assert!(handled, "an https link must be handled, not passed to the index");
        // *Which* url reached the platform matters as much as that one
        // did: the destination, not the link text, and unmangled.
        assert_eq!(
            cx.opened_url().as_deref(), Some("https://apple.com"),
            "the link's destination is what gets opened"
        );
    }

    /// Clicking a table-of-contents entry moves the cursor to that
    /// heading. Anchors used to be classified as relative paths, joined
    /// onto a directory, resolved to nothing, and silently do nothing —
    /// so every link the `toc` plugin generates was dead.
    /// The link cache must answer exactly what a fresh scan would. A
    /// cache that drifts from the authority is worse than no cache:
    /// clicks would follow links that are no longer there, or miss ones
    /// that are.
    /// Hovering a note link previews the note it points at, without
    /// opening anything.
    #[gpui::test]
    fn hovering_a_note_link_previews_the_note(cx: &mut TestAppContext) {
        let fx = tempfile::tempdir().unwrap();
        let target = fx.path().join("Target.md");
        std::fs::write(&target, "# The Target\n\nFirst line of it.\n").unwrap();
        let note = fx.path().join("n.md");
        std::fs::write(&note, "see [[Target]] here\n").unwrap();
        cx.update(|cx| {
            cx.set_global(crate::knowledge::KnowledgeState(std::sync::Arc::new(
                std::sync::Mutex::new(crate::knowledge::Index::scan(fx.path())),
            )));
        });
        let (_backups, editor, cx) = open_editor_path(cx, &note);

        let link = editor.update(cx, |ed, _| ed.link_at_offset(6).cloned().expect("on the link"));
        let preview = cx.update(|_, app| editor.read(app).preview_for(&link, app));
        let crate::preview::Preview::Note { title, excerpt } = preview else {
            panic!("expected a note preview, got {preview:?}")
        };
        assert_eq!(title, "The Target");
        assert!(excerpt.contains("First line of it."), "shows the note's opening: {excerpt}");
    }

    /// A wiki link with nothing behind it says so, rather than looking
    /// like a failure — clicking it is what creates the note.
    /// Reaching a popover means crossing whatever sits between it and
    /// the link — in a bullet list of links, that is another link.
    /// Closing on the first one crossed made the popover unreachable,
    /// which is exactly what a list of external links looked like.
    /// Drives `hover_moved` rather than assigning the fields it sets:
    /// the previous version of this test asserted the property while
    /// skipping the exact line that broke it, and the popover really
    /// was torn down.
    #[gpui::test]
    fn crossing_another_link_does_not_tear_down_the_popover(cx: &mut TestAppContext) {
        let (_fx, editor, cx) =
            open_editor(cx, "n.md", "- [[Alpha]]\n- [[Beta]]\n");
        let (a, b) = editor.update(cx, |ed, _| {
            let a = ed.link_at_offset(4).cloned().expect("alpha");
            let b = ed.link_at_offset(16).cloned().expect("beta");
            (a, b)
        });
        assert_ne!(a.target, b.target, "two distinct links");

        editor.update(cx, |ed, cx| {
            ed.hover_link = Some(a.clone());
            ed.open_hover_preview(cx);
            assert!(ed.hover_preview.is_some(), "the first link's popover is up");
        });

        // Travelling towards it crosses the second link. Drive the real
        // handler: assigning `hover_link` by hand skips the line that
        // used to clear `hover_preview`, which is how this passed while
        // the popover was in fact torn down.
        let at_b = editor.update(cx, |ed, _| {
            ed.layout_cache.clear();
            ed.link_anchor(b.range.start)
        });
        editor.update_in(cx, |ed, window, cx| {
            if let Some(p) = at_b {
                ed.hover_moved(p, window, cx);
            } else {
                // No layout yet in a headless test: exercise the same
                // branch directly.
                ed.hover_close_task = None;
                ed.hover_link = Some(b.clone());
            }
            assert!(
                ed.hover_preview.is_some(),
                "crossing a link must not close the popover being walked to"
            );
        });
    }

    /// Once the pointer is inside the popover, a dwell that fires late
    /// must not swap its contents — that moves the button out from
    /// under the click.
    #[gpui::test]
    fn a_held_popover_is_not_replaced_underneath_the_pointer(cx: &mut TestAppContext) {
        let (_fx, editor, cx) =
            open_editor(cx, "n.md", "- [[Alpha]]\n- [[Beta]]\n");
        editor.update(cx, |ed, cx| {
            let a = ed.link_at_offset(4).cloned().expect("alpha");
            let b = ed.link_at_offset(16).cloned().expect("beta");
            ed.hover_link = Some(a);
            ed.open_hover_preview(cx);
            let shown = ed.hover_preview.clone();

            // Pointer now inside the popover; a stale dwell fires.
            ed.hover_held = true;
            ed.hover_link = Some(b);
            ed.open_hover_preview(cx);
            assert_eq!(ed.hover_preview, shown, "the popover under the pointer is left alone");
        });
    }

    #[gpui::test]
    fn hovering_an_unresolved_link_says_it_does_not_exist(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "see [[Nowhere]] here\n");
        let link = editor.update(cx, |ed, _| ed.link_at_offset(6).cloned().expect("on the link"));
        let preview = cx.update(|_, app| editor.read(app).preview_for(&link, app));
        assert!(
            matches!(preview, crate::preview::Preview::Missing { .. }),
            "got {preview:?}"
        );
    }

    /// The load-bearing privacy property: hovering an external link on
    /// a domain the user has not enabled must not fetch anything. The
    /// popover shows only what is knowable locally.
    #[gpui::test]
    fn hovering_an_external_link_fetches_nothing_without_consent(cx: &mut TestAppContext) {
        let (_fx, editor, cx) =
            open_editor(cx, "n.md", "see [paypal.com](https://evil.example) here\n");
        let link = editor.update(cx, |ed, _| ed.link_at_offset(6).cloned().expect("on the link"));
        let preview = cx.update(|_, app| editor.read(app).preview_for(&link, app));
        let crate::preview::Preview::External { domain, consent, fetched, mismatch, .. } = preview
        else {
            panic!("expected an external preview")
        };
        assert_eq!(domain, "evil.example");
        assert_eq!(consent, crate::preview::Consent::Ungranted);
        assert_eq!(fetched, None, "nothing is fetched before consent");
        assert!(mismatch, "and the text naming another site is flagged");
    }

    /// Enabling a site is what causes the first request, and it is a
    /// click that does it — never the hover. Drives the whole flow
    /// through an injected transport, so no test touches the network.
    #[gpui::test]
    fn enabling_a_site_is_what_triggers_the_first_fetch(cx: &mut TestAppContext) {
        // Settings are written by the grant, so redirect HOME -- through
        // the crate's one lock-guarded helper, since HOME is process-wide
        // and another test file swapping it concurrently would race.
        let _home = crate::workspace::tests::temp_home();

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = calls.clone();
        cx.update(|cx| {
            cx.set_global(crate::preview::PreviewState::new(Arc::new(move |_: &str| {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(b"<head><title>Fetched Title</title></head>".to_vec())
            })));
        });

        let (_fx, editor, cx) =
            open_editor(cx, "n.md", "see [docs](https://example.test/a) here\n");
        let link = editor.update(cx, |ed, _| ed.link_at_offset(6).cloned().expect("link"));

        // Hovering an ungranted site: no request, ever.
        editor.update(cx, |ed, cx| {
            ed.hover_link = Some(link.clone());
            ed.open_hover_preview(cx);
        });
        cx.run_until_parked();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "hovering an ungranted site must not fetch"
        );

        // The click consents, and only then does the request happen.
        editor.update(cx, |ed, cx| ed.enable_previews_for_hovered_site(cx));
        cx.run_until_parked();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "enabling the site fetches once"
        );
        editor.update(cx, |ed, _| {
            let Some(crate::preview::Preview::External { consent, fetched, .. }) =
                ed.hover_preview.as_ref()
            else {
                panic!("external preview")
            };
            assert_eq!(*consent, crate::preview::Consent::Granted);
            assert_eq!(
                fetched.as_ref().map(|m| m.title.as_str()),
                Some("Fetched Title"),
                "and the popover shows what it read"
            );
        });

        // The grant is persisted in the same place plugin grants live.
        let settings = crate::settings::load(&crate::settings::config_dir());
        assert_eq!(settings.plugin_grants["supermd"], ["net:example.test"]);
    }

    #[gpui::test]
    fn the_link_cache_agrees_with_a_fresh_scan(cx: &mut TestAppContext) {
        let doc = "[[Alpha]] and [b](c.md) and <https://x.dev> `[[not a link]]`\n\n                   ```\n[[fenced]]\n```\n\nlast [[Omega]]\n";
        let (_fx, editor, cx) = open_editor(cx, "n.md", doc);
        editor.update(cx, |ed, _| {
            let fresh = crate::knowledge::extract_all_links(&ed.core.buffer.text());
            for offset in 0..doc.len() {
                let cached = ed.link_at_offset(offset).map(|l| l.range.clone());
                let expected =
                    fresh.iter().find(|l| l.range.contains(&offset)).map(|l| l.range.clone());
                assert_eq!(cached, expected, "offset {offset} disagrees");
            }
        });
    }

    /// An edit must invalidate the cache. `restyle` rebuilds it, and
    /// this is the test that fails if a future edit path forgets to
    /// call through it.
    #[gpui::test]
    fn editing_rebuilds_the_link_cache(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "no links here\n");
        editor.update(cx, |ed, _| assert!(ed.link_at_offset(3).is_none()));

        cx.simulate_input("[[Added]] ");
        cx.run_until_parked();
        editor.update(cx, |ed, _| {
            let text = ed.core.buffer.text();
            let at = text.find("Added").expect("typed");
            assert_eq!(
                ed.link_at_offset(at).map(|l| l.target.clone()),
                Some("Added".to_string()),
                "a link typed just now is in the cache"
            );
        });
    }

    #[gpui::test]
    fn an_anchor_link_moves_the_cursor_to_its_heading(cx: &mut TestAppContext) {
        let doc = "# Top\n\n[jump](#the-target)\n\n## The target\n\ntail\n";
        let (_fx, editor, cx) = open_editor(cx, "n.md", doc);
        let target = doc.find("## The target").expect("heading present");

        // Offset 10 sits inside the link text `jump`.
        let handled = editor.update(cx, |ed, cx| ed.follow_link_at(10, cx));
        assert!(handled, "an anchor is handled, not passed to the index");
        editor.update(cx, |ed, _| {
            assert_eq!(
                ed.core.selection.head, target,
                "the cursor lands on the heading the anchor names"
            );
        });
        assert!(cx.opened_url().is_none(), "an anchor never leaves the app");
    }

    #[gpui::test]
    fn an_anchor_naming_no_heading_is_not_handled(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "# Top\n\n[x](#nowhere)\n");
        let handled = editor.update(cx, |ed, cx| ed.follow_link_at(10, cx));
        assert!(!handled, "a dangling anchor does nothing rather than guessing");
    }

    #[gpui::test]
    fn non_http_schemes_are_not_opened(cx: &mut TestAppContext) {
        let (_fx, editor, cx) =
            open_editor(cx, "n.md", "see [x](supermd://install-plugin?name=evil)\n");
        let handled = editor.update(cx, |ed, cx| ed.follow_link_at(9, cx));
        assert!(!handled, "only http(s) is opened from a document");
    }

    #[gpui::test]
    fn wiki_completion_filters_and_confirms(cx: &mut TestAppContext) {
        let _ws = knowledge_fixture(cx);
        let (_fx, editor, cx) = open_editor(cx, "note.md", "");
        cx.simulate_input("[[R");
        cx.update(|_, app| {
            let comp = editor.read(app).completion.as_ref().expect("popup open");
            assert_eq!(comp.matches.len(), 2, "Roadmap and Recipes");
        });
        cx.simulate_input("oa");
        cx.update(|_, app| {
            let comp = editor.read(app).completion.as_ref().unwrap();
            assert_eq!(comp.matches.len(), 1);
            assert_eq!(comp.matches[0].0, "Roadmap");
        });
        // Enter confirms instead of inserting a newline.
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "[[Roadmap]]");
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.completion.is_none(), "popup closed");
            assert_eq!(ed.core.selection.head, 11, "cursor after the link");
        });
    }

    #[gpui::test]
    fn completion_navigates_and_dismisses(cx: &mut TestAppContext) {
        let _ws = knowledge_fixture(cx);
        let (_fx, editor, cx) = open_editor(cx, "note.md", "");
        cx.simulate_input("[[");
        cx.update(|_, app| {
            let comp = editor.read(app).completion.as_ref().expect("popup on bare [[");
            assert_eq!(comp.selected, 0);
        });
        // Arrows steer the popup, not the cursor.
        cx.dispatch_action(MoveDown);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.completion.as_ref().unwrap().selected, 1);
            assert_eq!(ed.core.selection.head, 2, "cursor pinned");
        });
        cx.dispatch_action(MoveDown);
        cx.update(|_, app| {
            assert_eq!(editor.read(app).completion.as_ref().unwrap().selected, 0, "wraps");
        });
        cx.dispatch_action(DismissCompletion);
        cx.update(|_, app| assert!(editor.read(app).completion.is_none()));
        // Typing the closing bracket keeps it closed.
        cx.simulate_input("x]]");
        cx.update(|_, app| assert!(editor.read(app).completion.is_none()));
    }

    #[gpui::test]
    fn pasting_a_clipboard_image_saves_an_asset_and_links_it(cx: &mut TestAppContext) {
        let (fx, editor, cx) = open_editor(cx, "doc.md", "start ");
        cx.dispatch_action(DocEnd);
        cx.update(|_, app| {
            app.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                vec![1, 2, 3, 4],
            )));
        });
        cx.dispatch_action(Paste);
        let text = buffer_text(&editor, cx);
        assert!(
            text.starts_with("start ![](assets/pasted-") && text.ends_with(".png)"),
            "{text:?}"
        );
        let name = text
            .strip_prefix("start ![](assets/")
            .unwrap()
            .strip_suffix(")")
            .unwrap();
        let on_disk = fx.path.parent().unwrap().join("assets").join(name);
        assert_eq!(std::fs::read(on_disk).unwrap(), vec![1, 2, 3, 4]);

        // Code buffers ignore image pastes entirely.
        let (fx2, editor, cx) = open_editor(cx, "code.rs", "fn x() {}");
        cx.update(|_, app| {
            app.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                vec![9],
            )));
        });
        cx.dispatch_action(Paste);
        assert_eq!(buffer_text(&editor, cx), "fn x() {}");
        assert!(!fx2.path.parent().unwrap().join("assets").exists());
    }

    #[gpui::test]
    fn typing_a_marker_wraps_the_selection(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "wrap.md", "pick me now");
        select(&editor, cx, 5..7);
        cx.simulate_input("*");
        assert_eq!(buffer_text(&editor, cx), "pick *me* now");
        // The content stays selected, so a second press upgrades to bold.
        cx.simulate_input("*");
        assert_eq!(buffer_text(&editor, cx), "pick **me** now");
        select(&editor, cx, 7..9);
        cx.simulate_input("`");
        assert_eq!(buffer_text(&editor, cx), "pick **`me`** now");
        // Ordinary characters still replace the selection.
        let (_fx2, editor, cx) = open_editor(cx, "wrap2.md", "pick me now");
        select(&editor, cx, 5..7);
        cx.simulate_input("x");
        assert_eq!(buffer_text(&editor, cx), "pick x now");
        // Markers replace as usual outside markdown.
        let (_fx3, editor, cx) = open_editor(cx, "wrap.rs", "let x = 1;");
        select(&editor, cx, 4..5);
        cx.simulate_input("*");
        assert_eq!(buffer_text(&editor, cx), "let * = 1;");
    }

    /// The single table block a text holds, exactly as the projection
    /// scanner sees it -- the thing that decides whether a line renders
    /// as part of the table or as a stray paragraph of pipes.
    fn one_table(text: &str) -> String {
        let blocks = crate::editor::blocks::blocks(text);
        let tables: Vec<_> = blocks
            .iter()
            .filter(|b| b.kind == crate::editor::blocks::BlockKind::Table)
            .collect();
        assert_eq!(tables.len(), 1, "exactly one table block in {text:?}");
        text[tables[0].range.clone()].to_string()
    }

    /// The most natural first use of a brand-new command is with the
    /// cursor still in the header row. A row wedged between the header
    /// and its separator drops the header out of the table entirely.
    #[gpui::test]
    fn insert_row_from_the_header_lands_below_the_separator(cx: &mut TestAppContext) {
        let doc = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let (_fx, editor, cx) = open_editor(cx, "hdr.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(2); // inside the header cell "a"
            cx.notify();
        });
        cx.dispatch_action(TableInsertRow);
        let text = buffer_text(&editor, cx);
        let table = one_table(&text);
        assert_eq!(table.lines().count(), 4, "header, separator and two body rows: {table:?}");
        assert!(table.lines().next().unwrap().contains('a'), "the header is still row 0: {table:?}");
        assert!(table_edit::rows(&table)[1].is_separator, "separator still row 1: {table:?}");
        // The cursor follows the new row, so typing lands in it.
        cx.simulate_input("x");
        let text = buffer_text(&editor, cx);
        assert!(text.lines().nth(2).unwrap().contains('x'), "typed into the new row: {text:?}");
        assert!(
            crate::editor::blocks::is_separator_row(text.lines().nth(1).unwrap()),
            "not into the separator: {text:?}"
        );
    }

    /// The commands insert next to the *cursor*. Nothing else in the
    /// suite pins that down: an insert hard-wired to the top of the
    /// table passes every other table test.
    #[gpui::test]
    fn table_inserts_land_at_the_cursor(cx: &mut TestAppContext) {
        let doc = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n";

        // A row, from the last body row: the new row goes below it.
        let (_fx, editor, cx) = open_editor(cx, "ins.md", doc);
        let at = doc.find('3').unwrap();
        editor.update_in(cx, |ed, _, cx| ed.core.set_cursor(at));
        cx.dispatch_action(TableInsertRow);
        cx.simulate_input("z");
        let text = buffer_text(&editor, cx);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[3].contains('3'), "the row below the cursor is still row 3: {text:?}");
        assert!(lines[4].contains('z'), "the new row is row 4: {text:?}");

        // A column, from the second column: the new column goes right.
        let (_fx, editor, cx) = open_editor(cx, "ins2.md", doc);
        let at = doc.find('b').unwrap();
        editor.update_in(cx, |ed, _, cx| ed.core.set_cursor(at));
        cx.dispatch_action(TableInsertColumn);
        cx.simulate_input("z");
        let text = buffer_text(&editor, cx);
        assert_eq!(
            text.lines().next().unwrap().replace(' ', ""),
            "|a|b|z|",
            "the new column follows the cursor's own: {text:?}"
        );
    }

    /// The other three commands, from the header and from the separator
    /// line: each must leave something the scanner still reads as one
    /// whole table.
    #[gpui::test]
    fn table_commands_from_header_and_separator_keep_the_table_whole(cx: &mut TestAppContext) {
        let doc = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let sep_cell = doc.find("---").unwrap();

        // Delete Row from the header: the header is structure too.
        let (_fx, editor, cx) = open_editor(cx, "t1.md", doc);
        editor.update_in(cx, |ed, _, cx| ed.core.set_cursor(2));
        cx.dispatch_action(TableDeleteRow);
        assert_eq!(buffer_text(&editor, cx), doc, "the header cannot be deleted away");
        // ... and from the separator, as before.
        editor.update_in(cx, |ed, _, cx| ed.core.set_cursor(sep_cell));
        cx.dispatch_action(TableDeleteRow);
        assert_eq!(buffer_text(&editor, cx), doc, "nor the separator");

        // Insert Row from the separator line: the new row is the first
        // body row, and the separator keeps its place.
        let (_fx, editor, cx) = open_editor(cx, "t2.md", doc);
        editor.update_in(cx, |ed, _, cx| ed.core.set_cursor(sep_cell));
        cx.dispatch_action(TableInsertRow);
        let text = buffer_text(&editor, cx);
        let table = one_table(&text);
        assert_eq!(table.lines().count(), 4, "{table:?}");
        assert!(table_edit::rows(&table)[1].is_separator, "{table:?}");

        // Insert Column, from the header and from the separator.
        for (name, at) in [("t3.md", 2), ("t4.md", sep_cell)] {
            let (_fx, editor, cx) = open_editor(cx, name, doc);
            editor.update_in(cx, |ed, _, cx| ed.core.set_cursor(at));
            cx.dispatch_action(TableInsertColumn);
            let text = buffer_text(&editor, cx);
            let table = one_table(&text);
            assert_eq!(table.lines().count(), 3, "{table:?}");
            for line in table.lines() {
                assert_eq!(line.matches('|').count(), 4, "three cells now: {line:?}");
            }
            assert!(table_edit::rows(&table)[1].is_separator, "{table:?}");
        }

        // Delete Column, from the header and from the separator.
        for (name, at) in [("t5.md", 2), ("t6.md", sep_cell)] {
            let (_fx, editor, cx) = open_editor(cx, name, doc);
            editor.update_in(cx, |ed, _, cx| ed.core.set_cursor(at));
            cx.dispatch_action(TableDeleteColumn);
            let text = buffer_text(&editor, cx);
            let table = one_table(&text);
            assert_eq!(table.lines().count(), 3, "{table:?}");
            for line in table.lines() {
                assert_eq!(line.matches('|').count(), 2, "one cell left: {line:?}");
            }
            assert!(table_edit::rows(&table)[1].is_separator, "{table:?}");
        }
    }

    /// After deleting a row the caret must land somewhere you can
    /// type. The delimiter is not such a place: one keystroke there
    /// turns the table into a paragraph of pipes.
    #[gpui::test]
    fn delete_row_never_parks_the_caret_in_the_delimiter(cx: &mut TestAppContext) {
        let doc = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let (_fx, editor, cx) = open_editor(cx, "t.md", doc);
        editor.update_in(cx, |ed, _, _| ed.core.set_cursor(doc.find('1').unwrap()));
        cx.dispatch_action(TableDeleteRow);
        cx.simulate_input("z");
        let text = buffer_text(&editor, cx);
        assert!(!text.contains("z---"), "the caret was in the delimiter: {text:?}");
        assert_eq!(
            crate::editor::blocks::blocks(&text)
                .iter()
                .filter(|b| matches!(b.kind, crate::editor::blocks::BlockKind::Table))
                .count(),
            1,
            "still one table: {text:?}"
        );
    }

    /// A command that declines tells the user why. Silence is
    /// indistinguishable from a broken command.
    #[gpui::test]
    fn deleting_the_header_row_says_why_it_refused(cx: &mut TestAppContext) {
        let doc = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let (_fx, editor, cx) = open_editor(cx, "t.md", doc);
        let refusals = command_error_sink(cx, &editor);
        editor.update_in(cx, |ed, _, _| ed.core.set_cursor(2));
        cx.dispatch_action(TableDeleteRow);
        assert_eq!(buffer_text(&editor, cx), doc, "unchanged");
        assert!(!refusals.borrow().is_empty(), "the refusal reached the user");
    }

    /// The same for the other three refusals: a table shortcut pressed
    /// outside a table, and the last column of a one-column table.
    #[gpui::test]
    fn the_other_table_refusals_reach_the_user_too(cx: &mut TestAppContext) {
        let doc = "a paragraph\n\n| a |\n| --- |\n| 1 |\n";
        let (_fx, editor, cx) = open_editor(cx, "one.md", doc);
        let refusals = command_error_sink(cx, &editor);

        // Caret in the paragraph: all four commands decline out loud.
        editor.update_in(cx, |ed, _, _| ed.core.set_cursor(3));
        cx.dispatch_action(TableInsertRow);
        cx.dispatch_action(TableDeleteRow);
        cx.dispatch_action(TableInsertColumn);
        cx.dispatch_action(TableDeleteColumn);
        assert_eq!(buffer_text(&editor, cx), doc, "and change nothing");
        assert_eq!(
            *refusals.borrow(),
            vec![Editor::NOT_IN_A_TABLE.to_string(); 4],
            "every one of them said so"
        );

        // And the last column of a one-column table.
        editor.update_in(cx, |ed, _, _| ed.core.set_cursor(doc.find('1').unwrap()));
        cx.dispatch_action(TableDeleteColumn);
        assert_eq!(buffer_text(&editor, cx), doc, "still a table");
        let said = refusals.borrow();
        assert_eq!(said.len(), 5, "{said:?}");
        assert!(said[4].contains("column"), "{said:?}");
    }

    #[gpui::test]
    fn table_command_on_a_pipe_line_in_a_code_file_is_a_no_op(cx: &mut TestAppContext) {
        // rustfmt's own style puts a leading `|` on an or-pattern arm —
        // this must never be mistaken for a markdown table row just
        // because the line starts with `|`.
        let doc = "match x {\n    Foo::A\n    | Foo::B => 1,\n    _ => 0,\n}\n";
        let (_fx, editor, cx) = open_editor(cx, "match.rs", doc);
        let pipe_line_start = doc.find("| Foo::B").unwrap();
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(pipe_line_start + 2); // inside "| Foo::B => 1,"
            cx.notify();
        });
        cx.dispatch_action(TableInsertRow);
        assert_eq!(buffer_text(&editor, cx), doc, "not a table — the code is untouched");
        cx.dispatch_action(TableDeleteRow);
        assert_eq!(buffer_text(&editor, cx), doc);
        cx.dispatch_action(TableInsertColumn);
        assert_eq!(buffer_text(&editor, cx), doc);
        cx.dispatch_action(TableDeleteColumn);
        assert_eq!(buffer_text(&editor, cx), doc);
    }

    #[gpui::test]
    fn table_command_under_diff_view_leaves_the_buffer_untouched(cx: &mut TestAppContext) {
        let repo = tempfile::tempdir().unwrap();
        sh_git(repo.path(), &["init", "-q"]);
        let file = repo.path().join("table.md");
        let doc = "| a | b |\n| - | - |\n| 1 | 2 |\n";
        std::fs::write(&file, doc).unwrap();
        commit_all(repo.path());

        let (_bk, editor, cx) = open_editor_path(cx, &file);
        editor.update_in(cx, |ed, _, cx| {
            let langs = crate::highlight::languages(cx);
            ed.enter_diff(&langs, cx);
        });
        cx.run_until_parked();
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(3); // inside "a", real buffer's selection
            cx.notify();
        });
        cx.dispatch_action(TableInsertRow);
        assert_eq!(buffer_text(&editor, cx), doc, "the real buffer is read-only under a diff");
        cx.update(|_, app| {
            assert!(editor.read(app).diff.is_some(), "still in diff mode");
        });
    }

    #[gpui::test]
    fn tab_hops_table_cells_aligning_and_appending_rows(cx: &mut TestAppContext) {
        let doc = "| h1 | h2 |\n|---|---|\n| a | bbbb |";
        let (_fx, editor, cx) = open_editor(cx, "table.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(3); // inside "h1"
            cx.notify();
        });
        let aligned = "| h1 | h2   |\n| -- | ---- |\n| a  | bbbb |";
        cx.dispatch_action(InsertTab);
        assert_eq!(buffer_text(&editor, cx), aligned);
        let sel_text = |cx: &mut VisualTestContext| {
            cx.update(|_, app| {
                let ed = editor.read(app);
                ed.core.buffer.text()[ed.core.selection.range()].to_string()
            })
        };
        assert_eq!(sel_text(cx), "h2", "tab selects the next cell");
        cx.dispatch_action(InsertTab);
        assert_eq!(sel_text(cx), "a", "skips the separator row");
        cx.dispatch_action(InsertTab);
        assert_eq!(sel_text(cx), "bbbb");
        // Tab off the last cell appends an empty row.
        cx.dispatch_action(InsertTab);
        assert_eq!(buffer_text(&editor, cx), format!("{aligned}\n|    |      |"));
        cx.update(|_, app| {
            assert!(editor.read(app).core.selection.is_cursor(), "empty cell is a cursor");
        });
        // Shift-Tab walks back from the new row.
        cx.dispatch_action(Outdent);
        assert_eq!(sel_text(cx), "bbbb");
        // One undo drops the appended row (single group per press).
        cx.dispatch_action(Undo);
        assert_eq!(buffer_text(&editor, cx), aligned);
    }

    /// Numbers inside a fenced code block are the user's literal text.
    /// Enter-continuation in the list *around* a fence must not reach
    /// inside it -- the file on disk is the source of truth, and this
    /// rewrote lines the user never touched, with no indication.
    #[gpui::test]
    fn enter_in_a_list_does_not_renumber_inside_a_fence(cx: &mut TestAppContext) {
        let doc = "1. Steps:\n   ```text\n   1. alpha\n   1. beta\n   ```\n2. Done\n";
        let (_fx, editor, cx) = open_editor(cx, "fence.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(doc.find('\n').unwrap()); // end of "1. Steps:"
            cx.notify();
        });
        cx.dispatch_action(Newline);
        assert_eq!(
            buffer_text(&editor, cx),
            "1. Steps:\n2. \n   ```text\n   1. alpha\n   1. beta\n   ```\n3. Done\n",
            "the fence body is byte-for-byte what the user wrote"
        );
    }

    /// Same for the explicit command, which renumbers without inserting.
    #[gpui::test]
    fn the_renumber_command_does_not_renumber_inside_a_fence(cx: &mut TestAppContext) {
        let doc = "1. Steps:\n   ```text\n   1. alpha\n   1. beta\n   ```\n1. Done\n";
        let (_fx, editor, cx) = open_editor(cx, "fence2.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(3); // inside "Steps:"
            cx.notify();
        });
        cx.dispatch_action(RenumberList);
        assert_eq!(
            buffer_text(&editor, cx),
            "1. Steps:\n   ```text\n   1. alpha\n   1. beta\n   ```\n2. Done\n",
            "only the outer list renumbers"
        );
    }

    #[gpui::test]
    fn enter_continuation_renumber_is_one_undo(cx: &mut TestAppContext) {
        let doc = "1. one\n2. two\n";
        let (_fx, editor, cx) = open_editor(cx, "list.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(6); // right after "one"
            cx.notify();
        });
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "1. one\n2. \n3. two\n", "the run renumbers");
        cx.dispatch_action(Undo);
        assert_eq!(buffer_text(&editor, cx), doc, "one Enter costs exactly one Undo");
    }

    /// The explicit command is its own undo step. It shares its helper
    /// with Enter-continuation, which deliberately coalesces into the
    /// Enter's group -- so without its own break, Renumber List
    /// coalesces into whatever typing is still in the window and one
    /// Undo takes the user's words with it.
    #[gpui::test]
    fn the_renumber_command_is_its_own_undo_step(cx: &mut TestAppContext) {
        let doc = "1. one\n1. two\n";
        let (_fx, editor, cx) = open_editor(cx, "cmd.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(6); // right after "one"
            cx.notify();
        });
        cx.simulate_input("!");
        assert_eq!(buffer_text(&editor, cx), "1. one!\n1. two\n");
        cx.dispatch_action(RenumberList);
        assert_eq!(buffer_text(&editor, cx), "1. one!\n2. two\n", "the run renumbers");
        cx.dispatch_action(Undo);
        assert_eq!(
            buffer_text(&editor, cx),
            "1. one!\n1. two\n",
            "one Undo takes the renumber and leaves the typing"
        );
    }

    /// Redo has to put the cursor back where the edit left it. The
    /// renumber's replacement is not where the user is typing, so
    /// redoing an Enter threw the cursor to the end of the document.
    #[gpui::test]
    fn redo_of_an_ordered_list_enter_restores_the_cursor(cx: &mut TestAppContext) {
        let doc = "1. one\n2. two\n";
        let (_fx, editor, cx) = open_editor(cx, "redo.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(6); // right after "one"
            cx.notify();
        });
        cx.dispatch_action(Newline);
        let landed = head(&editor, cx);
        assert_eq!(landed, 10, "after the new \"2. \" marker");
        cx.dispatch_action(Undo);
        cx.dispatch_action(Redo);
        assert_eq!(buffer_text(&editor, cx), "1. one\n2. \n3. two\n");
        assert_eq!(head(&editor, cx), landed, "redo puts the cursor back, not at the end");
    }

    /// And the renumber rewrites the list, not the file: an Enter in a
    /// three-line list inside a long document must not push an undo
    /// entry holding two whole copies of it.
    #[gpui::test]
    fn the_renumber_edit_is_scoped_to_the_list(cx: &mut TestAppContext) {
        let filler = "lorem ipsum dolor sit amet\n\n".repeat(200);
        let doc = format!("{filler}1. one\n2. two\n");
        let (_fx, editor, cx) = open_editor(cx, "big.md", &doc);
        let at = doc.find("1. one").unwrap() + 6;
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(at);
            cx.notify();
        });
        cx.dispatch_action(Newline);
        let bytes = cx.update(|_, app| editor.read(app).core.last_group_bytes());
        assert!(
            bytes < doc.len(),
            "the undo entry is the list, not the document: {bytes} vs {}",
            doc.len()
        );
    }

    /// The opposite side of the same bug: typing right up against the
    /// Enter (no pause, no separate group of its own) must not let the
    /// renumber's coalesced group reach back and undo the typing too.
    #[gpui::test]
    fn enter_continuation_renumber_does_not_undo_prior_typing(cx: &mut TestAppContext) {
        let doc = "1. one\n2. two\n";
        let (_fx, editor, cx) = open_editor(cx, "list.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(6); // right after "one"
            cx.notify();
        });
        cx.simulate_input("!!!");
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "1. one!!!\n2. \n3. two\n", "the run renumbers");
        cx.dispatch_action(Undo);
        assert_eq!(
            buffer_text(&editor, cx),
            "1. one!!!\n2. two\n",
            "the typing survives — only the Enter and its renumber are undone"
        );
    }

    /// A third side of the same bug: Enter *replacing a selection*
    /// (not just continuing after a collapsed cursor) must also start
    /// its own undo group, or it coalesces with whatever typing is
    /// still in its coalescing window. The selection is set directly
    /// (not via the `SelectAll` action, whose own handler already
    /// calls `break_undo_group()` and would mask the gap this covers)
    /// so this exercises exactly the fallthrough path `newline()` takes
    /// when the selection isn't a collapsed cursor.
    #[gpui::test]
    fn enter_replacing_a_selection_does_not_undo_prior_typing(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "");
        cx.simulate_input("hello");
        editor.update_in(cx, |ed, _, cx| {
            ed.core.selection = Selection { anchor: 0, head: 5 };
            cx.notify();
        });
        cx.dispatch_action(Newline);
        assert_eq!(buffer_text(&editor, cx), "\n");
        cx.dispatch_action(Undo);
        assert_eq!(
            buffer_text(&editor, cx),
            "hello",
            "the typed text survives — Enter alone is undone, not the whole document"
        );
    }

    #[gpui::test]
    fn enter_in_a_table_aligns_and_leaves_the_row_intact(cx: &mut TestAppContext) {
        let doc = "| a | bbbb |\n| ppp | q |";
        let (_fx, editor, cx) = open_editor(cx, "table.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(doc.find('\n').unwrap()); // end of row one
            cx.notify();
        });
        cx.dispatch_action(Newline);
        assert_eq!(
            buffer_text(&editor, cx),
            "| a   | bbbb |\n\n| ppp | q    |",
            "row aligned, newline after the full row"
        );
    }

    #[gpui::test]
    fn clicking_away_from_a_table_tidies_it(cx: &mut TestAppContext) {
        let doc = "| a | bbbb |\n| ppp | q |\n\ntail here";
        let (_fx, editor, cx) = open_editor(cx, "table.md", doc);
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(2); // inside "a"
            cx.notify();
        });
        cx.run_until_parked();
        let p = point_for_index(&editor, cx, 3, 2); // "tail here" line
        cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
        assert_eq!(buffer_text(&editor, cx), "| a   | bbbb |\n| ppp | q    |\n\ntail here");
        cx.update(|_, app| {
            let ed = editor.read(app);
            let line = ed.core.buffer.line_of_byte(ed.core.selection.head);
            assert_eq!(line, 3, "cursor landed on the clicked line");
        });
    }

    #[gpui::test]
    fn format_toolbar_shows_after_a_mouse_selection_settles(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "alpha beta gamma\n");
        let p2 = point_for_index(&editor, cx, 0, 2);
        let p10 = point_for_index(&editor, cx, 0, 10);
        cx.simulate_mouse_down(p2, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(p10, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(p10, MouseButton::Left, Modifiers::none());
        cx.update(|_, app| {
            assert!(!editor.read(app).toolbar_showing(), "waits for the settle delay");
        });
        cx.executor().advance_clock(std::time::Duration::from_millis(200));
        cx.run_until_parked();
        cx.update(|_, app| assert!(editor.read(app).toolbar_showing()));

        // Formatting from the toolbar keeps the selection — and the bar.
        cx.dispatch_action(ToggleBold);
        assert_eq!(buffer_text(&editor, cx), "al**pha beta** gamma\n");
        cx.update(|_, app| assert!(editor.read(app).toolbar_showing(), "bar survives a toggle"));

        // Typing replaces the selection; the collapsed selection hides it.
        cx.simulate_input("x");
        cx.update(|_, app| assert!(!editor.read(app).toolbar_showing()));
    }

    #[gpui::test]
    fn format_toolbar_ignores_clicks_and_code_files(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "alpha beta\n");
        let p2 = point_for_index(&editor, cx, 0, 2);
        // A plain click ends with a cursor: no toolbar.
        cx.simulate_mouse_down(p2, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(p2, MouseButton::Left, Modifiers::none());
        cx.executor().advance_clock(std::time::Duration::from_millis(200));
        cx.run_until_parked();
        cx.update(|_, app| assert!(!editor.read(app).toolbar_showing()));

        // A settled selection dies on the next mouse-down.
        let p8 = point_for_index(&editor, cx, 0, 8);
        cx.simulate_mouse_down(p2, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(p8, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(p8, MouseButton::Left, Modifiers::none());
        cx.executor().advance_clock(std::time::Duration::from_millis(200));
        cx.run_until_parked();
        cx.update(|_, app| assert!(editor.read(app).toolbar_showing()));
        cx.simulate_mouse_down(p2, MouseButton::Left, Modifiers::none());
        cx.update(|_, app| assert!(!editor.read(app).toolbar_showing()));
        cx.simulate_mouse_up(p2, MouseButton::Left, Modifiers::none());
    }

    #[gpui::test]
    fn format_toolbar_never_shows_on_code_files(cx: &mut TestAppContext) {
        let (_fx, code, cx) = open_editor(cx, "main.rs", "let value = 1;\n");
        let q2 = point_for_index(&code, cx, 0, 2);
        let q9 = point_for_index(&code, cx, 0, 9);
        cx.simulate_mouse_down(q2, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(q9, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(q9, MouseButton::Left, Modifiers::none());
        cx.executor().advance_clock(std::time::Duration::from_millis(200));
        cx.run_until_parked();
        cx.update(|_, app| assert!(!code.read(app).toolbar_showing()));
    }

    #[gpui::test]
    fn clicking_a_checkbox_glyph_toggles_the_task(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "todo.md", "- [ ] milk\n- [x] eggs\n");
        // Park the cursor on the trailing line so both tasks render as
        // glyph replacements.
        cx.dispatch_action(DocEnd);
        let end = head(&editor, cx);
        cx.run_until_parked();

        let click = cx.update(|_, app| {
            let ed = editor.read(app);
            let entry = ed.layout_cache.get(&0).expect("task line painted");
            let seg = entry
                .display
                .segs
                .iter()
                .find(|s| s.toggle.is_some())
                .expect("checkbox replacement segment");
            assert_eq!(seg.toggle, Some(false));
            let lh = entry.line_height;
            let pos = entry.line.position_for_index(seg.disp.start, lh).unwrap();
            point(entry.origin.x + pos.x + px(1.), entry.origin.y + pos.y + lh * 0.5)
        });
        cx.simulate_mouse_down(click, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(click, MouseButton::Left, Modifiers::none());
        assert_eq!(buffer_text(&editor, cx), "- [x] milk\n- [x] eggs\n");
        assert_eq!(head(&editor, cx), end, "toggle never moves the cursor");
        assert!(cx.update(|_, app| !editor.read(app).dragging));

        // The checked task on line 1 toggles back the other way.
        cx.run_until_parked();
        let click = cx.update(|_, app| {
            let ed = editor.read(app);
            let entry = ed.layout_cache.get(&1).expect("second task painted");
            let seg = entry
                .display
                .segs
                .iter()
                .find(|s| s.toggle.is_some())
                .expect("checkbox replacement segment");
            assert_eq!(seg.toggle, Some(true));
            let lh = entry.line_height;
            let pos = entry.line.position_for_index(seg.disp.start, lh).unwrap();
            point(entry.origin.x + pos.x + px(1.), entry.origin.y + pos.y + lh * 0.5)
        });
        cx.simulate_mouse_down(click, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(click, MouseButton::Left, Modifiers::none());
        assert_eq!(buffer_text(&editor, cx), "- [x] milk\n- [ ] eggs\n");
    }

    /// `---` hidden and nothing drawn is a blank line, which is worse
    /// than the faded hyphens it replaced. The divider is the other half
    /// of hiding the source: present while the caret is away, centred
    /// on the line and as thick as the reading view's rule, and gone the
    /// moment the caret lands on the line and the hyphens come back.
    #[gpui::test]
    fn a_thematic_break_draws_a_divider_until_the_caret_lands(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "rule.md", "before\n\n---\n\nafter\n");
        cx.dispatch_action(DocEnd);
        cx.run_until_parked();

        let rule = cx.debug_bounds("rule-line-2").expect("a divider is drawn on the break");
        let (origin, line_height) = cx.update(|_, app| {
            let entry = editor.read(app).layout_cache.get(&2).expect("rule line painted");
            (entry.origin, entry.line_height)
        });
        let (thick, _) = crate::view::rule_style(&crate::theme::Theme::dark());
        assert_eq!(rule.size.height, px(thick), "as thick as the reading view's rule");
        // Layout rounds to whole pixels, so centred means within one.
        let off = (rule.origin.y + rule.size.height / 2.) - (origin.y + line_height / 2.);
        assert!(off.abs() <= px(1.), "centred on the line, off by {off:?}");
        assert!(rule.size.width > px(100.), "spans the column, got {:?}", rule.size.width);
        assert_eq!(cx.debug_bounds("rule-line-0"), None, "prose lines draw no divider");

        // Caret onto the break: the source returns, and the line the
        // shell paints no longer asks for a divider.
        editor.update(cx, |ed, cx| {
            ed.core.set_cursor(9);
            cx.notify();
        });
        cx.run_until_parked();
        let (shown, divider) = cx.update(|_, app| {
            let ed = editor.read(app);
            let entry = ed.layout_cache.get(&2).unwrap();
            (entry.display.text.clone(), display::draws_rule(&entry.display, ed.view_spans()))
        });
        assert_eq!(shown, "---");
        assert!(!divider, "revealed source has no rule over it");
    }

    /// The absence half through a real paint. gpui's debug-bounds map
    /// is never cleared between frames, so a divider that disappears
    /// cannot be observed going; a caret that starts on the break (the
    /// editor opens with it at offset 0) means one was never drawn.
    #[gpui::test]
    fn a_thematic_break_under_the_caret_draws_no_divider(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "rule.md", "---\n\nafter\n");
        cx.run_until_parked();
        assert_eq!(head(&editor, cx), 0);
        assert_eq!(cx.debug_bounds("rule-line-0"), None, "revealed source has no rule over it");
        let shown = cx.update(|_, app| {
            editor.read(app).layout_cache.get(&0).unwrap().display.text.clone()
        });
        assert_eq!(shown, "---");
    }

    /// The editor shows metadata the way the reading view does: small
    /// muted mono, never a heading line, and absent from the outline.
    #[gpui::test]
    fn frontmatter_lines_are_small_muted_mono_and_not_outlined(cx: &mut TestAppContext) {
        let src = "---\ntitle: x\ntags: [a]\n---\n\n# Real\n";
        let (_fx, editor, cx) = open_editor(cx, "fm.md", src);
        cx.dispatch_action(DocEnd);
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            let t = crate::theme::Theme::dark();
            let style = crate::view::frontmatter_style(&t);
            for ix in 0..4 {
                let (size, weight, family, _) = ed.line_typography(ix, &t);
                assert_eq!(size, style.size, "line {ix} at the metadata size");
                assert_eq!(family, style.family, "line {ix} in mono");
                assert_eq!(weight, FontWeight::NORMAL, "line {ix} not bold");
                let (_, attrs) = ed.line_attrs(ix, &t);
                assert!(attrs.iter().all(|a| a.color == style.ink), "line {ix} in muted ink");
            }
            let (size, ..) = ed.line_typography(5, &t);
            assert_eq!(size, t.heading_size(1), "the real heading is untouched");
            let outline: Vec<_> = ed.heading_lines().into_iter().map(|(l, s, _)| (l, s)).collect();
            assert_eq!(outline, vec![(1, "Real".to_string())]);
        });
    }

    // ── widget interactions ────────────────────────────────────────────

    #[gpui::test]
    fn clicking_a_table_row_drops_the_cursor_onto_its_source_line(cx: &mut TestAppContext) {
        let src = "intro\n\n|h1|h2|\n|-|-|\n|a|b|\n\ntail\n";
        let (_fx, editor, cx) = open_editor(cx, "table.md", src);
        assert_eq!(widget_count(&editor, cx), 1, "the table projects a widget");

        // The widget fills the vertical gap between the painted lines
        // around it; the header row sits at its top.
        let click = cx.update(|_, app| {
            let ed = editor.read(app);
            let above = ed.layout_cache.get(&1).expect("blank line above painted");
            let below = ed.layout_cache.get(&5).expect("blank line below painted");
            let widget_top = above.origin.y + above.line.size(above.line_height).height;
            assert!(below.origin.y - widget_top > px(30.), "widget occupies space");
            point(above.origin.x + px(40.), widget_top + px(15.))
        });
        cx.simulate_click(click, Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, app| {
            let ed = editor.read(app);
            let header_start = ed.core.buffer.line_range(2).start;
            assert_eq!(ed.core.selection.head, header_start, "cursor lands on the header row");
            assert!(ed.focus_handle.is_focused(window));
        });
        assert_eq!(widget_count(&editor, cx), 0, "the table dissolves under the cursor");
    }

    #[gpui::test]
    fn missing_image_renders_fallback_and_click_dissolves_to_source(cx: &mut TestAppContext) {
        let src = "intro\n\n![pic](missing.png)\n\ntail\n";
        let (_fx, editor, cx) = open_editor(cx, "img.md", src);
        assert_eq!(widget_count(&editor, cx), 1, "the image projects a widget");

        let click = cx.update(|_, app| {
            let ed = editor.read(app);
            let above = ed.layout_cache.get(&1).expect("blank line above painted");
            let below = ed.layout_cache.get(&3).expect("blank line below painted");
            let widget_top = above.origin.y + above.line.size(above.line_height).height;
            point(above.origin.x + px(40.), (widget_top + below.origin.y) * 0.5)
        });
        cx.simulate_click(click, Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(
                ed.core.selection.head,
                ed.core.buffer.line_range(2).start,
                "click drops the cursor onto the image's source line"
            );
        });
        assert_eq!(widget_count(&editor, cx), 0);
    }

    #[gpui::test]
    fn existing_local_image_renders_the_image_widget(cx: &mut TestAppContext) {
        // Minimal valid 1x1 transparent PNG.
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49,
            0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
            0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44,
            0x41, 0x54, 0x78, 0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D,
            0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42,
            0x60, 0x82,
        ];
        let (fx, editor, cx) = open_editor(cx, "img.md", "intro\n\n![p](pic.png)\n");
        std::fs::write(fx.path.parent().unwrap().join("pic.png"), PNG).unwrap();
        editor.update_in(cx, |_, _, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(widget_count(&editor, cx), 1, "existing image stays a widget");
    }

    // ── scrolling: wheel, scrollbar, animated outline jumps ────────────

    fn long_doc() -> String {
        (0..300).map(|i| format!("line {i}\n")).collect()
    }

    #[gpui::test]
    fn wheel_scrolling_moves_the_list_and_notifies(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "long.md", &long_doc());
        assert_eq!(scroll_offset_y(&editor, cx), px(0.));
        let center = cx.update(|_, app| {
            editor.read(app).list_state.viewport_bounds().center()
        });
        cx.simulate_event(ScrollWheelEvent {
            position: center,
            delta: ScrollDelta::Lines(point(0., -5.)),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        cx.run_until_parked();
        assert!(
            scroll_offset_y(&editor, cx) > px(0.),
            "wheel scroll moves the list down"
        );
    }

    #[gpui::test]
    fn scrollbar_drag_scrubs_through_the_document(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "long.md", &long_doc());
        let vp = cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(
                ed.list_state.max_offset_for_scrollbar().height > px(0.),
                "long doc overflows the viewport"
            );
            ed.list_state.viewport_bounds()
        });
        let track_x = vp.origin.x + vp.size.width - px(6.);

        cx.simulate_mouse_down(
            point(track_x, vp.origin.y + vp.size.height * 0.8),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.update(|_, app| assert!(editor.read(app).scrollbar_dragging));

        // While dragging, gpui compensates the reported offset to hold
        // the thumb steady, so meaningful reads happen after release.
        cx.simulate_mouse_move(
            point(track_x, vp.origin.y + vp.size.height * 0.3),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.simulate_mouse_up(
            point(track_x, vp.origin.y + vp.size.height * 0.3),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.update(|_, app| assert!(!editor.read(app).scrollbar_dragging));
        assert!(
            scroll_offset_y(&editor, cx) > px(0.),
            "the drag scrolled into the document"
        );
    }

    #[gpui::test]
    fn scroll_to_line_animates_long_jumps_and_skips_short_ones(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "long.md", &long_doc());
        // Tiny jump: already at the top, no animation task.
        editor.update_in(cx, |ed, _, cx| ed.scroll_to_line(0, cx));
        cx.update(|_, app| assert!(editor.read(app).scroll_anim.is_none()));

        editor.update_in(cx, |ed, _, cx| ed.scroll_to_line(200, cx));
        cx.update(|_, app| {
            assert!(editor.read(app).scroll_anim.is_some(), "long jump animates")
        });
        for _ in 0..40 {
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(12));
        }
        cx.run_until_parked();
        assert!(
            scroll_offset_y(&editor, cx) > px(500.),
            "animation lands deep in the document"
        );
    }

    // ── vertical movement geometry ─────────────────────────────────────

    #[gpui::test]
    fn vertical_movement_navigates_wrapped_rows_and_line_edges(cx: &mut TestAppContext) {
        let text = format!("{}\ntail", "word ".repeat(60));
        let (_fx, editor, cx) = open_editor(cx, "wrap.md", &text);
        let line0_end = text.find('\n').unwrap();
        cx.update(|_, app| {
            let ed = editor.read(app);
            let entry = ed.layout_cache.get(&0).expect("wrapped line painted");
            assert!(
                entry.line.size(entry.line_height).height > entry.line_height,
                "the long line wraps into multiple rows"
            );
        });

        // Down from the first visual row stays inside the wrapped line.
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(5);
            cx.notify();
        });
        cx.dispatch_action(MoveDown);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.core.buffer.line_of_byte(ed.core.selection.head), 0);
            assert!(ed.core.selection.head > 5, "moved down a visual row");
        });
        cx.dispatch_action(MoveUp);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.core.selection.head < 10, "back near the start of row one");
        });
        // Up from the very first row clamps to offset 0.
        cx.dispatch_action(MoveUp);
        assert_eq!(head(&editor, cx), 0);

        // Down from the last wrapped row crosses into the painted neighbor.
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(line0_end);
            cx.notify();
        });
        cx.run_until_parked();
        cx.dispatch_action(MoveDown);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.core.buffer.line_of_byte(ed.core.selection.head), 1);
        });
        // And back up into the neighbor's bottom row.
        cx.dispatch_action(MoveUp);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.core.buffer.line_of_byte(ed.core.selection.head), 0);
        });

        // Down past the last line clamps to the end of the document.
        editor.update_in(cx, |ed, _, cx| {
            ed.core.set_cursor(line0_end + 3);
            cx.notify();
        });
        cx.run_until_parked();
        cx.dispatch_action(MoveDown);
        assert_eq!(head(&editor, cx), text.len());
    }

    #[gpui::test]
    fn vertical_movement_degrades_to_logical_lines_without_layout(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "alpha\nbeta\ngamma\n");
        editor.update_in(cx, |ed, _, cx| {
            // Neighbor missing from the cache: land on its line start.
            ed.core.set_cursor(2);
            ed.layout_cache.remove(&1);
            ed.vertical_move(1, false, cx);
            assert_eq!(ed.core.selection.head, ed.core.buffer.line_range(1).start);
            // Current line missing entirely: logical movement.
            ed.layout_cache.clear();
            ed.vertical_move(1, false, cx);
            assert_eq!(ed.core.selection.head, ed.core.buffer.line_range(2).start);
            ed.layout_cache.clear();
            ed.vertical_move(-1, false, cx);
            assert_eq!(ed.core.selection.head, ed.core.buffer.line_range(1).end);
        });
    }

    // ── IME protocol details ───────────────────────────────────────────

    #[gpui::test]
    fn ime_protocol_queries_use_utf16_and_painted_geometry(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "note.md", "héllo\nworld\n");
        // text_for_range round-trips through UTF-16 offsets.
        let mut actual = None;
        let text = editor.update_in(cx, |ed, window, cx| {
            ed.text_for_range(0..5, &mut actual, window, cx)
        });
        assert_eq!(text.as_deref(), Some("héllo"));
        assert_eq!(actual, Some(0..5));

        // Caret rectangle for the composition popup.
        let bounds = editor.update_in(cx, |ed, window, cx| {
            ed.bounds_for_range(0..1, Bounds::default(), window, cx)
        });
        let bounds = bounds.expect("line 0 is painted");
        assert_eq!(bounds.size.width, px(2.), "caret-width rectangle");
        assert!(bounds.size.height > px(0.));

        // Point → UTF-16 character index over the same glyphs.
        let p3 = point_for_index(&editor, cx, 0, 3);
        let ix = editor.update_in(cx, |ed, window, cx| {
            ed.character_index_for_point(p3, window, cx)
        });
        assert_eq!(ix, Some(2), "byte 3 (after the 2-byte é) is UTF-16 index 2");

        // Marking over an explicit range, with an explicit selection.
        editor.update_in(cx, |ed, window, cx| {
            ed.replace_and_mark_text_in_range(Some(0..1), "ab", Some(1..1), window, cx);
        });
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.text(), "abéllo\nworld\n");
            assert_eq!(ed.marked_range, Some(0..2));
            assert_eq!(ed.core.selection.range(), 1..1, "selection sits inside the mark");
        });
        // unmark_text drops the composition without editing.
        editor.update_in(cx, |ed, window, cx| ed.unmark_text(window, cx));
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.marked_range, None);
            assert_eq!(ed.text(), "abéllo\nworld\n");
        });
        // An empty replacement clears the marked range.
        editor.update_in(cx, |ed, window, cx| {
            ed.replace_and_mark_text_in_range(Some(0..2), "", None, window, cx);
        });
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.text(), "éllo\nworld\n");
            assert_eq!(ed.marked_range, None);
        });
    }

    // ── find guards and integration with edits ─────────────────────────

    #[gpui::test]
    fn find_guards_and_live_recompute_on_edit_and_reload(cx: &mut TestAppContext) {
        let (fx, editor, cx) = open_editor(cx, "note.md", "one two\n");
        // Cycling without a find bar is a no-op.
        cx.dispatch_action(FindNext);
        assert_eq!(head(&editor, cx), 0);
        // Recomputing without a find bar is a no-op.
        editor.update_in(cx, |ed, _, _| ed.recompute_matches("one"));

        cx.dispatch_action(OpenFind);
        // Empty query: cycling is a no-op.
        cx.dispatch_action(FindNext);
        cx.dispatch_action(FindPrev);
        assert_eq!(head(&editor, cx), 0);
        // Opening again just refocuses the existing input.
        cx.dispatch_action(OpenFind);
        cx.update(|window, app| {
            let ed = editor.read(app);
            let input = ed.find.as_ref().unwrap().input.clone();
            assert!(input.read(app).focus_handle.is_focused(window));
        });

        editor.update_in(cx, |ed, _, cx| {
            let input = ed.find.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| {
                input.content = "one".into();
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(editor.read(app).find.as_ref().unwrap().matches.len(), 1);
        });

        // Editing while the bar is open recomputes matches.
        editor.update_in(cx, |ed, _, cx| ed.insert_str("one ", cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(editor.read(app).find.as_ref().unwrap().matches.len(), 2);
        });

        // Reloading from disk recomputes them too.
        std::fs::write(&fx.path, "one one one\n").unwrap();
        editor.update_in(cx, |ed, _, cx| ed.reload_from_disk(cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(editor.read(app).find.as_ref().unwrap().matches.len(), 3);
        });
    }

    #[gpui::test]
    fn reload_from_disk_handles_missing_files_and_char_boundaries(cx: &mut TestAppContext) {
        let (fx, editor, cx) = open_editor(cx, "note.md", "abc");
        cx.dispatch_action(MoveRight);
        assert_eq!(head(&editor, cx), 1);

        // Vanished file: the buffer is left untouched.
        std::fs::remove_file(&fx.path).unwrap();
        editor.update_in(cx, |ed, _, cx| ed.reload_from_disk(cx));
        assert_eq!(buffer_text(&editor, cx), "abc");

        // New content puts the clamped cursor inside a multibyte char:
        // it backs up to the previous boundary.
        std::fs::write(&fx.path, "é\n").unwrap();
        editor.update_in(cx, |ed, _, cx| ed.reload_from_disk(cx));
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.text(), "é\n");
            assert_eq!(ed.core.selection.head, 0, "head backs off the é's mid-byte");
        });
    }

    // ── flush failure paths ────────────────────────────────────────────

    #[gpui::test]
    fn backup_failures_never_block_the_save(cx: &mut TestAppContext) {
        let (fx, editor, cx) = open_editor(cx, "note.md", "v1\n");
        // Re-root the session backups under a plain file so every
        // create_dir_all inside the registry fails.
        let blocker = fx.backups.path().join("blocker");
        std::fs::write(&blocker, "not a dir").unwrap();
        cx.update(|_, app| {
            app.set_global(SessionBackups(Arc::new(Mutex::new(
                autosave::BackupRegistry::new(blocker.join("backups")),
            ))));
        });

        cx.simulate_input("A");
        cx.dispatch_action(SaveNow);
        assert_eq!(
            std::fs::read_to_string(&fx.path).unwrap(),
            "Av1\n",
            "the save succeeds even though the backup failed"
        );
        cx.update(|_, app| assert!(!editor.read(app).save.is_dirty()));

        // Same failure on the conflict path: disk changed underneath us,
        // the forced backup fails, and the write still goes through.
        std::fs::write(&fx.path, "theirs\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        let f = std::fs::File::options().write(true).open(&fx.path).unwrap();
        f.set_modified(later).unwrap();
        cx.simulate_input("B");
        cx.dispatch_action(SaveNow);
        assert_eq!(std::fs::read_to_string(&fx.path).unwrap(), "ABv1\n");
    }

    #[cfg(unix)]
    #[gpui::test]
    fn failed_write_keeps_the_buffer_dirty(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;
        let (fx, editor, cx) = open_editor(cx, "note.md", "v1\n");
        cx.simulate_input("A");
        let dir = fx.path.parent().unwrap().to_path_buf();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        cx.dispatch_action(SaveNow);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&fx.path).unwrap(),
            "v1\n",
            "the write never happened"
        );
        cx.update(|_, app| {
            assert!(editor.read(app).save.is_dirty(), "a failed save stays dirty and retries")
        });
    }

    // ── styled rendering ───────────────────────────────────────────────

    #[gpui::test]
    fn rich_markdown_renders_every_style_kind(cx: &mut TestAppContext) {
        let src = "# Title\n\n**bold** *em* ~~gone~~ `code` [l](https://x)\n\n- item\n- [ ] task\n1. ordered\n\n> quote\n\n***\n\n```rust\n// note\nlet s = \"hi\";\n```\n\ntext\n";
        let (_fx, editor, cx) = open_editor(cx, "rich.md", src);
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(!ed.is_code_mode());
            assert_eq!(ed.heading_lines(), vec![(1, "Title".to_string(), 0)]);
            assert!(
                ed.line_kinds.iter().any(|k| matches!(k, LineKind::Code)),
                "fence content lines are marked as code"
            );
        });
        // Select everything so lines render both with markers revealed
        // and with whole-line selection quads.
        cx.dispatch_action(SelectAll);
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.core.selection.range(), 0..ed.core.buffer.len_bytes());
        });
    }

    #[gpui::test]
    fn wrapped_selection_paints_across_visual_rows(cx: &mut TestAppContext) {
        let text = "word ".repeat(80);
        let (_fx, editor, cx) = open_editor(cx, "wrap.md", &text);
        cx.dispatch_action(SelectAll);
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            let entry = ed.layout_cache.get(&0).expect("line painted");
            assert!(
                entry.line.size(entry.line_height).height >= entry.line_height * 3.,
                "selection spans at least three visual rows"
            );
        });
    }

    // ── diff mode against a real repository ────────────────────────────

    #[gpui::test]
    fn diff_view_shows_word_level_changes_against_head(cx: &mut TestAppContext) {
        let repo = tempfile::tempdir().unwrap();
        sh_git(repo.path(), &["init", "-q"]);
        let file = repo.path().join("note.md");
        std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
        commit_all(repo.path());
        std::fs::write(&file, "alpha\nBETA now\ngamma\ndelta\n").unwrap();

        let (_bk, editor, cx) = open_editor_path(cx, &file);
        editor.update_in(cx, |ed, _, cx| {
            let langs = crate::highlight::languages(cx);
            ed.enter_diff(&langs, cx);
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.diff_active());
            let d = ed.diff.as_ref().unwrap();
            assert!(d.missing.is_none(), "a committed baseline was found");
            assert!(d.adds > 0, "added words counted");
            assert!(d.dels > 0, "deleted words counted");
            assert!(!d.changes.is_empty());
            let merged = ed.view_buffer().text();
            assert!(merged.contains("beta"), "deleted text stays in the merged doc");
            assert!(merged.contains("BETA now"));
            assert!(merged.contains("delta"));
        });

        // The diff view is read-only: clicking never touches the buffer
        // or the selection.
        let before = buffer_text(&editor, cx);
        let sel_before = cx.update(|_, app| editor.read(app).core.selection.range());
        let target = cx.update(|_, app| {
            let ed = editor.read(app);
            let entry = ed.layout_cache.get(&0).expect("diff line painted");
            point(
                entry.origin.x + px(4.),
                entry.origin.y + entry.line_height * 0.5,
            )
        });
        cx.simulate_mouse_down(target, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
        assert_eq!(buffer_text(&editor, cx), before);
        cx.update(|_, app| {
            assert_eq!(editor.read(app).core.selection.range(), sel_before);
        });

        // Outline jumps are disabled while diffing.
        editor.update_in(cx, |ed, _, cx| ed.scroll_to_line(2, cx));
        cx.update(|_, app| assert!(editor.read(app).scroll_anim.is_none()));

        // refresh recomputes in place; exit restores the projection.
        editor.update_in(cx, |ed, _, cx| {
            let langs = crate::highlight::languages(cx);
            ed.refresh_diff(&langs, cx);
        });
        cx.run_until_parked();
        cx.update(|_, app| assert!(editor.read(app).diff_active()));
        editor.update_in(cx, |ed, _, cx| ed.exit_diff(cx));
        cx.run_until_parked();
        cx.update(|_, app| assert!(!editor.read(app).diff_active()));
    }

    /// Diff mode hides every marker, so a changed `---` shows only its
    /// divider -- and the diff wash is painted per character, on
    /// characters that are no longer drawn. The divider itself has to
    /// carry the change, or a section break added or removed is
    /// invisible to the person reviewing it.
    #[gpui::test]
    fn a_changed_rule_carries_its_diff_colour_in_the_diff_view(cx: &mut TestAppContext) {
        let repo = tempfile::tempdir().unwrap();
        sh_git(repo.path(), &["init", "-q"]);
        let file = repo.path().join("note.md");
        std::fs::write(&file, "a\n\n---\n\nb\n\nkeep\n\n___\n\nc\n").unwrap();
        commit_all(repo.path());
        std::fs::write(&file, "a\n\nb\n\nkeep\n\n___\n\nc\n\n***\n\nd\n").unwrap();

        let (_bk, editor, cx) = open_editor_path(cx, &file);
        let t = crate::theme::Theme::dark();
        let (_, plain) = crate::view::rule_style(&t);
        editor.update_in(cx, |ed, _, cx| {
            let langs = crate::highlight::languages(cx);
            ed.enter_diff(&langs, cx);
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            let merged = ed.view_buffer().text();
            let line_of = |needle: &str| merged.split('\n').position(|l| l == needle).unwrap();
            assert_eq!(ed.rule_color(line_of("---"), &t), t.diff_deleted_fg, "a removed break");
            assert_eq!(ed.rule_color(line_of("***"), &t), t.diff_added_fg, "an added break");
            assert_eq!(ed.rule_color(line_of("___"), &t), plain, "an unchanged break");
        });
        editor.update_in(cx, |ed, _, cx| ed.exit_diff(cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            let line = ed.core.buffer.text().split('\n').position(|l| l == "***").unwrap();
            assert_eq!(ed.rule_color(line, &t), plain, "outside diff mode, the plain rule");
        });
    }

    #[gpui::test]
    fn diff_view_on_code_files_shows_diff_gutter_labels(cx: &mut TestAppContext) {
        let repo = tempfile::tempdir().unwrap();
        sh_git(repo.path(), &["init", "-q"]);
        let file = repo.path().join("main.rs");
        std::fs::write(&file, "fn main() {\n}\n").unwrap();
        commit_all(repo.path());
        std::fs::write(&file, "fn main() {\n    let x = 1;\n}\n").unwrap();

        let (_bk, editor, cx) = open_editor_path(cx, &file);
        editor.update_in(cx, |ed, _, cx| {
            let langs = crate::highlight::languages(cx);
            ed.enter_diff(&langs, cx);
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert!(ed.is_code_mode());
            let d = ed.diff.as_ref().unwrap();
            assert!(d.missing.is_none());
            assert!(!d.gutter.is_empty(), "code diffs carry gutter labels");
            // Diff gutter labels come from the diff doc, not raw indices.
            assert_eq!(ed.gutter_label(0), d.gutter[0]);
        });
    }

    #[gpui::test]
    fn diff_view_reports_untracked_files(cx: &mut TestAppContext) {
        let repo = tempfile::tempdir().unwrap();
        sh_git(repo.path(), &["init", "-q"]);
        std::fs::write(repo.path().join("old.md"), "x\n").unwrap();
        commit_all(repo.path());
        // Plain-text provider exercises the no-highlight diff path too.
        let file = repo.path().join("fresh.txt");
        std::fs::write(&file, "brand new\n").unwrap();

        let (_bk, editor, cx) = open_editor_path(cx, &file);
        editor.update_in(cx, |ed, _, cx| {
            let langs = crate::highlight::languages(cx);
            ed.enter_diff(&langs, cx);
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let d = editor.read(app).diff.as_ref().unwrap().missing.as_ref();
            assert!(matches!(d, Some(crate::git::Baseline::Untracked)));
        });
    }

    #[gpui::test]
    fn diff_view_reports_binary_baselines(cx: &mut TestAppContext) {
        let repo = tempfile::tempdir().unwrap();
        sh_git(repo.path(), &["init", "-q"]);
        let file = repo.path().join("data.md");
        std::fs::write(&file, [0u8, 159, 146, 150]).unwrap();
        commit_all(repo.path());
        std::fs::write(&file, "now text\n").unwrap();

        let (_bk, editor, cx) = open_editor_path(cx, &file);
        editor.update_in(cx, |ed, _, cx| {
            let langs = crate::highlight::languages(cx);
            ed.enter_diff(&langs, cx);
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let d = editor.read(app).diff.as_ref().unwrap().missing.as_ref();
            assert!(matches!(d, Some(crate::git::Baseline::Binary)));
        });
    }

    #[gpui::test]
    fn diff_view_with_no_changes_shows_the_empty_state(cx: &mut TestAppContext) {
        let repo = tempfile::tempdir().unwrap();
        sh_git(repo.path(), &["init", "-q"]);
        let file = repo.path().join("clean.md");
        std::fs::write(&file, "same\n").unwrap();
        commit_all(repo.path());

        let (_bk, editor, cx) = open_editor_path(cx, &file);
        editor.update_in(cx, |ed, _, cx| {
            let langs = crate::highlight::languages(cx);
            ed.enter_diff(&langs, cx);
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let ed = editor.read(app);
            let d = ed.diff.as_ref().unwrap();
            assert!(d.missing.is_none());
            assert!(d.changes.is_empty(), "identical content diffs to nothing");
            assert_eq!((d.adds, d.dels), (0, 0));
        });
    }
}
