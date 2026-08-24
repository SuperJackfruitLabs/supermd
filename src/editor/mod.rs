//! The Editor: GPUI shell around the tested core. Renders one logical
//! line per virtualized list item with styled-source typography; input
//! flows through EntityInputHandler (IME-correct) and editor actions.

pub mod autosave;
pub mod blocks;
pub mod buffer;
pub mod core;
pub mod display;
pub mod find;
pub mod movement;
pub mod projection;
pub mod projector;
pub mod spans;

use std::collections::HashMap;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use gpui::prelude::*;
use gpui::{
    actions, div, fill, list, point, px, relative, size, App, AvailableSpace, Bounds,
    ClipboardItem, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    Focusable, Font, FontFeatures, FontStyle, FontWeight, GlobalElementId, Hsla, IntoElement,
    LayoutId, ListAlignment, ListOffset, ListState, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, Render, SharedString, StrikethroughStyle, Style,
    TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine,
};

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
        OpenFind, FindNext, FindPrev, CloseFind
    ]
);

struct FindState {
    input: Entity<crate::input::TextInput>,
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
    core: EditorCore,
    provider: Provider,
    diff: Option<DiffState>,
    spans: Vec<StyleSpan>,
    line_kinds: Vec<LineKind>,
    blocks: Vec<blocks::BlockInfo>,
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
}

/// Snapshot taken right after a paste lands, so a background enricher
/// can replace the pasted range iff the document has not moved.
struct PendingEnrich {
    range: Range<usize>,
    snapshot: String,
    pasted: String,
}

pub enum EditorEvent {
    /// A net-capable enricher needs a per-domain grant
    /// (cap is "net:<domain>").
    ConsentNeeded { plugin: String, cap: String },
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

impl Editor {
    /// Read a file's text. Call `from_text` inside `cx.new` (which cannot
    /// be fallible) with the result.
    pub fn read_file(path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub fn from_text(path: &Path, text: String, langs: &Languages, cx: &mut Context<Self>) -> Self {
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
            preferred_x: None,
            save_task: None,
            find: None,
            scrollbar_dragging: false,
            scroll_anim: None,
            pending_enrich: None,
            status_text: None,
            status_task: None,
        };
        editor.restyle(langs);
        editor.schedule_status(cx);
        editor
    }

