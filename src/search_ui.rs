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
            cx.processor(move |_this, range: std::ops::Range<usize>, _window, cx| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    fn open_overlay(
        cx: &mut TestAppContext,
        root: PathBuf,
    ) -> (gpui::Entity<SearchOverlay>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(
                crate::theme::Theme::dark(),
            )))
        });
        let (overlay, cx) = cx.add_window_view(|_, cx| SearchOverlay::new(root, cx));
        cx.update(|window, app| {
            let handle = overlay.read(app).focus_handle(app);
            window.focus(&handle);
        });
        cx.run_until_parked();
        (overlay, cx)
    }

    fn set_query(overlay: &gpui::Entity<SearchOverlay>, cx: &mut VisualTestContext, q: &str) {
        overlay.update_in(cx, |o, _, cx| {
            o.input.update(cx, |input, cx| {
                input.content = q.to_string().into();
                cx.notify();
            });
        });
        cx.run_until_parked();
    }

    /// Drive the debounce (120 ms) and the 50 ms channel-poll loop until
    /// the overlay reports the stream finished.
    fn settle(overlay: &gpui::Entity<SearchOverlay>, cx: &mut VisualTestContext) {
        for _ in 0..100 {
            cx.executor().advance_clock(Duration::from_millis(60));
            cx.run_until_parked();
            if !cx.update(|_, app| overlay.read(app).searching) {
                return;
            }
        }
        panic!("search never settled");
    }

    /// alpha.md (2 hits for "alpha"), sub/beta.md (1 hit), wrap.md
    /// (3 hits for "wrapme"), solo.txt (1 hit for "unique-needle").
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.md"), "one alpha\ntwo\nthree alpha\n").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/beta.md"), "beta alpha here\n").unwrap();
        std::fs::write(dir.path().join("wrap.md"), "wrapme\nx wrapme\nwrapme y\n").unwrap();
        std::fs::write(dir.path().join("solo.txt"), "intro\nunique-needle here\n").unwrap();
        dir
    }

    #[gpui::test]
    fn empty_query_is_idle_and_confirm_emits_nothing(cx: &mut TestAppContext) {
        let dir = fixture();
        let (overlay, cx) = open_overlay(cx, dir.path().to_path_buf());
        cx.update(|_, app| {
            let o = overlay.read(app);
            assert!(!o.searching);
            assert!(o.matches.is_empty());
            assert!(o.rows().is_empty());
        });
        let opened = Rc::new(RefCell::new(false));
        cx.update(|_, app| {
            let flag = opened.clone();
            app.subscribe(&overlay, move |_, event: &SearchEvent, _| {
                if matches!(event, SearchEvent::Open { .. }) {
                    *flag.borrow_mut() = true;
                }
            })
            .detach();
        });
        cx.dispatch_action(SearchConfirm);
        cx.run_until_parked();
        assert!(!*opened.borrow());
    }

    #[gpui::test]
    fn query_streams_results_grouped_by_file(cx: &mut TestAppContext) {
        let dir = fixture();
        let (overlay, cx) = open_overlay(cx, dir.path().to_path_buf());
        set_query(&overlay, cx, "alpha");
        cx.update(|_, app| assert!(overlay.read(app).searching, "streaming in flight"));
        settle(&overlay, cx);
        cx.update(|_, app| {
            let o = overlay.read(app);
            assert_eq!(o.matches.len(), 3, "{:?}", o.matches);
            assert!(!o.capped);
            assert_eq!(o.selected, 0);
            // Per-file batches keep each file's hits contiguous and in
            // line order; file order itself follows the walk.
            let alpha: Vec<u64> = o
                .matches
                .iter()
                .filter(|m| m.path == PathBuf::from("alpha.md"))
                .map(|m| m.line_number)
                .collect();
            assert_eq!(alpha, vec![1, 3]);
            let beta: Vec<u64> = o
                .matches
                .iter()
                .filter(|m| m.path == PathBuf::from("sub/beta.md"))
                .map(|m| m.line_number)
                .collect();
            assert_eq!(beta, vec![1]);
            assert!(o.matches.iter().all(|m| !m.ranges.is_empty()));
            // rows(): one File header per contiguous file group, headers
            // pointing at the group's first hit, hits in match order.
            let rows = o.rows();
            assert_eq!(rows.len(), o.matches.len() + 2);
            let Row::File(0) = rows[0] else { panic!("first row must be a file header") };
            let mut seen_hits = Vec::new();
            for (i, row) in rows.iter().enumerate() {
                match row {
                    Row::File(ix) => {
                        assert!(
                            *ix == 0 || o.matches[*ix].path != o.matches[ix - 1].path,
                            "header only where the file changes"
                        );
                        let Some(Row::Hit(h)) = rows.get(i + 1) else { panic!("header not followed by hit") };
                        assert_eq!(h, ix, "header points at its first hit");
                    }
                    Row::Hit(ix) => seen_hits.push(*ix),
                }
            }
            assert_eq!(seen_hits, (0..o.matches.len()).collect::<Vec<_>>());
        });
    }

    #[gpui::test]
    fn up_and_down_wrap_selection_across_hits(cx: &mut TestAppContext) {
        let dir = fixture();
        let (overlay, cx) = open_overlay(cx, dir.path().to_path_buf());
        set_query(&overlay, cx, "wrapme");
        settle(&overlay, cx);
        cx.update(|_, app| assert_eq!(overlay.read(app).matches.len(), 3));
        cx.dispatch_action(SearchDown);
        cx.dispatch_action(SearchDown);
        cx.update(|_, app| assert_eq!(overlay.read(app).selected, 2));
        cx.dispatch_action(SearchDown);
        cx.update(|_, app| assert_eq!(overlay.read(app).selected, 0, "down wraps to first"));
        cx.dispatch_action(SearchUp);
        cx.update(|_, app| assert_eq!(overlay.read(app).selected, 2, "up wraps to last"));
    }

    #[gpui::test]
    fn confirm_opens_selected_hit_and_dismiss_cancels(cx: &mut TestAppContext) {
        let dir = fixture();
        let (overlay, cx) = open_overlay(cx, dir.path().to_path_buf());
        set_query(&overlay, cx, "unique-needle");
        settle(&overlay, cx);
        let events: Rc<RefCell<Vec<String>>> = Rc::default();
        cx.update(|_, app| {
            let sink = events.clone();
            app.subscribe(&overlay, move |_, event: &SearchEvent, _| {
                sink.borrow_mut().push(match event {
                    SearchEvent::Open { path, line } => format!("open:{}:{line}", path.display()),
                    SearchEvent::Dismissed => "dismissed".to_string(),
                });
            })
            .detach();
        });
        // Preview window is anchored on the selected hit's line.
        overlay.update(cx, |o, _| {
            let Some((text, start, line)) = o.preview_for_selected() else { panic!("preview") };
            assert!(text.contains("unique-needle here"));
            assert_eq!(start, 0, "window starts at file top for early lines");
            assert_eq!(line, 2);
        });
        cx.dispatch_action(SearchConfirm);
        cx.dispatch_action(SearchDismiss);
        cx.run_until_parked();
        assert_eq!(
            *events.borrow(),
            vec![
                format!("open:{}:2", dir.path().join("solo.txt").display()),
                "dismissed".to_string()
            ]
        );
        cx.update(|_, app| {
            assert!(
                overlay.read(app).cancelled.load(Ordering::Relaxed),
                "dismiss cancels any running stream"
            );
        });
    }

    #[gpui::test]
    fn no_match_query_settles_empty(cx: &mut TestAppContext) {
        let dir = fixture();
        let (overlay, cx) = open_overlay(cx, dir.path().to_path_buf());
        set_query(&overlay, cx, "zzz-not-here");
        settle(&overlay, cx);
        cx.update(|_, app| {
            let o = overlay.read(app);
            assert!(o.matches.is_empty());
            assert!(!o.searching);
        });
        let opened = Rc::new(RefCell::new(false));
        cx.update(|_, app| {
            let flag = opened.clone();
            app.subscribe(&overlay, move |_, event: &SearchEvent, _| {
                if matches!(event, SearchEvent::Open { .. }) {
                    *flag.borrow_mut() = true;
                }
            })
            .detach();
        });
        cx.dispatch_action(SearchConfirm);
        cx.run_until_parked();
        assert!(!*opened.borrow());
    }

    #[gpui::test]
    fn editing_query_restarts_and_resets_selection(cx: &mut TestAppContext) {
        let dir = fixture();
        let (overlay, cx) = open_overlay(cx, dir.path().to_path_buf());
        set_query(&overlay, cx, "alpha");
        settle(&overlay, cx);
        cx.dispatch_action(SearchDown);
        cx.update(|_, app| assert_eq!(overlay.read(app).selected, 1));
        set_query(&overlay, cx, "wrapme");
        cx.update(|_, app| {
            let o = overlay.read(app);
            assert!(o.matches.is_empty(), "restart clears stale results");
            assert_eq!(o.selected, 0, "selection resets");
            assert!(o.searching);
        });
        settle(&overlay, cx);
        cx.update(|_, app| {
            let o = overlay.read(app);
            assert_eq!(o.matches.len(), 3);
            assert!(o.matches.iter().all(|m| m.path == PathBuf::from("wrap.md")));
        });
    }

    #[gpui::test]
    fn retype_within_debounce_drops_the_stale_search(cx: &mut TestAppContext) {
        let dir = fixture();
        let (overlay, cx) = open_overlay(cx, dir.path().to_path_buf());
        set_query(&overlay, cx, "alpha");
        // Only 60 ms: still inside the 120 ms debounce of the first search.
        cx.executor().advance_clock(Duration::from_millis(60));
        cx.run_until_parked();
        set_query(&overlay, cx, "unique-needle");
        settle(&overlay, cx);
        cx.update(|_, app| {
            let o = overlay.read(app);
            assert_eq!(o.matches.len(), 1, "{:?}", o.matches);
            assert_eq!(o.matches[0].path, PathBuf::from("solo.txt"));
            assert_eq!(o.matches[0].line_number, 2);
        });
    }

    #[gpui::test]
    fn results_cap_is_reported(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let body = "hit\n".repeat(SEARCH_CAP + 50);
        std::fs::write(dir.path().join("big.txt"), body).unwrap();
        let (overlay, cx) = open_overlay(cx, dir.path().to_path_buf());
        set_query(&overlay, cx, "hit");
        settle(&overlay, cx);
        cx.update(|_, app| {
            let o = overlay.read(app);
            assert!(o.capped);
            assert_eq!(o.matches.len(), SEARCH_CAP);
        });
    }
}
