//! Fuzzy file finder overlay (⌘P), scored with nucleo.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    actions, div, px, uniform_list, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window,
};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::input::TextInput;
use crate::theme::theme;

actions!(finder, [FinderUp, FinderDown, FinderConfirm, FinderDismiss]);

pub enum FinderEvent {
    OpenPath(PathBuf),
    Dismissed,
}

pub struct Finder {
    pub input: Entity<TextInput>,
    /// (display path relative to root, absolute path)
    files: Vec<(String, PathBuf)>,
    /// Indices into `files`, best match first.
    matches: Vec<usize>,
    /// Per entry of `matches`: matched char indices in the display path.
    match_indices: Vec<Vec<u32>>,
    selected: usize,
    last_query: String,
    preview: Option<(usize, PreviewContent)>,
    scroll: gpui::UniformListScrollHandle,
    _watch_input: Subscription,
}

enum PreviewContent {
    Text(SharedString),
    Image(PathBuf),
    Unreadable,
}

/// Score candidates against `query` with nucleo: returns candidate
/// indices best-first plus, per result, the matched char indices in the
/// candidate string (for highlight styling).
pub(crate) fn score_candidates(query: &str, rels: &[String]) -> (Vec<usize>, Vec<Vec<u32>>) {
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, usize, Vec<u32>)> = rels
        .iter()
        .enumerate()
        .filter_map(|(ix, rel)| {
            let mut indices = Vec::new();
            pattern
                .indices(Utf32Str::new(rel, &mut buf), &mut matcher, &mut indices)
                .map(|score| {
                    indices.sort_unstable();
                    indices.dedup();
                    (score, ix, indices)
                })
        })
        .collect();
    // Ties (same score) go to the shorter path — "notes.md" should beat
    // "some/deep/path/notes.md".
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(rels[a.1].len().cmp(&rels[b.1].len())));
    scored.truncate(MAX_RESULTS);
    scored.into_iter().map(|(_, ix, ind)| (ix, ind)).unzip()
}

/// Byte ranges (coalesced) inside a segment of the display path for
/// matched chars. `seg` starts at char offset `seg_char_off` of the
/// string the indices refer to.
fn segment_highlight_ranges(seg: &str, seg_char_off: usize, indices: &[u32]) -> Vec<std::ops::Range<usize>> {
    let starts: Vec<usize> = seg.char_indices().map(|(b, _)| b).collect();
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for &i in indices {
        let Some(local) = (i as usize).checked_sub(seg_char_off) else {
            continue;
        };
        if local >= starts.len() {
            continue;
        }
        let b = starts[local];
        let e = starts.get(local + 1).copied().unwrap_or(seg.len());
        match ranges.last_mut() {
            Some(last) if last.end == b => last.end = e,
            _ => ranges.push(b..e),
        }
    }
    ranges
}

fn load_preview(path: &std::path::Path) -> PreviewContent {
    if crate::files::is_image_path(path) {
        return PreviewContent::Image(path.to_path_buf());
    }
    match std::fs::read(path) {
        Ok(bytes) => {
            let slice = &bytes[..bytes.len().min(16 * 1024)];
            let text = String::from_utf8_lossy(slice);
            let mut out = String::new();
            for (i, line) in text.lines().enumerate() {
                if i >= 120 {
                    break;
                }
                out.push_str(line);
                out.push('\n');
            }
            PreviewContent::Text(out.into())
        }
        Err(_) => PreviewContent::Unreadable,
    }
}

impl EventEmitter<FinderEvent> for Finder {}

const MAX_RESULTS: usize = 64;

