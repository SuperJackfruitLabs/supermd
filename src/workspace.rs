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
        ToggleSearch,
        TogglePreview,
        ShowChanges,
        ToggleFocusMode,
        FocusSidebar,
        SidebarUp,
        SidebarDown,
        SidebarExpand,
        SidebarCollapse,
        SidebarOpen,
        ToggleShortcuts,
        ToggleThemePicker,
        ThemePickerUp,
        ThemePickerDown,
        ThemePickerConfirm,
        ThemePickerCancel,
        ZoomIn,
        ZoomOut,
        ZoomReset,
    ]
);

/// Shown by the ⌘/ dialog. Kept adjacent to the actual bindings in
/// main.rs — update both together.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "General",
        &[
            ("⌘ N", "New file"),
            ("⌘ O", "Open file or folder"),
            ("⌘ P", "Go to file"),
            ("⌘ F", "Find in file"),
            ("⌘ E", "Toggle edit / preview"),
            ("⌘ ⇧ D", "Show changes vs git HEAD"),
            ("⌘ ⇧ F", "Search in workspace"),
            ("⌃ ⌘ F", "Focus mode"),
            ("⌘ B", "Toggle sidebar"),
            ("⌘ ⇧ O", "Toggle outline"),
            ("⌘ 1", "Focus sidebar"),
            ("⌘ W", "Close tab"),
            ("⌃ Tab / ⌘ ⇧ ]", "Next tab"),
            ("⌘ ⇧ [", "Previous tab"),
            ("⌘ S", "Save now"),
            ("⌘ + / − / 0", "Zoom image tab"),
            ("⌘ T", "Theme picker"),
            ("⌘ /", "This dialog"),
        ],
    ),
    (
        "Editor",
        &[
            ("⌘ Z / ⌘ ⇧ Z", "Undo / redo"),
            ("⌥ ← →", "Move by word"),
            ("⌘ ← →", "Line start / end"),
            ("⌘ ↑ ↓", "Document start / end"),
            ("⌥ ⌫", "Delete word"),
            ("⌘ G / ⌘ ⇧ G", "Next / previous match"),
            ("Click ✓ / ○", "Toggle task checkbox"),
            ("Click table / image", "Edit its source"),
        ],
    ),
    (
        "Sidebar",
        &[
            ("↑ ↓", "Move selection"),
            ("→", "Expand folder"),
            ("←", "Collapse / to parent"),
            ("⏎", "Open"),
        ],
    ),
];

/// How an editor tab presents its buffer.
pub enum EditorView {
    /// The editable styled-source view.
    Edit,
    /// ⌘E pretty preview (rendered read-only Reader).
    Preview(Entity<Reader>),
    /// ⌘⇧D read-only diff vs git HEAD.
    Diff,
}

pub enum Tab {
    /// Read-only document (Welcome, and anything not editable).
    Reader(Entity<Reader>),
    /// Editable file, presented per `view`.
    Editor {
        editor: Entity<Editor>,
        view: EditorView,
    },
    /// Read-only image viewer. `zoom` is relative to fit (1.0 = fit).
    Image { path: PathBuf, title: SharedString, zoom: f32 },
}

impl Tab {
    fn title(&self, cx: &App) -> SharedString {
        match self {
            Tab::Reader(reader) => reader.read(cx).title.clone(),
            Tab::Editor { editor, .. } => editor.read(cx).title(),
            Tab::Image { title, .. } => title.clone(),
        }
    }

    fn path(&self, cx: &App) -> Option<PathBuf> {
        match self {
            Tab::Reader(reader) => reader.read(cx).path.clone(),
            Tab::Editor { editor, .. } => Some(editor.read(cx).path().to_path_buf()),
            Tab::Image { path, .. } => Some(path.clone()),
        }
    }
}

