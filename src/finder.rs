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
    selected: usize,
    last_query: String,
    preview: Option<(usize, PreviewContent)>,
    _watch_input: Subscription,
}

enum PreviewContent {
    Text(SharedString),
    Image(PathBuf),
    Unreadable,
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
            selected: 0,
            last_query: String::new(),
            preview: None,
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
        }
        cx.notify();
    }

    fn rescore(&mut self, query: &str) {
        if query.is_empty() {
            self.matches = (0..self.files.len().min(MAX_RESULTS)).collect();
            return;
        }
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = self
            .files
            .iter()
            .enumerate()
            .filter_map(|(ix, (rel, _))| {
                pattern
                    .score(Utf32Str::new(rel, &mut buf), &mut matcher)
                    .map(|score| (score, ix))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(MAX_RESULTS);
        self.matches = scored.into_iter().map(|(_, ix)| ix).collect();
    }

    fn up(&mut self, _: &FinderUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
            cx.notify();
        }
    }

    fn down(&mut self, _: &FinderDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
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
                        div()
                            .id(ix)
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
                                div()
                                    .text_size(px(13.))
                                    .text_color(t.fg_strong)
                                    .child(SharedString::from(name.to_string())),
                            )
                            .when(!dir.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(t.fg_muted)
                                        .overflow_hidden()
                                        .child(SharedString::from(dir.to_string())),
                                )
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.confirm_index(ix, cx);
                            }))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .h_full();

        div()
            .key_context("Finder")
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(860.))
            .h(px(460.))
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
                    .w(px(340.))
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
