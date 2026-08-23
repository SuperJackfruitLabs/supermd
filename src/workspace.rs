//! The root view: sidebar, tab bar, document pane, and outline panel.

use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::{
    actions, div, px, uniform_list, AnyElement, App, ClickEvent, Entity, FocusHandle, Focusable,
    IntoElement, ParentElement, PathPromptOptions, Render, SharedString, Styled, Window,
};

use crate::editor::Editor;
use crate::files::FileTree;
use crate::seti::{self, SetiColor};
use crate::theme::Theme;
use crate::finder::{Finder, FinderEvent};
use crate::highlight::languages;
use crate::reader::Reader;
use crate::theme::theme;

actions!(
    workspace,
    [
        NewFile,
        OpenDialog,
        CloseTab,
        NextTab,
        PrevTab,
        ToggleSidebar,
        ToggleOutline,
        ToggleFinder,
        TogglePreview,
    ]
);

pub enum Tab {
    /// Read-only document (Welcome, and anything not editable).
    Reader(Entity<Reader>),
    /// Editable file; `preview` present while ⌘E preview mode is on.
    Editor {
        editor: Entity<Editor>,
        preview: Option<Entity<Reader>>,
    },
}

impl Tab {
    fn title(&self, cx: &App) -> SharedString {
        match self {
            Tab::Reader(reader) => reader.read(cx).title.clone(),
            Tab::Editor { editor, .. } => editor.read(cx).title(),
        }
    }

    fn path(&self, cx: &App) -> Option<PathBuf> {
        match self {
            Tab::Reader(reader) => reader.read(cx).path.clone(),
            Tab::Editor { editor, .. } => Some(editor.read(cx).path().to_path_buf()),
        }
    }
}

/// Seti's 12 palette variables mapped onto our theme so icons read well
/// in both appearances.
fn seti_tint(color: SetiColor, t: &Theme) -> gpui::Hsla {
    let s = &t.syntax;
    match color {
        SetiColor::Blue => s.function,
        SetiColor::Green => s.string,
        SetiColor::Grey | SetiColor::Ignore => t.fg_muted,
        SetiColor::GreyLight => t.fg,
        SetiColor::Orange | SetiColor::SetiPrimary => s.constant,
        SetiColor::Pink => s.property,
        SetiColor::Purple => s.keyword,
        SetiColor::Red => t.accent,
        SetiColor::White => t.fg,
        SetiColor::Yellow => s.kind,
    }
}

#[derive(Clone)]
enum OutlineTarget {
    Reader(Entity<Reader>),
    Editor(Entity<Editor>),
}

