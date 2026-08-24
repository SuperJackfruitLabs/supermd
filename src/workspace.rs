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
        OpenRecent0,
        OpenRecent1,
        OpenRecent2,
        OpenRecent3,
        OpenRecent4,
        OpenRecent5,
        OpenRecent6,
        OpenRecent7,
    ]
);

/// The welcome tour must be editable (it promises clickable checkboxes),
/// so it lives as a real file the user owns. Written once; never
/// clobbers user edits.
pub(crate) fn ensure_welcome_file(config_dir: &Path) -> PathBuf {
    let path = config_dir.join("Welcome.md");
    if !path.exists() {
        let _ = std::fs::create_dir_all(config_dir);
        if let Err(err) = std::fs::write(&path, include_str!("../WELCOME.md")) {
            eprintln!("supermd: cannot write welcome file: {err}");
        }
    }
    path
}

/// Persist a just-opened workspace root into the recents list.
fn record_recent(root: &Path) {
    let dir = crate::settings::config_dir();
    let mut settings = crate::settings::load(&dir);
    settings.note_workspace(root);
    if let Err(err) = crate::settings::save(&dir, &settings) {
        eprintln!("supermd: cannot save settings: {err}");
    }
}

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

/// Where a preview-open lands: activate an existing permanent tab,
/// reuse the current preview slot, or push a fresh tab.
#[derive(PartialEq, Eq, Debug)]
pub(crate) enum PreviewPlan {
    ActivateExisting(usize),
    ReplacePreview(usize),
    PushNew,
}