impl Finder {
    pub fn new(files: Vec<(String, PathBuf)>, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Go to file…", cx));
        let watch = cx.observe(&input, |this: &mut Finder, _, cx| {
            this.refilter(cx);
        });
        let mut finder = Self {
            input,
            files,
            matches: Vec::new(),
            match_indices: Vec::new(),
            selected: 0,
            last_query: String::new(),
            preview: None,
            scroll: gpui::UniformListScrollHandle::default(),
            _watch_input: watch,
        };
        finder.rescore("");
        finder
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).content.to_string();
        if query != self.last_query {
            self.last_query = query.clone();
            self.rescore(&query);
            self.selected = 0;
            self.scroll
                .scroll_to_item(0, gpui::ScrollStrategy::Top);
        }
        cx.notify();
    }

    fn reveal_selected(&self) {
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
    }

    fn rescore(&mut self, query: &str) {
        if query.is_empty() {
            self.matches = (0..self.files.len().min(MAX_RESULTS)).collect();
            self.match_indices = vec![Vec::new(); self.matches.len()];
            return;
        }
        let rels: Vec<String> = self.files.iter().map(|(rel, _)| rel.clone()).collect();
        let (matches, match_indices) = score_candidates(query, &rels);
        self.matches = matches;
        self.match_indices = match_indices;
    }

    fn up(&mut self, _: &FinderUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
            self.reveal_selected();
            cx.notify();
        }
    }

    fn down(&mut self, _: &FinderDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
            self.reveal_selected();
            cx.notify();
        }
    }

    fn confirm(&mut self, _: &FinderConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_index(self.selected, cx);
    }

    fn confirm_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(&file_ix) = self.matches.get(ix) {
            let path = self.files[file_ix].1.clone();
            cx.emit(FinderEvent::OpenPath(path));
        }
    }

    fn dismiss(&mut self, _: &FinderDismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(FinderEvent::Dismissed);
    }
}

impl Focusable for Finder {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.read(cx).focus_handle.clone()
    }
}

impl Render for Finder {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let selected = self.selected;

