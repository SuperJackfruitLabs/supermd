//! The root view: sidebar, tab bar, document pane, and outline panel.

use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::{
    actions, div, px, uniform_list, AnyElement, ClickEvent, Entity, FocusHandle, Focusable,
    IntoElement, ParentElement, PathPromptOptions, Render, SharedString, Styled, Window,
};

use crate::files::FileTree;
use crate::finder::{Finder, FinderEvent};
use crate::highlight::languages;
use crate::reader::Reader;
use crate::theme::theme;

actions!(
    workspace,
    [
        OpenDialog,
        CloseTab,
        NextTab,
        PrevTab,
        ToggleSidebar,
        ToggleOutline,
        ToggleFinder,
    ]
);

pub struct Workspace {
    pub tree: Option<FileTree>,
    readers: Vec<Entity<Reader>>,
    active: usize,
    show_sidebar: bool,
    show_outline: bool,
    finder: Option<(Entity<Finder>, gpui::Subscription)>,
    // TEMP: Task 9 verification, replaced in Task 12
    debug_editor: Option<Entity<crate::editor::Editor>>,
    focus_handle: FocusHandle,
}

impl Workspace {
    pub fn new(arg: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut tree = None;
        let mut readers = Vec::new();

        match arg {
            Some(path) if path.is_dir() => {
                tree = Some(FileTree::new(path));
            }
            Some(path) => match Reader::open(&path, &languages(cx)) {
                Ok(reader) => readers.push(cx.new(|_| reader)),
                Err(err) => eprintln!("supermd: cannot open {}: {err}", path.display()),
            },
            None => {
                if let Ok(cwd) = std::env::current_dir() {
                    tree = Some(FileTree::new(cwd));
                }
                let welcome = Reader::welcome(&languages(cx));
                readers.push(cx.new(|_| welcome));
            }
        }

        Self {
            tree,
            readers,
            active: 0,
            show_sidebar: true,
            show_outline: true,
            finder: None,
            debug_editor: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn active_reader(&self) -> Option<&Entity<Reader>> {
        self.readers.get(self.active)
    }

    pub fn open_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        if path.is_dir() {
            self.tree = Some(FileTree::new(path.to_path_buf()));
            self.show_sidebar = true;
            cx.notify();
            return;
        }
        // TEMP: Task 9 verification, replaced in Task 12
        if let Ok(text) = crate::editor::Editor::read_file(path) {
            let langs = languages(cx);
            let path_buf = path.to_path_buf();
            let editor =
                cx.new(|cx| crate::editor::Editor::from_text(&path_buf, text, &langs, cx));
            self.debug_editor = Some(editor);
            cx.notify();
            return;
        }
        if let Some(ix) = self
            .readers
            .iter()
            .position(|r| r.read(cx).path.as_deref() == Some(path))
        {
            self.active = ix;
            cx.notify();
            return;
        }
        match Reader::open(path, &languages(cx)) {
            Ok(reader) => {
                self.readers.push(cx.new(|_| reader));
                self.active = self.readers.len() - 1;
                cx.notify();
            }
            Err(err) => eprintln!("supermd: cannot open {}: {err}", path.display()),
        }
    }

    fn open_dialog(&mut self, _: &OpenDialog, _window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update(cx, |workspace, cx| {
                    for path in paths {
                        workspace.open_path(&path, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn close_tab(&mut self, _: &CloseTab, _window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, cx);
    }

    fn close_tab_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.readers.len() {
            self.readers.remove(ix);
            if self.active >= ix && self.active > 0 {
                self.active -= 1;
            }
            cx.notify();
        }
    }

    fn next_tab(&mut self, _: &NextTab, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.readers.is_empty() {
            self.active = (self.active + 1) % self.readers.len();
            cx.notify();
        }
    }

    fn prev_tab(&mut self, _: &PrevTab, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.readers.is_empty() {
            self.active = (self.active + self.readers.len() - 1) % self.readers.len();
            cx.notify();
        }
    }

    fn toggle_finder(&mut self, _: &ToggleFinder, window: &mut Window, cx: &mut Context<Self>) {
        if self.finder.is_some() {
            self.dismiss_finder(window, cx);
            return;
        }
        let Some(tree) = self.tree.as_ref() else {
            return;
        };
        let root = tree.root.clone();
        let files: Vec<(String, PathBuf)> = tree
            .all_files(10_000)
            .into_iter()
            .map(|path| {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                (rel, path)
            })
            .collect();

        let finder = cx.new(|cx| Finder::new(files, cx));
        let subscription = cx.subscribe_in(
            &finder,
            window,
            |this, _finder, event, window, cx| match event {
                FinderEvent::OpenPath(path) => {
                    let path = path.clone();
                    this.dismiss_finder(window, cx);
                    this.open_path(&path, cx);
                }
                FinderEvent::Dismissed => this.dismiss_finder(window, cx),
            },
        );
        window.focus(&finder.focus_handle(cx));
        self.finder = Some((finder, subscription));
        cx.notify();
    }

    fn dismiss_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finder = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _window: &mut Window, cx: &mut Context<Self>) {
        self.show_sidebar = !self.show_sidebar;
        cx.notify();
    }

    fn toggle_outline(&mut self, _: &ToggleOutline, _window: &mut Window, cx: &mut Context<Self>) {
        self.show_outline = !self.show_outline;
        cx.notify();
    }

    // ── Rendering ───────────────────────────────────────────────────────

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_sidebar {
            return None;
        }
        let tree = self.tree.as_mut()?;
        let t = theme(cx);
        let root_name = tree.root_name();
        let rows = tree.visible();
        let active_path = self
            .active_reader()
            .and_then(|r| r.read(cx).path.clone());

        let items = rows.into_iter().map(|(depth, entry)| {
            let is_active = active_path.as_deref() == Some(entry.path.as_path());
            let id = SharedString::from(format!("file-{}", entry.path.display()));
            let path = entry.path.clone();
            let is_dir = entry.is_dir;
            let expanded = is_dir && self.tree.as_ref().is_some_and(|t| t.is_expanded(&path));

            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .py(px(3.))
                .ml(px(depth as f32 * 12.))
                .rounded_md()
                .cursor_pointer()
                .hover(|s| s.bg(t.hover_bg))
                .when(is_active, |d| d.bg(t.selected_bg))
                .when(is_dir, |d| {
                    d.child(
                        div()
                            .text_size(px(9.))
                            .text_color(t.fg_muted)
                            .child(if expanded { "▼" } else { "▶" }),
                    )
                })
                .child(
                    div()
                        .text_size(px(t.ui_size))
                        .text_color(if is_dir { t.fg_strong } else { t.fg })
                        .overflow_hidden()
                        .child(SharedString::from(entry.name.clone())),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    if is_dir {
                        if let Some(tree) = this.tree.as_mut() {
                            tree.toggle(&path);
                        }
                        cx.notify();
                    } else {
                        this.open_path(&path, cx);
                    }
                }))
        });

        Some(
            div()
                .w(px(240.))
                .h_full()
                .flex_none()
                .bg(t.panel_bg)
                .border_r_1()
                .border_color(t.border)
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_3()
                        .pt(px(14.))
                        .pb(px(6.))
                        .text_size(px(11.))
                        .text_color(t.fg_muted)
                        .child(SharedString::from(root_name.to_uppercase())),
                )
                .child(
                    div()
                        .id("sidebar-scroll")
                        .flex_1()
                        .overflow_y_scroll()
                        .px_2()
                        .pb_4()
                        .flex()
                        .flex_col()
                        .children(items),
                )
                .into_any_element(),
        )
    }

    fn render_tab_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.readers.is_empty() {
            return None;
        }
        let t = theme(cx);
        let active = self.active;

        let tabs = self.readers.iter().enumerate().map(|(ix, reader)| {
            let title = reader.read(cx).title.clone();
            let is_active = ix == active;
            div()
                .id(SharedString::from(format!("tab-{ix}")))
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .h_full()
                .border_r_1()
                .border_color(t.border)
                .cursor_pointer()
                .when(is_active, |d| d.bg(t.bg))
                .when(!is_active, |d| d.hover(|s| s.bg(t.hover_bg)))
                .child(
                    div()
                        .text_size(px(t.ui_size))
                        .text_color(if is_active { t.fg_strong } else { t.fg_muted })
                        .child(title),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("tab-close-{ix}")))
                        .text_size(px(12.))
                        .text_color(t.fg_muted)
                        .px(px(2.))
                        .rounded_sm()
                        .hover(|s| s.bg(t.selected_bg))
                        .child("×")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            cx.stop_propagation();
                            this.close_tab_at(ix, cx);
                        })),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.active = ix;
                    cx.notify();
                }))
        });

        Some(
            div()
                .h(px(34.))
                .flex_none()
                .w_full()
                .bg(t.panel_bg)
                .border_b_1()
                .border_color(t.border)
                .flex()
                .flex_row()
                .overflow_hidden()
                .children(tabs)
                .into_any_element(),
        )
    }

    fn render_outline(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_outline {
            return None;
        }
        let reader = self.active_reader()?.clone();
        let t = theme(cx);
        let toc_len = reader.read(cx).toc.len();
        if toc_len < 2 {
            return None;
        }

        let entries = uniform_list(
            "outline",
            toc_len,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = theme(cx);
                let Some(reader) = this.active_reader().cloned() else {
                    return Vec::new();
                };
                range
                    .map(|ix| {
                        let (level, text, block_ix) = {
                            let r = reader.read(cx);
                            let e = &r.toc[ix];
                            (e.level, e.text.clone(), e.block_ix)
                        };
                        let reader = reader.clone();
                        div()
                            .id(ix)
                            .px_2()
                            .py(px(3.))
                            .ml(px((level.saturating_sub(1)) as f32 * 10.))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover_bg))
                            .text_size(px(t.ui_size - 1.))
                            .text_color(if level <= 2 { t.fg } else { t.fg_muted })
                            .overflow_hidden()
                            .child(text)
                            .on_click(move |_: &ClickEvent, _window, cx| {
                                reader.update(cx, |reader, cx| {
                                    reader.scroll_to_block(block_ix);
                                    cx.notify();
                                });
                            })
                    })
                    .collect()
            }),
        )
        .h_full();

        Some(
            div()
                .w(px(220.))
                .h_full()
                .flex_none()
                .bg(t.panel_bg)
                .border_l_1()
                .border_color(t.border)
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_3()
                        .pt(px(14.))
                        .pb(px(6.))
                        .text_size(px(11.))
                        .text_color(t.fg_muted)
                        .child("OUTLINE"),
                )
                .child(div().flex_1().px_2().pb_4().child(entries))
                .into_any_element(),
        )
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .text_color(t.fg_muted)
                    .text_size(px(t.ui_size))
                    .child(div().text_size(px(28.)).child("𝕄"))
                    .child("⌘O  open a file or folder")
                    .child("⌘P  go to file"),
            )
            .into_any_element()
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let sidebar = self.render_sidebar(cx);
        let tab_bar = self.render_tab_bar(cx);
        let outline = self.render_outline(cx);
        // TEMP: Task 9 verification, replaced in Task 12
        let content: AnyElement = if let Some(editor) = &self.debug_editor {
            editor.clone().into_any_element()
        } else {
            match self.active_reader() {
                Some(reader) => reader.clone().into_any_element(),
                None => self.render_empty(cx),
            }
        };

        div()
            .size_full()
            .bg(t.bg)
            .text_color(t.fg)
            .font_family(t.body_family.clone())
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::open_dialog))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_outline))
            .on_action(cx.listener(Self::toggle_finder))
            .flex()
            .flex_row()
            .children(sidebar)
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .children(tab_bar)
                    .child(div().flex_1().min_h_0().child(content)),
            )
            .children(outline)
            .when_some(self.finder.as_ref(), |root, (finder, _)| {
                let finder = finder.clone();
                root.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(110.))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.dismiss_finder(window, cx);
                            }),
                        )
                        .child(div().on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        }).child(finder)),
                )
            })
    }
}
