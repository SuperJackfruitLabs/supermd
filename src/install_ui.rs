//! "Install Plugins…" overlay: the catalog as a keyboard-driven list.
//! Selection and events only — the download/install work happens in
//! the workspace, off the UI thread.

use gpui::prelude::*;
use gpui::{
    actions, div, px, uniform_list, ClickEvent, Context, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Styled, Window,
};

use crate::catalog::CatalogEntry;
use crate::theme::theme;

actions!(install_ui, [InstallUp, InstallDown, InstallConfirm, InstallDismiss]);

pub enum InstallEvent {
    Install(CatalogEntry),
    Dismissed,
}

pub struct InstallOverlay {
    entries: Vec<CatalogEntry>,
    installed: Vec<String>,
    selected: usize,
    focus_handle: FocusHandle,
    scroll: gpui::UniformListScrollHandle,
}

impl EventEmitter<InstallEvent> for InstallOverlay {}

/// The capability tag a user sees, in plain words.
pub(crate) fn capability_blurb(capabilities: &[String]) -> Option<&'static str> {
    if capabilities.iter().any(|c| c == "net") {
        Some("needs network access — asks per site")
    } else if capabilities.iter().any(|c| c == "workspace-read") {
        Some("reads your open folder — asks first")
    } else {
        None
    }
}

/// The App Store build shows no browsable catalog: a list of
/// downloadable plugins reads as a storefront for other code under
/// DPLA 3.3.2(b). Install arrives via Import… or a supermd:// link.
pub fn catalog_browsable() -> bool {
    !cfg!(feature = "mas")
}

impl InstallOverlay {
    pub fn new(
        entries: Vec<CatalogEntry>,
        installed: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            // Dropped rather than merely hidden: an unlistable entry must
            // not be reachable by keyboard selection either.
            entries: if catalog_browsable() { entries } else { Vec::new() },
            installed,
            selected: 0,
            focus_handle: cx.focus_handle(),
            scroll: gpui::UniformListScrollHandle::default(),
        }
    }

    fn is_installed(&self, entry: &CatalogEntry) -> bool {
        self.installed.iter().any(|n| n == &entry.name)
    }

    fn up(&mut self, _: &InstallUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + self.entries.len() - 1) % self.entries.len();
            self.reveal_selected();
            cx.notify();
        }
    }

    fn down(&mut self, _: &InstallDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1) % self.entries.len();
            self.reveal_selected();
            cx.notify();
        }
    }

    fn reveal_selected(&self) {
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
    }

    fn confirm(&mut self, _: &InstallConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_index(self.selected, cx);
    }

    fn confirm_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.entries.get(ix) {
            if !self.is_installed(entry) {
                cx.emit(InstallEvent::Install(entry.clone()));
            }
        }
    }

    fn dismiss(&mut self, _: &InstallDismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InstallEvent::Dismissed);
    }
}

impl Focusable for InstallOverlay {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for InstallOverlay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let selected = self.selected;

