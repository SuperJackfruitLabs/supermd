//! Project-wide search overlay (⌘⇧F): a finder-style two-pane dialog
//! fed by the streaming engine in `search.rs` running on the
//! background executor.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    actions, div, px, uniform_list, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, HighlightStyle, IntoElement, ParentElement, Render, SharedString, Styled,
    StyledText, Subscription, Window,
};

use crate::input::TextInput;
use crate::search::{self, SearchMatch, SEARCH_CAP};
use crate::theme::theme;

actions!(search_ui, [SearchUp, SearchDown, SearchConfirm, SearchDismiss]);

pub enum SearchEvent {
    /// Open `path` (absolute) at 1-based `line` in a permanent tab.
    Open { path: PathBuf, line: u64 },
    Dismissed,
}

/// One visual row: a file header or a hit (both index into `matches`;
/// a header points at the first hit of its file).
#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Hit(usize),
}

pub struct SearchOverlay {
    pub input: Entity<TextInput>,
    root: PathBuf,
    matches: Vec<SearchMatch>,
    /// Index into `matches` (hits only).
    selected: usize,
    capped: bool,
    searching: bool,
    /// Bumped per restart so stale streams are ignored.
    generation: u64,
    cancelled: Arc<AtomicBool>,
    last_query: String,
    preview: Option<(PathBuf, u64, SharedString, usize)>, // path, line, text, line-offset of window
    scroll: gpui::UniformListScrollHandle,
    _watch_input: Subscription,
}

impl EventEmitter<SearchEvent> for SearchOverlay {}

impl SearchOverlay {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search in workspace…", cx));
        let watch = cx.observe(&input, |this: &mut SearchOverlay, _, cx| {
            let query = this.input.read(cx).content.to_string();
            if query != this.last_query {
                this.last_query = query;
                this.restart(cx);
            }
        });
        Self {
            input,
            root,
            matches: Vec::new(),
            selected: 0,
            capped: false,
            searching: false,
            generation: 0,
            cancelled: Arc::new(AtomicBool::new(false)),
            last_query: String::new(),
            preview: None,
            scroll: gpui::UniformListScrollHandle::default(),
            _watch_input: watch,
        }
    }

    /// Cancel any running search and start a fresh one for the current
    /// query (after a 120 ms debounce).
    fn restart(&mut self, cx: &mut Context<Self>) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.cancelled = Arc::new(AtomicBool::new(false));
        self.generation += 1;
        self.matches.clear();
        self.selected = 0;
        self.capped = false;
        self.preview = None;
        let query = self.last_query.clone();
        if query.is_empty() {
            self.searching = false;
            cx.notify();
            return;
        }
        self.searching = true;
        cx.notify();

        let generation = self.generation;
        let root = self.root.clone();
        let cancelled = self.cancelled.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<SearchMatch>>();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            let search_cancel = cancelled.clone();
            cx.background_executor()
                .spawn(async move {
                    search::search_workspace(&root, &query, &search_cancel, tx);
                })
                .detach();
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let mut batch: Vec<SearchMatch> = Vec::new();
                let mut done = false;
                loop {
                    match rx.try_recv() {
                        Ok(b) => batch.extend(b),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            done = true;
                            break;
                        }
                    }
                }
                let live = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        if !batch.is_empty() {
                            this.matches.extend(batch);
                            if this.matches.len() >= SEARCH_CAP {
                                this.capped = true;
                            }
                            cx.notify();
                        }
                        if done {
                            this.searching = false;
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !live || done {
                    break;
                }
            }
        })
        .detach();
    }

    /// Visual rows: file header whenever the path changes, then hits.
    fn rows(&self) -> Vec<Row> {
        let mut out = Vec::with_capacity(self.matches.len() + 8);
        let mut prev: Option<&PathBuf> = None;
        for (ix, m) in self.matches.iter().enumerate() {
            if prev != Some(&m.path) {
                out.push(Row::File(ix));
                prev = Some(&m.path);
            }
            out.push(Row::Hit(ix));
        }
        out
    }

    fn reveal_selected(&self) {
        let row_ix = self
            .rows()
            .iter()
            .position(|r| matches!(r, Row::Hit(ix) if *ix == self.selected))
            .unwrap_or(0);
        self.scroll.scroll_to_item(row_ix, gpui::ScrollStrategy::Center);
    }

    fn up(&mut self, _: &SearchUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
            self.reveal_selected();
            cx.notify();
        }
    }

    fn down(&mut self, _: &SearchDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
            self.reveal_selected();
            cx.notify();
        }
    }

    fn confirm(&mut self, _: &SearchConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_index(self.selected, cx);
    }

    fn confirm_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(m) = self.matches.get(ix) {
            cx.emit(SearchEvent::Open {
                path: self.root.join(&m.path),
                line: m.line_number,
            });
        }
    }

    fn dismiss(&mut self, _: &SearchDismiss, _: &mut Window, cx: &mut Context<Self>) {
        self.cancelled.store(true, Ordering::Relaxed);
        cx.emit(SearchEvent::Dismissed);
    }

    /// Load (cached) a ±20-line window around the selected match.
    fn preview_for_selected(&mut self) -> Option<(SharedString, usize, u64)> {
        let m = self.matches.get(self.selected)?;
        let cached = self
            .preview
            .as_ref()
            .is_some_and(|(p, l, _, _)| p == &m.path && *l == m.line_number);
        if !cached {
            let abs = self.root.join(&m.path);
            let bytes = std::fs::read(&abs).ok()?;
            let text = String::from_utf8_lossy(&bytes);
            let line0 = (m.line_number as usize).saturating_sub(1);
            let start = line0.saturating_sub(20);
            let window: String = text
                .lines()
                .skip(start)
                .take(45)
                .map(|l| {
                    let mut l = l.to_string();
                    if l.len() > 400 {
                        let mut cut = 400;
                        while !l.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        l.truncate(cut);
                    }
                    l.push('\n');
                    l
                })
                .collect();
            self.preview = Some((m.path.clone(), m.line_number, window.into(), start));
        }
        let (_, _, text, start) = self.preview.as_ref()?;
        Some((text.clone(), *start, m.line_number))
    }
}