pub(crate) fn preview_plan(preview: Option<usize>, existing_ix: Option<usize>) -> PreviewPlan {
    match (existing_ix, preview) {
        (Some(ix), _) => PreviewPlan::ActivateExisting(ix),
        (None, Some(slot)) => PreviewPlan::ReplacePreview(slot),
        (None, None) => PreviewPlan::PushNew,
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
    /// Index of the transient preview tab (at most one; italic title).
    preview_tab: Option<usize>,
    /// Newer released version tag, when the launch check found one.
    update_available: Option<SharedString>,
    /// Recents snapshot taken at launch (existing dirs only); the
    /// Open Recent menu indexes into this.
    startup_recents: Vec<PathBuf>,
    /// Move-to-Applications offer (Some = banner visible with message).
    install_banner: Option<SharedString>,
    /// ☰ popover on platforms without a global menu bar.
    app_menu_open: bool,
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
                record_recent(&path);
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
                let path = ensure_welcome_file(&crate::settings::config_dir());
                match Editor::read_file(&path) {
                    Ok(text) => {
                        let langs = languages(cx);
                        let editor = cx.new(|cx| Editor::from_text(&path, text, &langs, cx));
                        tabs.push(Tab::Editor { editor, view: EditorView::Edit });
                    }
                    Err(_) => {
                        // Unwritable config dir: fall back to read-only.
                        let welcome = Reader::welcome(&languages(cx));
                        tabs.push(Tab::Reader(cx.new(|_| welcome)));
                    }
                }
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
            preview_tab: None,
            update_available: None,
            startup_recents: crate::settings::load(&crate::settings::config_dir())
                .recent_workspaces
                .iter()
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
                .collect(),
            app_menu_open: false,
            install_banner: std::env::current_exe()
                .ok()
                .filter(|exe| crate::install::needs_install(exe))
                .map(|_| "SuperMD is running from the disk image.".into()),
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

        // One quiet update check per launch; failures are silent.
        cx.spawn(async move |this, cx| {
            let tag = cx
                .background_executor()
                .spawn(async { crate::update::fetch_latest_tag() })
                .await;
            if let Some(tag) = tag {
                if crate::update::is_newer(env!("CARGO_PKG_VERSION"), &tag) {
                    this.update(cx, |this, cx| {
                        this.update_available = Some(SharedString::from(tag));
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();

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

    /// Open paths handed to us from outside (Finder open events, drops):
    /// folders become the workspace root, files become permanent tabs.
    pub fn open_external_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (dirs, files): (Vec<_>, Vec<_>) = paths
            .into_iter()
            .filter(|p| p.exists())
            .partition(|p| p.is_dir());
        for dir in dirs {
            self.open_path(&dir, window, cx);
        }
        for file in files {
            self.open_path(&file, window, cx);
        }
    }

    /// Poll the shared open-event queue (fed by `on_open_urls`).
    pub fn watch_external_opens(
        &mut self,
        pending: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(300))
                    .await;
                let paths: Vec<PathBuf> = std::mem::take(&mut *pending.lock().unwrap());
                if paths.is_empty() {
                    continue;
                }
                let live = this
                    .update_in(cx, |workspace, window, cx| {
                        workspace.open_external_paths(paths, window, cx);
                    })
                    .is_ok();
                if !live {
                    break;
                }
            }
        })
        .detach();
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.flush_tab(ix, cx);
        self.tabs.remove(ix);
        match self.preview_tab {
            Some(p) if p == ix => self.preview_tab = None,
            Some(p) if p > ix => self.preview_tab = Some(p - 1),
            _ => {}
        }
        if self.active >= ix && self.active > 0 {
            self.active -= 1;
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Open a file transiently: reuse the single preview-tab slot,
    /// activating an existing permanent tab instead when one has the
    /// path. `focus: false` keeps focus where it is (sidebar browsing).
    fn open_path_preview(
        &mut self,
        path: &Path,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path.is_dir() {
            return;
        }
        let existing = self
            .tabs
            .iter()
            .position(|tab| tab.path(cx).as_deref() == Some(path));
        let plan = preview_plan(self.preview_tab, existing);
        if let PreviewPlan::ActivateExisting(ix) = plan {
            if ix != self.active {
                self.flush_tab(self.active, cx);
                self.active = ix;
            }
            if focus {
                self.focus_active(window, cx);
            }
            cx.notify();
            return;
        }
        let tab = if crate::files::is_image_path(path) {
            let title: SharedString = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
                .into();
            Tab::Image { path: path.to_path_buf(), title, zoom: 1.0 }
        } else {
            match Editor::read_file(path) {
                Ok(text) => {
                    let langs = languages(cx);
                    let path_buf = path.to_path_buf();
                    let editor = cx.new(|cx| Editor::from_text(&path_buf, text, &langs, cx));
                    Tab::Editor { editor, view: EditorView::Edit }
                }
                Err(err) => {
                    eprintln!("supermd: cannot open {}: {err}", path.display());
                    return;
                }
            }
        };
        match plan {
            PreviewPlan::ReplacePreview(slot) => {
                self.flush_tab(slot, cx);
                if self.active != slot {
                    self.flush_tab(self.active, cx);
                }
                self.tabs[slot] = tab;
                self.active = slot;
            }
            PreviewPlan::PushNew => {
                self.flush_tab(self.active, cx);
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                self.preview_tab = Some(self.active);
            }
            PreviewPlan::ActivateExisting(_) => unreachable!(),
        }
        if let Some(tree) = &mut self.tree {
            tree.expand_to(path);
        }
        if focus {
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    pub fn open_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if path.is_dir() {
            record_recent(path);
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
            // A deliberate open pins the tab it lands on.
            if self.preview_tab == Some(ix) {
                self.preview_tab = None;
            }
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

    fn open_recent_ix(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.startup_recents.get(ix).cloned() {
            if path.is_dir() {
                self.open_path(&path, window, cx);
            }
        }
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

    fn sidebar_move(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.sidebar_rows();
        if rows.is_empty() {
            return;
        }
        let target = (self.sidebar_selected as isize + delta).clamp(0, rows.len() as isize - 1);
        self.sidebar_selected = target as usize;
        // Browsing with the keyboard previews files as you land on them;
        // focus stays in the sidebar so arrows keep working.
        if let Some((_, entry)) = rows.get(self.sidebar_selected) {
            if !entry.is_dir {
                let path = entry.path.clone();
                self.open_path_preview(&path, false, window, cx);
            }
        }
        cx.notify();
    }

    fn sidebar_up(&mut self, _: &SidebarUp, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_move(-1, window, cx);
    }

    fn sidebar_down(&mut self, _: &SidebarDown, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_move(1, window, cx);
    }

    fn sidebar_expand(&mut self, _: &SidebarExpand, window: &mut Window, cx: &mut Context<Self>) {
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
            self.sidebar_move(1, window, cx); // step into
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
                                .child(SharedString::from(crate::platform::shortcut_glyphs(
                                    keys,
                                ))),
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
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(t.fg_muted)
                                    .child("…or drop a folder here"),
                            )
                            .children((!self.startup_recents.is_empty()).then(|| {
                                div()
                                    .w_full()
                                    .mt_2()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(t.fg_muted)
                                            .child("RECENT"),
                                    )
                                    .children(
                                        self.startup_recents
                                            .iter()
                                            .filter(|p| p.is_dir())
                                            .take(5)
                                            .cloned()
                                            .enumerate()
                                            .map(|(i, path)| {
                                                let name = path
                                                    .file_name()
                                                    .map(|n| n.to_string_lossy().into_owned())
                                                    .unwrap_or_else(|| path.display().to_string());
                                                let parent = path
                                                    .parent()
                                                    .map(|p| p.to_string_lossy().into_owned())
                                                    .unwrap_or_default();
                                                div()
                                                    .id(SharedString::from(format!("recent-{i}")))
                                                    .w_full()
                                                    .px_2()
                                                    .py(px(4.))
                                                    .rounded_md()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(t.hover_bg))
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .text_size(px(t.ui_size))
                                                            .text_color(t.fg)
                                                            .overflow_hidden()
                                                            .child(SharedString::from(name)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(10.))
                                                            .text_color(t.fg_muted)
                                                            .overflow_hidden()
                                                            .truncate()
                                                            .child(SharedString::from(parent)),
                                                    )
                                                    .on_click(cx.listener(
                                                        move |this, _: &ClickEvent, window, cx| {
                                                            let path = path.clone();
                                                            this.open_path(&path, window, cx);
                                                        },
                                                    ))
                                            }),
                                    )
                            })),
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
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.sidebar_selected = row_ix;
                    if is_dir {
                        if let Some(tree) = this.tree.as_mut() {
                            tree.toggle(&path);
                        }
                        window.focus(&this.sidebar_focus);
                        cx.notify();
                    } else if event.click_count() >= 2 {
                        this.open_path(&path, window, cx);
                    } else {
                        this.open_path_preview(&path, true, window, cx);
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
    /// Min/Max/Close for platforms without native overlay controls
    /// (Windows always; Linux when the compositor grants CSD).
    fn render_window_controls(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if crate::platform::MACOS {
            return None;
        }
        if matches!(window.window_decorations(), gpui::Decorations::Server { .. }) {
            return None;
        }
        let t = theme(cx);
        let btn = |id: &'static str,
                   glyph: &'static str,
                   area: gpui::WindowControlArea,
                   danger: bool| {
            div()
                .id(id)
                .w(px(44.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .window_control_area(area)
                .text_size(px(13.))
                .text_color(t.fg_muted)
                .when(!danger, |d| d.hover(|s| s.bg(t.hover_bg)))
                .when(danger, |d| d.hover(|s| s.bg(t.diff_deleted_bg)))
                .child(glyph)
        };
        Some(
            div()
                .h_full()
                .flex_none()
                .flex()
                .flex_row()
                .child(
                    btn("win-min", "–", gpui::WindowControlArea::Min, false).on_mouse_down(
                        gpui::MouseButton::Left,
                        |_, window, _| window.minimize_window(),
                    ),
                )
                .child(
                    btn("win-max", "□", gpui::WindowControlArea::Max, false).on_mouse_down(
                        gpui::MouseButton::Left,
                        |_, window, _| window.zoom_window(),
                    ),
                )
                .child(
                    btn("win-close", "✕", gpui::WindowControlArea::Close, true).on_mouse_down(
                        gpui::MouseButton::Left,
                        |_, window, _| window.remove_window(),
                    ),
                )
                .into_any_element(),
        )
    }

    fn render_titlebar(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let active = self.active;
        let show_tabs = !self.tabs.is_empty() && !self.focus_mode;

        let preview_tab = self.preview_tab;
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let title = tab.title(cx);
            let is_preview = matches!(tab, Tab::Editor { view: EditorView::Preview(_), .. });
            let is_transient = preview_tab == Some(ix);
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
                        .when(is_transient, |d| d.italic())
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
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    // Double-clicking a preview tab pins it.
                    if event.click_count() >= 2 && this.preview_tab == Some(ix) {
                        this.preview_tab = None;
                    }
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
            .when(crate::platform::MACOS && !self.show_sidebar, |d| {
                // With the sidebar hidden the traffic lights sit over
                // the tab bar — inset past them; the sidebar's own drag
                // strip covers them otherwise. (macOS only.)
                d.child(
                    div()
                        .w(px(76.))
                        .h_full()
                        .flex_none()
                        .window_control_area(gpui::WindowControlArea::Drag),
                )
            })
            .when(!crate::platform::MACOS, |d| {
                // No global menu bar off macOS — the ☰ popover carries
                // the same actions.
                d.child(
                    div()
                        .id("app-menu-btn")
                        .w(px(40.))
                        .h_full()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .text_size(px(14.))
                        .text_color(t.fg_muted)
                        .hover(|s| s.bg(t.hover_bg))
                        .child("☰")
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.app_menu_open = !this.app_menu_open;
                            cx.notify();
                        })),
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
            .when_some(self.update_available.clone(), |d, tag| {
                d.child(
                    div()
                        .h_full()
                        .flex_none()
                        .flex()
                        .items_center()
                        .pr_2()
                        .child(
                            div()
                                .id("update-pill")
                                .px_2()
                                .py(px(2.))
                                .rounded_full()
                                .bg(t.hover_bg)
                                .cursor_pointer()
                                .hover(|s| s.bg(t.selected_bg))
                                .text_size(px(11.))
                                .text_color(t.accent)
                                .child(SharedString::from(format!("{tag} available")))
                                .on_click(|_: &ClickEvent, _, cx| {
                                    cx.open_url(crate::update::RELEASES_URL);
                                }),
                        ),
                )
            })
            .children(self.render_window_controls(window, cx))
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
        // Editing a previewed buffer pins its tab.
        if let Some(ix) = self.preview_tab {
            if let Some(Tab::Editor { editor, .. }) = self.tabs.get(ix) {
                if editor.read(cx).save.is_dirty() {
                    self.preview_tab = None;
                }
            }
        }
        let t = theme(cx);
        let sidebar = self.render_sidebar(cx);
        let titlebar = self.render_titlebar(window, cx);
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
            .on_action(cx.listener(|this, _: &OpenRecent0, w, cx| this.open_recent_ix(0, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent1, w, cx| this.open_recent_ix(1, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent2, w, cx| this.open_recent_ix(2, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent3, w, cx| this.open_recent_ix(3, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent4, w, cx| this.open_recent_ix(4, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent5, w, cx| this.open_recent_ix(5, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent6, w, cx| this.open_recent_ix(6, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent7, w, cx| this.open_recent_ix(7, w, cx)))
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
            .on_drop(cx.listener(
                |this, paths: &gpui::ExternalPaths, window, cx| {
                    this.open_external_paths(paths.paths().to_vec(), window, cx);
                },
            ))
            .drag_over::<gpui::ExternalPaths>(|style, _, _, cx| {
                let t = theme(cx);
                style.border_2().border_color(t.accent)
            })
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
                    .children(self.install_banner.clone().map(|message| {
                        div()
                            .w_full()
                            .flex_none()
                            .px_3()
                            .py(px(6.))
                            .bg(t.panel_bg)
                            .border_b_1()
                            .border_color(t.border)
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .text_size(px(12.))
                            .child(div().flex_1().text_color(t.fg).child(message))
                            .child(
                                div()
                                    .id("install-move")
                                    .px_2()
                                    .py(px(3.))
                                    .rounded_md()
                                    .bg(t.accent)
                                    .text_color(t.bg)
                                    .cursor_pointer()
                                    .child("Move to Applications")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        let result = std::env::current_exe()
                                            .ok()
                                            .and_then(|exe| crate::install::bundle_path(&exe))
                                            .ok_or_else(|| "not running from an app bundle".to_string())
                                            .and_then(|b| crate::install::move_to_applications(&b));
                                        match result {
                                            Ok(()) => cx.quit(),
                                            Err(e) => {
                                                this.install_banner =
                                                    Some(format!("Couldn't move: {e}").into());
                                                cx.notify();
                                            }
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .id("install-later")
                                    .px_2()
                                    .py(px(3.))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_color(t.fg_muted)
                                    .hover(|s| s.bg(t.hover_bg))
                                    .child("Not now")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        this.install_banner = None;
                                        cx.notify();
                                    })),
                            )
                    }))
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
            .when(self.app_menu_open && !crate::platform::MACOS, |root| {
                let t = theme(cx);
                let item = |id: &'static str,
                            label: &'static str,
                            shortcut: &'static str,
                            cx: &mut Context<Self>,
                            action: fn(&mut Self, &mut Window, &mut Context<Self>)| {
                    div()
                        .id(id)
                        .w_full()
                        .px_3()
                        .py(px(5.))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover_bg))
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(t.ui_size))
                                .text_color(t.fg)
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(t.fg_muted)
                                .child(SharedString::from(crate::platform::shortcut_glyphs(
                                    shortcut,
                                ))),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.app_menu_open = false;
                            action(this, window, cx);
                        }))
                };
                let recents: Vec<AnyElement> = self
                    .startup_recents
                    .iter()
                    .filter(|p| p.is_dir())
                    .take(5)
                    .cloned()
                    .enumerate()
                    .map(|(i, path)| {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        div()
                            .id(SharedString::from(format!("menu-recent-{i}")))
                            .w_full()
                            .px_3()
                            .py(px(5.))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover_bg))
                            .text_size(px(t.ui_size))
                            .text_color(t.fg)
                            .child(SharedString::from(name))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.app_menu_open = false;
                                let path = path.clone();
                                this.open_path(&path, window, cx);
                            }))
                            .into_any_element()
                    })
                    .collect();
                let divider = || div().h(px(1.)).w_full().my_1().bg(t.border);
                root.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _w, cx| {
                                this.app_menu_open = false;
                                cx.notify();
                            }),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(40.))
                                .left(px(8.))
                                .w(px(280.))
                                .bg(t.panel_bg)
                                .border_1()
                                .border_color(t.border)
                                .rounded_lg()
                                .shadow_lg()
                                .p_1()
                                .flex()
                                .flex_col()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .child(item("m-new", "New File", "⌘ N", cx, |t, w, c| {
                                    t.new_file(&NewFile, w, c)
                                }))
                                .child(item("m-open", "Open…", "⌘ O", cx, |t, w, c| {
                                    t.open_dialog(&OpenDialog, w, c)
                                }))
                                .children((!recents.is_empty()).then(divider))
                                .children(recents)
                                .child(divider())
                                .child(item("m-find", "Go to File", "⌘ P", cx, |t, w, c| {
                                    t.toggle_finder(&ToggleFinder, w, c)
                                }))
                                .child(item(
                                    "m-search",
                                    "Search in Workspace",
                                    "⌘ ⇧ F",
                                    cx,
                                    |t, w, c| t.toggle_search(&ToggleSearch, w, c),
                                ))
                                .child(item(
                                    "m-diff",
                                    "Show Changes",
                                    "⌘ ⇧ D",
                                    cx,
                                    |t, w, c| t.show_changes(&ShowChanges, w, c),
                                ))
                                .child(item(
                                    "m-preview",
                                    "Toggle Preview",
                                    "⌘ E",
                                    cx,
                                    |t, w, c| t.toggle_preview(&TogglePreview, w, c),
                                ))
                                .child(divider())
                                .child(item("m-theme", "Theme…", "⌘ T", cx, |t, w, c| {
                                    t.toggle_theme_picker(&ToggleThemePicker, w, c)
                                }))
                                .child(item(
                                    "m-shortcuts",
                                    "Shortcuts",
                                    "⌘ /",
                                    cx,
                                    |t, w, c| t.toggle_shortcuts(&ToggleShortcuts, w, c),
                                )),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_file_written_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let p = ensure_welcome_file(dir.path());
        assert!(p.ends_with("Welcome.md"));
        let first = std::fs::read_to_string(&p).unwrap();
        assert!(first.contains("Welcome to SuperMD"));
        std::fs::write(&p, "user edited").unwrap();
        ensure_welcome_file(dir.path());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "user edited");
    }

    #[test]
    fn preview_plan_rules() {
        use PreviewPlan::*;
        assert_eq!(preview_plan(None, Some(2)), ActivateExisting(2));
        assert_eq!(preview_plan(Some(1), Some(1)), ActivateExisting(1));
        assert_eq!(preview_plan(Some(1), Some(3)), ActivateExisting(3));
        assert_eq!(preview_plan(Some(1), None), ReplacePreview(1));
        assert_eq!(preview_plan(None, None), PushNew);
    }

    // ── shell interaction tests (headless gpui test platform) ──────────

    use gpui::TestAppContext;
    use std::sync::{Arc, Mutex, MutexGuard};

    /// Workspace construction reads and writes settings under the HOME
    /// env var; point it at a tempdir (serialized — env is process-wide).
    static HOME_LOCK: Mutex<()> = Mutex::new(());

    struct TempHome {
        _dir: tempfile::TempDir,
        prev: Option<std::ffi::OsString>,
        _guard: MutexGuard<'static, ()>,
    }

    fn temp_home() -> TempHome {
        let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());
        TempHome { _dir: dir, prev, _guard: guard }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    fn workspace_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a.md");
        let b = root.path().join("b.md");
        std::fs::write(&a, "# a\n").unwrap();
        std::fs::write(&b, "# b\n").unwrap();
        (root, a, b)
    }

    fn open_workspace<'a>(
        cx: &'a mut TestAppContext,
        root: &Path,
    ) -> (Entity<Workspace>, &'a mut gpui::VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(
                crate::theme::Theme::dark(),
            )));
            cx.set_global(crate::highlight::SyntaxLanguages(Arc::new(
                crate::highlight::Languages::new(),
            )));
            // Editor flushes go through the session-backup registry;
            // root it under the temp HOME (never the real ~/.supermd).
            cx.set_global(crate::editor::SessionBackups(Arc::new(Mutex::new(
                crate::editor::autosave::BackupRegistry::new(
                    crate::settings::config_dir().join("backups"),
                ),
            ))));
            cx.set_global(crate::theme::ThemeState {
                themes: crate::theme::builtin_themes(),
                settings: crate::settings::Settings::default(),
                system_dark: false,
            });
        });
        let root = root.to_path_buf();
        cx.add_window_view(|_, cx| Workspace::new(Some(root), cx))
    }

    fn tab_paths(ws: &Workspace, cx: &App) -> Vec<Option<PathBuf>> {
        ws.tabs.iter().map(|t| t.path(cx)).collect()
    }

    #[gpui::test]
    fn browsing_reuses_the_single_preview_slot(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, false, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(tab_paths(w, app), vec![Some(a.clone())]);
            assert_eq!(w.preview_tab, Some(0));
            assert_eq!(w.active, 0);
        });

        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&b, false, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(tab_paths(w, app), vec![Some(b.clone())], "slot replaced, not pushed");
            assert_eq!(w.preview_tab, Some(0));
        });
    }

    #[gpui::test]
    fn deliberate_open_pins_the_preview_tab(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, false, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.preview_tab, None, "deliberate open pins");
            assert_eq!(tab_paths(w, app), vec![Some(a.clone())]);
        });
    }

    #[gpui::test]
    fn preview_activates_existing_pinned_tab_instead_of_replacing(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx)); // pinned tab 0
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&b, false, window, cx)); // preview tab 1
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(tab_paths(w, app), vec![Some(a.clone()), Some(b.clone())]);
            assert_eq!(w.preview_tab, Some(1));
            assert_eq!(w.active, 1);
        });

        // Browsing back to the pinned file activates it; the preview
        // slot (and its tab) survives untouched.
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, false, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(tab_paths(w, app).len(), 2);
            assert_eq!(w.active, 0);
            assert_eq!(w.preview_tab, Some(1));
        });
    }

    #[gpui::test]
    fn closing_a_tab_shifts_the_preview_index(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&b, false, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.close_tab_at(0, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(tab_paths(w, app), vec![Some(b.clone())]);
            assert_eq!(w.preview_tab, Some(0), "preview index shifted down");
            assert_eq!(w.active, 0);
        });

        ws.update_in(cx, |ws, window, cx| ws.close_tab_at(0, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.tabs.is_empty());
            assert_eq!(w.preview_tab, None, "closing the preview clears the slot");
        });
    }

    #[gpui::test]
    fn opening_a_directory_switches_root_and_records_recents(cx: &mut TestAppContext) {
        let home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        let other = tempfile::tempdir().unwrap();
        let other_root = other.path().canonicalize().unwrap();
        ws.update_in(cx, |ws, window, cx| ws.open_path(&other_root, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.tree.as_ref().map(|t| t.root.clone()), Some(other_root.clone()));
        });
        let settings =
            std::fs::read_to_string(home._dir.path().join(".supermd/settings.toml")).unwrap();
        assert!(
            settings.contains(other_root.to_str().unwrap()),
            "recents recorded under temp HOME: {settings}"
        );
    }

    // ── additional interaction-test helpers ────────────────────────────

    /// Author git fixtures with the system git CLI (same pattern as the
    /// src/git.rs tests).
    fn sh_git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn active_editor(ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> Entity<Editor> {
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { editor, .. }) = w.tabs.get(w.active) else { panic!("active tab is not an editor") };
            editor.clone()
        })
    }

    // ── sidebar keyboard browsing ───────────────────────────────────────

    #[gpui::test]
    fn sidebar_keyboard_browsing_previews_and_enter_pins(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.focus_sidebar(&FocusSidebar, window, cx));
        cx.update(|window, app| {
            let w = ws.read(app);
            assert!(w.show_sidebar);
            assert!(w.sidebar_focus.is_focused(window));
        });

        // Rows are [a.md, b.md]; ↓ from row 0 lands on b and previews it.
        ws.update_in(cx, |ws, window, cx| ws.sidebar_down(&SidebarDown, window, cx));
        cx.update(|window, app| {
            let w = ws.read(app);
            assert_eq!(w.sidebar_selected, 1);
            assert_eq!(tab_paths(w, app), vec![Some(b.clone())]);
            assert_eq!(w.preview_tab, Some(0));
            assert!(
                w.sidebar_focus.is_focused(window),
                "preview-on-navigate keeps focus in the sidebar"
            );
        });

        // ↑ back to a: the single preview slot is reused, not pushed.
        ws.update_in(cx, |ws, window, cx| ws.sidebar_up(&SidebarUp, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.sidebar_selected, 0);
            assert_eq!(tab_paths(w, app), vec![Some(a.clone())]);
            assert_eq!(w.preview_tab, Some(0));
        });

        // ⏎ pins the previewed file and moves focus to the document.
        ws.update_in(cx, |ws, window, cx| ws.sidebar_open(&SidebarOpen, window, cx));
        cx.update(|window, app| {
            let w = ws.read(app);
            assert_eq!(w.preview_tab, None, "enter pins the preview tab");
            assert_eq!(tab_paths(w, app), vec![Some(a.clone())]);
            assert!(!w.sidebar_focus.is_focused(window), "enter focuses the document");
        });
    }

    #[gpui::test]
    fn sidebar_expand_collapse_and_step_into(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let sub = root.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let c = sub.join("c.md");
        std::fs::write(&c, "# c\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.focus_sidebar(&FocusSidebar, window, cx));
        // Directories sort first, so row 0 is `sub`; → expands it.
        ws.update_in(cx, |ws, window, cx| ws.sidebar_expand(&SidebarExpand, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.tree.as_ref().is_some_and(|t| t.is_expanded(&sub)));
        });
        // → on an already-expanded folder steps into its first child,
        // previewing it.
        ws.update_in(cx, |ws, window, cx| ws.sidebar_expand(&SidebarExpand, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.sidebar_selected, 1);
            assert_eq!(tab_paths(w, app), vec![Some(c.clone())]);
        });
        // ← on a file jumps to its parent directory row…
        ws.update_in(cx, |ws, window, cx| ws.sidebar_collapse(&SidebarCollapse, window, cx));
        cx.update(|_, app| assert_eq!(ws.read(app).sidebar_selected, 0));
        // …and on the expanded directory folds it shut.
        ws.update_in(cx, |ws, window, cx| ws.sidebar_collapse(&SidebarCollapse, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.tree.as_ref().is_some_and(|t| !t.is_expanded(&sub)));
        });
    }

    // ── panel toggles and focus mode ────────────────────────────────────

    #[gpui::test]
    fn focus_mode_restores_previous_panel_state(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.toggle_outline(&ToggleOutline, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.show_sidebar);
            assert!(!w.show_outline);
        });

        ws.update_in(cx, |ws, window, cx| ws.toggle_focus_mode(&ToggleFocusMode, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.focus_mode);
            assert!(!w.show_sidebar);
            assert!(!w.show_outline);
        });

        ws.update_in(cx, |ws, window, cx| ws.toggle_focus_mode(&ToggleFocusMode, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(!w.focus_mode);
            assert!(w.show_sidebar, "leaving focus mode restores the sidebar");
            assert!(!w.show_outline, "outline stays hidden as it was before");
        });

        ws.update_in(cx, |ws, window, cx| ws.toggle_sidebar(&ToggleSidebar, window, cx));
        cx.update(|_, app| assert!(!ws.read(app).show_sidebar));
        ws.update_in(cx, |ws, window, cx| ws.toggle_sidebar(&ToggleSidebar, window, cx));
        cx.update(|_, app| assert!(ws.read(app).show_sidebar));
    }

    // ── tab cycling and closing via dispatched actions ─────────────────

    #[gpui::test]
    fn tab_actions_cycle_and_close(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        // With no tabs, cycling is a no-op rather than a panic.
        cx.update(|window, app| window.focus(&ws.read(app).focus_handle));
        cx.run_until_parked();
        cx.dispatch_action(NextTab);
        cx.dispatch_action(PrevTab);
        cx.update(|_, app| assert!(ws.read(app).tabs.is_empty()));

        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path(&b, window, cx));
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(ws.read(app).active, 1));

        // Actions dispatched at the focused editor bubble to the workspace.
        cx.dispatch_action(NextTab);
        cx.update(|_, app| assert_eq!(ws.read(app).active, 0, "next wraps around"));
        cx.dispatch_action(PrevTab);
        cx.update(|_, app| assert_eq!(ws.read(app).active, 1));

        cx.dispatch_action(CloseTab);
        cx.update(|window, app| {
            let w = ws.read(app);
            assert_eq!(tab_paths(w, app), vec![Some(a.clone())]);
            assert_eq!(w.active, 0);
            let Some(Tab::Editor { editor, .. }) = w.tabs.first() else { panic!("expected editor tab") };
            assert!(
                editor.focus_handle(app).is_focused(window),
                "surviving tab regains focus"
            );
        });
    }

    // ── finder and search overlay integration ───────────────────────────

    #[gpui::test]
    fn finder_events_route_open_and_dismiss(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.toggle_finder(&ToggleFinder, window, cx));
        let finder = cx.update(|window, app| {
            let w = ws.read(app);
            let Some((finder, _)) = &w.finder else { panic!("finder should be open") };
            assert!(finder.focus_handle(app).is_focused(window));
            finder.clone()
        });

        cx.update(|_, app| {
            finder.update(app, |_, cx| cx.emit(FinderEvent::OpenPath(b.clone())))
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.finder.is_none(), "opening a hit dismisses the finder");
            assert_eq!(tab_paths(w, app), vec![Some(b.clone())]);
            assert_eq!(w.preview_tab, None, "finder opens are deliberate (pinned)");
        });

        ws.update_in(cx, |ws, window, cx| ws.toggle_finder(&ToggleFinder, window, cx));
        let finder = cx.update(|_, app| {
            let Some((finder, _)) = &ws.read(app).finder else { panic!("finder should be open") };
            finder.clone()
        });
        cx.update(|_, app| finder.update(app, |_, cx| cx.emit(FinderEvent::Dismissed)));
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app).finder.is_none()));

        // ⌘P again while open: the second toggle dismisses.
        ws.update_in(cx, |ws, window, cx| ws.toggle_finder(&ToggleFinder, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.toggle_finder(&ToggleFinder, window, cx));
        cx.update(|_, app| assert!(ws.read(app).finder.is_none()));
    }

    #[gpui::test]
    fn search_events_open_the_hit_and_dismiss_the_overlay(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, b) = workspace_fixture();
        std::fs::write(&b, "one\ntwo\nthree\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.toggle_search(&ToggleSearch, window, cx));
        let overlay = cx.update(|window, app| {
            let w = ws.read(app);
            let Some((overlay, _)) = &w.search else { panic!("search should be open") };
            assert!(overlay.focus_handle(app).is_focused(window));
            overlay.clone()
        });

        cx.update(|_, app| {
            overlay.update(app, |_, cx| {
                cx.emit(crate::search_ui::SearchEvent::Open { path: b.clone(), line: 3 })
            })
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.search.is_none(), "opening a hit dismisses the overlay");
            assert_eq!(w.tabs.get(w.active).and_then(|t| t.path(app)), Some(b.clone()));
            assert_eq!(w.preview_tab, None);
        });

        ws.update_in(cx, |ws, window, cx| ws.toggle_search(&ToggleSearch, window, cx));
        let overlay = cx.update(|_, app| {
            let Some((overlay, _)) = &ws.read(app).search else { panic!("search should be open") };
            overlay.clone()
        });
        cx.update(|_, app| {
            overlay.update(app, |_, cx| cx.emit(crate::search_ui::SearchEvent::Dismissed))
        });
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app).search.is_none()));
    }

    // ── theme picker ────────────────────────────────────────────────────

    #[gpui::test]
    fn theme_picker_arrows_preview_live_and_cancel_restores(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        let initial = cx.update(|_, app| theme(app));

        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_theme_picker(&ToggleThemePicker, window, cx)
        });
        cx.update(|window, app| {
            let w = ws.read(app);
            let Some(picker) = &w.theme_picker else { panic!("picker should be open") };
            assert_eq!(picker.pos, 0);
            assert!(w.theme_picker_focus.is_focused(window));
        });

        ws.update_in(cx, |ws, window, cx| ws.theme_picker_down(&ThemePickerDown, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(picker) = &w.theme_picker else { panic!("picker should be open") };
            assert_eq!(picker.pos, 1);
            let expected = app.global::<crate::theme::ThemeState>().themes[picker.order[1]]
                .theme
                .clone();
            assert!(Arc::ptr_eq(&theme(app), &expected), "arrows preview the theme live");
        });

        ws.update_in(cx, |ws, window, cx| ws.theme_picker_up(&ThemePickerUp, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(picker) = &w.theme_picker else { panic!("picker should be open") };
            assert_eq!(picker.pos, 0);
        });

        ws.update_in(cx, |ws, window, cx| {
            ws.theme_picker_cancel(&ThemePickerCancel, window, cx)
        });
        cx.update(|_, app| {
            assert!(ws.read(app).theme_picker.is_none());
            assert!(Arc::ptr_eq(&theme(app), &initial), "escape restores the saved theme");
        });

        // Toggling while open cancels too.
        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_theme_picker(&ToggleThemePicker, window, cx)
        });
        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_theme_picker(&ToggleThemePicker, window, cx)
        });
        cx.update(|_, app| assert!(ws.read(app).theme_picker.is_none()));
    }

    #[gpui::test]
    fn theme_picker_confirm_persists_the_choice(cx: &mut TestAppContext) {
        let home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_theme_picker(&ToggleThemePicker, window, cx)
        });
        ws.update_in(cx, |ws, window, cx| ws.theme_picker_down(&ThemePickerDown, window, cx));
        let (picked_name, picked_theme) = cx.update(|_, app| {
            let w = ws.read(app);
            let Some(picker) = &w.theme_picker else { panic!("picker should be open") };
            let state = app.global::<crate::theme::ThemeState>();
            let loaded = &state.themes[picker.order[picker.pos]];
            assert!(!loaded.theme.is_dark, "second row is still a light theme");
            (loaded.name.clone(), loaded.theme.clone())
        });

        ws.update_in(cx, |ws, window, cx| {
            ws.theme_picker_confirm(&ThemePickerConfirm, window, cx)
        });
        cx.update(|_, app| {
            assert!(ws.read(app).theme_picker.is_none());
            let state = app.global::<crate::theme::ThemeState>();
            assert_eq!(state.settings.light_theme, picked_name);
            assert!(
                Arc::ptr_eq(&theme(app), &picked_theme),
                "confirm re-resolves the active theme"
            );
        });
        let settings = std::fs::read_to_string(
            home._dir.path().join(".supermd").join("settings.toml"),
        )
        .unwrap();
        assert!(settings.contains(&picked_name), "picked theme persisted: {settings}");
    }

    // ── edit/preview flip, new file ─────────────────────────────────────

    #[gpui::test]
    fn toggle_preview_flips_the_active_editor_tab(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        // With no tabs, ⌘E is a no-op.
        ws.update_in(cx, |ws, window, cx| ws.toggle_preview(&TogglePreview, window, cx));
        cx.update(|_, app| assert!(ws.read(app).tabs.is_empty()));

        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.toggle_preview(&TogglePreview, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { view, .. }) = w.tabs.get(w.active) else { panic!("expected editor tab") };
            assert!(matches!(view, EditorView::Preview(_)), "⌘E shows the rendered preview");
        });
        ws.update_in(cx, |ws, window, cx| ws.toggle_preview(&TogglePreview, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { view, .. }) = w.tabs.get(w.active) else { panic!("expected editor tab") };
            assert!(matches!(view, EditorView::Edit), "⌘E again returns to the editor");
        });
    }

    #[gpui::test]
    fn new_file_creates_untitled_files_in_the_workspace_root(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
        let first = root.path().join("Untitled.md");
        assert!(first.exists(), "new file written to disk");
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(tab_paths(w, app), vec![Some(first.clone())]);
            assert_eq!(w.active, 0);
            assert_eq!(w.preview_tab, None, "new files open pinned");
        });

        ws.update_in(cx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
        let second = root.path().join("Untitled 2.md");
        assert!(second.exists(), "second new file picks the next free name");
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(tab_paths(w, app), vec![Some(first.clone()), Some(second.clone())]);
            assert_eq!(w.active, 1);
        });
    }

    // ── external file changes ───────────────────────────────────────────

    #[gpui::test]
    fn fs_events_reload_clean_buffers_and_keep_dirty_edits(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();
        let editor = active_editor(&ws, cx);

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&a, "# changed\n").unwrap();
        // Events under ignored paths are dropped without touching tabs.
        let hidden = root.path().join(".git").join("HEAD");
        ws.update_in(cx, |ws, _, cx| ws.on_fs_events(std::slice::from_ref(&hidden), cx));
        cx.update(|_, app| {
            assert_eq!(editor.read(app).text(), "# a\n", "ignored paths do not reload")
        });

        ws.update_in(cx, |ws, _, cx| ws.on_fs_events(std::slice::from_ref(&a), cx));
        cx.update(|_, app| {
            let ed = editor.read(app);
            assert_eq!(ed.text(), "# changed\n", "clean buffers reload from disk");
            assert!(!ed.save.is_dirty());
        });

        // A dirty buffer keeps its unsaved edits when the disk changes.
        cx.simulate_input("XY");
        cx.update(|_, app| assert!(editor.read(app).save.is_dirty()));
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&a, "# external\n").unwrap();
        ws.update_in(cx, |ws, _, cx| ws.on_fs_events(std::slice::from_ref(&a), cx));
        cx.update(|_, app| {
            let text = editor.read(app).text();
            assert!(text.contains("XY"), "dirty buffers keep unsaved edits: {text}");
            assert!(!text.contains("external"));
        });

        // flush_all force-writes the dirty buffer over the conflicting
        // disk copy (which gets backed up under the temp HOME first).
        ws.update_in(cx, |ws, _, cx| ws.flush_all(cx));
        let expected = cx.update(|_, app| editor.read(app).text());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), expected);
        cx.update(|_, app| assert!(!editor.read(app).save.is_dirty()));
    }

    #[gpui::test]
    fn watcher_reloads_files_changed_on_disk(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        // The macOS fs-events backend reports canonical paths; open the
        // canonicalized root so they match the tab's path.
        let root_canon = root.path().canonicalize().unwrap();
        let a = root_canon.join("a.md");
        let (ws, cx) = open_workspace(cx, &root_canon);
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        ws.update_in(cx, |ws, _, cx| ws.setup_watcher(cx));
        cx.update(|_, app| assert!(ws.read(app)._watcher.is_some()));
        cx.run_until_parked();
        let editor = active_editor(&ws, cx);

        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&a, "# watched\n").unwrap();
        // Real fs events arrive on wall-clock time; the drain loop runs
        // on the test clock. Interleave both until the reload lands.
        let mut reloaded = false;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(250));
            cx.run_until_parked();
            if cx.update(|_, app| editor.read(app).text()) == "# watched\n" {
                reloaded = true;
                break;
            }
        }
        assert!(reloaded, "watcher-driven reload of a clean buffer");
    }

    // ── git status dots ─────────────────────────────────────────────────

    #[gpui::test]
    fn refresh_git_status_marks_uncommitted_files(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        sh_git(root.path(), &["init", "-q"]);
        sh_git(root.path(), &["add", "-A"]);
        sh_git(root.path(), &["commit", "-qm", "init"]);
        let (ws, cx) = open_workspace(cx, root.path());
        cx.update(|_, app| {
            assert!(ws.read(app).git_modified.is_empty(), "clean repo has no dots")
        });

        std::fs::write(&a, "# modified\n").unwrap();
        ws.update_in(cx, |ws, _, _| ws.refresh_git_status());
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.git_modified.contains(&a), "modified file gets a dot");
            assert!(!w.git_modified.contains(&b), "clean file stays clean");
        });
    }

    // ── update pill, image zoom, shortcuts overlay ──────────────────────

    #[gpui::test]
    fn update_available_pill_state_renders(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, _, cx| {
            ws.update_available = Some("v99.0.0".into());
            cx.notify();
        });
        // Force render passes with the pill visible; an action still
        // routes through the workspace while it is shown.
        cx.update(|window, app| window.focus(&ws.read(app).focus_handle));
        cx.run_until_parked();
        cx.dispatch_action(ToggleSidebar);
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.update_available, Some(SharedString::from("v99.0.0")));
            assert!(!w.show_sidebar, "action routed while the pill is shown");
        });
    }

    #[gpui::test]
    fn image_tabs_zoom_and_shortcut_overlay_toggles(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let pic = root.path().join("pic.png");
        std::fs::write(&pic, b"\x89PNG\r\n\x1a\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.open_path(&pic, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Image { path, zoom, .. }) = w.tabs.get(w.active) else { panic!("expected image tab") };
            assert_eq!(path, &pic);
            assert_eq!(*zoom, 1.0);
        });

        ws.update_in(cx, |ws, window, cx| ws.zoom_in(&ZoomIn, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.zoom_in(&ZoomIn, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Image { zoom, .. }) = w.tabs.get(w.active) else { panic!("expected image tab") };
            assert!((zoom - 1.5625).abs() < 1e-4, "zoom in compounds: {zoom}");
        });
        ws.update_in(cx, |ws, window, cx| ws.zoom_out(&ZoomOut, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Image { zoom, .. }) = w.tabs.get(w.active) else { panic!("expected image tab") };
            assert!((zoom - 1.25).abs() < 1e-4, "zoom out steps back: {zoom}");
        });
        ws.update_in(cx, |ws, window, cx| ws.zoom_reset(&ZoomReset, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Image { zoom, .. }) = w.tabs.get(w.active) else { panic!("expected image tab") };
            assert_eq!(*zoom, 1.0);
        });

        ws.update_in(cx, |ws, window, cx| ws.toggle_shortcuts(&ToggleShortcuts, window, cx));
        cx.update(|window, app| {
            let w = ws.read(app);
            assert!(w.show_shortcuts);
            assert!(w.shortcuts_focus.is_focused(window));
        });
        ws.update_in(cx, |ws, window, cx| ws.toggle_shortcuts(&ToggleShortcuts, window, cx));
        cx.update(|_, app| assert!(!ws.read(app).show_shortcuts));
    }
}