pub struct Workspace {
    pub tree: Option<FileTree>,
    tabs: Vec<Tab>,
    active: usize,
    show_sidebar: bool,
    show_outline: bool,
    finder: Option<(Entity<Finder>, gpui::Subscription)>,
    focus_handle: FocusHandle,
    last_title: String,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl Workspace {
    pub fn new(arg: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut tree = None;
        let mut tabs = Vec::new();

        match arg {
            Some(path) if path.is_dir() => {
                tree = Some(FileTree::new(path));
            }
            Some(path) => match Editor::read_file(&path) {
                Ok(text) => {
                    let langs = languages(cx);
                    let editor = cx.new(|cx| Editor::from_text(&path, text, &langs, cx));
                    tabs.push(Tab::Editor { editor, preview: None });
                }
                Err(err) => eprintln!("supermd: cannot open {}: {err}", path.display()),
            },
            None => {
                if let Ok(cwd) = std::env::current_dir() {
                    tree = Some(FileTree::new(cwd));
                }
                let welcome = Reader::welcome(&languages(cx));
                tabs.push(Tab::Reader(cx.new(|_| welcome)));
            }
        }

        Self {
            tree,
            tabs,
            active: 0,
            show_sidebar: true,
            show_outline: true,
            finder: None,
            focus_handle: cx.focus_handle(),
            last_title: String::new(),
            _watcher: None,
        }
    }

    /// (Re)start the fs watcher on the current workspace root and spawn
    /// the event drain loop (200 ms coalescing).
    pub fn setup_watcher(&mut self, cx: &mut Context<Self>) {
        use notify::Watcher as _;
        self._watcher = None;
        let Some(tree) = &self.tree else {
            return;
        };
        let root = tree.root.clone();
        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = match notify::recommended_watcher(move |res| {
            tx.send(res).ok();
        }) {
            Ok(watcher) => watcher,
            Err(err) => {
                eprintln!("supermd: file watcher unavailable: {err}");
                return;
            }
        };
        if let Err(err) = watcher.watch(&root, notify::RecursiveMode::Recursive) {
            eprintln!("supermd: cannot watch {}: {err}", root.display());
            return;
        }
        self._watcher = Some(watcher);

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
                let mut paths: Vec<PathBuf> = Vec::new();
                let mut disconnected = false;
                loop {
                    match rx.try_recv() {
                        Ok(Ok(event)) => paths.extend(event.paths),
                        Ok(Err(_)) => {}
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
                if disconnected {
                    break;
                }
                if !paths.is_empty()
                    && this
                        .update(cx, |workspace, cx| workspace.on_fs_events(&paths, cx))
                        .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_fs_events(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if let Some(tree) = &mut self.tree {
            tree.refresh();
        }
        for tab in &self.tabs {
            if let Tab::Editor { editor, .. } = tab {
                let editor_path = editor.read(cx).path().to_path_buf();
                if paths.iter().any(|p| *p == editor_path) {
                    editor.update(cx, |editor, cx| {
                        let changed =
                            crate::editor::autosave::disk_mtime(&editor_path) != editor.disk_mtime;
                        if crate::editor::autosave::should_reload(
                            editor.save.is_dirty(),
                            changed,
                        ) {
                            editor.reload_from_disk(cx);
                        } else if changed && editor.save.is_dirty() {
                            eprintln!(
                                "supermd: {} changed on disk; keeping unsaved edits",
                                editor_path.display()
                            );
                        }
                    });
                }
            }
        }
        cx.notify();
    }

    fn sync_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = match self.tabs.get(self.active) {
            Some(tab) => format!("supermd — {}", tab.title(cx)),
            None => "supermd".to_string(),
        };
        if title != self.last_title {
            window.set_window_title(&title);
            self.last_title = title;
        }
    }

    // ── tab management ─────────────────────────────────────────────────

    fn flush_tab(&self, ix: usize, cx: &mut Context<Self>) {
        if let Some(Tab::Editor { editor, .. }) = self.tabs.get(ix) {
            editor.update(cx, |editor, cx| editor.flush(cx));
        }
    }

    pub fn flush_all(&mut self, cx: &mut Context<Self>) {
        for ix in 0..self.tabs.len() {
            self.flush_tab(ix, cx);
        }
    }

    fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.tabs.get(self.active) {
            Some(Tab::Editor { editor, preview: None }) => {
                window.focus(&editor.focus_handle(cx))
            }
            _ => window.focus(&self.focus_handle),
        }
    }

    fn set_active(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        if ix != self.active {
            self.flush_tab(self.active, cx);
        }
        self.active = ix;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.flush_tab(ix, cx);
        self.tabs.remove(ix);
        if self.active >= ix && self.active > 0 {
            self.active -= 1;
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    pub fn open_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if path.is_dir() {
            self.tree = Some(FileTree::new(path.to_path_buf()));
            self.show_sidebar = true;
            self.setup_watcher(cx);
            cx.notify();
            return;
        }
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|tab| tab.path(cx).as_deref() == Some(path))
        {
            self.set_active(ix, window, cx);
            return;
        }
        match Editor::read_file(path) {
            Ok(text) => {
                self.flush_tab(self.active, cx);
                if let Some(tree) = &mut self.tree {
                    tree.expand_to(path);
                }
                let langs = languages(cx);
                let path = path.to_path_buf();
                let editor = cx.new(|cx| Editor::from_text(&path, text, &langs, cx));
                self.tabs.push(Tab::Editor { editor, preview: None });
                self.active = self.tabs.len() - 1;
                self.focus_active(window, cx);
                cx.notify();
            }
            Err(err) => eprintln!("supermd: cannot open {}: {err}", path.display()),
        }
    }

    // ── actions ────────────────────────────────────────────────────────

    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else {
            return; // single-file mode: no workspace to create into
        };
        let root = tree.root.clone();
        let existing: Vec<String> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .collect();
        let name = crate::files::pick_untitled(&existing);
        let path = root.join(&name);
        if let Err(err) = std::fs::write(&path, "") {
            eprintln!("supermd: cannot create {}: {err}", path.display());
            return;
        }
        if let Some(tree) = &mut self.tree {
            tree.refresh();
        }
        self.open_path(&path, window, cx);
    }

    fn open_dialog(&mut self, _: &OpenDialog, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update_in(cx, |workspace, window, cx| {
                    for path in paths {
                        workspace.open_path(&path, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, window, cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.set_active((self.active + 1) % self.tabs.len(), window, cx);
        }
    }

    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            let ix = (self.active + self.tabs.len() - 1) % self.tabs.len();
            self.set_active(ix, window, cx);
        }
    }

    fn toggle_preview(&mut self, _: &TogglePreview, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Tab::Editor { editor, preview }) = self.tabs.get(self.active) else {
            return;
        };
        let editor = editor.clone();
        let showing = preview.is_some();
        if showing {
            if let Some(Tab::Editor { preview, .. }) = self.tabs.get_mut(self.active) {
                *preview = None;
            }
        } else {
            editor.update(cx, |editor, cx| editor.flush(cx));
            let title = editor.read(cx).title();
            let text = editor.read(cx).text();
            let langs = languages(cx);
            let reader = cx.new(|_| Reader::from_source(title, &text, &langs));
            if let Some(Tab::Editor { preview, .. }) = self.tabs.get_mut(self.active) {
                *preview = Some(reader);
            }
        }
        self.focus_active(window, cx);
        cx.notify();
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
                    this.open_path(&path, window, cx);
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
        self.focus_active(window, cx);
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
        let active_path = self.tabs.get(self.active).and_then(|tab| tab.path(cx));
        let tree = self.tree.as_mut()?;
        let t = theme(cx);
        let root_name = tree.root_name();
        let rows = tree.visible();

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
                .child(
                    // Fixed chevron slot on every row so icons align in a
                    // column whether or not the row is a directory.
                    div()
                        .w(px(10.))
                        .flex_none()
                        .text_size(px(9.))
                        .text_color(t.fg_muted)
                        .when(is_dir, |d| d.child(if expanded { "▼" } else { "▶" })),
                )
                .child({
                    let (icon, tint) = if is_dir {
                        ("folder", t.fg_muted)
                    } else {
                        let (icon, color) = seti::icon_for(&entry.name);
                        (icon, seti_tint(color, &t))
                    };
                    // Seti glyphs carry ~30% internal padding, so the box
                    // runs larger than the text for a matched visual size.
                    gpui::svg()
                        .path(SharedString::from(format!("icons/seti/{icon}.svg")))
                        .size(px(20.))
                        .flex_none()
                        .text_color(tint)
                })
                .child(
                    div()
                        .text_size(px(t.ui_size))
                        .text_color(if is_dir { t.fg_strong } else { t.fg })
                        .overflow_hidden()
                        .child(SharedString::from(entry.name.clone())),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    if is_dir {
                        if let Some(tree) = this.tree.as_mut() {
                            tree.toggle(&path);
                        }
                        cx.notify();
                    } else {
                        this.open_path(&path, window, cx);
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
        if self.tabs.is_empty() {
            return None;
        }
        let t = theme(cx);
        let active = self.active;

        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let title = tab.title(cx);
            let is_preview = matches!(tab, Tab::Editor { preview: Some(_), .. });
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
                .when(is_preview, |d| {
                    d.child(
                        div()
                            .text_size(px(9.))
                            .text_color(t.fg_muted)
                            .child("PREVIEW"),
                    )
                })
                .child(
                    div()
                        .id(SharedString::from(format!("tab-close-{ix}")))
                        .text_size(px(12.))
                        .text_color(t.fg_muted)
                        .px(px(2.))
                        .rounded_sm()
                        .hover(|s| s.bg(t.selected_bg))
                        .child("×")
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.close_tab_at(ix, window, cx);
                        })),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_active(ix, window, cx);
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
        let t = theme(cx);

        let (entries, target): (Vec<(u8, SharedString, usize)>, OutlineTarget) =
            match self.tabs.get(self.active)? {
                Tab::Reader(reader) => (
                    reader
                        .read(cx)
                        .toc
                        .iter()
                        .map(|e| (e.level, e.text.clone(), e.block_ix))
                        .collect(),
                    OutlineTarget::Reader(reader.clone()),
                ),
                Tab::Editor { preview: Some(preview), .. } => (
                    preview
                        .read(cx)
                        .toc
                        .iter()
                        .map(|e| (e.level, e.text.clone(), e.block_ix))
                        .collect(),
                    OutlineTarget::Reader(preview.clone()),
                ),
                Tab::Editor { editor, preview: None } => (
                    editor
                        .read(cx)
                        .heading_lines()
                        .into_iter()
                        .map(|(level, text, line)| (level, SharedString::from(text), line))
                        .collect(),
                    OutlineTarget::Editor(editor.clone()),
                ),
            };

        if entries.len() < 2 {
            return None;
        }
        let count = entries.len();

        let list = uniform_list(
            "outline",
            count,
            cx.processor(move |_this, range: std::ops::Range<usize>, _window, cx| {
                let t = theme(cx);
                range
                    .filter_map(|ix| {
                        let (level, text, target_ix) = entries.get(ix)?.clone();
                        let target = target.clone();
                        Some(
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
                                .on_click(move |_: &ClickEvent, _window, cx| match &target {
                                    OutlineTarget::Reader(reader) => {
                                        reader.update(cx, |reader, cx| {
                                            reader.scroll_to_block(target_ix);
                                            cx.notify();
                                        });
                                    }
                                    OutlineTarget::Editor(editor) => {
                                        editor.update(cx, |editor, cx| {
                                            editor.scroll_to_line(target_ix);
                                            cx.notify();
                                        });
                                    }
                                }),
                        )
                    })
                    .collect::<Vec<_>>()
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
                .child(div().flex_1().px_2().pb_4().child(list))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_title(window, cx);
        let t = theme(cx);
        let sidebar = self.render_sidebar(cx);
        let tab_bar = self.render_tab_bar(cx);
        let outline = self.render_outline(cx);
        let content: AnyElement = match self.tabs.get(self.active) {
            Some(Tab::Reader(reader)) => reader.clone().into_any_element(),
            Some(Tab::Editor { preview: Some(preview), .. }) => {
                preview.clone().into_any_element()
            }
            Some(Tab::Editor { editor, preview: None }) => editor.clone().into_any_element(),
            None => self.render_empty(cx),
        };

        div()
            .size_full()
            .bg(t.bg)
            .text_color(t.fg)
            .font_family(t.body_family.clone())
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::open_dialog))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_outline))
            .on_action(cx.listener(Self::toggle_finder))
            .on_action(cx.listener(Self::toggle_preview))
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
                        .child(
                            div()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .child(finder),
                        ),
                )
            })
    }
}