impl Focusable for SearchOverlay {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.read(cx).focus_handle.clone()
    }
}

impl Render for SearchOverlay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let rows = self.rows();
        let selected = self.selected;

        let status: SharedString = if self.last_query.is_empty() {
            "Type to search the workspace".into()
        } else if self.searching && self.matches.is_empty() {
            "Searching…".into()
        } else if self.matches.is_empty() {
            "No matches".into()
        } else if self.capped {
            format!("{SEARCH_CAP}+ matches (capped)").into()
        } else {
            format!("{} matches", self.matches.len()).into()
        };

        let preview_pane: gpui::AnyElement = match self.preview_for_selected() {
            Some((text, start, line_number)) => {
                let hit_row = (line_number as usize).saturating_sub(1) - start;
                let lines: Vec<gpui::AnyElement> = text
                    .lines()
                    .enumerate()
                    .map(|(i, l)| {
                        div()
                            .px_3()
                            .w_full()
                            .when(i == hit_row, |d| d.bg(t.find_match_bg))
                            .child(SharedString::from(if l.is_empty() {
                                " ".to_string()
                            } else {
                                l.to_string()
                            }))
                            .into_any_element()
                    })
                    .collect();
                div()
                    .size_full()
                    .overflow_hidden()
                    .py_2()
                    .font_family(t.mono_family.clone())
                    .text_size(px(11.))
                    .line_height(gpui::relative(1.45))
                    .text_color(t.fg_muted)
                    .flex()
                    .flex_col()
                    .children(lines)
                    .into_any_element()
            }
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(t.fg_muted)
                .child(status.clone())
                .into_any_element(),
        };

        let matches = self.matches.clone();
        let results = uniform_list(
            "search-results",
            rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = theme(cx);
                range
                    .filter_map(|row_ix| {
                        let row = *rows.get(row_ix)?;
                        Some(match row {
                            Row::File(ix) => {
                                let m = &matches[ix];
                                let name = m.path.to_string_lossy();
                                let (icon, color) = crate::seti::icon_for(&name);
                                div()
                                    .id(row_ix)
                                    .w_full()
                                    .h(px(26.))
                                    .px_2()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        gpui::svg()
                                            .path(SharedString::from(format!(
                                                "icons/seti/{icon}.svg"
                                            )))
                                            .size(px(16.))
                                            .flex_none()
                                            .text_color(crate::workspace::seti_tint(color, &t)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(t.fg_strong)
                                            .overflow_hidden()
                                            .child(SharedString::from(name.into_owned())),
                                    )
                            }
                            Row::Hit(ix) => {
                                let m = &matches[ix];
                                let is_selected = ix == selected;
                                let highlights: Vec<_> = m
                                    .ranges
                                    .iter()
                                    .map(|r| {
                                        (
                                            r.clone(),
                                            HighlightStyle {
                                                background_color: Some(t.find_match_bg),
                                                color: Some(t.fg_strong),
                                                ..Default::default()
                                            },
                                        )
                                    })
                                    .collect();
                                div()
                                    .id(row_ix)
                                    .w_full()
                                    .h(px(24.))
                                    .pl(px(26.))
                                    .pr_2()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .cursor_pointer()
                                    .when(is_selected, |d| d.bg(t.selected_bg))
                                    .when(!is_selected, |d| d.hover(|s| s.bg(t.hover_bg)))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(t.fg_muted)
                                            .flex_none()
                                            .min_w(px(28.))
                                            .child(SharedString::from(m.line_number.to_string())),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(t.fg)
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(
                                                StyledText::new(m.line_text.clone())
                                                    .with_highlights(highlights),
                                            ),
                                    )
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                        this.selected = ix;
                                        this.confirm_index(ix, cx);
                                    }))
                            }
                        })
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .h_full();

        div()
            .key_context("Search")
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(680.))
            .max_w(gpui::relative(0.9))
            .h(px(440.))
            .flex_none()
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .rounded_lg()
            .shadow_lg()
            .overflow_hidden()
            .flex()
            .flex_row()
            .child(
                div()
                    .w(px(320.))
                    .flex_none()
                    .h_full()
                    .border_r_1()
                    .border_color(t.border)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(t.border)
                            .child(self.input.clone()),
                    )
                    .child(div().flex_1().min_h_0().child(results))
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .border_t_1()
                            .border_color(t.border)
                            .text_size(px(10.))
                            .text_color(t.fg_muted)
                            .child(status),
                    ),
            )
            .child(div().flex_1().min_w_0().h_full().child(preview_pane))
    }
}