        let list = uniform_list(
            "install-rows",
            self.entries.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = theme(cx);
                range
                    .map(|ix| {
                        let Some(entry) = this.entries.get(ix) else {
                            return div().id(ix);
                        };
                        let installed = this.is_installed(entry);
                        let is_selected = ix == this.selected;
                        let title_color = if installed { t.fg_muted } else { t.fg_strong };
                        let title = if installed {
                            format!("{}  ✓ installed", entry.name)
                        } else {
                            entry.name.clone()
                        };
                        let mut header = div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(title_color)
                                    .child(SharedString::from(title)),
                            );
                        if let Some(blurb) = capability_blurb(&entry.capabilities) {
                            header = header.child(
                                div().text_size(px(11.)).text_color(t.accent).child(blurb),
                            );
                        }
                        let mut row = div()
                            .id(ix)
                            .w_full()
                            .h(px(46.))
                            .px_3()
                            .py(px(5.))
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .when(is_selected, |d| d.bg(t.selected_bg))
                            .when(!is_selected && !installed, |d| {
                                d.hover(|st| st.bg(t.hover_bg))
                            })
                            .child(header)
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(t.fg_muted)
                                    .overflow_hidden()
                                    .child(SharedString::from(entry.description.clone())),
                            );
                        if !installed {
                            row = row.cursor_pointer().on_click(cx.listener(
                                move |this, _: &ClickEvent, _w, cx| {
                                    this.confirm_index(ix, cx);
                                },
                            ));
                        }
                        row
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .h_full();

        div()
            .key_context("InstallOverlay")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(480.))
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
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(t.border)
                    .text_size(px(13.))
                    .text_color(t.fg_strong)
                    .child("Install Plugins"),
            )
            .child(if catalog_browsable() {
                div().flex_1().min_h_0().py_1().child(list).into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(t.fg_muted)
                    .child(
                        "Browse plugins at supermd.app and click Install there, \
                         or use Tools → Import Plugin… for one you have downloaded.",
                    )
                    .into_any_element()
            })
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(t.border)
                    .text_size(px(11.))
                    .text_color(t.fg_muted)
                    .child("Plugins are built and published by the SuperMD project"),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    fn entry(name: &str, caps: &[&str]) -> CatalogEntry {
        CatalogEntry {
            name: name.into(),
            description: format!("{name} description"),
            version: "0.1.0".into(),
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            download: format!(
                "https://github.com/SuperJackfruitLabs/supermd/releases/download/v0/plugin-{name}.zip"
            ),
            sha256: "x".into(),
        }
    }

    fn open(
        cx: &mut TestAppContext,
        entries: Vec<CatalogEntry>,
        installed: Vec<String>,
    ) -> (gpui::Entity<InstallOverlay>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(crate::theme::Theme::dark())))
        });
        let (overlay, cx) = cx.add_window_view(|_, cx| InstallOverlay::new(entries, installed, cx));
        cx.update(|window, app| {
            let handle = overlay.read(app).focus_handle(app);
            window.focus(&handle);
        });
        cx.run_until_parked();
        (overlay, cx)
    }

    #[gpui::test]
    fn app_store_builds_list_no_catalog(cx: &mut TestAppContext) {
        assert_eq!(catalog_browsable(), !cfg!(feature = "mas"));
        if catalog_browsable() {
            return;
        }
        let overlay =
            cx.update(|cx| cx.new(|cx| InstallOverlay::new(vec![entry("a", &[])], vec![], cx)));
        cx.update(|cx| assert!(overlay.read(cx).entries.is_empty()));
    }

    #[test]
    fn capability_blurbs_speak_user() {
        assert!(capability_blurb(&["net".to_string()]).unwrap().contains("network"));
        assert!(capability_blurb(&["workspace-read".to_string()]).unwrap().contains("folder"));
        assert!(capability_blurb(&[]).is_none());
    }

    #[gpui::test]
    fn navigation_wraps_and_confirm_emits_install(cx: &mut TestAppContext) {
        // Rows only exist where the catalog is browsable; see
        // app_store_builds_list_no_catalog for the other build.
        if !catalog_browsable() {
            return;
        }
        let entries = vec![entry("alpha", &[]), entry("beta", &["net"]), entry("gamma", &[])];
        let (overlay, cx) = open(cx, entries, Vec::new());
        let got: Rc<RefCell<Option<String>>> = Rc::default();
        cx.update(|_, app| {
            let sink = got.clone();
            app.subscribe(&overlay, move |_, event: &InstallEvent, _| {
                if let InstallEvent::Install(e) = event {
                    *sink.borrow_mut() = Some(e.name.clone());
                }
            })
            .detach();
        });
        cx.dispatch_action(InstallUp); // wraps to the last row
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(overlay.read(app).selected, 2));
        cx.dispatch_action(InstallDown);
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(overlay.read(app).selected, 0));
        cx.dispatch_action(InstallDown);
        cx.dispatch_action(InstallConfirm);
        cx.run_until_parked();
        assert_eq!(got.borrow().clone(), Some("beta".to_string()));
    }

    #[gpui::test]
    fn installed_entries_do_not_emit_and_dismiss_works(cx: &mut TestAppContext) {
        if !catalog_browsable() {
            return;
        }
        let entries = vec![entry("alpha", &[])];
        let (overlay, cx) = open(cx, entries, vec!["alpha".to_string()]);
        let events: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        cx.update(|_, app| {
            let sink = events.clone();
            app.subscribe(&overlay, move |_, event: &InstallEvent, _| {
                sink.borrow_mut().push(match event {
                    InstallEvent::Install(_) => "install",
                    InstallEvent::Dismissed => "dismissed",
                });
            })
            .detach();
        });
        cx.dispatch_action(InstallConfirm);
        cx.dispatch_action(InstallDismiss);
        cx.run_until_parked();
        assert_eq!(*events.borrow(), vec!["dismissed"], "installed row is inert");
    }
}