/// Seti's 12 palette variables mapped onto our theme so icons read well
/// in both appearances.
pub(crate) fn seti_tint(color: SetiColor, t: &Theme) -> gpui::Hsla {
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

struct ThemePickerState {
    /// Theme indices in display order (lights, then darks).
    order: Vec<usize>,
    pos: usize,
    saved_theme: std::sync::Arc<Theme>,
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
    focus_mode: bool,
    pre_focus_panels: (bool, bool),
    finder: Option<(Entity<Finder>, gpui::Subscription)>,
    search: Option<(Entity<crate::search_ui::SearchOverlay>, gpui::Subscription)>,
    focus_handle: FocusHandle,
    sidebar_focus: FocusHandle,
    sidebar_selected: usize,
    shortcuts_focus: FocusHandle,
    show_shortcuts: bool,
    theme_picker: Option<ThemePickerState>,
    theme_picker_focus: FocusHandle,
    last_title: String,
    /// Absolute paths of files with uncommitted git changes (sidebar dots).
    git_modified: std::collections::HashSet<PathBuf>,
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
                    tabs.push(Tab::Editor { editor, view: EditorView::Edit });
                }
                Err(err) => eprintln!("supermd: cannot open {}: {err}", path.display()),
            },
            None => {
                // No explicit target: start with an empty workspace and
                // the welcome document. (A Finder/Dock launch has an
                // arbitrary cwd — listing it would show the filesystem.)
                let welcome = Reader::welcome(&languages(cx));
                tabs.push(Tab::Reader(cx.new(|_| welcome)));
            }
        }

        let mut workspace = Self {
            tree,
            tabs,
            active: 0,
            show_sidebar: true,
            show_outline: true,
            focus_mode: false,
            pre_focus_panels: (true, true),
            finder: None,
            search: None,
            focus_handle: cx.focus_handle(),
            sidebar_focus: cx.focus_handle(),
            sidebar_selected: 0,
            shortcuts_focus: cx.focus_handle(),
            show_shortcuts: false,
            theme_picker: None,
            theme_picker_focus: cx.focus_handle(),
            last_title: String::new(),
            git_modified: Default::default(),
            _watcher: None,
        };
        workspace.refresh_git_status();
        workspace
    }

    /// Rescan uncommitted changes for the sidebar dots. Cheap enough to
    /// run on every watcher drain; skipped instantly outside a repo.
    fn refresh_git_status(&mut self) {
        self.git_modified = match &self.tree {
            Some(tree) => {
                let root = tree.root.clone();
                crate::git::modified_paths(&root)
                    .into_iter()
                    .map(|rel| root.join(rel))
                    .collect()
            }
            None => Default::default(),
        };
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
        // Ignore churn from ignored paths (target/, node_modules/, …) so
        // builds in an open workspace don't hammer the UI.
        if let Some(tree) = &self.tree {
            let root = tree.root.clone();
            if !paths.iter().any(|p| crate::files::is_visible(&root, p)) {
                return;
            }
        }
        if let Some(tree) = &mut self.tree {
            tree.refresh();
        }
        self.refresh_git_status();
        let langs = languages(cx);
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
                            editor.refresh_diff(&langs, cx);
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
            Some(tab) => format!("SuperMD — {}", tab.title(cx)),
            None => "SuperMD".to_string(),
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
            Some(Tab::Editor { editor, view: EditorView::Edit }) => {
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
            self.refresh_git_status();
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
        if crate::files::is_image_path(path) {
            self.flush_tab(self.active, cx);
            if let Some(tree) = &mut self.tree {
                tree.expand_to(path);
            }
            let title: SharedString = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
                .into();
            self.tabs.push(Tab::Image { path: path.to_path_buf(), title, zoom: 1.0 });
            self.active = self.tabs.len() - 1;
            self.focus_active(window, cx);
            cx.notify();
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
                self.tabs.push(Tab::Editor { editor, view: EditorView::Edit });
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
        let Some(Tab::Editor { editor, view }) = self.tabs.get(self.active) else {
            return;
        };
        let editor = editor.clone();
        let showing = matches!(view, EditorView::Preview(_));
        if showing {
            if let Some(Tab::Editor { view, .. }) = self.tabs.get_mut(self.active) {
                *view = EditorView::Edit;
            }
        } else {
            editor.update(cx, |editor, cx| editor.flush(cx));
            let title = editor.read(cx).title();
            let text = editor.read(cx).text();
            let langs = languages(cx);
            let reader = cx.new(|_| Reader::from_source(title, &text, &langs));
            if let Some(Tab::Editor { view, .. }) = self.tabs.get_mut(self.active) {
                *view = EditorView::Preview(reader);
            }
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    fn show_changes(&mut self, _: &ShowChanges, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Tab::Editor { editor, view }) = self.tabs.get(self.active) else {
            return;
        };
        let editor = editor.clone();
        let leaving = matches!(view, EditorView::Diff);
        if leaving {
            editor.update(cx, |editor, cx| editor.exit_diff(cx));
            if let Some(Tab::Editor { view, .. }) = self.tabs.get_mut(self.active) {
                *view = EditorView::Edit;
            }
        } else {
            editor.update(cx, |editor, cx| editor.flush(cx));
            let langs = languages(cx);
            editor.update(cx, |editor, cx| editor.enter_diff(&langs, cx));
            if let Some(Tab::Editor { view, .. }) = self.tabs.get_mut(self.active) {
                *view = EditorView::Diff;
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

    fn toggle_search(&mut self, _: &ToggleSearch, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.dismiss_search(window, cx);
            return;
        }
        let Some(tree) = self.tree.as_ref() else {
            return;
        };
        let root = tree.root.clone();
        let overlay = cx.new(|cx| crate::search_ui::SearchOverlay::new(root, cx));
        let subscription = cx.subscribe_in(
            &overlay,
            window,
            |this, _overlay, event, window, cx| match event {
                crate::search_ui::SearchEvent::Open { path, line } => {
                    let (path, line) = (path.clone(), *line);
                    this.dismiss_search(window, cx);
                    this.open_path(&path, window, cx);
                    if let Some(Tab::Editor { editor, .. }) = this.tabs.get(this.active) {
                        editor.update(cx, |editor, cx| {
                            editor.scroll_to_line((line as usize).saturating_sub(1), cx);
                        });
                    }
                }
                crate::search_ui::SearchEvent::Dismissed => this.dismiss_search(window, cx),
            },
        );
        window.focus(&overlay.focus_handle(cx));
        self.search = Some((overlay, subscription));
        cx.notify();
    }

    fn dismiss_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    // ── sidebar keyboard navigation ────────────────────────────────────

    fn sidebar_rows(&mut self) -> Vec<(usize, crate::files::FsEntry)> {
        self.tree.as_mut().map(|t| t.visible()).unwrap_or_default()
    }

    fn focus_sidebar(&mut self, _: &FocusSidebar, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.is_none() {
            return;
        }
        self.show_sidebar = true;
        window.focus(&self.sidebar_focus);
        cx.notify();
    }

    fn sidebar_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.sidebar_rows().len();
        if count == 0 {
            return;
        }
        let target = (self.sidebar_selected as isize + delta).clamp(0, count as isize - 1);
        self.sidebar_selected = target as usize;
        cx.notify();
    }

    fn sidebar_up(&mut self, _: &SidebarUp, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_move(-1, cx);
    }

    fn sidebar_down(&mut self, _: &SidebarDown, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_move(1, cx);
    }

    fn sidebar_expand(&mut self, _: &SidebarExpand, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.sidebar_rows();
        let Some((_, entry)) = rows.get(self.sidebar_selected) else {
            return;
        };
        if !entry.is_dir {
            return;
        }
        let path = entry.path.clone();
        let Some(tree) = self.tree.as_mut() else {
            return;
        };
        if tree.is_expanded(&path) {
            self.sidebar_move(1, cx); // step into
        } else {
            tree.toggle(&path);
            cx.notify();
        }
    }

    fn sidebar_collapse(&mut self, _: &SidebarCollapse, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.sidebar_rows();
        let Some((depth, entry)) = rows.get(self.sidebar_selected).cloned() else {
            return;
        };
        if entry.is_dir && self.tree.as_ref().is_some_and(|t| t.is_expanded(&entry.path)) {
            if let Some(tree) = self.tree.as_mut() {
                tree.toggle(&entry.path);
            }
            cx.notify();
            return;
        }
        // Jump to the parent row (nearest earlier row with smaller depth).
        if let Some(parent) = rows[..self.sidebar_selected]
            .iter()
            .rposition(|(d, _)| *d < depth)
        {
            self.sidebar_selected = parent;
            cx.notify();
        }
    }

    fn sidebar_open(&mut self, _: &SidebarOpen, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.sidebar_rows();
        let Some((_, entry)) = rows.get(self.sidebar_selected).cloned() else {
            return;
        };
        if entry.is_dir {
            if let Some(tree) = self.tree.as_mut() {
                tree.toggle(&entry.path);
            }
            cx.notify();
        } else {
            self.open_path(&entry.path, window, cx);
        }
    }

    // ── theme picker ────────────────────────────────────────────────────

    fn toggle_theme_picker(
        &mut self,
        _: &ToggleThemePicker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.theme_picker.is_some() {
            self.theme_picker_cancel(&ThemePickerCancel, window, cx);
            return;
        }
        let state = cx.global::<crate::theme::ThemeState>();
        let mut order: Vec<usize> = (0..state.themes.len())
            .filter(|&i| !state.themes[i].theme.is_dark)
            .collect();
        order.extend((0..state.themes.len()).filter(|&i| state.themes[i].theme.is_dark));
        let current = theme(cx);
        let pos = order
            .iter()
            .position(|&i| std::sync::Arc::ptr_eq(&state.themes[i].theme, &current))
            .unwrap_or(0);
        self.theme_picker = Some(ThemePickerState { order, pos, saved_theme: current });
        window.focus(&self.theme_picker_focus);
        cx.notify();
    }

    fn theme_picker_apply(&mut self, pos: usize, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.theme_picker else {
            return;
        };
        picker.pos = pos;
        let ix = picker.order[picker.pos];
        let theme = cx.global::<crate::theme::ThemeState>().themes[ix].theme.clone();
        cx.set_global(crate::theme::ActiveTheme(theme));
        cx.notify();
    }

    fn theme_picker_up(&mut self, _: &ThemePickerUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = &self.theme_picker {
            let pos = picker.pos.saturating_sub(1);
            self.theme_picker_apply(pos, cx);
        }
    }

    fn theme_picker_down(&mut self, _: &ThemePickerDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = &self.theme_picker {
            let pos = (picker.pos + 1).min(picker.order.len().saturating_sub(1));
            self.theme_picker_apply(pos, cx);
        }
    }

    fn theme_picker_confirm(
        &mut self,
        _: &ThemePickerConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.theme_picker.take() else {
            return;
        };
        let ix = picker.order[picker.pos];
        {
            let state = cx.global_mut::<crate::theme::ThemeState>();
            let picked = &state.themes[ix];
            if picked.theme.is_dark {
                state.settings.dark_theme = picked.name.clone();
            } else {
                state.settings.light_theme = picked.name.clone();
            }
            if let Err(err) =
                crate::settings::save(&crate::settings::config_dir(), &state.settings)
            {
                eprintln!("supermd: cannot save settings: {err}");
            }
        }
        crate::theme::refresh_active_theme(cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    fn theme_picker_cancel(
        &mut self,
        _: &ThemePickerCancel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(picker) = self.theme_picker.take() {
            cx.set_global(crate::theme::ActiveTheme(picker.saved_theme));
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn render_theme_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.theme_picker.as_ref()?;
        let t = theme(cx);
        let state = cx.global::<crate::theme::ThemeState>();

        let mut rows: Vec<AnyElement> = Vec::new();
        let mut last_dark: Option<bool> = None;
        for (pos, &ix) in picker.order.iter().enumerate() {
            let loaded = &state.themes[ix];
            let is_dark = loaded.theme.is_dark;
            if last_dark != Some(is_dark) {
                last_dark = Some(is_dark);
                rows.push(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_size(px(10.))
                        .text_color(t.fg_muted)
                        .child(if is_dark { "DARK" } else { "LIGHT" })
                        .into_any_element(),
                );
            }
            let chosen = if is_dark {
                state.settings.dark_theme == loaded.name
            } else {
                state.settings.light_theme == loaded.name
            };
            let selected = pos == picker.pos;
            rows.push(
                div()
                    .id(("theme-row", pos))
                    .w_full()
                    .px_2()
                    .py(px(4.))
                    .rounded_md()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.selected_bg))
                    .when(!selected, |d| d.hover(|s| s.bg(t.hover_bg)))
                    .child(
                        div()
                            .size(px(14.))
                            .rounded_full()
                            .border_1()
                            .border_color(t.border)
                            .bg(loaded.theme.bg),
                    )
                    .child(div().size(px(14.)).rounded_full().bg(loaded.theme.accent))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(t.ui_size))
                            .text_color(t.fg)
                            .child(SharedString::from(loaded.name.clone())),
                    )
                    .when(chosen, |d| {
                        d.child(div().text_size(px(11.)).text_color(t.accent).child("✓"))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.theme_picker_apply(pos, cx);
                    }))
                    .into_any_element(),
            );
        }

        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(gpui::Hsla { h: 0., s: 0., l: 0., a: 0.25 })
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.theme_picker_cancel(&ThemePickerCancel, window, cx);
                    }),
                )
                .child(
                    div()
                        .key_context("ThemePicker")
                        .track_focus(&self.theme_picker_focus)
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .w(px(340.))
                        .max_h(px(480.))
                        .id("theme-picker-panel")
                        .overflow_y_scroll()
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded_lg()
                        .shadow_lg()
                        .p_2()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .text_size(px(13.))
                                .text_color(t.fg_strong)
                                .child("Theme    ↑↓ preview · ⏎ apply · esc cancel"),
                        )
                        .children(rows),
                )
                .into_any_element(),
        )
    }

    fn adjust_zoom(&mut self, factor: Option<f32>, cx: &mut Context<Self>) {
        if let Some(Tab::Image { zoom, .. }) = self.tabs.get_mut(self.active) {
            *zoom = match factor {
                Some(f) => (*zoom * f).clamp(0.25, 8.0),
                None => 1.0,
            };
            cx.notify();
        }
    }

    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.adjust_zoom(Some(1.25), cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.adjust_zoom(Some(0.8), cx);
    }

    fn zoom_reset(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.adjust_zoom(None, cx);
    }

    fn toggle_shortcuts(
        &mut self,
        _: &ToggleShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_shortcuts = !self.show_shortcuts;
        if self.show_shortcuts {
            window.focus(&self.shortcuts_focus);
        } else {
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    fn render_shortcuts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_shortcuts {
            return None;
        }
        let t = theme(cx);
        let groups = SHORTCUTS.iter().map(|(title, rows)| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.fg_muted)
                        .pb_1()
                        .child(SharedString::from(title.to_uppercase())),
                )
                .children(rows.iter().map(|(keys, desc)| {
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .w(px(140.))
                                .flex_none()
                                .font_family(t.mono_family.clone())
                                .text_size(px(11.))
                                .text_color(t.fg_strong)
                                .px_2()
                                .py(px(2.))
                                .bg(t.code_bg)
                                .rounded_md()
                                .child(SharedString::from(*keys)),
                        )
                        .child(
                            div()
                                .text_size(px(t.ui_size))
                                .text_color(t.fg)
                                .child(SharedString::from(*desc)),
                        )
                }))
        });

        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(gpui::Hsla { h: 0., s: 0., l: 0., a: 0.35 })
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.toggle_shortcuts(&ToggleShortcuts, window, cx);
                    }),
                )
                .child(
                    div()
                        .key_context("Shortcuts")
                        .track_focus(&self.shortcuts_focus)
                        .on_action(cx.listener(Self::toggle_shortcuts))
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .w(px(520.))
                        .max_h(px(620.))
                        .id("shortcuts-panel")
                        .overflow_y_scroll()
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded_lg()
                        .shadow_lg()
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(
                            div()
                                .text_size(px(15.))
                                .text_color(t.fg_strong)
                                .child("Keyboard Shortcuts"),
                        )
                        .children(groups),
                )
                .into_any_element(),
        )
    }

    fn toggle_focus_mode(
        &mut self,
        _: &ToggleFocusMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focus_mode {
            self.focus_mode = false;
            (self.show_sidebar, self.show_outline) = self.pre_focus_panels;
        } else {
            self.pre_focus_panels = (self.show_sidebar, self.show_outline);
            self.focus_mode = true;
            self.show_sidebar = false;
            self.show_outline = false;
        }
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
        let t = theme(cx);
        let Some(tree) = self.tree.as_mut() else {
            // Empty workspace: no listing, just a way to open one.
            return Some(
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
                        // Traffic lights live over the sidebar's top strip.
                        div()
                            .h(px(34.))
                            .w_full()
                            .flex_none()
                            .window_control_area(gpui::WindowControlArea::Drag),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_3()
                            .px_4()
                            .child(
                                div()
                                    .text_size(px(t.ui_size))
                                    .text_color(t.fg_muted)
                                    .child("No folder open"),
                            )
                            .child(
                                div()
                                    .id("open-folder")
                                    .px_3()
                                    .py(px(6.))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(t.hover_bg)
                                    .hover(|s| s.bg(t.selected_bg))
                                    .text_size(px(t.ui_size))
                                    .text_color(t.fg)
                                    .child("Open Folder…")
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, window, cx| {
                                            this.open_dialog(&OpenDialog, window, cx);
                                        },
                                    )),
                            ),
                    )
                    .into_any_element(),
            );
        };
        let root_name = tree.root_name();
        let rows = tree.visible();

        let kb_selected = self.sidebar_selected;
        let git_modified = self.git_modified.clone();
        let items = rows.into_iter().enumerate().map(|(row_ix, (depth, entry))| {
            let is_active = active_path.as_deref() == Some(entry.path.as_path());
            let is_kb_selected = row_ix == kb_selected;
            let id = SharedString::from(format!("file-{}", entry.path.display()));
            let path = entry.path.clone();
            let is_dir = entry.is_dir;
            let expanded = is_dir && self.tree.as_ref().is_some_and(|t| t.is_expanded(&path));
            let is_modified = !is_dir && git_modified.contains(&entry.path);

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
                .when(is_kb_selected, |d| d.bg(t.hover_bg))
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
                .child(div().flex_1())
                .when(is_modified, |d| {
                    d.child(
                        div()
                            .size(px(5.))
                            .flex_none()
                            .mr(px(2.))
                            .rounded_full()
                            .bg(t.accent),
                    )
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.sidebar_selected = row_ix;
                    if is_dir {
                        if let Some(tree) = this.tree.as_mut() {
                            tree.toggle(&path);
                        }
                        window.focus(&this.sidebar_focus);
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
                .key_context("Sidebar")
                .track_focus(&self.sidebar_focus)
                .on_action(cx.listener(Self::sidebar_up))
                .on_action(cx.listener(Self::sidebar_down))
                .on_action(cx.listener(Self::sidebar_expand))
                .on_action(cx.listener(Self::sidebar_collapse))
                .on_action(cx.listener(Self::sidebar_open))
                .flex()
                .flex_col()
                .child(
                    // Traffic lights live over the sidebar's top strip.
                    div()
                        .h(px(34.))
                        .w_full()
                        .flex_none()
                        .window_control_area(gpui::WindowControlArea::Drag),
                )
                .child(
                    div()
                        .px_3()
                        .pt(px(4.))
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

    /// The custom title bar: traffic-light inset, tabs, drag regions.
    fn render_titlebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let active = self.active;
        let show_tabs = !self.tabs.is_empty() && !self.focus_mode;

        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let title = tab.title(cx);
            let is_preview = matches!(tab, Tab::Editor { view: EditorView::Preview(_), .. });
            let is_active = ix == active;
            let (icon, color) = seti::icon_for(&title);
            let tint = seti_tint(color, &t);
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
                    gpui::svg()
                        .path(SharedString::from(format!("icons/seti/{icon}.svg")))
                        .size(px(18.))
                        .flex_none()
                        .text_color(if is_active { tint } else { t.fg_muted }),
                )
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

        div()
            .h(px(34.))
            .flex_none()
            .w_full()
            .bg(t.panel_bg)
            .when(!self.focus_mode, |d| d.border_b_1().border_color(t.border))
            .flex()
            .flex_row()
            .overflow_hidden()
            .when(!self.show_sidebar, |d| {
                // With the sidebar hidden the traffic lights sit over
                // the tab bar — inset past them; the sidebar's own drag
                // strip covers them otherwise.
                d.child(
                    div()
                        .w(px(76.))
                        .h_full()
                        .flex_none()
                        .window_control_area(gpui::WindowControlArea::Drag),
                )
            })
            .when(show_tabs, |d| d.children(tabs))
            .child(
                // Remaining space drags the window.
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(gpui::WindowControlArea::Drag),
            )
            .into_any_element()
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
                Tab::Editor { view: EditorView::Preview(preview), .. } => (
                    preview
                        .read(cx)
                        .toc
                        .iter()
                        .map(|e| (e.level, e.text.clone(), e.block_ix))
                        .collect(),
                    OutlineTarget::Reader(preview.clone()),
                ),
                Tab::Editor { editor, .. } => (
                    editor
                        .read(cx)
                        .heading_lines()
                        .into_iter()
                        .map(|(level, text, line)| (level, SharedString::from(text), line))
                        .collect(),
                    OutlineTarget::Editor(editor.clone()),
                ),
                Tab::Image { .. } => return None,
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
                                // uniform_list rows can't grow — long
                                // headings truncate instead of wrapping
                                // over their neighbors.
                                .overflow_hidden()
                                .truncate()
                                .child(text)
                                .on_click(move |_: &ClickEvent, _window, cx| match &target {
                                    OutlineTarget::Reader(reader) => {
                                        reader.update(cx, |reader, cx| {
                                            reader.scroll_to_block(target_ix, cx);
                                        });
                                    }
                                    OutlineTarget::Editor(editor) => {
                                        editor.update(cx, |editor, cx| {
                                            editor.scroll_to_line(target_ix, cx);
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
        let titlebar = self.render_titlebar(cx);
        let outline = self.render_outline(cx);
        let content: AnyElement = match self.tabs.get(self.active) {
            Some(Tab::Reader(reader)) => reader.clone().into_any_element(),
            Some(Tab::Editor { view: EditorView::Preview(preview), .. }) => {
                preview.clone().into_any_element()
            }
            Some(Tab::Editor { editor, .. }) => editor.clone().into_any_element(),
            Some(Tab::Image { path, zoom, .. }) => {
                let zoom = *zoom;
                if zoom <= 1.0 + f32::EPSILON && zoom >= 1.0 - f32::EPSILON {
                    div()
                        .size_full()
                        .bg(t.bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .p(px(32.))
                        .child(gpui::img(path.clone()).max_w_full().max_h_full().rounded_md())
                        .into_any_element()
                } else {
                    div()
                        .id("image-zoom-scroll")
                        .size_full()
                        .bg(t.bg)
                        .overflow_scroll()
                        .p(px(32.))
                        .child(gpui::img(path.clone()).w(gpui::relative(zoom)))
                        .into_any_element()
                }
            }
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
            .on_action(cx.listener(Self::toggle_search))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::show_changes))
            .on_action(cx.listener(Self::toggle_focus_mode))
            .on_action(cx.listener(Self::focus_sidebar))
            .on_action(cx.listener(Self::toggle_shortcuts))
            .on_action(cx.listener(Self::toggle_theme_picker))
            .on_action(cx.listener(Self::theme_picker_up))
            .on_action(cx.listener(Self::theme_picker_down))
            .on_action(cx.listener(Self::theme_picker_confirm))
            .on_action(cx.listener(Self::theme_picker_cancel))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .flex()
            .flex_row()
            .children(sidebar)
            .child(
                // Tabs sit above the document area only — the sidebar
                // runs the full window height beside them.
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(titlebar)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .flex()
                            .flex_row()
                            .child(
                                div()
                                    .flex_1()
                                    .h_full()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(div().flex_1().min_h_0().child(content)),
                            )
                            .children(outline),
                    ),
            )
            .children(self.render_shortcuts(cx))
            .children(self.render_theme_picker(cx))
            .when_some(self.finder.as_ref(), |root, (finder, _)| {
                let finder = finder.clone();
                root.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
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
            .when_some(self.search.as_ref(), |root, (overlay, _)| {
                let overlay = overlay.clone();
                root.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(110.))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.dismiss_search(window, cx);
                            }),
                        )
                        .child(
                            div()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .child(overlay),
                        ),
                )
            })
    }
}
