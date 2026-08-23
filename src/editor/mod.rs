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
    Code(&'static str),
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

pub struct Editor {
    core: EditorCore,
    provider: Provider,
    spans: Vec<StyleSpan>,
    line_kinds: Vec<LineKind>,
    blocks: Vec<blocks::BlockInfo>,
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
            spans: Vec::new(),
            line_kinds: Vec::new(),
            blocks: Vec::new(),
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
        };
        editor.restyle(langs);
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
        self.spans = match self.provider {
            Provider::Markdown => spans::markdown_spans_highlighted(&text, langs),
            Provider::Code(lang) => spans::code_spans(&text, lang, langs),
            Provider::Plain => Vec::new(),
        };
        self.line_kinds = spans::line_kinds(&text, &self.spans);
        self.blocks = match self.provider {
            Provider::Markdown => blocks::blocks(&text),
            _ => Vec::new(),
        };
        self.layout_cache.clear();
        self.projection = self.compute_projection();
        self.list_state.reset(self.projection.len());
    }

    fn compute_projection(&self) -> Vec<projection::Item> {
        let line_ranges: Vec<Range<usize>> = (0..self.core.buffer.line_count())
            .map(|ix| self.core.buffer.line_range(ix))
            .collect();
        projection::project(&line_ranges, &self.blocks, self.core.selection.range())
    }

    /// Recompute the projection for the current selection; reset the
    /// list only when the item structure actually changed.
    fn reproject(&mut self) {
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
        cx.notify();
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
    pub fn flush(&mut self, cx: &mut Context<Self>) {
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
            self.insert_str(&text, cx);
        }
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
        match self.line_kinds.get(ix) {
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
        let range = self.core.buffer.line_range(ix);
        let text = self.core.buffer.line_text(ix);
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
        for span in &self.spans {
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
        let range = self.core.buffer.line_range(ix);
        let (text, attrs) = self.line_attrs(ix, t);
        let dl = display::display_line(
            &text,
            range.start,
            &self.spans,
            self.core.selection.range(),
        );

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
            let sel = editor.core.selection;
            let head = sel.head;
            (
                sel.range(),
                (head >= self.range.start && head <= self.range.end).then_some(head),
            )
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
            (cursor_line == self.line_ix, editor.focus_handle.clone())
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

        let find_bar = self.find.as_ref().map(|state| {
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
            .key_context("Editor")
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
            .children(find_bar)
            .child(div().flex_1().min_h_0().relative().child(
                list(self.list_state.clone(), move |ix, _window, cx| {
                    let Some(editor_entity) = entity.upgrade() else {
                        return div().into_any_element();
                    };
                    let t = theme(cx);
                    let item = editor_entity.read(cx).projection.get(ix).cloned();
                    let item_count = editor_entity.read(cx).projection.len();
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
                                    editor.core.buffer.line_range(line_ix),
                                    text,
                                    runs,
                                    dl,
                                    px(size_f),
                                    px(size_f * mult),
                                    matches!(editor.line_kinds.get(line_ix), Some(LineKind::Code)),
                                    editor.is_code_mode(),
                                    editor.core.buffer.line_count(),
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
                                                (line_ix + 1).to_string(),
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
                        Some(projection::Item::Table { lines }) => column(
                            render_table(&editor_entity, ix, lines, &t, cx),
                        ),
                        Some(projection::Item::Image { line, alt, dest }) => column(
                            render_image(&editor_entity, ix, line, &alt, &dest, &t, cx),
                        ),
                        None => div().into_any_element(),
                    }
                })
                .size_full(),
            ).children(scrollbar))
    }
}
