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
    _watch_input: Subscription,
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
        let row_height = 30.0_f32;
        let list_height = (self.matches.len().min(10) as f32) * row_height;
        let selected = self.selected;

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
        .h(px(list_height));

        div()
            .key_context("Finder")
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(560.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .rounded_lg()
            .shadow_lg()
            .overflow_hidden()
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
            .child(results)
    }
}
