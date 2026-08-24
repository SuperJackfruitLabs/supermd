//! Command palette (⌘⇧P): plugin commands filtered with the finder's
//! nucleo scorer. List-only finder-family overlay.

use gpui::prelude::*;
use gpui::{
    actions, div, px, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window,
};

use crate::input::TextInput;
use crate::theme::theme;

actions!(palette, [PaletteUp, PaletteDown, PaletteConfirm, PaletteDismiss]);

pub enum PaletteEvent {
    Run { plugin: String, id: String },
    Dismissed,
}

#[derive(Clone)]
pub struct PaletteEntry {
    pub plugin: String,
    pub id: String,
    pub title: String,
}

pub struct Palette {
    pub input: Entity<TextInput>,
    entries: Vec<PaletteEntry>,
    /// Plugin-load failures, shown dimmed and unclickable.
    failures: Vec<String>,
    /// Indices into `entries`, best match first.
    matches: Vec<usize>,
    selected: usize,
    last_query: String,
    _watch_input: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

/// Filter entry titles with the shared scorer; empty query keeps all.
pub(crate) fn filter(query: &str, titles: &[String]) -> Vec<usize> {
    if query.is_empty() {
        return (0..titles.len()).collect();
    }
    let (order, _) = crate::finder::score_candidates(query, titles);
    order
}

impl Palette {
    pub fn new(
        entries: Vec<PaletteEntry>,
        failures: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new("Run a command…", cx));
        let watch = cx.observe(&input, |this: &mut Palette, _, cx| {
            let query = this.input.read(cx).content.to_string();
            if query != this.last_query {
                this.last_query = query;
                this.refilter();
                cx.notify();
            }
        });
        let mut palette = Self {
            input,
            entries,
            failures,
            matches: Vec::new(),
            selected: 0,
            last_query: String::new(),
            _watch_input: watch,
        };
        palette.refilter();
        palette
    }

    fn refilter(&mut self) {
        let titles: Vec<String> = self.entries.iter().map(|e| e.title.clone()).collect();
        self.matches = filter(&self.last_query, &titles);
        self.selected = 0;
    }

    fn up(&mut self, _: &PaletteUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
            cx.notify();
        }
    }

    fn down(&mut self, _: &PaletteDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
            cx.notify();
        }
    }

    fn confirm(&mut self, _: &PaletteConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_index(self.selected, cx);
    }

    fn confirm_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(&entry_ix) = self.matches.get(ix) {
            let entry = self.entries[entry_ix].clone();
            cx.emit(PaletteEvent::Run { plugin: entry.plugin, id: entry.id });
        }
    }

    fn dismiss(&mut self, _: &PaletteDismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismissed);
    }
}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.read(cx).focus_handle.clone()
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let selected = self.selected;

        let rows: Vec<gpui::AnyElement> = self
            .matches
            .iter()
            .enumerate()
            .map(|(ix, &entry_ix)| {
                let entry = &self.entries[entry_ix];
                let is_selected = ix == selected;
                div()
                    .id(("palette-row", ix))
                    .w_full()
                    .px_3()
                    .py(px(6.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .cursor_pointer()
                    .when(is_selected, |d| d.bg(t.selected_bg))
                    .when(!is_selected, |d| d.hover(|s| s.bg(t.hover_bg)))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.))
                            .text_color(t.fg_strong)
                            .child(SharedString::from(entry.title.clone())),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.fg_muted)
                            .child(SharedString::from(entry.plugin.clone())),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.confirm_index(ix, cx);
                    }))
                    .into_any_element()
            })
            .collect();

        let failures: Vec<gpui::AnyElement> = self
            .failures
            .iter()
            .map(|f| {
                div()
                    .w_full()
                    .px_3()
                    .py(px(4.))
                    .text_size(px(11.))
                    .text_color(t.fg_muted)
                    .child(SharedString::from(format!("failed: {f}")))
                    .into_any_element()
            })
            .collect();

        let empty = self.matches.is_empty() && self.failures.is_empty();

        div()
            .key_context("Palette")
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(480.))
            .max_w(gpui::relative(0.9))
            .max_h(px(400.))
            .flex_none()
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
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .py_1()
                    .flex()
                    .flex_col()
                    .children(rows)
                    .children(failures)
                    .children(empty.then(|| {
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(t.fg_muted)
                            .child("No commands — install plugins in ~/.supermd/plugins")
                            .into_any_element()
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_filters_by_title() {
        let titles = vec![
            "Insert Table of Contents".to_string(),
            "About Dot".to_string(),
        ];
        assert_eq!(filter("toc", &titles), vec![0]);
        assert_eq!(filter("", &titles), vec![0, 1]);
        assert!(filter("zzz", &titles).is_empty());
    }
}
