//! Command palette (⌘⇧P): plugin commands filtered with the finder's
//! nucleo scorer. List-only finder-family overlay.

use gpui::prelude::*;
use gpui::{
    actions, div, px, uniform_list, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window,
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
    scroll: gpui::UniformListScrollHandle,
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
            scroll: gpui::UniformListScrollHandle::default(),
            _watch_input: watch,
        };
        palette.refilter();
        palette
    }

    fn refilter(&mut self) {
        let titles: Vec<String> = self.entries.iter().map(|e| e.title.clone()).collect();
        self.matches = filter(&self.last_query, &titles);
        self.selected = 0;
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
    }

    fn reveal_selected(&self) {
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
    }

    fn up(&mut self, _: &PaletteUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
            self.reveal_selected();
            cx.notify();
        }
    }

    fn down(&mut self, _: &PaletteDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
            self.reveal_selected();
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

        let list = uniform_list(
            "palette-rows",
            self.matches.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = theme(cx);
                range
                    .map(|ix| {
                        let Some(&entry_ix) = this.matches.get(ix) else {
                            return div().id(ix);
                        };
                        let entry = &this.entries[entry_ix];
                        let is_selected = ix == this.selected;
                        div()
                            .id(ix)
                            .w_full()
                            .h(px(30.))
                            .px_3()
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
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .h_full();

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
            .h(px(400.))
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
                    .child(div().flex_1().min_h_0().child(list))
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

#[cfg(test)]
mod gpui_tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    fn entries() -> Vec<PaletteEntry> {
        let e = |plugin: &str, id: &str, title: &str| PaletteEntry {
            plugin: plugin.into(),
            id: id.into(),
            title: title.into(),
        };
        vec![
            e("toc", "toc.insert", "Insert Table of Contents"),
            e("toc", "toc.update", "Update Table of Contents"),
            e("tidy", "__format", "Format: tidy"),
            e("html-export", "__export:html", "Export: HTML"),
            e("daily-note", "__template:daily", "New: Daily Note"),
        ]
    }

    fn open_palette(
        cx: &mut TestAppContext,
        entries: Vec<PaletteEntry>,
        failures: Vec<String>,
    ) -> (gpui::Entity<Palette>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(
                crate::theme::Theme::dark(),
            )))
        });
        let (palette, cx) = cx.add_window_view(|_, cx| Palette::new(entries, failures, cx));
        cx.update(|window, app| {
            let handle = palette.read(app).focus_handle(app);
            window.focus(&handle);
        });
        cx.run_until_parked();
        (palette, cx)
    }

    fn set_query(palette: &gpui::Entity<Palette>, cx: &mut VisualTestContext, q: &str) {
        palette.update_in(cx, |p, _, cx| {
            p.input.update(cx, |input, cx| {
                input.content = q.to_string().into();
                cx.notify();
            });
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn renders_entries_and_failures_and_refilters(cx: &mut TestAppContext) {
        let (palette, cx) =
            open_palette(cx, entries(), vec!["badplugin: manifest broke".into()]);
        cx.update(|_, app| {
            let p = palette.read(app);
            assert_eq!(p.matches.len(), 5);
            assert_eq!(p.selected, 0);
        });
        set_query(&palette, cx, "export");
        cx.update(|_, app| {
            let p = palette.read(app);
            assert_eq!(p.matches, vec![3], "only Export: HTML matches");
            assert_eq!(p.selected, 0, "selection resets on refilter");
        });
        set_query(&palette, cx, "");
        cx.update(|_, app| assert_eq!(palette.read(app).matches.len(), 5));
    }

    #[gpui::test]
    fn up_and_down_wrap_selection(cx: &mut TestAppContext) {
        let (palette, cx) = open_palette(cx, entries(), Vec::new());
        cx.dispatch_action(PaletteDown);
        cx.dispatch_action(PaletteDown);
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(palette.read(app).selected, 2));
        for _ in 0..3 {
            cx.dispatch_action(PaletteUp);
        }
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(palette.read(app).selected, 4, "up wraps past zero"));
        cx.dispatch_action(PaletteDown);
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(palette.read(app).selected, 0, "down wraps to start"));
    }

    #[gpui::test]
    fn confirm_emits_the_filtered_selection(cx: &mut TestAppContext) {
        let (palette, cx) = open_palette(cx, entries(), Vec::new());
        let run: Rc<RefCell<Option<(String, String)>>> = Rc::default();
        cx.update(|_, app| {
            let sink = run.clone();
            app.subscribe(&palette, move |_, event: &PaletteEvent, _| {
                if let PaletteEvent::Run { plugin, id } = event {
                    *sink.borrow_mut() = Some((plugin.clone(), id.clone()));
                }
            })
            .detach();
        });
        set_query(&palette, cx, "daily");
        cx.dispatch_action(PaletteConfirm);
        cx.run_until_parked();
        assert_eq!(
            run.borrow().clone(),
            Some(("daily-note".to_string(), "__template:daily".to_string()))
        );
    }

    #[gpui::test]
    fn dismiss_emits_and_empty_palette_confirms_nothing(cx: &mut TestAppContext) {
        let (palette, cx) = open_palette(cx, Vec::new(), Vec::new());
        let events: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        cx.update(|_, app| {
            let sink = events.clone();
            app.subscribe(&palette, move |_, event: &PaletteEvent, _| {
                sink.borrow_mut().push(match event {
                    PaletteEvent::Run { .. } => "run",
                    PaletteEvent::Dismissed => "dismissed",
                });
            })
            .detach();
        });
        // Nothing to confirm (also exercises the empty-state render).
        cx.dispatch_action(PaletteConfirm);
        cx.dispatch_action(PaletteDismiss);
        cx.run_until_parked();
        assert_eq!(*events.borrow(), vec!["dismissed"]);
    }
}