        // Preview of the currently selected match (cached per file index).
        let preview_pane: gpui::AnyElement = {
            let selected_file = self.matches.get(self.selected).copied();
            if let Some(file_ix) = selected_file {
                if self.preview.as_ref().map(|(ix, _)| *ix) != Some(file_ix) {
                    let path = &self.files[file_ix].1;
                    self.preview = Some((file_ix, load_preview(path)));
                }
            } else {
                self.preview = None;
            }
            match (&self.preview, selected_file) {
                (Some((_, content)), Some(file_ix)) => {
                    let (rel, _) = &self.files[file_ix];
                    let body: gpui::AnyElement = match content {
                        PreviewContent::Text(text) => div()
                            .size_full()
                            .overflow_hidden()
                            .font_family(t.mono_family.clone())
                            .text_size(px(11.))
                            .line_height(gpui::relative(1.45))
                            .text_color(t.fg_muted)
                            .child(text.clone())
                            .into_any_element(),
                        PreviewContent::Image(path) => div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(gpui::img(path.clone()).max_w_full().max_h_full().rounded_md())
                            .into_any_element(),
                        PreviewContent::Unreadable => div()
                            .text_size(px(12.))
                            .text_color(t.fg_muted)
                            .child("binary or unreadable file")
                            .into_any_element(),
                    };
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .px_3()
                                .py_2()
                                .border_b_1()
                                .border_color(t.border)
                                .text_size(px(11.))
                                .text_color(t.fg_muted)
                                .overflow_hidden()
                                .child(SharedString::from(rel.clone())),
                        )
                        .child(div().flex_1().min_h_0().p_3().child(body))
                        .into_any_element()
                }
                _ => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(t.fg_muted)
                    .child("No matches")
                    .into_any_element(),
            }
        };

        let results = uniform_list(
            "finder-results",
            self.matches.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = theme(cx);
                range
                    .map(|ix| {
                        let Some(&file_ix) = this.matches.get(ix) else {
                            return div().id(ix);
                        };
                        let (rel, _) = &this.files[file_ix];
                        let (dir, name) = match rel.rfind('/') {
                            Some(pos) => (&rel[..pos], &rel[pos + 1..]),
                            None => ("", rel.as_str()),
                        };
                        let is_selected = ix == selected;
                        let indices: &[u32] =
                            this.match_indices.get(ix).map(|v| v.as_slice()).unwrap_or(&[]);
                        let name_char_off = rel[..rel.len() - name.len()].chars().count();
                        let hl = |seg: &str, off: usize| {
                            segment_highlight_ranges(seg, off, indices)
                                .into_iter()
                                .map(|r| {
                                    (
                                        r,
                                        gpui::HighlightStyle {
                                            color: Some(t.accent),
                                            font_weight: Some(gpui::FontWeight::SEMIBOLD),
                                            ..Default::default()
                                        },
                                    )
                                })
                                .collect::<Vec<_>>()
                        };
                        let name_hl = hl(name, name_char_off);
                        let dir_hl = hl(dir, 0);
                        div()
                            .id(ix)
                            .w_full()
                            .h(px(30.))
                            .px_3()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .when(is_selected, |d| d.bg(t.selected_bg))
                            .when(!is_selected, |d| d.hover(|s| s.bg(t.hover_bg)))
                            .child(
                                div().text_size(px(13.)).text_color(t.fg_strong).child(
                                    gpui::StyledText::new(name.to_string())
                                        .with_highlights(name_hl),
                                ),
                            )
                            .when(!dir.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(t.fg_muted)
                                        .overflow_hidden()
                                        .child(
                                            gpui::StyledText::new(dir.to_string())
                                                .with_highlights(dir_hl),
                                        ),
                                )
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.confirm_index(ix, cx);
                            }))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .h_full();

        div()
            .key_context("Finder")
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
                    .w(px(280.))
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
                    .child(div().flex_1().min_h_0().child(results)),
            )
            .child(div().flex_1().min_w_0().h_full().child(preview_pane))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_indices_returned_for_highlighting() {
        let rels = vec!["readme.md".to_string(), "zzz.txt".to_string()];
        let (order, indices) = score_candidates("rm", &rels);
        assert_eq!(order, vec![0]);
        assert!(!indices[0].is_empty());
        for &i in &indices[0] {
            assert!("readme.md".chars().nth(i as usize).is_some());
        }
    }

    #[test]
    fn segment_ranges_map_chars_to_bytes_and_coalesce() {
        // indices 0,1 within "réadme" → bytes 0..1 and 1..3 coalesced
        let r = segment_highlight_ranges("réadme", 0, &[0, 1]);
        assert_eq!(r, vec![0..3]);
        // segment offset: index 5 with offset 4 → second char
        let r = segment_highlight_ranges("ab", 4, &[5]);
        assert_eq!(r, vec![1..2]);
        // out-of-segment indices dropped
        assert!(segment_highlight_ranges("ab", 4, &[0, 9]).is_empty());
    }

    #[test]
    fn ranking_prefers_tighter_matches() {
        let rels = vec![
            "some/deep/path/notes.md".to_string(),
            "notes.md".to_string(),
        ];
        let (order, _) = score_candidates("notes", &rels);
        assert_eq!(order[0], 1, "shorter exact-name match ranks first");
        assert_eq!(order.len(), 2);
    }

    #[test]
    fn non_matching_candidates_are_dropped() {
        let rels = vec!["readme.md".to_string(), "zzz.bin".to_string()];
        let (order, indices) = score_candidates("xyq", &rels);
        assert!(order.is_empty());
        assert!(indices.is_empty());
    }

    #[test]
    fn results_truncate_at_max() {
        let rels: Vec<String> = (0..MAX_RESULTS + 20).map(|i| format!("file{i}.md")).collect();
        let (order, indices) = score_candidates("file", &rels);
        assert_eq!(order.len(), MAX_RESULTS);
        assert_eq!(indices.len(), MAX_RESULTS);
    }

    #[test]
    fn smart_case_matches_case_insensitively_for_lowercase_query() {
        let rels = vec!["README.md".to_string()];
        let (order, _) = score_candidates("readme", &rels);
        assert_eq!(order, vec![0]);
    }

    #[test]
    fn preview_reads_text_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "hello\nworld\n").unwrap();
        match load_preview(&path) {
            PreviewContent::Text(text) => assert_eq!(text.as_ref(), "hello\nworld\n"),
            _ => panic!("expected text preview"),
        }
    }

    #[test]
    fn preview_truncates_long_files_to_120_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.txt");
        let content: String = (0..500).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, content).unwrap();
        match load_preview(&path) {
            PreviewContent::Text(text) => {
                assert_eq!(text.lines().count(), 120);
                assert!(text.starts_with("line 0\n"));
            }
            _ => panic!("expected text preview"),
        }
    }

    #[test]
    fn preview_detects_images_and_unreadable_paths() {
        assert!(matches!(
            load_preview(std::path::Path::new("shot.png")),
            PreviewContent::Image(_)
        ));
        assert!(matches!(
            load_preview(std::path::Path::new("/nonexistent/definitely-missing.txt")),
            PreviewContent::Unreadable
        ));
    }

    // ── entity/interaction tests (headless gpui test platform) ─────────

    use gpui::{TestAppContext, VisualTestContext};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    fn test_files() -> Vec<(String, PathBuf)> {
        [
            "notes/readme.md",
            "notes/deep/nested/readme.md",
            "todo.md",
        ]
        .iter()
        .map(|rel| (rel.to_string(), PathBuf::from("/nonexistent").join(rel)))
        .collect()
    }

    fn open_finder(cx: &mut TestAppContext) -> (gpui::Entity<Finder>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(
                crate::theme::Theme::dark(),
            )))
        });
        let (finder, cx) = cx.add_window_view(|_, cx| Finder::new(test_files(), cx));
        cx.update(|window, app| {
            let handle = finder.read(app).focus_handle(app);
            window.focus(&handle);
        });
        cx.run_until_parked();
        (finder, cx)
    }

    #[gpui::test]
    fn empty_query_lists_all_files_unhighlighted(cx: &mut TestAppContext) {
        let (finder, cx) = open_finder(cx);
        cx.update(|_, app| {
            let f = finder.read(app);
            assert_eq!(f.matches, vec![0, 1, 2]);
            assert!(f.match_indices.iter().all(|v| v.is_empty()));
            assert_eq!(f.selected, 0);
        });
    }

    #[gpui::test]
    fn editing_the_query_refilters_and_resets_selection(cx: &mut TestAppContext) {
        let (finder, cx) = open_finder(cx);
        // Move selection off 0 first so we can observe the reset.
        cx.dispatch_action(FinderDown);
        cx.update(|_, app| assert_eq!(finder.read(app).selected, 1));
        finder.update_in(cx, |f, _, cx| {
            f.input.update(cx, |input, cx| {
                input.content = "todo".into();
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let f = finder.read(app);
            assert_eq!(f.last_query, "todo");
            assert_eq!(f.matches, vec![2], "only todo.md matches");
            assert!(!f.match_indices[0].is_empty(), "match chars recorded");
            assert_eq!(f.selected, 0, "selection resets on refilter");
        });
    }

    #[gpui::test]
    fn arrow_actions_wrap_around(cx: &mut TestAppContext) {
        let (finder, cx) = open_finder(cx);
        cx.dispatch_action(FinderUp);
        cx.update(|_, app| assert_eq!(finder.read(app).selected, 2, "up from 0 wraps to last"));
        cx.dispatch_action(FinderDown);
        cx.update(|_, app| assert_eq!(finder.read(app).selected, 0, "down from last wraps to 0"));
    }

    #[gpui::test]
    fn confirm_emits_selected_path_and_dismiss_emits_dismissed(cx: &mut TestAppContext) {
        let (finder, cx) = open_finder(cx);
        let events: Rc<RefCell<Vec<String>>> = Rc::default();
        cx.update(|_, app| {
            let sink = events.clone();
            app.subscribe(&finder, move |_, event: &FinderEvent, _| {
                sink.borrow_mut().push(match event {
                    FinderEvent::OpenPath(p) => format!("open:{}", p.display()),
                    FinderEvent::Dismissed => "dismissed".to_string(),
                });
            })
            .detach();
        });
        cx.dispatch_action(FinderDown);
        cx.dispatch_action(FinderConfirm);
        cx.dispatch_action(FinderDismiss);
        cx.run_until_parked();
        assert_eq!(
            *events.borrow(),
            vec![
                "open:/nonexistent/notes/deep/nested/readme.md".to_string(),
                "dismissed".to_string()
            ]
        );
    }

    #[gpui::test]
    fn confirm_on_no_matches_emits_nothing(cx: &mut TestAppContext) {
        let (finder, cx) = open_finder(cx);
        let confirmed = Rc::new(RefCell::new(false));
        cx.update(|_, app| {
            let flag = confirmed.clone();
            app.subscribe(&finder, move |_, event: &FinderEvent, _| {
                if matches!(event, FinderEvent::OpenPath(_)) {
                    *flag.borrow_mut() = true;
                }
            })
            .detach();
        });
        finder.update_in(cx, |f, _, cx| {
            f.input.update(cx, |input, cx| {
                input.content = "zzzzqqq".into();
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| assert!(finder.read(app).matches.is_empty()));
        cx.dispatch_action(FinderConfirm);
        cx.run_until_parked();
        assert!(!*confirmed.borrow());
    }
}