    pub fn text(&self) -> String {
        self.core.buffer.text()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn title(&self) -> SharedString {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
            .into()
    }

    fn restyle(&mut self, langs: &Languages) {
        let text = self.core.buffer.text();
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
            self.projection = items;
            self.list_state.reset(self.projection.len());
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
        cx.notify();
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
        let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() else {
            return;
        };
        let host = state.0.clone();
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
        let plugins = crate::extensions::format_plugins();
        let Some(plugin) = plugins.first() else {
            return;
        };
        let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() else {
            return;
        };
        let snapshot = self.core.buffer.text();
        let result = state.0.lock().unwrap().format_document(plugin, &snapshot);
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
        let plugins = crate::extensions::hook_plugins();
        if plugins.is_empty() {
            return;
        }
        let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() else {
            return;
        };
        let snapshot = self.core.buffer.text();
        let path = self.path.to_string_lossy().into_owned();
        let result = chain_save_hooks(snapshot.clone(), &path, &plugins, |p, path, doc| {
            state.0.lock().unwrap().on_save(p, path, doc)
        });
        if result != snapshot {
            self.apply_command_output(
                &crate::extensions::CommandOutput::ReplaceDocument(result),
                cx,
            );
        }
    }

    pub fn flush(&mut self, cx: &mut Context<Self>) {
        self.maybe_format_before_save(cx);
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
        self.vertical_move(-1, false, cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
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
        if self.is_code_mode() {
            self.core.insert_newline_auto_indent(Instant::now());
            self.after_edit(cx);
        } else {
            self.insert_str("\n", cx);
        }
    }

    fn insert_tab(&mut self, _: &InsertTab, _: &mut Window, cx: &mut Context<Self>) {
        self.insert_str("\t", cx);
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
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            // Paste processors: first Some wins; errors/None pass the
            // original through. Synchronous under the epoch deadline —
            // paste is an explicit action.
            let mut out = text.clone();
            let paste_plugins = crate::extensions::paste_plugins();
            if !paste_plugins.is_empty() {
                if let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() {
                    let mut host = state.0.lock().unwrap();
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

    /// Run net-capable paste plugins in the background; first Some
    /// wins. A consent-shaped failure keeps `pending_enrich` so the
    /// workspace can retry after the grant.
    fn start_enrich(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_enrich.as_ref() else {
            return;
        };
        let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() else {
            return;
        };
        let host = state.0.clone();
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

    fn open_find(&mut self, _: &OpenFind, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = &self.find {
            window.focus(&state.input.read(cx).focus_handle);
            return;
        }
        let input = cx.new(|cx| crate::input::TextInput::new("Find…", cx));
        let watch = cx.observe(&input, |this: &mut Editor, input, cx| {
            let query = input.read(cx).content.to_string();
            this.recompute_matches(&query);
            cx.notify();
        });
        window.focus(&input.read(cx).focus_handle);
        self.find = Some(FindState { input, matches: Vec::new(), active: 0, _watch: watch });
        cx.notify();
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

    fn on_root_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.scrollbar_dragging {
            self.scrollbar_scrub(event.position, cx);
            return;
        }
        if self.dragging {
            if let Some(offset) = self.offset_at_point(event.position) {
                self.core.select_to(offset);
                cx.notify();
            }
        }
    }

    fn on_root_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.dragging = false;
        if self.scrollbar_dragging {
            self.scrollbar_dragging = false;
            self.list_state.scrollbar_drag_ended();
            cx.notify();
        }
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
            _ => (t.body_size, FontWeight::NORMAL, t.body_family.clone(), 1.65),
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
            && !matches!(self.view_line_kinds().get(ix), Some(LineKind::Code))
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
    {
        let ed = editor.read(cx);
        for line in lines {
            let text = ed.core.buffer.line_text(line);
            if blocks::is_separator_row(&text) {
                continue;
            }
            rows.push((line, blocks::parse_row(&text)));
        }
    }
    let ncols = rows.iter().map(|(_, cells)| cells.len()).max().unwrap_or(1);

    let mut container = div()
        .my_1()
        .rounded_lg()
        .border_1()
        .border_color(t.border)
        .font_family(t.body_family.clone())
        .flex()
        .flex_col()
        .overflow_hidden();

    for (row_ix, (line, cells)) in rows.into_iter().enumerate() {
        let is_header = row_ix == 0;
        let handle = editor.clone();
        let mut row = div()
            .id(("trow", item_ix * 1024 + row_ix))
            .flex()
            .flex_row()
            .w_full()
            .cursor_pointer()
            .when(is_header, |d| {
                d.bg(t.panel_bg)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.fg_strong)
            })
            .when(!is_header, |d| {
                d.border_t_1()
                    .border_color(t.border)
                    .text_color(t.fg)
                    .hover(|s| s.bg(t.hover_bg))
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
            row = row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_3()
                    .py_2()
                    .text_size(px(t.body_size - 1.))
                    .line_height(relative(1.45))
                    .child(SharedString::from(cell)),
            );
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
    let is_remote = dest.starts_with("http://") || dest.starts_with("https://");
    let local_path = (!is_remote).then(|| {
        editor
            .read(cx)
            .path()
            .parent()
            .map(|dir| dir.join(dest))
            .unwrap_or_else(|| PathBuf::from(dest))
    });

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

    let available = match &local_path {
        Some(path) => path.exists(),
        None => true, // remote: let gpui's loader handle it
    };
    if !available {
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

    let image = match local_path {
        Some(path) => gpui::img(path),
        None => gpui::img(dest.to_string()),
    };
    div()
        .id(("img", item_ix))
        .my_1()
        .w_full()
        .cursor_pointer()
        .on_click(on_click)
        .child(image.w_full().max_h(px(420.)).rounded_md())
        .into_any_element()
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.inline_gen != crate::extensions::inline_generation()
            && matches!(self.provider, Provider::Markdown)
        {
            let langs = crate::highlight::languages(cx);
            self.restyle(&langs);
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
        let diff_empty: Option<&'static str> = self.diff.as_ref().and_then(|d| {
            use crate::git::Baseline;
            match &d.missing {
                Some(Baseline::NotInRepo) => Some("Not in a git repository."),
                Some(Baseline::Untracked) => Some("Not tracked in git yet."),
                Some(Baseline::Binary) => Some("No text baseline at HEAD."),
                _ => d.changes.is_empty().then_some("No uncommitted changes."),
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
        });

        div()
            .size_full()
            .bg(t.bg)
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
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::save_now))
            .on_action(cx.listener(Self::open_find))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_prev))
            .on_action(cx.listener(Self::close_find))
            .on_mouse_move(cx.listener(Self::on_root_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_root_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_root_mouse_up))
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
                            ) = {
                                let editor = editor_entity.read(cx);
                                let (size_f, _, _, mult) = editor.line_typography(line_ix, &t);
                                let (text, runs, dl) = editor.display_for_line(line_ix, &t);
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
                                )
                            };
                            let mouse_editor = editor_entity.clone();
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
                                            .child(line_el),
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
                .into_any_element()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{KeyBinding, TestAppContext, VisualTestContext};
    use std::sync::{Arc, Mutex};

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
        cx.run_until_parked();
        (Fixture { _files: files, backups, path }, editor, cx)
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
        cx.run_until_parked();
        (backups, editor, cx)
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
