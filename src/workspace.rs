//! The root view: sidebar, tab bar, document pane, and outline panel.

use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::{
    actions, div, point, px, uniform_list, AnyElement, App, ClickEvent, Entity, FocusHandle,
    Focusable, Hsla, IntoElement, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathPromptOptions, Render, SharedString, Styled, Window,
};

use crate::editor::Editor;
use crate::editor::EditorEvent;
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
        NewWindow,
        OpenFolderInNewWindow,
        OpenDialog,
        CloseTab,
        NextTab,
        PrevTab,
        ToggleSidebar,
        ToggleOutline,
        ToggleFinder,
        ToggleSearch,
        TogglePalette,
        OpenPluginsFolder,
        ImportPlugin,
        RevealSettingsFolder,
        ReloadPlugins,
        TogglePreview,
        ShowChanges,
        ToggleFocusMode,
        FocusSidebar,
        ToggleKnowledge,
        ToggleGraph,
        ToggleFlux,
        InstallPlugins,
        MakeDefaultMarkdownApp,
        GraphDismiss,
        GraphFit,
        GraphColorBy,
        GraphSearch,
        GraphOrphans,
        GraphLocal,
        GraphDepthIn,
        GraphDepthOut,
        GraphFreeze,
        GraphSpread,
        SidebarUp,
        SidebarDown,
        SidebarRename,
        SidebarDelete,
        SidebarNewFile,
        SidebarNewFolder,
        SidebarMoveTo,
        RevealInFinder,
        CopyPath,
        SidebarEditCommit,
        SidebarEditCancel,
        SidebarExpand,
        SidebarCollapse,
        SidebarOpen,
        ToggleShortcuts,
        ToggleAbout,
        CheckForUpdates,
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
        NavigateBack,
        NavigateForward,
    ]
);

/// Where the nth window of this session opens. A cascade, so a second
/// window never lands exactly on top of the one that spawned it and
/// look like nothing happened; it wraps after ten so a long session
/// cannot walk windows off the bottom of the screen.
pub fn cascade_origin(n: usize) -> (f32, f32) {
    let step = (n % 10) as f32 * 24.0;
    (100.0 + step, 60.0 + step)
}

/// How many windows this session has opened, for the cascade.
static WINDOWS_OPENED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// The window chrome every SuperMD window wears. One place, so the
/// second window is the same window as the first.
pub fn window_options() -> gpui::WindowOptions {
    let n = WINDOWS_OPENED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (x, y) = cascade_origin(n);
    gpui::WindowOptions {
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("SuperMD".into()),
            // Client-side decorations: we draw the top bar, native
            // traffic lights overlay it.
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(px(12.), px(10.))),
        }),
        window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
            origin: gpui::point(px(x), px(y)),
            size: gpui::size(px(1200.), px(800.)),
        })),
        // Linux: ask for client-side decorations; we draw our own
        // window controls when the compositor grants them.
        window_decorations: if cfg!(target_os = "linux") {
            Some(gpui::WindowDecorations::Client)
        } else {
            None
        },
        ..Default::default()
    }
}

/// Open another window on `path` (None = an empty window with the
/// welcome document). Each window builds its own `Workspace`, and so
/// its own knowledge index, tabs, graph and plugin sandbox root: two
/// vaults open at once never see each other's notes.
pub fn open_in_new_window(
    path: Option<PathBuf>,
    cx: &mut App,
) -> Option<gpui::WindowHandle<Workspace>> {
    let handle = cx
        .open_window(window_options(), move |_window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(path, cx);
                workspace.setup_watcher(cx);
                workspace
            })
        })
        .ok()?;
    handle
        .update(cx, |workspace, window, cx| {
            crate::apply_system_appearance(window.appearance(), cx);
            window
                .observe_window_appearance(|window, cx| {
                    crate::apply_system_appearance(window.appearance(), cx);
                    window.refresh();
                })
                .detach();
            window.focus(&workspace.focus_handle(cx));
            cx.activate(true);
        })
        .ok();
    Some(handle)
}

/// What to say when *Open Folder in New Window* opened fewer windows
/// than the user chose folders. `None` when everything opened.
///
/// Pure, because the call site lives behind `cx.prompt_for_paths`,
/// which the test platform does not implement -- the same reason
/// `open_folders_in_new_windows` was split out of it.
pub fn shortfall_message(opened: usize, wanted: usize) -> Option<String> {
    match opened {
        _ if opened >= wanted => None,
        0 => Some("Could not open that folder in a new window".to_string()),
        n => Some(format!("Opened {n} of {wanted} folders")),
    }
}

/// One window per chosen folder. Anything that is not a directory is
/// skipped rather than opened as a single-file window: this is the
/// *folder* picker, and a file slipping through would give the new
/// window no sidebar, no index and no sandbox root. Returns how many
/// windows opened.
pub fn open_folders_in_new_windows(paths: Vec<PathBuf>, cx: &mut App) -> usize {
    paths
        .into_iter()
        .filter(|path| path.is_dir())
        .filter_map(|path| open_in_new_window(Some(path), cx))
        .count()
}

/// Flush every dirty editor in every open window. The app-quit hook's
/// whole job, extracted so it is a function with a test rather than a
/// closure inside `main`.
///
/// Returns how many windows it flushed. With one window this used to
/// be a single captured handle; a second window's unsaved edits must
/// not be the price of ⌘Q.
pub fn flush_all_windows(cx: &mut App) -> usize {
    let mut flushed = 0;
    for handle in cx.windows() {
        let Some(handle) = handle.downcast::<Workspace>() else {
            continue;
        };
        if handle
            .update(cx, |workspace, _window, cx| workspace.flush_all(cx))
            .is_ok()
        {
            flushed += 1;
        }
    }
    flushed
}

/// Put a one-line plugin error in front of the user, in whichever
/// window is in front. Returns whether a window took it.
///
/// The background plugin surfaces have no window of their own — inline
/// rendering is drained by a process-level task — so `eprintln!` was
/// the only report they made, and a Finder- or Dock-launched app sends
/// that to Console.app where the plugin author who needs it never
/// looks.
pub fn report_plugin_error(message: String, cx: &mut App) -> bool {
    let active = cx.active_window().into_iter();
    for handle in active.chain(cx.windows()) {
        let Some(handle) = handle.downcast::<Workspace>() else {
            continue;
        };
        if handle
            .update(cx, |workspace, _window, cx| {
                workspace.show_command_error(message.clone(), cx)
            })
            .is_ok()
        {
            return true;
        }
    }
    false
}

/// Hand one drained batch of external opens to a live window: the
/// active one if it is a workspace, otherwise any surviving workspace
/// window. Returns the batch back when *no* window could take it, so
/// the caller requeues instead of dropping what it just drained.
///
/// Window-agnostic on purpose. This used to be a per-window task
/// wired only to the window `main()` opened, which was fine while that
/// was the only window there could be: once ⌘⇧N exists, closing the
/// first window would have killed Finder opens, "Open With", and every
/// `supermd://` link for the rest of the session.
pub fn deliver_external_opens(
    queued: Vec<crate::PendingOpen>,
    cx: &mut App,
) -> Option<Vec<crate::PendingOpen>> {
    let mut paths = Vec::new();
    let mut installs = Vec::new();
    for item in &queued {
        match item {
            crate::PendingOpen::Path(p) => paths.push(p.clone()),
            crate::PendingOpen::InstallPlugin(name) => installs.push(name.clone()),
        }
    }
    // The window the user is looking at first; then any other, so a
    // batch that arrives while nothing is focused still lands.
    let active = cx.active_window().into_iter();
    for handle in active.chain(cx.windows()) {
        let Some(handle) = handle.downcast::<Workspace>() else {
            continue;
        };
        let delivered = handle
            .update(cx, |workspace, window, cx| {
                if !paths.is_empty() {
                    workspace.open_external_paths(std::mem::take(&mut paths), window, cx);
                }
                for name in std::mem::take(&mut installs) {
                    workspace.request_plugin_install(name, window, cx);
                }
            })
            .is_ok();
        if delivered {
            return None;
        }
    }
    Some(queued)
}

/// Poll the shared open-event queue (fed by `on_open_urls`) and route
/// each batch into a live window. App-level, and it never exits while
/// the app is alive: windows come and go under it.
pub fn watch_external_opens(
    pending: std::sync::Arc<std::sync::Mutex<Vec<crate::PendingOpen>>>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            let queued: Vec<crate::PendingOpen> = {
                let mut guard = pending.lock().unwrap_or_else(|e| e.into_inner());
                std::mem::take(&mut *guard)
            };
            if queued.is_empty() {
                continue;
            }
            // `cx.update` failing means the app itself is gone, which
            // is the only reason to stop polling.
            let Ok(undelivered) = cx.update(|cx| deliver_external_opens(queued, cx)) else {
                break;
            };
            // No window took it (none open yet, or all closing): put it
            // back at the front rather than losing a Finder open.
            if let Some(batch) = undelivered {
                let mut guard = pending.lock().unwrap_or_else(|e| e.into_inner());
                let rest = std::mem::take(&mut *guard);
                guard.extend(batch);
                guard.extend(rest);
            }
        }
    })
    .detach();
}

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

/// Why Show Changes found no baseline. Under the sandbox, `gix::discover`
/// walks upward out of the granted scope and fails silently; say so
/// rather than implying the folder has no history.
pub fn git_scope_hint(has_baseline: bool, repo_root_above_workspace: bool) -> Option<&'static str> {
    (!has_baseline && repo_root_above_workspace)
        .then_some("the git repository is outside the opened folder")
}

/// Whether "no baseline" here might mean "out of scope" rather than "no
/// repository". Only a sandboxed build can be fooled, and only for a
/// folder that is not itself a repo root — `gix::discover` stops at a
/// `.git` inside the grant, so finding one settles the question.
pub fn repo_root_may_be_out_of_scope(workspace_root: &Path) -> bool {
    crate::bookmarks::needs_scope() && !workspace_root.join(".git").exists()
}

/// Persist a just-opened workspace root into the recents list. Returns
/// a message when the on-disk settings could not be read: this
/// function immediately saves over whatever it loaded, so a caller
/// that can reach the user must surface the corruption here or lose
/// the chance -- by the next line the preserved copy is the only
/// record that anything was lost.
/// Persist a single setting the user just changed, without clobbering
/// whatever else has reached disk since launch.
///
/// `ThemeState.settings` is seeded once at startup and never refreshed,
/// while recents, workspace bookmarks, plugin grants and net-domain
/// grants are all read-modify-written straight against the file as the
/// session runs. Saving the global back whole reverts every one of
/// them -- and under the sandbox, dropping `recent_workspaces` drops
/// the security-scoped bookmarks keyed off it, costing the user access
/// to every folder opened this session.
///
/// So `edit` runs twice: once on the global, for the in-session readers
/// of the field, and once on a copy loaded fresh from disk, which is
/// the copy that gets saved. It must therefore set its field to an
/// absolute value -- never flip one, or the two copies diverge.
fn persist_setting(cx: &mut App, edit: impl Fn(&mut crate::settings::Settings)) {
    edit(&mut cx.global_mut::<crate::theme::ThemeState>().settings);
    let dir = crate::settings::config_dir();
    let mut on_disk = crate::settings::load(&dir);
    edit(&mut on_disk);
    if let Err(err) = crate::settings::save(&dir, &on_disk) {
        eprintln!("supermd: cannot save settings: {err}");
    }
}

fn record_recent(root: &Path) -> Option<String> {
    let dir = crate::settings::config_dir();
    let (mut settings, corrupt) = crate::settings::load_reporting(&dir);
    let blob = crate::bookmarks::create(root);
    settings.note_workspace(root, blob);
    if let Err(err) = crate::settings::save(&dir, &settings) {
        eprintln!("supermd: cannot save settings: {err}");
    }
    corrupt
}

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

/// The third Markdown file opened this session is the trigger for the
/// "make SuperMD the default" offer -- not the first (an app demanding
/// to be the default before it has been used is the behaviour people
/// resent), and not every one after the third (a prompt that keeps
/// reappearing before the user has even answered it is worse than one
/// that waits). The caller only calls this once `default_handler_asked`
/// is already known false; a fourth or later open is a no-op that
/// leaves an already-open banner alone rather than reopening it.
pub(crate) fn should_offer_default_handler(markdown_opens_this_session: u32) -> bool {
    markdown_opens_this_session == 3
}

/// Seti's 12 palette variables mapped onto our theme so icons read well
/// in both appearances.
/// The colour a sidebar row's name draws in. Ignored files -- the ones
/// a `.gitignore` keeps out of the index -- are listed but recede:
/// present when you need them, never competing with the notes, and the
/// only affordance saying "this file is not in your graph".
pub(crate) fn sidebar_row_color(ignored: bool, is_dir: bool, t: &Theme) -> gpui::Hsla {
    if ignored {
        t.fg_muted
    } else if is_dir {
        t.fg_strong
    } else {
        t.fg
    }
}

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
    /// This window's knowledge index. Per-workspace, never a process
    /// global: two windows on two folders must not see each other's
    /// notes in backlinks, completion or the graph.
    pub knowledge: crate::knowledge::KnowledgeHandle,
    /// This window's plugin host. One host per workspace, because the
    /// host carries the `workspace-read` preopen root: a shared host
    /// would let a plugin invoked from this window read another
    /// window's folder.
    pub host: crate::extensions::HostHandle,
    tabs: Vec<Tab>,
    active: usize,
    show_sidebar: bool,
    show_outline: bool,
    /// Right-hand knowledge panel (backlinks, tags).
    show_knowledge: bool,
    focus_mode: bool,
    pre_focus_panels: (bool, bool),
    finder: Option<(Entity<Finder>, gpui::Subscription)>,
    search: Option<(Entity<crate::search_ui::SearchOverlay>, gpui::Subscription)>,
    palette: Option<(Entity<crate::palette::Palette>, gpui::Subscription)>,
    install_overlay: Option<(Entity<crate::install_ui::InstallOverlay>, gpui::Subscription)>,
    /// Transient plugin-command error, auto-cleared.
    command_error: Option<SharedString>,
    /// (plugin, capability) awaiting a consent decision; capability is
    /// "workspace-read" or "net:<domain>".
    consent_request: Option<(String, String)>,
    /// A `supermd://` handoff resolved against the catalog, awaiting the
    /// user's yes or no.
    install_request: Option<crate::catalog::CatalogEntry>,
    /// Index of the transient preview tab (at most one; italic title).
    preview_tab: Option<usize>,
    /// Newer released version tag, when the launch check found one.
    update_available: Option<SharedString>,
    /// Recents snapshot taken at launch (existing dirs only); the
    /// Open Recent menu indexes into this.
    startup_recents: Vec<PathBuf>,
    /// Move-to-Applications offer (Some = banner visible with message).
    install_banner: Option<SharedString>,
    /// Markdown files opened so far this session. Counted only to
    /// decide when to show the "make SuperMD the default" offer --
    /// never on first launch, an app demanding to be the default before
    /// it has been used is the behaviour people resent.
    markdown_opens: u32,
    /// "Make SuperMD the default Markdown app" offer banner.
    show_default_handler_offer: bool,
    /// Result of the most recent `MakeDefaultMarkdownApp` command,
    /// shown once then cleared -- covers both the banner's Yes and the
    /// Tools-menu command reached after an earlier no.
    ///
    /// This deliberately does not reuse `command_error`/
    /// `show_command_error`: that mechanism auto-dismisses after 4
    /// seconds, which is fine for a short one-line status but would
    /// cut off the sandboxed-refusal message before a user finishes
    /// reading the multi-sentence Finder "Open with -> Change All…"
    /// instructions in `describe_default_handler_status`. This banner
    /// instead stays up until the user dismisses it.
    default_handler_result: Option<SharedString>,
    /// ☰ popover on platforms without a global menu bar.
    app_menu_open: bool,
    focus_handle: FocusHandle,
    sidebar_focus: FocusHandle,
    sidebar_selected: usize,
    shortcuts_focus: FocusHandle,
    about_focus: FocusHandle,
    show_shortcuts: bool,
    show_about: bool,
    /// Latest release tag once a manual About check has answered, and
    /// whether that check is still in flight.
    about_latest: Option<SharedString>,
    about_checking: bool,
    theme_picker: Option<ThemePickerState>,
    theme_picker_focus: FocusHandle,
    last_title: String,
    /// Absolute paths of files with uncommitted git changes (sidebar dots).
    git_modified: std::collections::HashSet<PathBuf>,
    /// Followed links waiting for a window to open them in.
    pending_link_opens: Vec<PathBuf>,
    /// Full-workspace graph overlay.
    graph: Option<GraphViewState>,
    graph_focus: FocusHandle,
    /// Inline rename / create in progress in the sidebar.
    sidebar_edit: Option<SidebarEdit>,
    /// "Move to…" folder picker (a Palette over workspace folders).
    move_picker: Option<(Entity<crate::palette::Palette>, gpui::Subscription)>,
    _watcher: Option<notify::RecommendedWatcher>,
    /// Back/forward across followed links.
    history: crate::nav::History,
    /// True while a back/forward move is in flight. A move *through*
    /// history must never record a visit — every entry point that opens
    /// a file records one now, so suppression is explicit rather than a
    /// matter of which call happens to come first.
    navigating: bool,
    /// The open right-click menu: where it was raised, and for what.
    /// The target itself (which sidebar row, which tab, which graph
    /// node) is whatever selection/active state the raise already set
    /// — `sidebar_selected`, `active`, `graph.hovered` — so dispatch
    /// reads the same state the keyboard shortcuts do. That claim was
    /// once true of the sidebar and tabs only: the graph raise set
    /// `graph.hovered` and nothing but the hover highlight read it, so
    /// the node menu acted on the active tab. `graph_local` reads it
    /// now — anything added to a graph surface must too.
    context_menu: Option<(gpui::Point<gpui::Pixels>, crate::menus::Surface)>,
}

enum SidebarEditKind {
    Rename(PathBuf),
    /// Payload is the directory the new entry lands in.
    NewFile(PathBuf),
    NewDir(PathBuf),
}

struct SidebarEdit {
    kind: SidebarEditKind,
    input: Entity<crate::input::TextInput>,
    error: Option<SharedString>,
}

/// The full-workspace graph: laid-out nodes plus view transform.
struct GraphViewState {
    sim: crate::graph::Simulation,
    pan: (f32, f32),
    zoom: f32,
    /// Last mouse position while panning.
    drag: Option<(f32, f32)>,
    /// The node being dragged, and the pointer offset within it.
    node_drag: Option<usize>,
    /// The pointer actually moved while a node was held. gpui only
    /// suppresses a click past its drag threshold when a drag listener
    /// is registered, and this drag is hand-rolled — so without this,
    /// releasing after moving a node fired `on_click` and opened the
    /// note, which made dragging impossible.
    node_dragged: bool,
    /// The node under the pointer: it and its neighbours stay lit while
    /// everything else dims.
    hovered: Option<usize>,
    /// Ticks the layout while it still has motion in it.
    ticker: Option<gpui::Task<()>>,
    /// What node colour means right now.
    color_by: crate::graph::ColorBy,
    /// How spread out the layout sits.
    spread: crate::graph::Spread,
    /// What the view is narrowed to. Non-matching nodes fade rather
    /// than vanish, so the layout does not jump as you type.
    filter: crate::graph::Filter,
    /// True while the query box is taking keystrokes.
    searching: bool,
}

impl GraphViewState {
    fn nodes(&self) -> &[crate::graph::GraphNode] {
        &self.sim.nodes
    }

    fn edges(&self) -> &[crate::graph::Edge] {
        &self.sim.edges
    }

    /// Nodes one hop from `ix`, plus itself.
    fn neighbourhood(&self, ix: usize) -> std::collections::BTreeSet<usize> {
        let mut set = std::collections::BTreeSet::new();
        set.insert(ix);
        for e in self.edges() {
            if e.from == ix {
                set.insert(e.to);
            } else if e.to == ix {
                set.insert(e.from);
            }
        }
        set
    }
}

/// Create an editor and subscribe the workspace to its events
/// (consent requests raised by background enrichment).
fn make_editor(
    path: &Path,
    text: String,
    langs: &crate::highlight::Languages,
    knowledge: &crate::knowledge::KnowledgeHandle,
    host: &crate::extensions::HostHandle,
    cx: &mut Context<Workspace>,
) -> Entity<Editor> {
    let (knowledge, host) = (knowledge.clone(), host.clone());
    let editor = cx.new(|cx| {
        Editor::from_text_in(path, text, langs, Some(knowledge), Some(host), cx)
    });
    cx.subscribe(&editor, |this, _editor, event, cx| match event {
        EditorEvent::ConsentNeeded { plugin, cap } => {
            this.consent_request = Some((plugin.clone(), cap.clone()));
            cx.notify();
        }
        // No window here; render drains the queue with one in hand.
        EditorEvent::OpenPath(path) => {
            this.pending_link_opens.push(path.clone());
            cx.notify();
        }
    })
    .detach();
    editor
}

/// Build a preview Reader and listen for link clicks in it.
///
/// The reader does not know which file it is showing, so it emits the
/// destination as written and the workspace resolves it against the
/// active tab — the reader always *is* the active tab's view.
fn make_reader(
    path: Option<PathBuf>,
    title: SharedString,
    source: &str,
    langs: &crate::highlight::Languages,
    knowledge: &crate::knowledge::KnowledgeHandle,
    cx: &mut Context<Workspace>,
) -> Entity<Reader> {
    let knowledge = knowledge.clone();
    let reader = cx.new(|cx| {
        let mut reader = Reader::from_source_at(path, title, source, langs, cx);
        reader.set_knowledge(knowledge);
        reader
    });
    cx.subscribe(&reader, |this, _reader, event, cx| {
        let crate::reader::ReaderEvent::Follow(dest) = event;
        this.follow_from_reader(dest, cx);
    })
    .detach();
    reader
}

impl Workspace {
    /// A link was clicked in a rendered preview. Same rules as the
    /// editor: only http(s) leaves the app, an anchor stays put, and a
    /// path is resolved inside the workspace or ignored.
    fn follow_from_reader(&mut self, dest: &str, cx: &mut Context<Self>) {
        // `markdown.rs` marks a wiki destination with a `[[` prefix,
        // because a wiki target is a stem to resolve against the index,
        // not a path to join onto a directory. Without the distinction
        // every `[[Wiki]]` in the reading view resolved to nothing.
        let (wiki, target) = match dest.strip_prefix("[[") {
            Some(stem) => (true, stem.to_string()),
            None => (false, dest.to_string()),
        };
        let link = crate::knowledge::RawLink {
            target,
            wiki,
            range: 0..0,
            context: String::new(),
        };
        match crate::knowledge::classify(&link) {
            crate::knowledge::LinkTarget::External(url) => cx.open_url(&url),
            // An anchor inside a rendered document has nowhere to go
            // yet: the reader scrolls by block, not by byte offset.
            crate::knowledge::LinkTarget::Anchor(_) => {}
            _ => {
                let Some(base) = self.tabs.get(self.active).and_then(|t| t.path(cx)) else {
                    return;
                };
                let resolved = self.knowledge.lock().unwrap().resolve(&base, &link);
                if let Some(path) = resolved {
                    self.pending_link_opens.push(path);
                    cx.notify();
                }
            }
        }
    }

    pub fn new(arg: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut tree = None;
        let mut tabs = Vec::new();
        // Set the first time any settings::load call during startup
        // finds a file it could not use, so it can be surfaced once
        // the workspace exists to show it -- by then a load-mutate-save
        // elsewhere in this function may already have overwritten the
        // preserved copy with defaults, so this must be captured at the
        // point of loading, not re-derived afterward.
        let mut settings_corrupt: Option<String> = None;
        // Both handles belong to this workspace alone. Their
        // *contents* are replaced when the folder changes, so editors
        // and readers holding a clone never go stale.
        let knowledge = crate::knowledge::KnowledgeHandle::default();
        let host: crate::extensions::HostHandle = std::sync::Arc::new(std::sync::Mutex::new(
            crate::extensions::ExtensionHost::load(
                &crate::settings::config_dir().join("plugins"),
            ),
        ));

        match arg {
            Some(path) if path.is_dir() => {
                settings_corrupt = record_recent(&path);
                *knowledge.lock().unwrap() = crate::knowledge::Index::scan(&path);
                host.lock().unwrap().set_workspace_root(Some(path.clone()));
                tree = Some(FileTree::new(path));
            }
            Some(path) => match Editor::read_file(&path) {
                Ok(text) => {
                    let langs = languages(cx);
                    let editor = make_editor(&path, text, &langs, &knowledge, &host, cx);
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
                        let editor = make_editor(&path, text, &langs, &knowledge, &host, cx);
                        tabs.push(Tab::Editor { editor, view: EditorView::Edit });
                    }
                    Err(_) => {
                        // Unwritable config dir: fall back to read-only.
                        let langs = languages(cx);
                        tabs.push(Tab::Reader(cx.new(|cx| Reader::welcome(&langs, cx))));
                    }
                }
            }
        }

        // Whichever branch above ran, its `settings::load*` call was
        // this startup's only chance to observe a corrupt file before
        // it might get silently overwritten with defaults; the field
        // below reads the same file again after that has possibly
        // already happened. `or` keeps whichever call actually saw it.
        let (startup_settings, startup_corrupt) =
            crate::settings::load_reporting(&crate::settings::config_dir());
        settings_corrupt = settings_corrupt.or(startup_corrupt);
        host.lock().unwrap().set_grants(startup_settings.plugin_grants.clone());

        let mut workspace = Self {
            tree,
            knowledge,
            host,
            tabs,
            active: 0,
            show_sidebar: true,
            show_outline: true,
            show_knowledge: false,
            focus_mode: false,
            pre_focus_panels: (true, true),
            finder: None,
            search: None,
            palette: None,
            install_overlay: None,
            command_error: None,
            consent_request: None,
            install_request: None,
            preview_tab: None,
            update_available: None,
            startup_recents: startup_settings
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
            markdown_opens: 0,
            show_default_handler_offer: false,
            default_handler_result: None,
            focus_handle: cx.focus_handle(),
            sidebar_focus: cx.focus_handle(),
            sidebar_selected: 0,
            pending_link_opens: Vec::new(),
            graph: None,
            graph_focus: cx.focus_handle(),
            sidebar_edit: None,
            move_picker: None,
            shortcuts_focus: cx.focus_handle(),
            about_focus: cx.focus_handle(),
            show_shortcuts: false,
            show_about: false,
            about_latest: None,
            about_checking: false,
            theme_picker: None,
            theme_picker_focus: cx.focus_handle(),
            last_title: String::new(),
            git_modified: Default::default(),
            _watcher: None,
            history: crate::nav::History::default(),
            navigating: false,
            context_menu: None,
        };
        workspace.refresh_git_status();

        if let Some(msg) = settings_corrupt {
            // A console eprintln! reaches nobody in a packaged app with
            // no console -- and the very next settings save (a theme
            // change, a plugin grant) would otherwise overwrite the
            // preserved copy with defaults for good.
            workspace.show_command_error(msg, cx);
        }

        // One quiet update check per launch; failures are silent. The
        // App Store build ships no checker, so it spawns no task either.
        if crate::update::checks_enabled() {
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
        }

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
        // builds in an open workspace don't hammer the UI. This is only
        // a whole-batch shortcut for the common case of an irrelevant
        // batch; it must never stand in for the per-path check below,
        // or one visible path in a batch would wave through every
        // ignored path riding alongside it.
        let root = self.tree.as_ref().map(|tree| tree.root.clone());
        // One matcher for the batch: it caches the ignore files of each
        // directory it walks through, and a batch is a snapshot anyway.
        let mut ignores = root.as_ref().map(|root| crate::files::index_matcher(root));
        if let Some(ignores) = ignores.as_mut() {
            if !paths.iter().any(|p| ignores.allows(p)) {
                return;
            }
        }
        if let Some(tree) = &mut self.tree {
            tree.refresh();
        }
        self.refresh_git_status();
        // Keep the knowledge index warm: saves re-index, deletions drop.
        {
            let mut index = self.knowledge.lock().unwrap();
            for path in paths {
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                // Same admission rule `Index::scan` applies at its own
                // walk -- literally the same matcher: a path under
                // `.git`/`target`/hidden, or excluded by any ignore file
                // that walk honours (nested `.gitignore`s,
                // `.git/info/exclude`, `.ignore`), never enters the
                // index, however the watcher heard about it. Without
                // this, a batch that also touched one visible file would
                // index every ignored `.md` path riding alongside it.
                if ignores.as_mut().is_some_and(|ignores| !ignores.allows(path)) {
                    index.remove_file(path);
                    continue;
                }
                // Same rule as Index::scan: a symlink never enters the
                // index, however it arrived. The watcher sees paths the
                // initial scan never walked, and `read_to_string` would
                // happily follow `<root>/leak.md -> ~/.ssh/id_rsa` into
                // an in-root index entry that later resolves without an
                // escape check. Drop any entry the path may already have.
                if std::fs::symlink_metadata(path)
                    .is_ok_and(|m| m.file_type().is_symlink())
                {
                    index.remove_file(path);
                    continue;
                }
                // And the same rule again for the link kind that is not
                // a symlink: `ln ~/.ssh/id_rsa <root>/leak.md` reads the
                // outside file's bytes under an in-root path. `scan`
                // drops those after its walk; a hardlink made while the
                // workspace is open only ever reaches the index here.
                if root
                    .as_ref()
                    .is_some_and(|root| crate::knowledge::escapes_via_hardlink(root, path))
                {
                    index.remove_file(path);
                    continue;
                }
                match std::fs::read_to_string(path) {
                    Ok(text) => index.update_file(path, &text),
                    Err(_) => index.remove_file(path),
                }
            }
        }
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
            // Read-only surfaces take focus too, so the keyboard can
            // scroll them (⌘E preview, viewer tabs, the welcome tour).
            Some(Tab::Reader(reader))
            | Some(Tab::Editor { view: EditorView::Preview(reader), .. }) => {
                window.focus(&reader.focus_handle(cx))
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
        // Switching tabs is navigation: ⌘⇧[ / ⌘⇧] and a click on a tab
        // move you between documents exactly as opening one does, and a
        // history that cannot see them sends Back *forward*.
        if let Some(path) = self.tabs[ix].path(cx) {
            self.record_visit(&path);
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Record the file the workspace just landed on, unless we got here
    /// through Back or Forward — those are moves within the stack.
    fn record_visit(&mut self, path: &Path) {
        if self.navigating {
            return;
        }
        self.history.visit(path.to_path_buf());
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
            self.record_visit(path);
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
                    let (k, h) = (self.knowledge.clone(), self.host.clone());
                    let editor = make_editor(&path_buf, text, &langs, &k, &h, cx);
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
        // A single click is a look, not an edit: the tab opens read-only
        // and rendered, whatever the file is. A double click goes
        // through `open_path` and lands in Edit view. An image tab has
        // no editable view to preview.
        if matches!(self.tabs.get(self.active), Some(Tab::Editor { .. })) {
            self.show_as_preview(self.active, window, cx);
        }
        if let Some(tree) = &mut self.tree {
            tree.expand_to(path);
        }
        // Sidebar single-click lands here, not in `open_path`: this is
        // the shared entry point, so this is where history is written.
        // Only a focused open counts as a navigation. Keyboard browsing
        // (`sidebar_move`) previews every row the arrows pass over with
        // `focus: false`; recording those would push an entry per
        // keypress and, worse, truncate the forward branch, so holding
        // the arrow key through a vault would destroy Forward and fill
        // Back with files the user only glanced at.
        if focus {
            self.record_visit(path);
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    pub fn open_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if path.is_file() {
            self.record_visit(path);
        }
        if path.is_dir() {
            // A new workspace: the old stack names files in a folder
            // that is no longer open, and under the App Store sandbox
            // they are outside the active security-scoped bookmark.
            self.history.clear();
            if let Some(msg) = record_recent(path) {
                self.show_command_error(msg, cx);
            }
            self.host.lock().unwrap().set_workspace_root(Some(path.to_path_buf()));
            // Every cached plugin diagram was rendered against the
            // folder we just left. `DiagramKey` carries the root, so
            // they can never be *served* to the new one -- but they are
            // dead weight in a process-wide, capped cache, and leaving
            // them keeps a render of the previous vault's files alive
            // in memory for no reason. Reload Plugins already does
            // this; re-rooting is the same event.
            if cx.try_global::<crate::diagram::DiagramCache>().is_some() {
                cx.global_mut::<crate::diagram::DiagramCache>().clear();
            }
            *self.knowledge.lock().unwrap() = crate::knowledge::Index::scan(path);
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
            // A deliberate open pins the tab it lands on, and puts it
            // in Edit view: a single click left it read-only, and the
            // double click is the user asking to edit.
            if self.preview_tab == Some(ix) {
                self.preview_tab = None;
            }
            if let Some(Tab::Editor { view, .. }) = self.tabs.get_mut(ix) {
                *view = EditorView::Edit;
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
                let (k, h) = (self.knowledge.clone(), self.host.clone());
                let editor = make_editor(&path, text, &langs, &k, &h, cx);
                self.tabs.push(Tab::Editor { editor, view: EditorView::Edit });
                self.active = self.tabs.len() - 1;
                if let Some(viewer) = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(crate::extensions::viewer_for_extension)
                {
                    self.spawn_viewer_render(viewer, self.tabs.len() - 1, window, cx);
                }
                self.note_markdown_open(&path, cx);
                self.focus_active(window, cx);
                cx.notify();
            }
            Err(err) => eprintln!("supermd: cannot open {}: {err}", path.display()),
        }
    }

    /// Counts Markdown files opened this session, and on the third one
    /// -- never on the first, an app demanding to be the default before
    /// it has been used is the behaviour people resent -- offers to
    /// make SuperMD the default Markdown app, unless the user has
    /// already answered that offer (yes or no).
    fn note_markdown_open(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !crate::platform::MACOS {
            return;
        }
        if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md" | "markdown" | "mdown" | "mdx")
        ) {
            return;
        }
        if cx.global::<crate::theme::ThemeState>().settings.default_handler_asked {
            return;
        }
        self.markdown_opens += 1;
        if should_offer_default_handler(self.markdown_opens) {
            self.show_default_handler_offer = true;
        }
    }

    /// Persist the user's answer (yes or no) so the offer never returns
    /// -- a prompt that comes back is worse than no prompt.
    fn record_default_handler_answer(&mut self, cx: &mut Context<Self>) {
        self.show_default_handler_offer = false;
        persist_setting(cx, |s| s.default_handler_asked = true);
    }

    fn make_default_markdown_app(
        &mut self,
        _: &MakeDefaultMarkdownApp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.record_default_handler_answer(cx);
        self.default_handler_result = Some(
            match crate::platform::request_default_markdown_handler() {
                Ok(()) => "SuperMD is now the default app for Markdown files.".into(),
                Err(err) => err.into(),
            },
        );
        cx.notify();
    }

    fn navigate_back(&mut self, _: &NavigateBack, window: &mut Window, cx: &mut Context<Self>) {
        // A path that is gone (deleted, or outside the sandbox's active
        // bookmark) is stepped over rather than stepped onto: dead-
        // stepping moved the cursor and then opened nothing.
        if let Some(path) = self.history.back_matching(|p| p.exists()) {
            self.open_path_without_history(&path, window, cx);
        }
    }

    fn navigate_forward(
        &mut self,
        _: &NavigateForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.history.forward_matching(|p| p.exists()) {
            self.open_path_without_history(&path, window, cx);
        }
    }

    /// Open a file without recording it — used by back and forward, which
    /// are moves through history rather than new visits. Every opening
    /// path records a visit now, so the suppression is an explicit flag
    /// rather than an assumption about which call comes first.
    fn open_path_without_history(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigating = true;
        self.open_path_preview(path, true, window, cx);
        self.navigating = false;
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

    /// A second window on nothing in particular.
    fn new_window(&mut self, _: &NewWindow, _window: &mut Window, cx: &mut Context<Self>) {
        if open_in_new_window(None, cx).is_none() {
            // Rare (the platform refused a window), but ⌘⇧N doing
            // nothing at all with no message is the worse answer.
            self.show_command_error("Could not open a new window".to_string(), cx);
        }
    }

    /// Pick a folder and open it *beside* this window rather than
    /// replacing what is already here.
    fn open_folder_in_new_window(
        &mut self,
        _: &OpenFolderInNewWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let wanted = paths.len();
            let Ok(opened) = cx.update(|_, cx| open_folders_in_new_windows(paths, cx)) else {
                return;
            };
            // Silently doing nothing is the failure mode this whole
            // command class keeps falling into: a path that is not a
            // directory, or a platform that refused the window.
            if let Some(message) = shortfall_message(opened, wanted) {
                this.update(cx, |this, cx| this.show_command_error(message, cx)).ok();
            }
        })
        .detach();
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

    /// Render a tab's file through its viewer plugin and swap the tab
    /// to Preview when done. Failure leaves the source editor — a
    /// broken viewer never hides a file.
    fn spawn_viewer_render(
        &mut self,
        plugin: String,
        tab_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Tab::Editor { editor, .. }) = self.tabs.get(tab_ix) else {
            return;
        };
        let editor = editor.clone();
        let host = self.host.clone();
        let filename = editor.read(cx).title().to_string();
        let content = editor.read(cx).text();
        let run = cx.background_executor().spawn(async move {
            host.lock().unwrap().render_view(&plugin, &filename, &content)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = run.await;
            if let Ok(markdown) = result {
                this.update_in(cx, |this, window, cx| {
                    let langs = languages(cx);
                    let title = editor.read(cx).title();
                    let k = this.knowledge.clone();
                    let reader = make_reader(Some(editor.read(cx).path().to_path_buf()), title, &markdown, &langs, &k, cx);
                    // Only swap if that tab still shows this editor in
                    // Edit view (the user may have toggled or closed).
                    if let Some(Tab::Editor { editor: e, view }) = this.tabs.get_mut(tab_ix) {
                        if *e == editor && matches!(view, EditorView::Edit) {
                            *view = EditorView::Preview(reader);
                            // The editor's focus handle just left the
                            // tree; refocus so keybindings keep
                            // dispatching (palette, ⌘E, …).
                            if tab_ix == this.active {
                                this.focus_active(window, cx);
                            }
                            cx.notify();
                        }
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn toggle_preview(&mut self, _: &TogglePreview, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Tab::Editor { view, .. }) = self.tabs.get(self.active) else {
            return;
        };
        if matches!(view, EditorView::Preview(_)) {
            if let Some(Tab::Editor { view, .. }) = self.tabs.get_mut(self.active) {
                *view = EditorView::Edit;
            }
            self.focus_active(window, cx);
            cx.notify();
        } else {
            self.show_as_preview(self.active, window, cx);
            self.focus_active(window, cx);
        }
    }

    /// Render `tab_ix` as a read-only preview. A viewer-claimed file
    /// re-renders through its plugin; a Markdown file is parsed as
    /// Markdown; anything else is wrapped as code, because handing
    /// source to the CommonMark parser reflows it into prose.
    fn show_as_preview(&mut self, tab_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Tab::Editor { editor, .. }) = self.tabs.get(tab_ix) else {
            return;
        };
        let editor = editor.clone();
        {
            // Only flush a buffer the user actually changed. `flush`
            // runs the save hooks *before* its dirty check, and a hook
            // that rewrites the document (the `toc` plugin does) marks
            // it dirty and causes a write. Calling it for every tab a
            // single click opens meant browsing the sidebar silently
            // rewrote and saved files nobody had edited.
            if editor.read(cx).save.is_dirty() {
                editor.update(cx, |editor, cx| editor.flush(cx));
            }
            // Viewer-claimed files re-render through the plugin so
            // edits show; everything else previews its own markdown.
            let viewer = editor
                .read(cx)
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .and_then(crate::extensions::viewer_for_extension);
            if let Some(plugin) = viewer {
                self.spawn_viewer_render(plugin, tab_ix, window, cx);
            } else {
                let title = editor.read(cx).title();
                let text = editor.read(cx).text();
                let path = editor.read(cx).path().to_path_buf();
                let langs = languages(cx);
                // Only a Markdown file is parsed as Markdown. Anything
                // else is wrapped in a fence and rendered as the code it
                // is: feeding Rust source to the CommonMark parser turned
                // doc comments into paragraphs, reflowed the source, and
                // made every 4-space-indented block an indented code
                // block.
                let source = if matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("md" | "markdown" | "mdown" | "mdx")
                ) {
                    text
                } else {
                    let lang = crate::reader::language_for_path(&path);
                    crate::reader::source_as_document(&text, lang.as_deref())
                };
                let k = self.knowledge.clone();
                let reader = make_reader(Some(path.clone()), title, &source, &langs, &k, cx);
                if let Some(Tab::Editor { view, .. }) = self.tabs.get_mut(tab_ix) {
                    *view = EditorView::Preview(reader);
                }
            }
        }
        // Focus is the caller's decision, not this method's: keyboard
        // browsing previews each row it lands on while focus stays in
        // the sidebar, so the arrows keep working.
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

    /// Open the workspace search pre-seeded with `#tag`.
    fn open_tag_search(&mut self, tag: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_none() {
            self.toggle_search(&ToggleSearch, window, cx);
        }
        if let Some((overlay, _)) = &self.search {
            let query = format!("#{tag}");
            let input = overlay.read(cx).input.clone();
            input.update(cx, |input, cx| {
                let end = query.len();
                input.seed(query, end..end);
                cx.notify();
            });
        }
    }

    fn dismiss_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn toggle_palette(&mut self, _: &TogglePalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.dismiss_palette(window, cx);
            return;
        }
        let (entries, failures) = {
            let host = self.host.lock().unwrap();
                let mut entries = host
                    .plugins()
                    .iter()
                    .flat_map(|p| {
                        p.commands.iter().map(|c| crate::palette::PaletteEntry {
                            plugin: p.name.clone(),
                            id: c.id.clone(),
                            title: c.title.clone(),
                        }).collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                for name in &crate::extensions::format_plugins() {
                    entries.push(crate::palette::PaletteEntry {
                        plugin: name.clone(),
                        id: "__format".into(),
                        title: format!("Format: {name}"),
                    });
                }
                for p in host.plugins() {
                    for e in &p.exports {
                        entries.push(crate::palette::PaletteEntry {
                            plugin: p.name.clone(),
                            id: format!("__export:{}", e.id),
                            title: format!("Export: {}", e.name),
                        });
                    }
                }
                entries.push(crate::palette::PaletteEntry {
                    plugin: "supermd".into(),
                    id: "__install".into(),
                    title: "Install Plugins…".into(),
                });
                for (plugin, id, name) in crate::extensions::template_entries() {
                    entries.push(crate::palette::PaletteEntry {
                        plugin,
                        id: format!("__template:{id}"),
                        title: format!("New: {name}"),
                    });
                }
                let failures = host
                    .failures()
                    .iter()
                    .map(|(dir, e)| format!("{}: {e}", dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
                    .collect();
            (entries, failures)
        };
        // App-level commands exist with or without plugins.
        let mut entries = entries;
        entries.push(crate::palette::PaletteEntry {
            plugin: "supermd".into(),
            id: "__graph".into(),
            title: "Graph View".into(),
        });
        entries.push(crate::palette::PaletteEntry {
            plugin: "supermd".into(),
            id: "__flux".into(),
            title: if cx.global::<crate::theme::ThemeState>().settings.flux.enabled {
                "Flux: Disable Adaptive Theme".into()
            } else {
                "Flux: Enable Adaptive Theme".into()
            },
        });
        let palette = cx.new(|cx| crate::palette::Palette::new(entries, failures, cx));
        let subscription = cx.subscribe_in(
            &palette,
            window,
            |this, _p, event, window, cx| match event {
                crate::palette::PaletteEvent::Run { plugin, id } => {
                    let (plugin, id) = (plugin.clone(), id.clone());
                    this.dismiss_palette(window, cx);
                    this.run_plugin_command(plugin, id, window, cx);
                }
                crate::palette::PaletteEvent::Dismissed => this.dismiss_palette(window, cx),
            },
        );
        window.focus(&palette.focus_handle(cx));
        self.palette = Some((palette, subscription));
        cx.notify();
    }

    fn dismiss_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Fetch the catalog (background) and show the install overlay.
    fn open_install_overlay(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // No browsable catalog means no reason to fetch one: the overlay
        // just points at Import… and the website.
        if !crate::install_ui::catalog_browsable() {
            self.show_install_overlay(Vec::new(), _window, cx);
            return;
        }
        let fetch = cx
            .try_global::<crate::catalog::CatalogFetcher>()
            .map(|f| f.0.clone())
            .unwrap_or_else(crate::catalog::ureq_fetcher);
        let run = cx.background_executor().spawn(async move {
            fetch(crate::catalog::CATALOG_URL)
                .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()))
                .and_then(|json| crate::catalog::parse_catalog(&json))
        });
        cx.spawn_in(_window, async move |this, cx| {
            let result = run.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(entries) => this.show_install_overlay(entries, window, cx),
                Err(e) => this.show_command_error(format!("catalog: {e}"), cx),
            })
            .ok();
        })
        .detach();
    }

    fn show_install_overlay(
        &mut self,
        entries: Vec<crate::catalog::CatalogEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plugins_dir = crate::settings::config_dir().join("plugins");
        let installed: Vec<String> = entries
            .iter()
            .filter(|e| plugins_dir.join(&e.name).is_dir())
            .map(|e| e.name.clone())
            .collect();
        let overlay =
            cx.new(|cx| crate::install_ui::InstallOverlay::new(entries, installed, cx));
        let subscription = cx.subscribe_in(
            &overlay,
            window,
            |this, _o, event, window, cx| match event {
                crate::install_ui::InstallEvent::Install(entry) => {
                    let entry = entry.clone();
                    this.dismiss_install_overlay(window, cx);
                    this.install_catalog_plugin(entry, window, cx);
                }
                crate::install_ui::InstallEvent::Dismissed => {
                    this.dismiss_install_overlay(window, cx)
                }
            },
        );
        window.focus(&overlay.focus_handle(cx));
        self.install_overlay = Some((overlay, subscription));
        cx.notify();
    }

    fn dismiss_install_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.install_overlay = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn install_catalog_plugin(
        &mut self,
        entry: crate::catalog::CatalogEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fetch = cx
            .try_global::<crate::catalog::CatalogFetcher>()
            .map(|f| f.0.clone())
            .unwrap_or_else(crate::catalog::ureq_fetcher);
        let plugins_dir = crate::settings::config_dir().join("plugins");
        let name = entry.name.clone();
        let run = cx.background_executor().spawn(async move {
            crate::catalog::install_plugin(&entry, &plugins_dir, &fetch)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = run.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    this.reload_plugins(&ReloadPlugins, window, cx);
                    this.show_command_error(format!("Installed {name}"), cx);
                }
                Err(e) => this.show_command_error(e, cx),
            })
            .ok();
        })
        .detach();
    }

    /// What the `supermd://` confirmation says. The name comes from the
    /// URL but the capabilities come from the pinned catalog entry, so a
    /// link can never understate what it is asking for.
    fn install_confirmation(entry: &crate::catalog::CatalogEntry) -> String {
        match crate::install_ui::capability_blurb(&entry.capabilities) {
            Some(blurb) => format!("Install plugin \"{}\"? It {blurb}.", entry.name),
            None => format!("Install plugin \"{}\"?", entry.name),
        }
    }

    /// Handle a `supermd://install-plugin?name=X` handoff: resolve the
    /// name against the pinned catalog and ASK. Never installs silently —
    /// the link supplies a name, not code.
    fn request_plugin_install(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if crate::settings::config_dir().join("plugins").join(&name).is_dir() {
            self.show_command_error(format!("{name} is already installed"), cx);
            return;
        }
        let fetch = cx
            .try_global::<crate::catalog::CatalogFetcher>()
            .map(|f| f.0.clone())
            .unwrap_or_else(crate::catalog::ureq_fetcher);
        let run = cx.background_executor().spawn(async move {
            let entries = fetch(crate::catalog::CATALOG_URL)
                .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()))
                .and_then(|json| crate::catalog::parse_catalog(&json))?;
            crate::catalog::entry_by_name(&entries, &name)
                .cloned()
                .ok_or_else(|| format!("no plugin named {name} in the catalog"))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = run.await;
            this.update(cx, |this, cx| match result {
                Ok(entry) => {
                    this.install_request = Some(entry);
                    cx.notify();
                }
                Err(e) => this.show_command_error(e, cx),
            })
            .ok();
        })
        .detach();
    }

    fn resolve_install_request(
        &mut self,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.install_request.take() else {
            return;
        };
        if allow {
            self.install_catalog_plugin(entry, window, cx);
        } else {
            cx.notify();
        }
    }

    fn show_command_error(&mut self, msg: String, cx: &mut Context<Self>) {
        self.command_error = Some(msg.into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(4))
                .await;
            this.update(cx, |this, cx| {
                this.command_error = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_graph(&mut self, _: &ToggleGraph, window: &mut Window, cx: &mut Context<Self>) {
        self.open_graph_view(window, cx);
    }

    fn install_plugins(
        &mut self,
        _: &InstallPlugins,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_install_overlay(window, cx);
    }

    /// Flip flux on or off, persist it, and re-resolve the active theme
    /// so the change is visible without waiting for the minute timer.
    fn toggle_flux(&mut self, _: &ToggleFlux, window: &mut Window, cx: &mut Context<Self>) {
        {
            let on = !cx.global::<crate::theme::ThemeState>().settings.flux.enabled;
            persist_setting(cx, move |s| s.flux.enabled = on);
            let state = cx.global_mut::<crate::theme::ThemeState>();
            state.flux_blend = crate::flux::current_blend(&state.settings.flux);
        }
        crate::theme::refresh_active_theme(cx);
        window.refresh();
        cx.notify();
    }

    fn run_plugin_command(
        &mut self,
        plugin: String,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // These three were string ids only. They are actions now; the
        // palette keeps working by delegating to the same handlers.
        if id == "__install" {
            self.install_plugins(&InstallPlugins, window, cx);
            return;
        }
        if id == "__graph" {
            self.toggle_graph(&ToggleGraph, window, cx);
            return;
        }
        if id == "__flux" {
            self.toggle_flux(&ToggleFlux, window, cx);
            return;
        }
        // Templates need a workspace, not an editor — handled before
        // the editable-tab guard.
        if let Some(template_id) = id.strip_prefix("__template:") {
            let Some(root) = self.tree.as_ref().map(|t| t.root.clone()) else {
                self.show_command_error("Open a folder to use templates".to_string(), cx);
                return;
            };
            let host = self.host.clone();
            let ctx_data = crate::extensions::template_context(
                &root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
            let template_id = template_id.to_string();
            let run = cx.background_executor().spawn(async move {
                host.lock().unwrap().render_template(&plugin, &template_id, &ctx_data)
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = run.await;
                this.update_in(cx, |this, window, cx| match result {
                    Ok((filename, content)) => {
                        match crate::extensions::materialize_template(&root, &filename, &content)
                        {
                            Ok((path, _created)) => {
                                if let Some(tree) = &mut this.tree {
                                    tree.refresh();
                                }
                                this.open_path(&path, window, cx);
                            }
                            Err(e) => this.show_command_error(e, cx),
                        }
                    }
                    Err(e) => this.show_command_error(e, cx),
                })
                .ok();
            })
            .detach();
            return;
        }
        let Some(Tab::Editor { editor, view: EditorView::Edit }) = self.tabs.get(self.active)
        else {
            self.show_command_error("Commands need an editable tab".to_string(), cx);
            return;
        };
        let editor = editor.clone();
        let host = self.host.clone();
        let (document, selection) = editor.read(cx).command_snapshot();
        if id == "__format" {
            let snapshot = document.clone();
            let plugin_bg = plugin.clone();
            let run = cx.background_executor().spawn(async move {
                host.lock().unwrap().format_document(&plugin_bg, &document)
            });
            cx.spawn(async move |this, cx| {
                let result = run.await;
                this.update(cx, |this, cx| match result {
                    Ok(formatted) => {
                        let current = editor.read(cx).command_snapshot().0;
                        match crate::extensions::apply_if_unchanged(&snapshot, &current, formatted)
                        {
                            Some(text) => editor.update(cx, |editor, cx| {
                                editor.apply_command_output(
                                    &crate::extensions::CommandOutput::ReplaceDocument(text),
                                    cx,
                                );
                            }),
                            None => this.show_command_error(
                                "document changed while formatting; run again".into(),
                                cx,
                            ),
                        }
                    }
                    Err(e) => this.handle_plugin_error(plugin.clone(), e, cx),
                })
                .ok();
            })
            .detach();
            return;
        }
        if let Some(format) = id.strip_prefix("__export:") {
            let format = format.to_string();
            let plugin_bg = plugin.clone();
            let theme =
                crate::diagram::DiagramTheme::from_theme(&crate::theme::theme(cx));
            let stem = editor
                .read(cx)
                .path()
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "export".to_string());
            let extension = host
                .lock()
                .unwrap()
                .plugins()
                .iter()
                .find(|p| p.name == plugin)
                .and_then(|p| {
                    p.exports.iter().find(|e| e.id == format).map(|e| e.extension.clone())
                })
                .unwrap_or_else(|| "txt".to_string());
            let run = cx.background_executor().spawn(async move {
                host.lock().unwrap().export_document(&plugin_bg, &document, &format, &theme)
            });
            cx.spawn(async move |this, cx| {
                let result = run.await;
                this.update(cx, |this, cx| match result {
                    Ok(files) => match crate::extensions::validate_export_paths(&files) {
                        Ok(()) => this.finish_export(files, stem, extension, cx),
                        Err(e) => this.show_command_error(e, cx),
                    },
                    Err(e) => this.handle_plugin_error(plugin.clone(), e, cx),
                })
                .ok();
            })
            .detach();
            return;
        }
        let plugin_bg = plugin.clone();
        let run = cx.background_executor().spawn(async move {
            host.lock().unwrap().run_command(&plugin_bg, &id, &document, selection)
        });
        cx.spawn(async move |this, cx| {
            let result = run.await;
            this.update(cx, |this, cx| match result {
                Ok(out) => {
                    editor.update(cx, |editor, cx| editor.apply_command_output(&out, cx));
                }
                Err(e) => this.handle_plugin_error(plugin.clone(), e, cx),
            })
            .ok();
        })
        .detach();
    }

    /// The plugin produced export bytes; the user picks where they
    /// land — a save dialog for one file, a directory for several.
    fn finish_export(
        &mut self,
        files: Vec<(String, Vec<u8>)>,
        stem: String,
        extension: String,
        cx: &mut Context<Self>,
    ) {
        use crate::extensions::{write_export, ExportDest};
        if files.len() == 1 {
            let rx = cx.prompt_for_new_path(
                &crate::platform::home_dir(),
                Some(&format!("{stem}.{extension}")),
            );
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(path))) = rx.await {
                    let result = write_export(&files, &ExportDest::File(path));
                    this.update(cx, |this, cx| {
                        this.show_command_error(
                            result.err().unwrap_or_else(|| "Exported".to_string()),
                            cx,
                        );
                    })
                    .ok();
                }
            })
            .detach();
        } else {
            let rx = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: None,
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = rx.await {
                    if let Some(dir) = paths.into_iter().next() {
                        let result = write_export(&files, &ExportDest::Dir(dir));
                        this.update(cx, |this, cx| {
                            this.show_command_error(
                                result.err().unwrap_or_else(|| "Exported".to_string()),
                                cx,
                            );
                        })
                        .ok();
                    }
                }
            })
            .detach();
        }
    }

    /// Consent-shaped errors raise the Allow/Deny banner; everything
    /// else is a transient strip.
    fn handle_plugin_error(&mut self, plugin: String, error: String, cx: &mut Context<Self>) {
        if error.contains("awaiting consent") {
            self.consent_request = Some((plugin, "workspace-read".to_string()));
            cx.notify();
        } else if let Some(domain) = error.split("consent required: ").nth(1) {
            self.consent_request = Some((plugin, format!("net:{}", domain.trim())));
            cx.notify();
        } else {
            self.show_command_error(error, cx);
        }
    }

    fn resolve_consent(&mut self, allow: bool, cx: &mut Context<Self>) {
        let Some((plugin, cap)) = self.consent_request.take() else {
            return;
        };
        let dir = crate::settings::config_dir();
        let mut settings = crate::settings::load(&dir);
        let grant = if allow { cap.clone() } else { format!("denied:{cap}") };
        settings.plugin_grants.entry(plugin).or_default().push(grant);
        let _ = crate::settings::save(&dir, &settings);
        self.host.lock().unwrap().set_grants(settings.plugin_grants.clone());
        // The shared rootless host renders inline plugin output for
        // every window; it needs the same grants (never a root).
        if let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() {
            state.0.lock().unwrap().set_grants(settings.plugin_grants.clone());
        }
        if allow && cap.starts_with("net:") {
            // Retry the enrichment that raised the banner.
            if let Some(Tab::Editor { editor, .. }) = self.tabs.get(self.active) {
                editor.clone().update(cx, |editor, cx| editor.retry_enrich(cx));
                return;
            }
        }
        self.show_command_error(
            if allow {
                "Access granted — run the command again".to_string()
            } else {
                "Access denied".to_string()
            },
            cx,
        );
    }

    /// Swap this window's plugin host for one freshly loaded from
    /// `plugins_dir`, keeping the window's own workspace root and its
    /// editors' root cell. Returns how many plugins loaded.
    fn adopt_reloaded_host(
        &mut self,
        plugins_dir: &Path,
        grants: &std::collections::BTreeMap<String, Vec<String>>,
        refresh_tables: bool,
    ) -> usize {
        let mut host = crate::extensions::ExtensionHost::load(plugins_dir);
        host.set_grants(grants.clone());
        if let Some(tree) = &self.tree {
            host.set_workspace_root(Some(tree.root.clone()));
        }
        if refresh_tables {
            crate::extensions::refresh_tables(&mut host);
            for (dir, err) in host.failures() {
                eprintln!("supermd: plugin failed: {}: {err}", dir.display());
            }
        }
        let count = host.plugins().len();
        let mut slot = self.host.lock().unwrap_or_else(|e| e.into_inner());
        // Every editor in this window holds a clone of the *old*
        // host's root cell. Carry it into the replacement, or they
        // are all left pointing at a cell nothing writes to again.
        host.adopt_root_handle(slot.root_handle());
        *slot = host;
        count
    }

    fn reload_plugins(&mut self, _: &ReloadPlugins, window: &mut Window, cx: &mut Context<Self>) {
        let plugins_dir = crate::settings::config_dir().join("plugins");
        let settings = crate::settings::load(&crate::settings::config_dir());
        let grants = settings.plugin_grants.clone();
        let count = self.adopt_reloaded_host(&plugins_dir, &grants, true);
        // `refresh_tables` just rewrote the *process-wide* fence,
        // decoration and inline tables, so every window's projectors
        // now claim blocks for the new plugin set. A window whose host
        // was left behind answers those claims with "no such plugin"
        // and shows a red error banner in place of every such block
        // until it restarts -- so every window gets the new host, each
        // keeping its own sandbox root.
        //
        // Skipped by window handle, not by entity: `self` is the
        // workspace being updated right now, and updating it again --
        // through its own window -- panics.
        //
        // `WindowHandle::root` would read the entity directly, but it
        // is `#[cfg(any(test, feature = "test-support"))]`, so using it
        // here compiles under `cargo test` and breaks `cargo build`.
        let me = window.window_handle();
        let others: Vec<gpui::WindowHandle<Workspace>> = cx
            .windows()
            .into_iter()
            .filter(|handle| *handle != me)
            .filter_map(|handle| handle.downcast::<Workspace>())
            .collect();
        for handle in others {
            let _ = handle.update(cx, |workspace, _window, _cx| {
                workspace.adopt_reloaded_host(&plugins_dir, &grants, false);
            });
        }
        // Rebuild the shared rootless host too, so inline rendering
        // sees the new plugin set. It never gets a workspace root.
        if let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() {
            let mut slot = state.0.lock().unwrap_or_else(|e| e.into_inner());
            let mut shared = crate::extensions::ExtensionHost::load(&plugins_dir);
            shared.set_grants(settings.plugin_grants.clone());
            shared.mark_shared_rootless();
            // No production editor holds *this* host's cell today --
            // but `editor/mod.rs` claims every path that replaces a
            // host keeps the cell, and the test helpers already hand
            // editors this one. An exemption that has to be remembered
            // is an exemption that gets forgotten: adopt here too and
            // the claim is true without a footnote.
            shared.adopt_root_handle(slot.root_handle());
            *slot = shared;
        }
        if cx.try_global::<crate::diagram::DiagramCache>().is_some() {
            cx.global_mut::<crate::diagram::DiagramCache>().clear();
        }
        self.show_command_error(format!("Plugins reloaded: {count}"), cx);
        cx.refresh_windows();
    }

    fn open_plugins_folder(
        &mut self,
        _: &OpenPluginsFolder,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let dir = crate::settings::config_dir().join("plugins");
        let _ = std::fs::create_dir_all(&dir);
        crate::platform::reveal_dir(&dir);
    }

    /// Install a plugin the user downloaded themselves. The floor of the
    /// install story: the app never fetches code, so App Review 2.4.5(iv)
    /// does not engage. Ships in every build, not just the sandboxed one.
    fn import_plugin(&mut self, _: &ImportPlugin, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let result = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    let name = crate::catalog::plugin_name_from_zip(&bytes)?;
                    crate::catalog::install_plugin_from_bytes(
                        &bytes,
                        &name,
                        &crate::settings::config_dir().join("plugins"),
                    )
                    .map(|_| name)
                });
            this.update(cx, |this, cx| {
                this.show_command_error(
                    match result {
                        Ok(name) => format!("Installed {name} — restart to load it"),
                        Err(e) => e,
                    },
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// Sandboxed builds relocate ~/.supermd into the app container,
    /// where themes and settings.toml are otherwise unreachable.
    fn reveal_settings_folder(
        &mut self,
        _: &RevealSettingsFolder,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let dir = crate::settings::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        crate::platform::reveal_dir(&dir);
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

    // ── sidebar file operations ──

    fn selected_entry(&mut self) -> Option<crate::files::FsEntry> {
        let ix = self.sidebar_selected;
        self.sidebar_rows().get(ix).map(|(_, e)| e.clone())
    }

    /// Where new entries land: the selected folder itself, a selected
    /// file's parent, or the workspace root.
    fn creation_dir(&mut self) -> Option<PathBuf> {
        let root = self.tree.as_ref()?.root.clone();
        Some(match self.selected_entry() {
            Some(e) if e.is_dir => e.path,
            Some(e) => e.path.parent().map(|p| p.to_path_buf()).unwrap_or(root),
            None => root,
        })
    }

    fn start_sidebar_edit(
        &mut self,
        kind: SidebarEditKind,
        seed_text: String,
        select: std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let mut input = crate::input::TextInput::new("name", cx);
            input.seed(seed_text, select);
            input
        });
        window.focus(&input.read(cx).focus_handle);
        self.sidebar_edit = Some(SidebarEdit { kind, input, error: None });
        cx.notify();
    }

    fn sidebar_rename(&mut self, _: &SidebarRename, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let name = entry.name.clone();
        // Pre-select the stem so typing replaces the name, not the
        // extension. Folders select whole.
        let stem = if entry.is_dir {
            name.len()
        } else {
            name.rfind('.').filter(|&i| i > 0).unwrap_or(name.len())
        };
        self.start_sidebar_edit(SidebarEditKind::Rename(entry.path), name, 0..stem, window, cx);
    }

    fn sidebar_new_file(&mut self, _: &SidebarNewFile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.creation_dir() else {
            return;
        };
        if let Some(tree) = self.tree.as_mut() {
            tree.expand_to(&dir.join("·"));
        }
        self.start_sidebar_edit(SidebarEditKind::NewFile(dir), String::new(), 0..0, window, cx);
    }

    fn sidebar_new_folder(
        &mut self,
        _: &SidebarNewFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = self.creation_dir() else {
            return;
        };
        if let Some(tree) = self.tree.as_mut() {
            tree.expand_to(&dir.join("·"));
        }
        self.start_sidebar_edit(SidebarEditKind::NewDir(dir), String::new(), 0..0, window, cx);
    }

    fn sidebar_edit_cancel(
        &mut self,
        _: &SidebarEditCancel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_edit = None;
        window.focus(&self.sidebar_focus);
        cx.notify();
    }

    fn sidebar_edit_commit(
        &mut self,
        _: &SidebarEditCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.sidebar_edit.as_ref() else {
            return;
        };
        let name = edit.input.read(cx).content.to_string();
        if name.trim().is_empty() {
            return self.sidebar_edit_cancel(&SidebarEditCancel, window, cx);
        }
        // (old path when this was a rename, resulting path, open it?)
        let result = match &edit.kind {
            SidebarEditKind::Rename(path) => {
                crate::fileops::rename(path, &name).map(|new| (Some(path.clone()), new, false))
            }
            SidebarEditKind::NewFile(dir) => {
                crate::fileops::create_file(dir, &name).map(|new| (None, new, true))
            }
            SidebarEditKind::NewDir(dir) => {
                crate::fileops::create_dir(dir, &name).map(|new| (None, new, false))
            }
        };
        match result {
            Err(err) => {
                if let Some(edit) = self.sidebar_edit.as_mut() {
                    edit.error = Some(err.into());
                }
                cx.notify();
            }
            Ok((old, new, open)) => {
                self.sidebar_edit = None;
                if let Some(old) = &old {
                    self.after_path_change(old, &new, cx);
                }
                self.refresh_tree_and_select(&new, cx);
                window.focus(&self.sidebar_focus);
                if open {
                    self.open_path(&new, window, cx);
                }
                cx.notify();
            }
        }
    }

    fn refresh_tree_and_select(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Some(tree) = self.tree.as_mut() {
            tree.refresh();
            tree.expand_to(path);
        }
        let rows = self.sidebar_rows();
        if let Some(ix) = rows.iter().position(|(_, e)| e.path == path) {
            self.sidebar_selected = ix;
        }
        cx.notify();
    }

    /// Everything that must happen when a path moves. Today: retarget
    /// open tabs (files and whole folders). Milestone 2 adds link
    /// rewriting here.
    fn after_path_change(&mut self, old: &Path, new: &Path, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            match tab {
                Tab::Editor { editor, .. } => {
                    let cur = editor.read(cx).path().to_path_buf();
                    if let Some(p) = crate::fileops::retarget(&cur, old, new) {
                        editor.update(cx, |editor, _| editor.retarget(p));
                    }
                }
                Tab::Reader(reader) => {
                    if let Some(cur) = reader.read(cx).path.clone() {
                        if let Some(p) = crate::fileops::retarget(&cur, old, new) {
                            reader.update(cx, |reader, _| reader.path = Some(p));
                        }
                    }
                }
                Tab::Image { .. } => {}
            }
        }
        for tab in &mut self.tabs {
            if let Tab::Image { path, .. } = tab {
                if let Some(p) = crate::fileops::retarget(path, old, new) {
                    *path = p;
                }
            }
        }
        // History is a list of paths like any other: it has to follow
        // the move too, or Forward walks its cursor onto a name that is
        // no longer on disk.
        self.history.rewrite(|entry| crate::fileops::retarget(entry, old, new));
        self.rewrite_knowledge_links(old, new, cx);
        cx.notify();
    }

    /// Milestone-2 half of a rename/move: every note pointing at the
    /// moved path gets its links rewritten, on disk and in open tabs.
    fn rewrite_knowledge_links(&mut self, old: &Path, new: &Path, cx: &mut Context<Self>) {
        let state = self.knowledge.clone();
        // Disk is the rewrite source: flush dirty buffers first.
        for tab in &self.tabs {
            if let Tab::Editor { editor, .. } = tab {
                if editor.read(cx).save.is_dirty() {
                    editor.update(cx, |editor, cx| editor.flush(cx));
                }
            }
        }
        let mut index = state.lock().unwrap();
        // A moved folder renames every note under it.
        let moved: Vec<(PathBuf, PathBuf)> = if new.is_dir() {
            index
                .note_names()
                .iter()
                .filter_map(|(_, p)| {
                    // Note paths are still keyed under `old` here.
                    crate::fileops::retarget(p, old, new).map(|to| (p.clone(), to))
                })
                .collect()
        } else {
            vec![(old.to_path_buf(), new.to_path_buf())]
        };
        let mut changed: Vec<(PathBuf, String)> = Vec::new();
        for (from, to) in moved {
            changed.extend(index.rename_note(&from, &to, |p| std::fs::read_to_string(p).ok()));
        }
        drop(index);
        for (path, text) in changed {
            if let Err(err) = std::fs::write(&path, &text) {
                eprintln!("supermd: cannot rewrite links in {}: {err}", path.display());
                continue;
            }
            for tab in &self.tabs {
                if let Tab::Editor { editor, .. } = tab {
                    if editor.read(cx).path() == path {
                        editor.update(cx, |editor, cx| editor.reload_from_disk(cx));
                    }
                }
            }
        }
    }

    fn sidebar_delete(&mut self, _: &SidebarDelete, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let path = entry.path;
        // Flush dirty buffers under the path first: the trashed copy
        // must be complete.
        for tab in &self.tabs {
            if let Tab::Editor { editor, .. } = tab {
                if editor.read(cx).path().starts_with(&path) {
                    editor.update(cx, |editor, cx| editor.flush(cx));
                }
            }
        }
        let result = match cx.try_global::<crate::fileops::TrashFn>() {
            Some(t) => (t.0.clone())(&path),
            None => crate::fileops::delete(&path),
        };
        if let Err(err) = result {
            self.show_command_error(err, cx);
            return;
        }
        let doomed: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(ix, tab)| tab.path(cx).filter(|p| p.starts_with(&path)).map(|_| ix))
            .collect();
        for ix in doomed.into_iter().rev() {
            self.close_tab_at(ix, window, cx);
        }
        if let Some(tree) = self.tree.as_mut() {
            tree.refresh();
        }
        let last = self.sidebar_rows().len().saturating_sub(1);
        self.sidebar_selected = self.sidebar_selected.min(last);
        cx.notify();
    }

    fn sidebar_move_to(&mut self, _: &SidebarMoveTo, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let Some(root) = self.tree.as_ref().map(|t| t.root.clone()) else {
            return;
        };
        let mut folders = vec![root.clone()];
        for item in crate::files::workspace_walk(&root).flatten() {
            let p = item.path();
            if p != root && item.file_type().is_some_and(|t| t.is_dir()) {
                folders.push(p.to_path_buf());
            }
        }
        let entries: Vec<crate::palette::PaletteEntry> = folders
            .iter()
            .map(|dir| crate::palette::PaletteEntry {
                plugin: "move".into(),
                id: dir.to_string_lossy().into_owned(),
                title: if dir == &root {
                    "/".to_string()
                } else {
                    format!("/{}", dir.strip_prefix(&root).unwrap_or(dir).display())
                },
            })
            .collect();
        let source = entry.path;
        let picker = cx.new(|cx| crate::palette::Palette::new(entries, Vec::new(), cx));
        let subscription = cx.subscribe_in(
            &picker,
            window,
            move |this, _p, event, window, cx| match event {
                crate::palette::PaletteEvent::Run { id, .. } => {
                    let dest = PathBuf::from(id);
                    this.dismiss_move_picker(window, cx);
                    match crate::fileops::move_entry(&source, &dest) {
                        Ok(new) => {
                            this.after_path_change(&source.clone(), &new, cx);
                            this.refresh_tree_and_select(&new, cx);
                        }
                        Err(err) => this.show_command_error(err, cx),
                    }
                }
                crate::palette::PaletteEvent::Dismissed => this.dismiss_move_picker(window, cx),
            },
        );
        window.focus(&picker.focus_handle(cx));
        self.move_picker = Some((picker, subscription));
        cx.notify();
    }

    fn dismiss_move_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.move_picker = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Show the file in Finder. `NSWorkspace` rather than a process
    /// spawn: the App Store build cannot spawn processes.
    fn reveal_in_finder(&mut self, _: &RevealInFinder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        // `reveal_dir` already exists and uses
        // `activateFileViewerSelectingURLs`, which selects the item —
        // so it reveals a file, not only a directory.
        crate::platform::reveal_dir(&entry.path);
        let _ = cx;
    }

    fn copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            entry.path.display().to_string(),
        ));
        self.show_command_error("Path copied".into(), cx);
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
            let picked = &cx.global::<crate::theme::ThemeState>().themes[ix];
            let (name, is_dark) = (picked.name.clone(), picked.theme.is_dark);
            persist_setting(cx, move |s| {
                if is_dark {
                    s.dark_theme = name.clone();
                } else {
                    s.light_theme = name.clone();
                }
            });
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

    fn toggle_about(&mut self, _: &ToggleAbout, window: &mut Window, cx: &mut Context<Self>) {
        self.show_about = !self.show_about;
        if self.show_about {
            window.focus(&self.about_focus);
        } else {
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    /// Ask GitHub for the latest release tag. Failure is silent and
    /// simply leaves the dialog reporting nothing, matching the quiet
    /// launch-time check.
    fn check_for_updates(
        &mut self,
        _: &CheckForUpdates,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.about_checking || !crate::update::checks_enabled() {
            return;
        }
        self.about_checking = true;
        cx.notify();
        let fetch = cx
            .background_executor()
            .spawn(async move { crate::update::fetch_latest_tag() });
        cx.spawn(async move |this, cx| {
            let tag = fetch.await;
            this.update(cx, |this, cx| {
                this.about_checking = false;
                if let Some(tag) = tag {
                    this.about_latest = Some(SharedString::from(tag));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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

    fn render_about(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_about {
            return None;
        }
        let t = theme(cx);
        let version = env!("CARGO_PKG_VERSION");
        let status = crate::update::update_status(
            version,
            self.about_latest.as_ref().map(|t| t.as_ref()),
            self.about_checking,
        );
        let downloadable = matches!(status, crate::update::UpdateStatus::Available(_));
        let button = |id: &'static str, label: SharedString, t: &Theme| {
            div()
                .id(id)
                .px_3()
                .py(px(4.))
                .rounded_md()
                .cursor_pointer()
                .border_1()
                .border_color(t.border)
                .text_size(px(t.ui_size))
                .text_color(t.fg)
                .hover(|s| s.bg(t.hover_bg))
                .child(label)
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.toggle_about(&ToggleAbout, window, cx);
                    }),
                )
                .child(
                    div()
                        .key_context("About")
                        .track_focus(&self.about_focus)
                        .on_action(cx.listener(Self::toggle_about))
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .w(px(320.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded_lg()
                        .shadow_lg()
                        .p_5()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(19.))
                                .text_color(t.fg_strong)
                                .child("SuperMD"),
                        )
                        .child(
                            div()
                                .text_size(px(t.ui_size))
                                .text_color(t.fg_muted)
                                .child(SharedString::from(format!("Version {version}"))),
                        )
                        .child(
                            div()
                                .pt_1()
                                .text_size(px(t.ui_size))
                                .text_color(t.fg)
                                .child(SharedString::from(status.message())),
                        )
                        .child(
                            div()
                                .pt_2()
                                .flex()
                                .flex_row()
                                .gap_2()
                                .child(
                                    button("about-check", "Check for Updates".into(), &t)
                                        .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                                            this.check_for_updates(&CheckForUpdates, w, cx);
                                        })),
                                )
                                .children(downloadable.then(|| {
                                    button("about-download", "Download".into(), &t).on_click(
                                        cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                            cx.open_url(crate::update::RELEASES_URL);
                                        }),
                                    )
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The right-click menu raised on a sidebar row, tab, or graph
    /// node. Built from `menus::items_for`, so a renamed command or a
    /// changed shortcut shows up here for free; each row dispatches the
    /// same boxed action the menu bar and the keystroke do, so all
    /// three paths stay one path.
    fn render_context_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (pos, surface) = self.context_menu?;
        let t = theme(cx);
        let rows: Vec<AnyElement> = crate::menus::items_for(surface)
            .into_iter()
            .map(|item| {
                let cmd = crate::commands::COMMANDS
                    .iter()
                    .find(|c| c.id == item.id)
                    .expect("menu item names a real command");
                let shortcut = if item.keys.is_empty() {
                    String::new()
                } else {
                    crate::platform::shortcut_glyphs(&crate::commands::glyphs(item.keys))
                };
                div()
                    .id(SharedString::from(format!("ctx-menu-{}", item.id)))
                    .w_full()
                    .px_3()
                    .py(px(5.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover_bg))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(t.ui_size))
                            .text_color(t.fg)
                            .child(item.label),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.fg_muted)
                            .child(SharedString::from(shortcut)),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.context_menu = None;
                        window.dispatch_action((cmd.action)(), cx);
                    }))
                    .into_any_element()
            })
            .collect();

        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                // A click anywhere else dismisses the menu without
                // otherwise reacting — the row underneath must not also
                // see the click that closed the menu.
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        cx.notify();
                    }),
                )
                .child(gpui::deferred(
                    gpui::anchored()
                        .position(pos)
                        .anchor(gpui::Corner::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .id("ctx-menu")
                                .w(px(220.))
                                .bg(t.panel_bg)
                                .border_1()
                                .border_color(t.border)
                                .rounded_lg()
                                .shadow_lg()
                                .overflow_hidden()
                                .flex()
                                .flex_col()
                                .py_1()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .children(rows),
                        ),
                ))
                .into_any_element(),
        )
    }

    fn render_shortcuts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_shortcuts {
            return None;
        }
        let t = theme(cx);
        let groups = crate::commands::help_sections().into_iter().map(|(title, rows)| {
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
                .children(rows.into_iter().map(|(keys, desc)| {
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
                                    &keys,
                                ))),
                        )
                        .child(
                            div()
                                .text_size(px(t.ui_size))
                                .text_color(t.fg)
                                .child(SharedString::from(desc)),
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


    /// One plain file/folder row of the sidebar list.
    #[allow(clippy::too_many_arguments)]
    fn sidebar_file_row(
        &self,
        row_ix: usize,
        depth: usize,
        entry: crate::files::FsEntry,
        kb_selected: usize,
        git_modified: &std::collections::HashSet<PathBuf>,
        active_path: Option<&Path>,
        t: &crate::theme::Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
                let is_active = active_path == Some(entry.path.as_path());
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
                        let (icon_path, tint) = if is_dir {
                            (crate::ui_icons::path("folder"), t.fg_muted)
                        } else {
                            let (icon, color) = seti::icon_for(&entry.name);
                            (format!("icons/seti/{icon}.svg"), seti_tint(color, &t))
                        };
                        // Seti glyphs carry ~30% internal padding, so the box
                        // runs larger than the text for a matched visual size.
                        gpui::svg()
                            .path(SharedString::from(icon_path))
                            .size(px(20.))
                            .flex_none()
                            .text_color(tint)
                    })
                    .child({
                        let row_color = sidebar_row_color(entry.ignored, is_dir, &t);
                        div()
                            .text_size(px(t.ui_size))
                            .text_color(row_color)
                            .overflow_hidden()
                            .child(SharedString::from(entry.name.clone()))
                    })
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
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.sidebar_selected = row_ix;
                            let surface = if is_dir {
                                crate::menus::Surface::SidebarFolder
                            } else {
                                crate::menus::Surface::SidebarFile
                            };
                            this.context_menu = Some((event.position, surface));
                            window.focus(&this.sidebar_focus);
                            cx.notify();
                        }),
                    )
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
                    .into_any_element()
    }

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

        // Inline rename / create rows swap in a text input.
        let edit_state = self.sidebar_edit.as_ref().map(|e| {
            let target = match &e.kind {
                SidebarEditKind::Rename(p) => p.clone(),
                SidebarEditKind::NewFile(d) | SidebarEditKind::NewDir(d) => d.clone(),
            };
            let renaming = matches!(e.kind, SidebarEditKind::Rename(_));
            (target, renaming, e.input.clone(), e.error.clone())
        });
        let edit_row = |input: &Entity<crate::input::TextInput>,
                        error: &Option<SharedString>,
                        depth: usize,
                        cx: &mut Context<Self>| {
            div()
                .key_context("SidebarEdit")
                .on_action(cx.listener(Self::sidebar_edit_commit))
                .on_action(cx.listener(Self::sidebar_edit_cancel))
                .ml(px(depth as f32 * 12. + 12.))
                .mr_2()
                .flex()
                .flex_col()
                .child(input.clone())
                .children(error.clone().map(|err| {
                    div()
                        .text_size(px(11.))
                        .text_color(t.diff_deleted_fg)
                        .child(err)
                }))
                .into_any_element()
        };

        let mut items: Vec<gpui::AnyElement> = Vec::new();
        if let Some((target, renaming, input, error)) = &edit_state {
            // Creating at the workspace root: the edit row leads.
            if !renaming && Some(target.as_path()) == self.tree.as_ref().map(|t| t.root.as_path())
            {
                items.push(edit_row(input, error, 0, cx));
            }
        }
        for (row_ix, (depth, entry)) in rows.into_iter().enumerate() {
            if let Some((target, renaming, input, error)) = &edit_state {
                if *renaming && *target == entry.path {
                    items.push(edit_row(input, error, depth, cx));
                    continue;
                }
            }
            items.push(self.sidebar_file_row(row_ix, depth, entry.clone(), kb_selected, &git_modified, active_path.as_deref(), &t, cx));
            if let Some((target, renaming, input, error)) = &edit_state {
                if !renaming && *target == entry.path && entry.is_dir {
                    items.push(edit_row(input, error, depth + 1, cx));
                }
            }
        }
        let items = items;

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
                .on_action(cx.listener(Self::sidebar_rename))
                .on_action(cx.listener(Self::sidebar_delete))
                .on_action(cx.listener(Self::sidebar_new_file))
                .on_action(cx.listener(Self::sidebar_new_folder))
                .on_action(cx.listener(Self::sidebar_move_to))
                .on_action(cx.listener(Self::reveal_in_finder))
                .on_action(cx.listener(Self::copy_path))
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
                    // Workspace name, with a `+` for new files — ⌘N and
                    // ⌘⇧N were keyboard-only.
                    div()
                        .px_3()
                        .pt(px(4.))
                        .pb(px(6.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.))
                                .text_color(t.fg_muted)
                                .overflow_hidden()
                                .truncate()
                                .child(SharedString::from(root_name.to_uppercase())),
                        )
                        .child(
                            div()
                                .id("sidebar-new-file")
                                .flex_none()
                                .p(px(2.))
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(|s| s.bg(t.hover_bg))
                                .child(
                                    gpui::svg()
                                        .path(SharedString::from(crate::ui_icons::path("plus")))
                                        .size(px(13.))
                                        .flex_none()
                                        .text_color(t.fg_muted),
                                )
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.sidebar_new_file(&SidebarNewFile, window, cx);
                                })),
                        ),
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

    /// Right-hand titlebar cluster: the three panel toggles, plus a Show
    /// Changes button that appears only when the open file differs from
    /// HEAD. Keyboard-only until now.
    /// The bottom status strip. Replaces the floating pill that used to
    /// carry widget-plugin text, and gives flux and the graph a home in
    /// the chrome. Hidden in focus mode, which is meant to be bare.
    fn render_status_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.focus_mode {
            return None;
        }
        let t = theme(cx);
        let flux_on = cx.global::<crate::theme::ThemeState>().settings.flux.enabled;
        let status = match self.tabs.get(self.active) {
            Some(Tab::Editor { editor, view: EditorView::Edit })
                if !crate::extensions::widget_plugins().is_empty() =>
            {
                editor.read(cx).status()
            }
            _ => None,
        };
        let button = |id: &'static str,
                      icon: &'static str,
                      on: bool,
                      cx: &mut Context<Self>,
                      act: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            div()
                .id(id)
                .px(px(5.))
                .h_full()
                .flex()
                .flex_none()
                .items_center()
                .cursor_pointer()
                .hover(|s| s.bg(t.hover_bg))
                .child(
                    gpui::svg()
                        .path(SharedString::from(crate::ui_icons::path(icon)))
                        .size(px(13.))
                        .flex_none()
                        .text_color(if on { t.accent } else { t.fg_muted }),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    act(this, window, cx);
                }))
        };
        Some(
            div()
                .h(px(22.))
                .w_full()
                .flex_none()
                .bg(t.panel_bg)
                .border_t_1()
                .border_color(t.border)
                .flex()
                .flex_row()
                .items_center()
                .px_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.))
                        .text_color(t.fg_muted)
                        .overflow_hidden()
                        .truncate()
                        .children(status),
                )
                .child(button("status-graph", "graph", false, cx, |t, w, c| {
                    t.toggle_graph(&ToggleGraph, w, c)
                }))
                .child(button("status-flux", "sun", flux_on, cx, |t, w, c| {
                    t.toggle_flux(&ToggleFlux, w, c)
                }))
                .into_any_element(),
        )
    }

    fn render_titlebar_chrome(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.focus_mode {
            return None;
        }
        let t = theme(cx);
        let button = |id: &'static str,
                      icon: &'static str,
                      on: bool,
                      cx: &mut Context<Self>,
                      act: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            div()
                .id(id)
                .w(px(28.))
                .h_full()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(t.hover_bg))
                .child(
                    gpui::svg()
                        .path(SharedString::from(crate::ui_icons::path(icon)))
                        .size(px(15.))
                        .flex_none()
                        .text_color(if on { t.fg_strong } else { t.fg_muted }),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    act(this, window, cx);
                }))
        };
        // Show Changes is offered only when there is a change to show.
        let modified = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.path(cx))
            .is_some_and(|p| self.git_modified.contains(&p));
        Some(
            div()
                .flex()
                .flex_row()
                .flex_none()
                .h_full()
                .items_center()
                .children(modified.then(|| {
                    button("chrome-changes", "changes", false, cx, |t, w, c| {
                        t.show_changes(&ShowChanges, w, c)
                    })
                }))
                .child(button(
                    "chrome-sidebar",
                    "sidebar",
                    self.show_sidebar,
                    cx,
                    |t, w, c| t.toggle_sidebar(&ToggleSidebar, w, c),
                ))
                .child(button(
                    "chrome-outline",
                    "outline",
                    self.show_outline,
                    cx,
                    |t, w, c| t.toggle_outline(&ToggleOutline, w, c),
                ))
                .child(button(
                    "chrome-knowledge",
                    "knowledge",
                    self.show_knowledge,
                    cx,
                    |t, w, c| t.toggle_knowledge(&ToggleKnowledge, w, c),
                ))
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
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.set_active(ix, window, cx);
                        this.context_menu = Some((event.position, crate::menus::Surface::Tab));
                        cx.notify();
                    }),
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
            .children(self.render_titlebar_chrome(cx))
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

    /// Build, lay out, and show the full-workspace graph.
    fn open_graph_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.is_none() {
            self.show_command_error("Open a folder to see its graph".to_string(), cx);
            return;
        }
        let (mut nodes, mut edges) = {
            let index = self.knowledge.lock().unwrap();
            let (mut n, mut e) = crate::graph::build(&index);
            // Notes the vault refers to but does not have. They are the
            // to-write list, and the graph is where they are visible.
            crate::graph::with_ghosts(&index, &mut n, &mut e);
            (n, e)
        };
        let _ = &mut edges;
        // Seed with a short run so the first frame is already sensible,
        // then let the ticker carry it the rest of the way on screen —
        // the graph settles in front of you instead of appearing done.
        let mut sim = crate::graph::Simulation::new(nodes, edges);
        sim.run(40);
        self.graph = Some(GraphViewState {
            sim,
            pan: (0.0, 0.0),
            zoom: 1.0,
            drag: None,
            node_drag: None,
            node_dragged: false,
            hovered: None,
            ticker: None,
            color_by: crate::graph::ColorBy::Folder,
            spread: crate::graph::Spread::Normal,
            filter: crate::graph::Filter::default(),
            searching: false,
        });
        window.focus(&self.graph_focus);
        // The layout is unbounded, so frame it before the first paint —
        // otherwise a large vault opens somewhere off screen.
        self.graph_fit(&GraphFit, window, cx);
        self.graph_tick(cx);
        cx.notify();
    }

    /// Step the layout on the next frame while it still has motion.
    ///
    /// Stops itself once the simulation settles, so an idle graph costs
    /// nothing — and any interaction reheats it, which starts this
    /// again.
    fn graph_tick(&mut self, cx: &mut Context<Self>) {
        let Some(graph) = self.graph.as_mut() else { return };
        if graph.sim.settled() {
            graph.ticker = None;
            return;
        }
        graph.ticker = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(16))
                .await;
            this.update(cx, |this, cx| {
                if let Some(graph) = this.graph.as_mut() {
                    graph.sim.step();
                    cx.notify();
                }
                this.graph_tick(cx);
            })
            .ok();
        }));
    }

    /// Frame the whole graph in the window. There was no way back from
    /// a pan before this short of closing and reopening the view.
    fn graph_fit(&mut self, _: &GraphFit, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = window.viewport_size();
        let Some(graph) = self.graph.as_mut() else { return };
        let (zoom, pan_x, pan_y) = crate::graph::fit_to(
            graph.sim.nodes.as_slice(),
            (f32::from(viewport.width), f32::from(viewport.height)),
            80.0,
        );
        graph.zoom = zoom;
        graph.pan = (pan_x, pan_y);
        cx.notify();
    }

    /// Cycle what a node's colour means: folder, tag, or nothing.
    fn graph_color_by(&mut self, _: &GraphColorBy, _: &mut Window, cx: &mut Context<Self>) {
        use crate::graph::ColorBy;
        let Some(graph) = self.graph.as_mut() else { return };
        graph.color_by = match graph.color_by {
            ColorBy::Folder => ColorBy::Tag,
            ColorBy::Tag => ColorBy::None,
            ColorBy::None => ColorBy::Folder,
        };
        let what = match graph.color_by {
            ColorBy::Folder => "folder",
            ColorBy::Tag => "tag",
            ColorBy::None => "nothing",
        };
        self.show_command_error(format!("Graph colour: {what}"), cx);
        cx.notify();
    }

    /// Start typing a query. Escape while searching clears it rather
    /// than closing the graph, so a mistyped filter is one key away
    /// from undone.
    fn graph_search(&mut self, _: &GraphSearch, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph.as_mut() {
            graph.searching = true;
            cx.notify();
        }
    }

    /// Show only notes nothing links to, and that link nowhere.
    fn graph_orphans(&mut self, _: &GraphOrphans, _: &mut Window, cx: &mut Context<Self>) {
        let Some(graph) = self.graph.as_mut() else { return };
        graph.filter.orphans_only = !graph.filter.orphans_only;
        let on = graph.filter.orphans_only;
        self.show_command_error(
            if on { "Graph: orphans only".into() } else { "Graph: all notes".to_string() },
            cx,
        );
        cx.notify();
    }

    /// Narrow the graph to what is near the node under the pointer —
    /// or, with the pointer off the nodes, the note you have open —
    /// and widen it back to the whole vault.
    fn graph_local(&mut self, _: &GraphLocal, _: &mut Window, cx: &mut Context<Self>) {
        let open = self.tabs.get(self.active).and_then(|tab| tab.path(cx));
        let Some(graph) = self.graph.as_mut() else { return };
        if graph.filter.local.is_some() {
            graph.filter.local = None;
            graph.filter.in_scope = None;
            self.show_command_error("Graph: whole vault".into(), cx);
            cx.notify();
            return;
        }
        // The node under the pointer wins. A right-click sets `hovered`
        // and the menu dispatches this action, so without reading it
        // the menu acted on the active tab instead of the node it was
        // raised over — the wrong note, or none at all.
        let hovered = graph.hovered.filter(|ix| *ix < graph.sim.nodes.len());
        let center = if let Some(ix) = hovered {
            ix
        } else {
            let Some(open) = open else {
                self.show_command_error("Open a note to centre the graph on it".into(), cx);
                return;
            };
            let Some(center) = graph.sim.nodes.iter().position(|n| n.path == open) else {
                self.show_command_error("That note is not in the graph".into(), cx);
                return;
            };
            center
        };
        graph.filter.local = Some((center, 1));
        let edges = graph.sim.edges.clone();
        graph.filter.resolve_scope(&edges);
        self.show_command_error("Graph: 1 hop from this note".into(), cx);
        cx.notify();
    }

    fn graph_depth(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(graph) = self.graph.as_mut() else { return };
        let Some((center, depth)) = graph.filter.local else {
            self.show_command_error("Turn on local mode first".into(), cx);
            return;
        };
        let depth = (depth as isize + delta).clamp(1, 6) as usize;
        graph.filter.local = Some((center, depth));
        let edges = graph.sim.edges.clone();
        graph.filter.resolve_scope(&edges);
        let n = graph.filter.in_scope.as_ref().map_or(0, |s| s.len());
        self.show_command_error(format!("Graph: {depth} hops · {n} notes"), cx);
        cx.notify();
    }

    fn graph_depth_out(&mut self, _: &GraphDepthOut, _: &mut Window, cx: &mut Context<Self>) {
        self.graph_depth(1, cx);
    }

    fn graph_depth_in(&mut self, _: &GraphDepthIn, _: &mut Window, cx: &mut Context<Self>) {
        self.graph_depth(-1, cx);
    }

    /// Hold the layout still, or let it settle again.
    fn graph_freeze(&mut self, _: &GraphFreeze, _: &mut Window, cx: &mut Context<Self>) {
        let Some(graph) = self.graph.as_mut() else { return };
        let frozen = !graph.sim.frozen();
        graph.sim.freeze(frozen);
        self.show_command_error(
            if frozen { "Graph: frozen".into() } else { "Graph: running".to_string() },
            cx,
        );
        self.graph_tick(cx);
        cx.notify();
    }

    /// Cycle how spread out the layout sits.
    fn graph_spread(&mut self, _: &GraphSpread, _: &mut Window, cx: &mut Context<Self>) {
        let Some(graph) = self.graph.as_mut() else { return };
        graph.spread = graph.spread.next();
        graph.sim.forces = graph.spread.forces();
        graph.sim.reheat(0.7);
        let label = graph.spread.label();
        self.show_command_error(format!("Graph spread: {label}"), cx);
        self.graph_tick(cx);
        cx.notify();
    }

    fn graph_dismiss(&mut self, _: &GraphDismiss, window: &mut Window, cx: &mut Context<Self>) {
        // A live filter is what Escape clears first.
        if let Some(graph) = self.graph.as_mut() {
            if graph.searching || !graph.filter.is_empty() {
                graph.searching = false;
                graph.filter = crate::graph::Filter::default();
                cx.notify();
                return;
            }
        }
        self.graph = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Open the note behind a graph node and close the overlay.
    ///
    /// A ghost has no file: its path is the bare name the link asked
    /// for. Clicking one creates the note beside the note that
    /// referenced it, which is what following that link would have
    /// done — previously it closed the overlay, failed to read a
    /// relative path against the process working directory, and
    /// printed to stderr.
    fn open_graph_node(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.graph.as_ref().and_then(|g| g.nodes().get(ix)).cloned() else {
            return;
        };
        self.graph = None;
        if node.ghost {
            let Some(source) = node.ghost_source else { return };
            let Some(dir) = source.parent() else { return };
            let name = node
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let root = self.knowledge.lock().unwrap().root.clone();
            if root.as_os_str().is_empty() {
                return;
            }
            // The same containment the editor applies before creating a
            // note from a link.
            let Some(path) = crate::knowledge::creatable_note_path(&root, dir, &name) else {
                self.show_command_error(format!("Cannot create {name} here"), cx);
                return;
            };
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) | Err(_) => {}
            }
            self.open_path(&path, window, cx);
            return;
        }
        self.open_path(&node.path, window, cx);
    }

    fn render_graph(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        /// Width of a node's label box. Fixed, and centred on the node,
        /// so label length never displaces the dot.
        const LABEL_W: f32 = 160.0;
        // Which note is open, so its node can be marked. Read before
        // borrowing the graph state.
        let open_path = self.tabs.get(self.active).and_then(|tab| tab.path(cx));
        let state = self.graph.as_ref()?;
        let t = theme(cx);
        // World transform: unit square → an 900px board, panned/zoomed.
        let base = 900.0 * state.zoom;
        let (pan_x, pan_y) = state.pan;
        let at = |n: &crate::graph::GraphNode| (pan_x + n.x * base + 60.0, pan_y + n.y * base + 60.0);

        let lit = state.hovered.map(|ix| state.neighbourhood(ix));

        // Colour groups. The palette comes from the theme's syntax
        // colours: they are already chosen to be distinct from each
        // other and to sit correctly on this background, so a graph
        // stays coherent when the theme changes.
        let palette: Vec<Hsla> = vec![
            t.syntax.function,
            t.syntax.string,
            t.syntax.keyword,
            t.syntax.constant,
            t.syntax.tag,
            t.syntax.property,
        ];
        // Sorted and deduplicated, so a group's colour is the same on
        // every open rather than depending on node order.
        let group_keys: Vec<String> = {
            let mut keys: Vec<String> = state
                .nodes()
                .iter()
                .filter_map(|n| match state.color_by {
                    crate::graph::ColorBy::None => None,
                    crate::graph::ColorBy::Folder => n.folder.clone(),
                    crate::graph::ColorBy::Tag => n.tag.clone(),
                })
                .collect();
            keys.sort();
            keys.dedup();
            keys
        };
        let label_alpha = crate::graph::label_opacity(state.zoom);
        // (from, to, lit, reciprocated, radius at each end) — the radii
        // let the arrowhead stop short of the node it points at.
        let node_r = |n: &crate::graph::GraphNode| {
            (5.0 + (n.degree as f32).sqrt() * 3.0) * state.zoom.sqrt()
        };
        let edge_px: Vec<((f32, f32), (f32, f32), bool, bool, f32, f32)> = state
            .edges()
            .iter()
            .map(|e| {
                let (a, b) = (&state.nodes()[e.from], &state.nodes()[e.to]);
                let on = lit
                    .as_ref()
                    .is_none_or(|l| l.contains(&e.from) && l.contains(&e.to))
                    && state.filter.matches_at(e.from, a)
                    && state.filter.matches_at(e.to, b);
                (at(a), at(b), on, e.both, node_r(a), node_r(b))
            })
            .collect();
        let edge_color = Hsla { a: 0.35, ..t.fg_muted };
        let dim_edge = Hsla { a: 0.08, ..t.fg_muted };
        let edges_canvas = gpui::canvas(
            move |bounds, _, _| bounds,
            move |bounds, _, window, _| {
                for (a, b, on, both, ra, rb) in &edge_px {
                    let pa = point(bounds.origin.x + px(a.0), bounds.origin.y + px(a.1));
                    let pb = point(bounds.origin.x + px(b.0), bounds.origin.y + px(b.1));
                    // Edges outside the hovered neighbourhood fade back
                    // rather than disappear, so the shape of the graph
                    // is still readable while one part is emphasised.
                    let color = if *on { edge_color } else { dim_edge };
                    window.paint_path(crate::graph::line_path(pa, pb, 1.5), color);
                    // Which way the link points. A pair that links both
                    // ways gets an arrowhead at each end rather than two
                    // lines drawn over each other.
                    window.paint_path(
                        crate::graph::arrow_path(pa, pb, rb + 2.0, 7.0),
                        color,
                    );
                    if *both {
                        window.paint_path(
                            crate::graph::arrow_path(pb, pa, ra + 2.0, 7.0),
                            color,
                        );
                    }
                }
            },
        )
        .absolute()
        .size_full();

        let mut board = div().absolute().inset_0().child(edges_canvas);
        for (ix, node) in state.nodes().iter().enumerate() {
            let (x, y) = at(node);
            let r = (5.0 + (node.degree as f32).sqrt() * 3.0) * state.zoom.sqrt();
            let name = node
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Hovering one node lights it and everything it links to,
            // and fades the rest back — the shape stays legible while
            // one neighbourhood is picked out.
            // Lit means: inside the hovered neighbourhood, and matching
            // whatever the view is narrowed to.
            let on = lit.as_ref().is_none_or(|l| l.contains(&ix))
                && state.filter.matches_at(ix, node);
            let is_open = open_path.as_deref() == Some(node.path.as_path());
            let is_ghost = node.ghost;
            let group = match state.color_by {
                crate::graph::ColorBy::None => None,
                crate::graph::ColorBy::Folder => node.folder.as_deref(),
                crate::graph::ColorBy::Tag => node.tag.as_deref(),
            };
            let grouped = crate::graph::color_slot(group, &group_keys, palette.len())
                .map(|slot| palette[slot]);
            let base_color = if is_open {
                t.link
            } else if node.ghost {
                // Hollow: it is a name, not a note. Clicking it creates
                // the file, the same as following the link would.
                Hsla { a: 0.30, ..t.fg_muted }
            } else if let Some(c) = grouped {
                c
            } else if node.degree > 0 {
                t.accent
            } else {
                t.fg_muted
            };
            let node_color = if on { base_color } else { Hsla { a: 0.25, ..base_color } };
            let label_color = Hsla {
                a: if on { label_alpha } else { label_alpha * 0.25 },
                ..t.fg
            };
            board = board.child(
                div()
                    .id(("graph-node", ix))
                    .absolute()
                    .left(px(x - r))
                    .top(px(y - r))
                    // Exactly the dot's size. A flex column sized by its
                    // widest child let a long label stretch the box, and
                    // `items_center` then centred the dot inside *that*
                    // — so nodes with long names sat right of where
                    // their edges met. The label is positioned below
                    // without contributing to this box.
                    .w(px(r * 2.0))
                    .h(px(r * 2.0))
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if let Some(graph) = &mut this.graph {
                            graph.hovered = hovered.then_some(ix);
                            cx.notify();
                        }
                    }))
                    // Press on a node grabs it rather than panning the
                    // board, and holds the layout warm so the graph
                    // keeps reacting while it is dragged around.
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if let Some(graph) = &mut this.graph {
                                graph.node_drag = Some(ix);
                                graph.node_dragged = false;
                                graph.sim.hold_warm(true);
                            }
                            this.graph_tick(cx);
                        }),
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if let Some(graph) = &mut this.graph {
                                graph.hovered = Some(ix);
                            }
                            let surface = if is_ghost {
                                crate::menus::Surface::GraphGhost
                            } else {
                                crate::menus::Surface::GraphNode
                            };
                            this.context_menu = Some((event.position, surface));
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .size(px(r * 2.0))
                            .rounded_full()
                            .bg(node_color)
                            .hover(|s| s.bg(t.link)),
                    )
                    .child(
                        // A fixed-width box centred on the node, so the
                        // label grows sideways from the dot rather than
                        // moving it.
                        div()
                            .absolute()
                            .top(px(r * 2.0 + 2.0))
                            .left(px(r - LABEL_W / 2.0))
                            .w(px(LABEL_W))
                            .text_center()
                            .text_size(px(11.))
                            .text_color(label_color)
                            .child(SharedString::from(name)),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        // A release that ends a drag is not a click.
                        if this.graph.as_ref().is_some_and(|g| g.node_dragged) {
                            return;
                        }
                        this.open_graph_node(ix, window, cx);
                    })),
            );
        }

        // A status strip: what the view is narrowed to, and what colour
        // means. Only drawn when it has something to say, so an
        // untouched graph stays clean.
        let matches = state
            .nodes()
            .iter()
            .enumerate()
            .filter(|(ix, n)| state.filter.matches_at(*ix, n))
            .count();
        let showing_strip = state.searching || !state.filter.is_empty();
        let strip = showing_strip.then(|| {
            let mut parts: Vec<String> = Vec::new();
            if state.searching || !state.filter.query.is_empty() {
                parts.push(format!("search: {}▏", state.filter.query));
            }
            if state.filter.orphans_only {
                parts.push("orphans only".into());
            }
            if let Some(tag) = &state.filter.tag {
                parts.push(format!("#{tag}"));
            }
            if let Some(folder) = &state.filter.folder {
                parts.push(folder.clone());
            }
            parts.push(format!("{matches} of {}", state.nodes().len()));
            div()
                .absolute()
                .top(px(16.))
                .left(px(16.))
                .px_3()
                .py_1p5()
                .rounded_lg()
                .bg(t.panel_bg)
                .border_1()
                .border_color(t.border)
                .text_size(px(t.ui_size))
                .text_color(t.fg)
                .child(SharedString::from(parts.join("  ·  ")))
        });

        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(t.bg)
                .key_context("GraphView")
                .track_focus(&self.graph_focus)
                // Typing narrows the view. Only while searching, so the
                // other single-key shortcuts keep working otherwise.
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    let Some(graph) = this.graph.as_mut() else { return };
                    if !graph.searching {
                        return;
                    }
                    let key = event.keystroke.key.as_str();
                    match key {
                        "backspace" => {
                            graph.filter.query.pop();
                        }
                        "enter" => graph.searching = false,
                        _ => {
                            // A printable character, and no modifier
                            // that would make it a command.
                            if event.keystroke.modifiers.platform
                                || event.keystroke.modifiers.control
                            {
                                return;
                            }
                            let text = event
                                .keystroke
                                .key_char
                                .as_deref()
                                .unwrap_or(key);
                            if text.chars().count() == 1 {
                                graph.filter.query.push_str(text);
                            } else {
                                return;
                            }
                        }
                    }
                    cx.stop_propagation();
                    cx.notify();
                }))
                .on_action(cx.listener(Self::graph_dismiss))
                .on_action(cx.listener(Self::graph_fit))
                .on_action(cx.listener(Self::graph_color_by))
                .on_action(cx.listener(Self::graph_search))
                .on_action(cx.listener(Self::graph_orphans))
                .on_action(cx.listener(Self::graph_local))
                .on_action(cx.listener(Self::graph_depth_in))
                .on_action(cx.listener(Self::graph_depth_out))
                .on_action(cx.listener(Self::graph_freeze))
                .on_action(cx.listener(Self::graph_spread))
                .overflow_hidden()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        if let Some(graph) = &mut this.graph {
                            graph.drag =
                                Some((f32::from(event.position.x), f32::from(event.position.y)));
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                    let Some(graph) = &mut this.graph else { return };
                    let (x, y) = (f32::from(event.position.x), f32::from(event.position.y));
                    // Dragging a node: convert the pointer back into
                    // layout space and pin the node there. The rest of
                    // the graph is pushed around by it, live.
                    if let Some(ix) = graph.node_drag {
                        graph.node_dragged = true;
                        let base = 900.0 * graph.zoom;
                        // Layout space is unbounded, so a dragged node
                        // follows the pointer anywhere rather than
                        // sticking at the edge of a box.
                        let nx = (x - graph.pan.0 - 60.0) / base;
                        let ny = (y - graph.pan.1 - 60.0) / base;
                        graph.sim.pin(ix, nx, ny);
                        graph.sim.reheat(0.3);
                        cx.notify();
                        return;
                    }
                    if let Some((lx, ly)) = graph.drag {
                        graph.pan.0 += x - lx;
                        graph.pan.1 += y - ly;
                        graph.drag = Some((x, y));
                        cx.notify();
                    }
                }))
                // A release outside the window still ends the drag.
                // Without this the node stayed pinned, kept following
                // the pointer with no button held, and `hold_warm` kept
                // the frame ticker running for as long as the graph was
                // open.
                .on_mouse_up_out(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        if let Some(graph) = &mut this.graph {
                            graph.drag = None;
                            if let Some(ix) = graph.node_drag.take() {
                                graph.sim.release(ix);
                                graph.sim.hold_warm(false);
                                graph.sim.reheat(0.6);
                            }
                            graph.node_dragged = false;
                            cx.notify();
                        }
                        this.graph_tick(cx);
                    }),
                )
                .on_mouse_up(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        if let Some(graph) = &mut this.graph {
                            graph.drag = None;
                            // Letting go hands the node back to the
                            // layout, which pulls it into place instead
                            // of leaving it stranded where it was
                            // dropped.
                            if let Some(ix) = graph.node_drag.take() {
                                graph.sim.release(ix);
                                graph.sim.hold_warm(false);
                                graph.sim.reheat(0.6);
                            }
                            cx.notify();
                        }
                        this.graph_tick(cx);
                    }),
                )
                .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                    if let Some(graph) = &mut this.graph {
                        let delta = match event.delta {
                            gpui::ScrollDelta::Pixels(p) => f32::from(p.y),
                            gpui::ScrollDelta::Lines(l) => l.y * 20.0,
                        };
                        graph.zoom = (graph.zoom * (1.0 + delta * 0.002)).clamp(0.3, 3.0);
                        cx.notify();
                    }
                }))
                .child(board)
                .children(strip)
                .child(
                    div()
                        .absolute()
                        .top(px(40.))
                        .left(px(16.))
                        .flex()
                        .flex_row()
                        .gap_3()
                        .items_center()
                        .text_size(px(13.))
                        .child(div().text_color(t.fg_strong).child("Workspace graph"))
                        .child(
                            div()
                                .text_color(t.fg_muted)
                                .child("drag to pan · scroll to zoom · esc to close"),
                        ),
                )
                .into_any_element(),
        )
    }

    fn toggle_knowledge(&mut self, _: &ToggleKnowledge, _: &mut Window, cx: &mut Context<Self>) {
        self.show_knowledge = !self.show_knowledge;
        cx.notify();
    }

    /// Backlinks of the active tab's file, from the knowledge index.
    fn active_backlinks(&self, cx: &App) -> Vec<(PathBuf, Vec<String>)> {
        let Some(path) = self.tabs.get(self.active).and_then(|t| t.path(cx)) else {
            return Vec::new();
        };
        let index = self.knowledge.lock().unwrap();
        index.backlinks(&path)
    }

    /// All workspace tags with counts, for the knowledge panel.
    fn all_tags(&self) -> Vec<(String, usize)> {
        self.knowledge.lock().unwrap().tags()
    }

    fn render_knowledge(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_knowledge {
            return None;
        }
        let t = theme(cx);
        let backlinks = self.active_backlinks(cx);
        let tags = self.all_tags();

        let section = |title: &'static str| {
            div()
                .px_3()
                .pt_3()
                .pb_1()
                .text_size(px(11.))
                .text_color(t.fg_muted)
                .child(title)
        };
        let mut panel = div()
            .w(px(240.))
            .h_full()
            .flex_none()
            .bg(t.panel_bg)
            .border_l_1()
            .border_color(t.border)
            .flex()
            .flex_col()
            .overflow_hidden();

        // Local graph: the active note and its one-hop neighborhood.
        let local = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.path(cx))
            .map(|path| crate::graph::local(&self.knowledge.lock().unwrap(), &path))
            .filter(|(nodes, _)| nodes.len() > 1);
        if let Some((nodes, edges)) = local {
            let (w, h) = (216.0f32, 140.0f32);
            let at = |n: &crate::graph::GraphNode| (12.0 + n.x * w, n.y * h);
            let edge_px: Vec<((f32, f32), (f32, f32))> = edges
                .iter()
                .map(|e| (at(&nodes[e.from]), at(&nodes[e.to])))
                .collect();
            let edge_color = Hsla { a: 0.3, ..t.fg_muted };
            let canvas_el = gpui::canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    for (a, b) in &edge_px {
                        let pa = point(bounds.origin.x + px(a.0), bounds.origin.y + px(a.1));
                        let pb = point(bounds.origin.x + px(b.0), bounds.origin.y + px(b.1));
                        window.paint_path(crate::graph::line_path(pa, pb, 1.0), edge_color);
                    }
                },
            )
            .absolute()
            .size_full();
            let mut board = div().relative().h(px(h + 10.0)).w_full().child(canvas_el);
            for (ix, node) in nodes.iter().enumerate() {
                let (x, y) = at(node);
                let r = if ix == 0 { 6.0 } else { 4.0 };
                let name = node
                    .path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let path = node.path.clone();
                board = board.child(
                    div()
                        .id(("local-node", ix))
                        .absolute()
                        .left(px(x - 30.0))
                        .top(px(y - r))
                        .w(px(60.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .cursor_pointer()
                        .child(
                            div()
                                .size(px(r * 2.0))
                                .rounded_full()
                                .bg(if ix == 0 { t.accent } else { t.fg_muted }),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(t.fg_muted)
                                .overflow_hidden()
                                .truncate()
                                .child(SharedString::from(name)),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.open_path(&path.clone(), window, cx);
                        })),
                );
            }
            panel = panel.child(section("GRAPH")).child(board).child(
                div()
                    .id("open-graph")
                    .px_3()
                    .py(px(2.))
                    .text_size(px(t.ui_size - 2.))
                    .text_color(t.link)
                    .cursor_pointer()
                    .child("Open workspace graph →")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_graph_view(window, cx);
                    })),
            );
        }

        panel = panel.child(section("BACKLINKS"));
        if backlinks.is_empty() {
            panel = panel.child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(t.ui_size - 1.))
                    .text_color(t.fg_muted)
                    .child("Nothing links here yet"),
            );
        }
        for (ix, (path, contexts)) in backlinks.into_iter().enumerate() {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let context = contexts.first().cloned().unwrap_or_default();
            panel = panel.child(
                div()
                    .id(("backlink", ix))
                    .px_3()
                    .py(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover_bg))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(t.ui_size))
                            .text_color(t.fg_strong)
                            .child(SharedString::from(name)),
                    )
                    .child(
                        div()
                            .text_size(px(t.ui_size - 2.))
                            .text_color(t.fg_muted)
                            .overflow_hidden()
                            .truncate()
                            .child(SharedString::from(context)),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_path(&path.clone(), window, cx);
                    })),
            );
        }
        if !tags.is_empty() {
            panel = panel.child(section("TAGS"));
            let mut wrap = div().px_3().py_1().flex().flex_row().flex_wrap().gap_1();
            for (ix, (tag, count)) in tags.into_iter().enumerate() {
                wrap = wrap.child(
                    div()
                        .id(("tag", ix))
                        .px_2()
                        .py(px(2.))
                        .rounded_md()
                        .bg(t.hover_bg)
                        .cursor_pointer()
                        .text_size(px(t.ui_size - 2.))
                        .text_color(t.fg)
                        .child(SharedString::from(format!("#{tag} {count}")))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.open_tag_search(&tag.clone(), window, cx);
                        })),
                );
            }
            panel = panel.child(wrap);
        }
        Some(panel.into_any_element())
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
        // Followed links queued by editors (which have no window).
        for path in std::mem::take(&mut self.pending_link_opens) {
            self.open_path(&path, window, cx);
        }
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
        let knowledge = self.render_knowledge(cx);
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
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::open_folder_in_new_window))
            .on_action(cx.listener(Self::open_dialog))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::navigate_back))
            .on_action(cx.listener(Self::navigate_forward))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_outline))
            .on_action(cx.listener(Self::toggle_knowledge))
            .on_action(cx.listener(Self::toggle_finder))
            .on_action(cx.listener(Self::toggle_search))
            .on_action(cx.listener(Self::toggle_palette))
            .on_action(cx.listener(Self::open_plugins_folder))
            .on_action(cx.listener(Self::reveal_settings_folder))
            .on_action(cx.listener(Self::import_plugin))
            .on_action(cx.listener(Self::reload_plugins))
            .on_action(cx.listener(Self::make_default_markdown_app))
            .on_action(cx.listener(|this, _: &OpenRecent0, w, cx| this.open_recent_ix(0, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent1, w, cx| this.open_recent_ix(1, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent2, w, cx| this.open_recent_ix(2, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent3, w, cx| this.open_recent_ix(3, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent4, w, cx| this.open_recent_ix(4, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent5, w, cx| this.open_recent_ix(5, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent6, w, cx| this.open_recent_ix(6, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent7, w, cx| this.open_recent_ix(7, w, cx)))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::toggle_about))
            .on_action(cx.listener(Self::check_for_updates))
            .on_action(cx.listener(Self::toggle_graph))
            .on_action(cx.listener(Self::toggle_flux))
            .on_action(cx.listener(Self::install_plugins))
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
                    .children(self.show_default_handler_offer.then(|| {
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
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(t.fg)
                                    .child("Make SuperMD your default app for Markdown files?"),
                            )
                            .child(
                                div()
                                    .id("default-handler-yes")
                                    .px_2()
                                    .py(px(3.))
                                    .rounded_md()
                                    .bg(t.accent)
                                    .text_color(t.bg)
                                    .cursor_pointer()
                                    .child("Use SuperMD for Markdown Files")
                                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                                        this.make_default_markdown_app(&MakeDefaultMarkdownApp, w, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("default-handler-no")
                                    .px_2()
                                    .py(px(3.))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_color(t.fg_muted)
                                    .hover(|s| s.bg(t.hover_bg))
                                    .child("Not now")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        this.record_default_handler_answer(cx);
                                        cx.notify();
                                    })),
                            )
                    }))
                    // Deliberately its own banner, not `command_error`'s
                    // 4s auto-dismissing one: the sandboxed-refusal
                    // message is multi-sentence Finder instructions
                    // that a 4s timer would cut off mid-read.
                    .children(self.default_handler_result.clone().map(|message| {
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
                                    .id("default-handler-dismiss")
                                    .px_2()
                                    .py(px(3.))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_color(t.fg_muted)
                                    .hover(|s| s.bg(t.hover_bg))
                                    .child("Dismiss")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        this.default_handler_result = None;
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
                            .children(outline)
                            .children(knowledge),
                    )
                    .children(self.render_status_bar(cx)),
            )
            .children(self.render_shortcuts(cx))
            .children(self.render_about(cx))
            .children(self.render_theme_picker(cx))
            .children(self.render_context_menu(cx))
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
                // One row per table entry. Clicking dispatches the very
                // action the menu bar and the keystroke dispatch, so all
                // three paths are the same path.
                let item = |cmd: &'static crate::commands::Command,
                            cx: &mut Context<Self>| {
                    let shortcut = cmd
                        .keys
                        .first()
                        .map(|k| crate::platform::shortcut_glyphs(&crate::commands::glyphs(k)))
                        .unwrap_or_default();
                    let label = cmd.label;
                    div()
                        .id(SharedString::from(format!("m-{}", cmd.id)))
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
                                .child(SharedString::from(shortcut)),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.app_menu_open = false;
                            window.dispatch_action((cmd.action)(), cx);
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

                // Same grouping as the macOS menu bar — off macOS this
                // popover *is* the menu, and the two must never diverge.
                let mut recents = recents;
                let mut menu_rows: Vec<AnyElement> = Vec::new();
                for (gi, (title, cmds)) in
                    crate::commands::popover_groups().into_iter().enumerate()
                {
                    if gi > 0 {
                        menu_rows.push(divider().into_any_element());
                    }
                    menu_rows.push(
                        div()
                            .px_3()
                            .py(px(3.))
                            .text_size(px(10.))
                            .text_color(t.fg_muted)
                            .child(SharedString::from(title.to_uppercase()))
                            .into_any_element(),
                    );
                    for cmd in cmds {
                        menu_rows.push(item(cmd, cx).into_any_element());
                        if cmd.id == "open" && !recents.is_empty() {
                            menu_rows.extend(std::mem::take(&mut recents));
                        }
                    }
                }
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
                                .children(menu_rows),
                        ),
                )
            })
            .when_some(self.install_overlay.as_ref(), |root, (overlay, _)| {
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
                                this.dismiss_install_overlay(window, cx);
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
            .children(self.render_graph(cx))
            .when_some(self.move_picker.as_ref(), |root, (picker, _)| {
                let picker = picker.clone();
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
                                this.dismiss_move_picker(window, cx);
                            }),
                        )
                        .child(picker),
                )
            })
            .when_some(self.palette.as_ref(), |root, (palette, _)| {
                let palette = palette.clone();
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
                                this.dismiss_palette(window, cx);
                            }),
                        )
                        .child(
                            div()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .child(palette),
                        ),
                )
            })
            .when_some(self.consent_request.clone(), |root, (plugin, cap)| {
                let t = theme(cx);
                let msg = if let Some(domain) = cap.strip_prefix("net:") {
                    format!("Plugin \"{plugin}\" wants to access {domain}")
                } else {
                    format!("Plugin \"{plugin}\" wants to read files in this workspace")
                };
                root.child(
                    div()
                        .absolute()
                        .bottom_4()
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py(px(8.))
                                .rounded_md()
                                .bg(t.panel_bg)
                                .border_1()
                                .border_color(t.border)
                                .shadow_lg()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .text_size(px(12.))
                                .child(div().text_color(t.fg).child(SharedString::from(msg)))
                                .child(
                                    div()
                                        .id("consent-allow")
                                        .px_2()
                                        .py(px(3.))
                                        .rounded_md()
                                        .bg(t.accent)
                                        .text_color(t.bg)
                                        .cursor_pointer()
                                        .child("Allow")
                                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                            this.resolve_consent(true, cx);
                                        })),
                                )
                                .child(
                                    div()
                                        .id("consent-deny")
                                        .px_2()
                                        .py(px(3.))
                                        .rounded_md()
                                        .cursor_pointer()
                                        .text_color(t.fg_muted)
                                        .hover(|s| s.bg(t.hover_bg))
                                        .child("Deny")
                                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                            this.resolve_consent(false, cx);
                                        })),
                                ),
                        ),
                )
            })
            .when_some(self.install_request.clone(), |root, entry| {
                let t = theme(cx);
                let msg = Self::install_confirmation(&entry);
                root.child(
                    div()
                        .absolute()
                        .bottom_4()
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py(px(8.))
                                .rounded_md()
                                .bg(t.panel_bg)
                                .border_1()
                                .border_color(t.border)
                                .shadow_lg()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .text_size(px(12.))
                                .child(div().text_color(t.fg).child(SharedString::from(msg)))
                                .child(
                                    div()
                                        .id("install-allow")
                                        .px_2()
                                        .py(px(3.))
                                        .rounded_md()
                                        .bg(t.accent)
                                        .text_color(t.bg)
                                        .cursor_pointer()
                                        .child("Install")
                                        .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                                            this.resolve_install_request(true, w, cx);
                                        })),
                                )
                                .child(
                                    div()
                                        .id("install-cancel")
                                        .px_2()
                                        .py(px(3.))
                                        .rounded_md()
                                        .cursor_pointer()
                                        .text_color(t.fg_muted)
                                        .hover(|s| s.bg(t.hover_bg))
                                        .child("Cancel")
                                        .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                                            this.resolve_install_request(false, w, cx);
                                        })),
                                ),
                        ),
                )
            })
            .when_some(self.command_error.clone(), |root, msg| {
                let t = theme(cx);
                root.child(
                    div()
                        .absolute()
                        .bottom_4()
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py(px(6.))
                                .rounded_md()
                                .bg(t.diff_deleted_bg)
                                .text_size(px(12.))
                                .text_color(t.diff_deleted_fg)
                                .child(msg),
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
pub(crate) mod tests {
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
    fn git_scope_hint_only_fires_when_sandboxed_and_baseline_is_missing() {
        // (has_baseline, in_repo_subdir) -> hint
        assert_eq!(git_scope_hint(true, true), None);
        assert_eq!(git_scope_hint(true, false), None);
        assert_eq!(git_scope_hint(false, false), None);
        assert_eq!(
            git_scope_hint(false, true),
            Some("the git repository is outside the opened folder"),
        );
    }

    #[test]
    fn repo_root_is_only_ambiguous_for_a_sandboxed_non_repo_folder() {
        let dir = tempfile::tempdir().unwrap();
        // A folder carrying its own .git is never ambiguous: discover
        // stops there, inside the granted scope.
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(!repo_root_may_be_out_of_scope(dir.path()));

        let plain = tempfile::tempdir().unwrap();
        assert_eq!(
            repo_root_may_be_out_of_scope(plain.path()),
            crate::bookmarks::needs_scope()
        );
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

    pub(crate) struct TempHome {
        _dir: tempfile::TempDir,
        prev: Option<std::ffi::OsString>,
        _guard: MutexGuard<'static, ()>,
    }

    /// Serialized against every other caller of this helper across the
    /// crate (env is process-wide, so two tests swapping HOME at once
    /// race) -- not just within this file's own tests.
    pub(crate) fn temp_home() -> TempHome {
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
        open_arg(cx, Some(root.to_path_buf()))
    }

    /// Like `open_workspace`, but takes the raw launch argument so tests
    /// can exercise the single-file / welcome-document startup paths.
    /// The globals a workspace needs before it can be built or drawn.
    /// Call under `temp_home()` -- the backup registry roots under it.
    pub(crate) fn install_test_globals(cx: &mut TestAppContext) {
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
                flux_blend: 0.0,
            });
        });
    }

    fn open_arg(
        cx: &mut TestAppContext,
        arg: Option<PathBuf>,
    ) -> (Entity<Workspace>, &mut gpui::VisualTestContext) {
        install_test_globals(cx);
        let (ws, vcx) = cx.add_window_view(|_, cx| Workspace::new(arg, cx));
        // The fixture host `with_plugins` loaded stands in for the one
        // a real workspace builds from ~/.supermd/plugins. Production
        // never adopts the shared global -- see `ExtensionState`.
        vcx.update(|_, app| {
            if let Some(state) = app.try_global::<crate::extensions::ExtensionState>() {
                let handle = state.0.clone();
                ws.update(app, |ws, _| {
                    if let Some(tree) = &ws.tree {
                        handle.lock().unwrap().set_workspace_root(Some(tree.root.clone()));
                    }
                    ws.host = handle;
                });
            }
        });
        (ws, vcx)
    }

    fn tab_paths(ws: &Workspace, cx: &App) -> Vec<Option<PathBuf>> {
        ws.tabs.iter().map(|t| t.path(cx)).collect()
    }

    /// Select the sidebar row whose entry name matches, focusing the
    /// sidebar first so Sidebar-context actions dispatch.
    fn select_sidebar_row(ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext, name: &str) {
        ws.update_in(cx, |ws, window, cx| {
            window.focus(&ws.sidebar_focus);
            let rows = ws.sidebar_rows();
            let ix = rows
                .iter()
                .position(|(_, e)| e.name == name)
                .unwrap_or_else(|| panic!("row {name:?} not found"));
            ws.sidebar_selected = ix;
            cx.notify();
        });
    }

    #[gpui::test]
    fn rename_rewrites_links_in_other_notes(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Roadmap.md"), "The plan.\n").unwrap();
        std::fs::write(root.path().join("Ideas.md"), "See [[Roadmap]] often.\n").unwrap();
        std::fs::write(
            root.path().join("Log.md"),
            "Daily [note](Roadmap.md) link.\n",
        )
        .unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        select_sidebar_row(&ws, cx, "Roadmap.md");
        cx.dispatch_action(SidebarRename);
        cx.run_until_parked();
        cx.simulate_input("Vision");
        cx.dispatch_action(SidebarEditCommit);
        cx.run_until_parked();

        let ideas = std::fs::read_to_string(root.path().join("Ideas.md")).unwrap();
        assert_eq!(ideas, "See [[Vision]] often.\n");
        let log = std::fs::read_to_string(root.path().join("Log.md")).unwrap();
        assert_eq!(log, "Daily [note](Vision.md) link.\n");
    }

    /// The plugin sandbox root is per window too. It decides which
    /// directory a `workspace-read` plugin may preopen, so a host
    /// shared between windows would let a plugin invoked from one
    /// window read the other window's folder.
    #[gpui::test]
    fn two_workspaces_keep_separate_plugin_sandbox_roots(cx: &mut TestAppContext) {
        let _home = temp_home();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();

        let (ws_a, cx) = open_workspace(cx, a.path());
        let (ws_b, cx) = open_workspace(cx, b.path());
        cx.run_until_parked();

        cx.update(|_, app| {
            let root_a = ws_a.read(app).host.lock().unwrap().workspace_root();
            let root_b = ws_b.read(app).host.lock().unwrap().workspace_root();
            assert_eq!(root_a.as_deref(), Some(a.path()));
            assert_eq!(root_b.as_deref(), Some(b.path()));
            assert_ne!(root_a, root_b, "one host for both windows is the leak");
        });
    }

    /// The folder picker opens a window per folder and ignores
    /// anything that is not one — a file would give the new window no
    /// sidebar, no index and no sandbox root.
    #[gpui::test]
    fn the_folder_picker_opens_a_window_per_folder_and_skips_files(
        cx: &mut TestAppContext,
    ) {
        let _home = temp_home();
        let (root, a, _) = workspace_fixture();
        let other = tempfile::tempdir().unwrap();
        let (_ws, cx) = open_workspace(cx, root.path());
        let before = cx.update(|_, app| app.windows().len());

        let opened = cx.update(|_, app| {
            open_folders_in_new_windows(
                vec![other.path().to_path_buf(), a.clone()],
                app,
            )
        });
        cx.run_until_parked();
        assert_eq!(opened, 1, "the folder opened, the file did not");
        assert_eq!(cx.update(|_, app| app.windows().len()), before + 1);
    }

    /// ⌘Q must flush unsaved edits in *every* window, not only the one
    /// the app started in. With a single window that distinction did
    /// not exist; ⌘⇧N is what made it a way to lose work.
    #[gpui::test]
    fn quitting_flushes_dirty_editors_in_every_window(cx: &mut TestAppContext) {
        let _home = temp_home();
        let a_dir = tempfile::tempdir().unwrap();
        let b_dir = tempfile::tempdir().unwrap();
        let a = a_dir.path().join("a.md");
        let b = b_dir.path().join("b.md");
        std::fs::write(&a, "a on disk\n").unwrap();
        std::fs::write(&b, "b on disk\n").unwrap();
        install_test_globals(cx);

        let first = cx
            .update(|app| open_in_new_window(Some(a_dir.path().to_path_buf()), app))
            .expect("first window");
        let second = cx
            .update(|app| open_in_new_window(Some(b_dir.path().to_path_buf()), app))
            .expect("second window");
        cx.run_until_parked();

        // A dirty, unwritten buffer in each window.
        for (handle, path, typed) in [(&first, &a, "AA"), (&second, &b, "BB")] {
            cx.update(|app| {
                handle.update(app, |ws, window, cx| ws.open_path(path, window, cx)).unwrap();
            });
            cx.run_until_parked();
            cx.simulate_input((*handle).into(), typed);
            cx.update(|app| {
                let ws = handle.read(app).expect("window alive");
                let Some(Tab::Editor { editor, .. }) = ws.tabs.get(ws.active) else {
                    panic!("expected an editor tab");
                };
                assert!(editor.read(app).save.is_dirty(), "{typed} is unsaved");
            });
        }
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a on disk\n", "not yet written");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b on disk\n");

        let flushed = cx.update(flush_all_windows);
        assert_eq!(flushed, 2, "both windows flushed");
        assert!(std::fs::read_to_string(&a).unwrap().contains("AA"));
        assert!(
            std::fs::read_to_string(&b).unwrap().contains("BB"),
            "the second window's edits survive the quit too"
        );
    }

    /// Reload Plugins replaces the whole host inside the `HostHandle`
    /// mutex, and `ExtensionHost::load` mints a fresh root cell. Every
    /// editor built before the reload holds a clone of the *old* cell,
    /// so unless the replacement adopts it they are left reading a cell
    /// nothing writes to again — and the very next Open… in this window
    /// re-roots the host while those editors keep keying their diagram
    /// renders under the previous vault. That is the cross-vault cache
    /// leak `DiagramKey::root_hash` exists to close, coming back in by
    /// the side door.
    #[gpui::test]
    fn reloading_plugins_keeps_the_root_cell_editors_already_hold(
        cx: &mut TestAppContext,
    ) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();

        let before = cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { editor, .. }) = w.tabs.get(w.active) else {
                panic!("expected an editor tab");
            };
            let cell = editor.read(app).plugin_root_handle().expect("editor has the root cell");
            // `make_editor` hands over the host's *own* cell, not a copy.
            assert!(
                Arc::ptr_eq(&cell, &w.host.lock().unwrap().root_handle()),
                "the editor shares the host's root cell from the moment it is built"
            );
            cell
        });

        // The process-shared rootless host is rebuilt by the same
        // command. No production editor holds *its* cell, but the test
        // helpers hand editors that very handle, and `editor/mod.rs`
        // claims every host-replacing path keeps the cell -- so it must
        // be true here too, not true-with-a-footnote.
        let shared_before = cx.update(|_, app| {
            let host = crate::extensions::ExtensionHost::load(std::path::Path::new("/nonexistent"));
            app.set_global(crate::extensions::ExtensionState(Arc::new(Mutex::new(host))));
            app.global::<crate::extensions::ExtensionState>().0.lock().unwrap().root_handle()
        });

        ws.update_in(cx, |ws, window, cx| ws.reload_plugins(&ReloadPlugins, window, cx));
        cx.run_until_parked();

        cx.update(|_, app| {
            let w = ws.read(app);
            let after = w.host.lock().unwrap().root_handle();
            assert!(
                Arc::ptr_eq(&before, &after),
                "the reloaded host adopted the cell the editor still holds"
            );
            let shared_after =
                app.global::<crate::extensions::ExtensionState>().0.lock().unwrap().root_handle();
            assert!(
                Arc::ptr_eq(&shared_before, &shared_after),
                "the rebuilt shared host adopted its cell too"
            );
        });

        // And the proof that matters: re-rooting the window after a
        // reload is visible to that editor.
        let second = tempfile::tempdir().unwrap();
        let second_root = second.path().canonicalize().unwrap();
        ws.update_in(cx, |ws, window, cx| ws.open_path(&second_root, window, cx));
        cx.run_until_parked();
        assert_eq!(
            before.read().unwrap().as_deref(),
            Some(second_root.as_path()),
            "the editor's cell followed the new workspace root"
        );
    }

    /// The background plugin surfaces have no window of their own, so
    /// their failures used to reach the user only through `eprintln!`
    /// — Console.app for a Finder- or Dock-launched build, i.e. nobody.
    #[gpui::test]
    fn a_background_plugin_failure_is_shown_in_a_window(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        install_test_globals(cx);
        let window = cx
            .update(|app| open_in_new_window(Some(root.path().to_path_buf()), app))
            .expect("window");
        cx.run_until_parked();

        let shown = cx.update(|app| report_plugin_error("renderer: boom".to_string(), app));
        assert!(shown, "a live window took it");
        cx.update(|app| {
            let ws = window.read(app).expect("window alive");
            assert_eq!(
                ws.command_error.as_ref().map(|m| m.to_string()),
                Some("renderer: boom".to_string())
            );
        });

        // With every window gone it reports that it could not be shown,
        // rather than pretending it was.
        cx.update(|app| {
            window.update(app, |_, w, _| w.remove_window()).ok();
        });
        cx.run_until_parked();
        assert!(!cx.update(|app| report_plugin_error("later".to_string(), app)));
    }

    /// The drainer must *report* a failed inline render, not merely
    /// cache it. Deleting `report_plugin_error` from the drain loop
    /// left the whole suite green: the plugin author's `workspace-read`
    /// refusal, or a render panic, went to Console.app and nowhere
    /// else. `a_background_plugin_failure_is_shown_in_a_window` tests
    /// the reporting function; this tests that anything calls it.
    ///
    /// It lives here rather than beside the drainer because the thing
    /// being asserted is `Workspace::command_error`, which is private
    /// to this module.
    #[gpui::test]
    fn a_failed_inline_render_is_reported_by_the_drainer(cx: &mut TestAppContext) {
        let fixtures =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
        if !fixtures.join("panic/plugin.wasm").exists() {
            eprintln!("SKIP: fixtures not built (scripts/build_plugins.sh --fixtures)");
            return;
        }
        let _tables = crate::extensions::table_test_guard();
        let _home = temp_home();
        install_test_globals(cx);
        cx.update(|app| {
            let host = crate::extensions::ExtensionHost::load(&fixtures);
            app.set_global(crate::extensions::ExtensionState(Arc::new(Mutex::new(host))));
        });
        let folder = tempfile::tempdir().unwrap();
        let window = cx
            .update(|app| open_in_new_window(Some(folder.path().to_path_buf()), app))
            .expect("window");
        cx.run_until_parked();

        crate::extensions::clear_inline_cache();
        // The `panic` fixture never moved off wit 0.1, so every inline
        // call against it comes back an error.
        crate::extensions::enqueue_inline(vec![(
            "panic".to_string(),
            "e".to_string(),
            "x".to_string(),
        )]);
        cx.update(|app| crate::extensions::start_inline_drainer(app));
        for _ in 0..50 {
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(60));
            cx.run_until_parked();
            let shown = cx.update(|app| {
                window.read(app).map(|ws| ws.command_error.is_some()).unwrap_or(false)
            });
            if shown {
                break;
            }
        }
        cx.update(|app| {
            let ws = window.read(app).expect("window alive");
            let shown = ws.command_error.as_ref().map(|m| m.to_string());
            assert!(
                shown.as_deref().is_some_and(|m| m.contains("panic")),
                "the window names the plugin that failed: {shown:?}"
            );
        });
        crate::extensions::clear_inline_cache();
    }

    /// Re-rooting a window retires the diagrams cached against the
    /// folder it just left. They can never be *served* to the new root
    /// (the root is in the key), but a capped process-wide cache should
    /// not go on holding renders of a vault nobody has open.
    #[gpui::test]
    fn opening_another_folder_clears_the_diagram_cache(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let second = tempfile::tempdir().unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        cx.update(|_, app| {
            app.set_global(crate::diagram::DiagramCache::default());
            app.global_mut::<crate::diagram::DiagramCache>().insert(
                crate::diagram::DiagramKey {
                    source_hash: 1,
                    root_hash: crate::diagram::DiagramKey::root(Some(root.path())),
                    theme_fingerprint: 0,
                    width_bucket: 704,
                },
                crate::diagram::DiagramState::Pending,
            );
            assert_eq!(app.global::<crate::diagram::DiagramCache>().len(), 1);
        });

        ws.update_in(cx, |ws, window, cx| ws.open_path(second.path(), window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(
                app.global::<crate::diagram::DiagramCache>().len(),
                0,
                "the previous vault's renders are gone"
            );
        });
    }

    #[test]
    fn a_shortfall_is_named_only_when_something_was_missed() {
        assert_eq!(shortfall_message(2, 2), None);
        assert_eq!(shortfall_message(3, 2), None, "never a negative shortfall");
        assert_eq!(
            shortfall_message(0, 1).as_deref(),
            Some("Could not open that folder in a new window")
        );
        assert_eq!(shortfall_message(1, 3).as_deref(), Some("Opened 1 of 3 folders"));
    }

    #[test]
    fn windows_cascade_and_wrap() {
        assert_eq!(cascade_origin(0), (100.0, 60.0));
        assert_eq!(cascade_origin(1), (124.0, 84.0));
        // Wraps rather than walking off the bottom of the screen.
        assert_eq!(cascade_origin(10), cascade_origin(0));
    }

    /// The production path, not the test harness: `open_in_new_window`
    /// must build a whole second workspace — its own index and its own
    /// plugin sandbox root — on the folder it was handed, while the
    /// window it was invoked from keeps its own.
    #[gpui::test]
    fn open_in_new_window_gives_the_second_folder_its_own_workspace(
        cx: &mut TestAppContext,
    ) {
        let _home = temp_home();
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("Alpha.md"), "# Alpha\n").unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(b.path().join("Beta.md"), "# Beta\n").unwrap();

        let (ws_a, cx) = open_workspace(cx, a.path());
        let before = cx.update(|_, app| app.windows().len());
        let handle = cx
            .update(|_, app| open_in_new_window(Some(b.path().to_path_buf()), app))
            .expect("the window opens");
        cx.run_until_parked();

        assert_eq!(cx.update(|_, app| app.windows().len()), before + 1);
        cx.update(|_, app| {
            let ws_b = handle.read(app).expect("the new window holds a workspace");
            let names_b: Vec<String> = ws_b
                .knowledge
                .lock()
                .unwrap()
                .note_names()
                .iter()
                .map(|(n, _)| n.clone())
                .collect();
            assert!(names_b.iter().any(|n| n.eq_ignore_ascii_case("beta")), "{names_b:?}");
            assert!(
                !names_b.iter().any(|n| n.eq_ignore_ascii_case("alpha")),
                "the new window has its own index: {names_b:?}"
            );
            assert_eq!(
                ws_b.host.lock().unwrap().workspace_root().as_deref(),
                Some(b.path()),
                "and its own plugin sandbox root"
            );
            let names_a: Vec<String> = ws_a
                .read(app)
                .knowledge
                .lock()
                .unwrap()
                .note_names()
                .iter()
                .map(|(n, _)| n.clone())
                .collect();
            assert!(names_a.iter().any(|n| n.eq_ignore_ascii_case("alpha")), "{names_a:?}");
            assert!(
                !names_a.iter().any(|n| n.eq_ignore_ascii_case("beta")),
                "and the window it came from is untouched: {names_a:?}"
            );
        });
    }

    /// ⌘⇧N means two things, and both have to survive.
    ///
    /// `SidebarNewFolder` has had it scoped to `Sidebar` all along;
    /// `NewWindow` claims it globally. gpui scores a context-free
    /// binding at the depth of the *deepest* context, so "Sidebar" does
    /// NOT outrank "no context" — the two tie, and the tie is broken by
    /// declaration order in `commands::COMMANDS`. That is exactly the
    /// collision class that made ⌘⇧G unreachable in 0.0.15, so this
    /// asserts the real keystroke through the real keymap, both ways
    /// round, rather than trusting the scoping.
    #[gpui::test]
    fn new_window_yields_cmd_shift_n_to_a_focused_sidebar(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        cx.update(|_, app| app.bind_keys(crate::app_keybindings()));
        let chord = crate::platform::keybinding("cmd-shift-n");

        // ── sidebar focused: New Folder Here, and no new window ──
        select_sidebar_row(&ws, cx, "a.md");
        let before = cx.update(|_, app| app.windows().len());
        cx.simulate_keystrokes(&chord);
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(
                matches!(
                    ws.read(app).sidebar_edit.as_ref().map(|e| &e.kind),
                    Some(SidebarEditKind::NewDir(_))
                ),
                "the sidebar binding still wins while the sidebar has focus"
            );
            assert_eq!(app.windows().len(), before, "and no window was opened");
        });
        cx.dispatch_action(SidebarEditCancel);
        cx.run_until_parked();

        // ── sidebar not focused: New Window, and no folder edit ──
        ws.update_in(cx, |ws, window, _| window.focus(&ws.focus_handle));
        cx.run_until_parked();
        cx.simulate_keystrokes(&chord);
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(
                app.windows().len(),
                before + 1,
                "off the sidebar, ⌘⇧N opens a window"
            );
            assert!(ws.read(app).sidebar_edit.is_none(), "and creates no folder");
        });
    }

    /// Two windows, two folders, two indexes. A process-wide
    /// KnowledgeState meant the second workspace's backlinks and graph
    /// showed the first workspace's notes.
    #[gpui::test]
    fn two_workspaces_keep_separate_indexes(cx: &mut TestAppContext) {
        let _home = temp_home();
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("Alpha.md"), "# Alpha\n").unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(b.path().join("Beta.md"), "# Beta\n").unwrap();

        let (ws_a, cx) = open_workspace(cx, a.path());
        let (ws_b, cx) = open_workspace(cx, b.path());
        cx.run_until_parked();

        cx.update(|_, app| {
            let names_a: Vec<String> = ws_a.read(app).knowledge.lock().unwrap()
                .note_names().iter().map(|(n, _)| n.clone()).collect();
            let names_b: Vec<String> = ws_b.read(app).knowledge.lock().unwrap()
                .note_names().iter().map(|(n, _)| n.clone()).collect();
            assert!(names_a.iter().any(|n| n.eq_ignore_ascii_case("alpha")));
            assert!(!names_a.iter().any(|n| n.eq_ignore_ascii_case("beta")),
                "window A does not see window B's notes: {names_a:?}");
            assert!(names_b.iter().any(|n| n.eq_ignore_ascii_case("beta")));
            assert!(!names_b.iter().any(|n| n.eq_ignore_ascii_case("alpha")),
                "and the reverse: {names_b:?}");
        });
    }

    #[gpui::test]
    fn knowledge_index_tracks_saves_through_the_watcher(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("A.md"), "plain\n").unwrap();
        std::fs::write(root.path().join("B.md"), "plain\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        let b = root.path().join("B.md");
        std::fs::write(&b, "now links [[A]]\n").unwrap();
        // Deterministic: feed the drain loop's callback directly.
        ws.update_in(cx, |ws, _, cx| ws.on_fs_events(std::slice::from_ref(&b), cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let index = ws.read(app).knowledge.lock().unwrap();
            let back = index.backlinks(&root.path().join("A.md"));
            assert!(
                back.iter().any(|(p, _)| p.ends_with("B.md")),
                "index saw the save"
            );
        });
    }

    #[cfg(unix)]
    #[gpui::test]
    fn the_watcher_never_indexes_a_symlinked_note(cx: &mut TestAppContext) {
        use std::os::unix::fs::symlink;

        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("A.md"), "plain\n").unwrap();
        let outside = outside_dir.path().join("outside.md");
        std::fs::write(&outside, "secret links [[A]]\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        // A symlink created *after* the scan reaches the index only
        // through the watcher, so the scan-time filter cannot cover it.
        let leak = root.path().join("leak.md");
        symlink(&outside, &leak).unwrap();
        ws.update_in(cx, |ws, _, cx| ws.on_fs_events(std::slice::from_ref(&leak), cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let index = ws.read(app).knowledge.lock().unwrap();
            let back = index.backlinks(&root.path().join("A.md"));
            assert!(
                !back.iter().any(|(p, _)| p.ends_with("leak.md")),
                "a symlinked note must never enter the index: {back:?}"
            );
        });
    }

    /// The watcher's admission rule has to be `Index::scan`'s. A note
    /// excluded by a *nested* `.gitignore` is absent from the scan and
    /// was re-admitted by the watcher the moment the user saved it,
    /// where it stayed in backlinks, the graph and `[[` completion
    /// until the next restart -- while the sidebar, reading the same
    /// rule, drew it undimmed.
    #[gpui::test]
    fn the_watcher_applies_nested_ignore_files_too(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        let notes = root.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(root.path().join("A.md"), "plain\n").unwrap();
        std::fs::write(notes.join(".gitignore"), "private.md\n").unwrap();
        let private = notes.join("private.md");
        std::fs::write(&private, "links [[A]]\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        let a = root.path().join("A.md");
        // `A.md` rides along so the whole-batch short-circuit does not
        // return early -- the per-path check is what is on trial.
        ws.update_in(cx, |ws, _, cx| ws.on_fs_events(&[a.clone(), private.clone()], cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let index = ws.read(app).knowledge.lock().unwrap();
            let back = index.backlinks(&a);
            assert!(
                !back.iter().any(|(p, _)| p.ends_with("private.md")),
                "a nested-gitignored note must stay out of the index: {back:?}"
            );
        });
    }

    /// The other half of the same rule: `Index::scan` drops a file
    /// hardlinked to something outside the workspace, and a hardlink
    /// made while the workspace is open reaches the index only through
    /// the watcher, which had a symlink check and no hardlink check.
    #[cfg(unix)]
    #[gpui::test]
    fn the_watcher_never_indexes_a_note_hardlinked_from_outside(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("A.md"), "plain\n").unwrap();
        let outside = outside_dir.path().join("outside.md");
        std::fs::write(&outside, "secret links [[A]]\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        let leak = root.path().join("leak.md");
        if std::fs::hard_link(&outside, &leak).is_err() {
            return; // cross-device temp dirs: nothing to test here
        }
        ws.update_in(cx, |ws, _, cx| ws.on_fs_events(std::slice::from_ref(&leak), cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let index = ws.read(app).knowledge.lock().unwrap();
            let back = index.backlinks(&root.path().join("A.md"));
            assert!(
                !back.iter().any(|(p, _)| p.ends_with("leak.md")),
                "a hardlink out of the workspace must never enter the index: {back:?}"
            );
        });
    }

    #[gpui::test]
    fn on_fs_events_checks_each_path_not_just_the_batch(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("A.md"), "plain\n").unwrap();
        std::fs::create_dir(root.path().join(".git")).unwrap();
        let hidden = root.path().join(".git").join("HIDDEN.md");
        std::fs::write(&hidden, "secret links [[A]]\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        let a = root.path().join("A.md");
        // The batch carries one path `is_visible` admits (so the
        // batch-level short-circuit above does not return early) and
        // one it does not: a `.md` under `.git`, which `Index::scan`
        // never walks. Each path must be checked on its own.
        ws.update_in(cx, |ws, _, cx| ws.on_fs_events(&[a.clone(), hidden.clone()], cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let index = ws.read(app).knowledge.lock().unwrap();
            let back = index.backlinks(&a);
            assert!(
                !back.iter().any(|(p, _)| p.ends_with("HIDDEN.md")),
                "a path under .git must not enter the index just because \
                 the batch also touched a visible file: {back:?}"
            );
        });
    }

    #[gpui::test]
    fn knowledge_panel_lists_backlinks_of_the_active_note(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Roadmap.md"), "the plan\n").unwrap();
        std::fs::write(root.path().join("Ideas.md"), "see [[Roadmap]] here\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(&root.path().join("Roadmap.md"), window, cx)
        });
        cx.run_until_parked();

        cx.update(|_, app| assert!(!ws.read(app).show_knowledge, "off by default"));
        cx.dispatch_action(ToggleKnowledge);
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.show_knowledge);
            let back = w.active_backlinks(app);
            assert_eq!(back.len(), 1);
            assert!(back[0].0.ends_with("Ideas.md"));
            assert!(back[0].1[0].contains("see [[Roadmap]]"));
        });
        // The panel follows the active tab: a file nobody links to.
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(&root.path().join("Ideas.md"), window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(ws.read(app).active_backlinks(app).is_empty());
        });
    }

    #[gpui::test]
    fn tag_chips_surface_counts_and_seed_the_search(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("A.md"), "alpha #planning #q3\n").unwrap();
        std::fs::write(root.path().join("B.md"), "beta #planning\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        cx.update(|_, app| {
            let tags = ws.read(app).all_tags();
            assert_eq!(tags[0], ("planning".to_string(), 2));
            assert!(tags.contains(&("q3".to_string(), 1)));
        });

        ws.update_in(cx, |ws, window, cx| ws.open_tag_search("planning", window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let ws = ws.read(app);
            let (overlay, _) = ws.search.as_ref().expect("search overlay open");
            assert_eq!(overlay.read(app).input.read(app).content.to_string(), "#planning");
        });
    }

    #[gpui::test]
    fn graph_view_opens_navigates_and_dismisses(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Hub.md"), "to [[SpokeA]] and [[SpokeB]]\n").unwrap();
        std::fs::write(root.path().join("SpokeA.md"), "back [[Hub]]\n").unwrap();
        std::fs::write(root.path().join("SpokeB.md"), "quiet\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("supermd".into(), "__graph".into(), window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let state = ws.read(app).graph.as_ref().expect("graph open");
            assert_eq!(state.nodes().len(), 3);
            assert_eq!(state.edges().len(), 2);
        });

        // Clicking a node opens its note and closes the graph.
        ws.update_in(cx, |ws, window, cx| ws.open_graph_node(0, window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.graph.is_none(), "closed after navigation");
            assert!(w.tabs.iter().any(|t| {
                t.path(app).is_some_and(|p| p.extension().is_some_and(|e| e == "md"))
            }));
        });

        // Esc path: reopen, dismiss.
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("supermd".into(), "__graph".into(), window, cx)
        });
        cx.dispatch_action(GraphDismiss);
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app).graph.is_none()));
    }

    /// A right-click on a graph node sets `graph.hovered` and the menu
    /// dispatches `graph_local` -- which centred on the *active tab's*
    /// path instead, so right-clicking a node while another note was
    /// open centred the graph on the wrong note, and with no note open
    /// it answered "Open a note to centre the graph on it" to a click
    /// that named a specific node.
    #[gpui::test]
    fn the_graph_menu_centres_on_the_clicked_node(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Hub.md"), "to [[SpokeA]] and [[SpokeB]]\n").unwrap();
        std::fs::write(root.path().join("SpokeA.md"), "back [[Hub]]\n").unwrap();
        std::fs::write(root.path().join("SpokeB.md"), "quiet\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        // A note open in the active tab -- the thing graph_local used
        // to centre on no matter which node was clicked.
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(&root.path().join("Hub.md"), window, cx)
        });
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("supermd".into(), "__graph".into(), window, cx)
        });
        cx.run_until_parked();

        let (hub, spoke) = cx.update(|_, app| {
            let nodes = ws.read(app).graph.as_ref().expect("graph open").nodes();
            let ix = |name: &str| {
                nodes.iter().position(|n| n.path.ends_with(name)).expect(name)
            };
            (ix("Hub.md"), ix("SpokeB.md"))
        });
        assert_ne!(hub, spoke);

        // What the right-click handler does before raising the menu.
        ws.update_in(cx, |ws, _, cx| {
            ws.graph.as_mut().unwrap().hovered = Some(spoke);
            cx.notify();
        });
        ws.update_in(cx, |ws, window, cx| ws.graph_local(&GraphLocal, window, cx));
        cx.update(|_, app| {
            let graph = ws.read(app).graph.as_ref().expect("graph open");
            assert_eq!(
                graph.filter.local,
                Some((spoke, 1)),
                "centred on the clicked node, not on the open note"
            );
        });

        // Off again, then with no node under the pointer it falls back
        // to the open note exactly as the keyboard action always has.
        ws.update_in(cx, |ws, window, cx| ws.graph_local(&GraphLocal, window, cx));
        ws.update_in(cx, |ws, _, cx| {
            ws.graph.as_mut().unwrap().hovered = None;
            cx.notify();
        });
        ws.update_in(cx, |ws, window, cx| ws.graph_local(&GraphLocal, window, cx));
        cx.update(|_, app| {
            let graph = ws.read(app).graph.as_ref().expect("graph open");
            assert_eq!(graph.filter.local, Some((hub, 1)), "the open note");
        });
    }

    /// `FsEntry.ignored` is computed and tested; this is the only test
    /// that anything *reads* it. Without the dim, an ignored file looks
    /// exactly like an indexed one and the sidebar quietly stops being
    /// the place you can tell them apart.
    #[test]
    fn an_ignored_sidebar_row_is_dimmed() {
        let t = crate::theme::Theme::dark();
        assert_eq!(sidebar_row_color(true, false, &t), t.fg_muted, "an ignored file recedes");
        assert_eq!(sidebar_row_color(true, true, &t), t.fg_muted, "an ignored folder too");
        assert_ne!(sidebar_row_color(false, false, &t), t.fg_muted, "an indexed file does not");
        assert_eq!(sidebar_row_color(false, false, &t), t.fg);
        assert_eq!(sidebar_row_color(false, true, &t), t.fg_strong, "folders lead");
    }

    #[gpui::test]
    fn sidebar_rename_updates_disk_and_open_tab(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();

        select_sidebar_row(&ws, cx, "a.md");
        cx.dispatch_action(SidebarRename);
        cx.run_until_parked();
        // The stem is pre-selected: typing replaces it, extension stays.
        cx.simulate_input("renamed");
        cx.dispatch_action(SidebarEditCommit);
        cx.run_until_parked();

        assert!(root.path().join("renamed.md").exists());
        assert!(!a.exists());
        cx.update(|_, app| {
            let ws = ws.read(app);
            assert!(ws.sidebar_edit.is_none(), "edit row closed");
            let paths = tab_paths(ws, app);
            assert!(
                paths.iter().flatten().any(|p| p.ends_with("renamed.md")),
                "open tab retargeted: {paths:?}"
            );
        });
    }

    #[gpui::test]
    fn sidebar_rename_to_duplicate_stays_editing_with_error(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        select_sidebar_row(&ws, cx, "a.md");
        cx.dispatch_action(SidebarRename);
        cx.run_until_parked();
        cx.simulate_input("b"); // stem selected → becomes "b.md"
        cx.dispatch_action(SidebarEditCommit);
        cx.run_until_parked();
        cx.update(|_, app| {
            let ws = ws.read(app);
            let edit = ws.sidebar_edit.as_ref().expect("still editing");
            assert!(edit.error.as_ref().unwrap().contains("already exists"));
        });
        cx.dispatch_action(SidebarEditCancel);
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app).sidebar_edit.is_none()));
        assert!(root.path().join("a.md").exists(), "cancel leaves disk alone");
    }

    #[gpui::test]
    fn sidebar_new_file_and_folder_target_the_selection(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _, _) = workspace_fixture();
        std::fs::create_dir(root.path().join("sub")).unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        // New folder beside a selected file lands in the root.
        select_sidebar_row(&ws, cx, "a.md");
        cx.dispatch_action(SidebarNewFolder);
        cx.run_until_parked();
        cx.simulate_input("ideas");
        cx.dispatch_action(SidebarEditCommit);
        cx.run_until_parked();
        assert!(root.path().join("ideas").is_dir());

        // New file inside a selected folder lands in that folder and opens.
        select_sidebar_row(&ws, cx, "sub");
        cx.dispatch_action(SidebarNewFile);
        cx.run_until_parked();
        cx.simulate_input("inner.md");
        cx.dispatch_action(SidebarEditCommit);
        cx.run_until_parked();
        assert!(root.path().join("sub/inner.md").exists());
        cx.update(|_, app| {
            let paths = tab_paths(ws.read(app), app);
            assert!(paths.iter().flatten().any(|p| p.ends_with("sub/inner.md")), "{paths:?}");
        });
    }

    #[gpui::test]
    fn sidebar_delete_trashes_and_closes_the_tab(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _) = workspace_fixture();
        let trashed: std::sync::Arc<Mutex<Vec<PathBuf>>> = Default::default();
        let (ws, cx) = open_workspace(cx, root.path());
        cx.update(|_, app| {
            let sink = trashed.clone();
            app.set_global(crate::fileops::TrashFn(std::sync::Arc::new(move |p| {
                sink.lock().unwrap().push(p.to_path_buf());
                std::fs::remove_file(p).map_err(|e| e.to_string())
            })));
        });
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();

        select_sidebar_row(&ws, cx, "a.md");
        cx.dispatch_action(SidebarDelete);
        cx.run_until_parked();
        assert_eq!(trashed.lock().unwrap().as_slice(), &[a.clone()]);
        assert!(!a.exists());
        cx.update(|_, app| {
            let paths = tab_paths(ws.read(app), app);
            assert!(!paths.iter().flatten().any(|p| p == &a), "tab closed: {paths:?}");
        });
    }

    #[gpui::test]
    fn sidebar_move_picker_relocates_and_retargets(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _) = workspace_fixture();
        std::fs::create_dir(root.path().join("sub")).unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();

        select_sidebar_row(&ws, cx, "a.md");
        cx.dispatch_action(SidebarMoveTo);
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app).move_picker.is_some(), "picker open"));
        cx.simulate_input("sub");
        cx.run_until_parked();
        cx.dispatch_action(crate::palette::PaletteConfirm);
        cx.run_until_parked();

        assert!(root.path().join("sub/a.md").exists());
        assert!(!a.exists());
        cx.update(|_, app| {
            let ws = ws.read(app);
            assert!(ws.move_picker.is_none(), "picker closed");
            let paths = tab_paths(ws, app);
            assert!(paths.iter().flatten().any(|p| p.ends_with("sub/a.md")), "{paths:?}");
        });
    }

    /// Graph, flux and Install were palette-only string ids, which is
    /// why they reached no menu and two had no shortcut. They are real
    /// actions now; the palette entries delegate to the same handlers.
    #[gpui::test]
    fn status_bar_renders_and_its_flux_toggle_works(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        cx.update(|window, app| {
            let handle = ws.read(app).focus_handle(app);
            window.focus(&handle);
        });
        cx.run_until_parked();

        ws.update_in(cx, |ws, _, cx| {
            assert!(ws.render_status_bar(cx).is_some(), "the strip renders");
        });

        // The sun icon drives the same handler the menu and palette do.
        let before =
            cx.update(|_, app| app.global::<crate::theme::ThemeState>().settings.flux.enabled);
        cx.dispatch_action(ToggleFlux);
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_ne!(
                app.global::<crate::theme::ThemeState>().settings.flux.enabled,
                before,
                "flux flipped"
            );
        });

        // Focus mode is meant to be bare: no strip.
        cx.dispatch_action(ToggleFocusMode);
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| {
            assert!(ws.render_status_bar(cx).is_none(), "hidden in focus mode");
        });
    }

    #[gpui::test]
    fn titlebar_chrome_tracks_panel_state_and_hides_in_focus_mode(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.md"), "# A\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        cx.update(|window, app| {
            let handle = ws.read(app).focus_handle(app);
            window.focus(&handle);
        });
        cx.run_until_parked();

        ws.update_in(cx, |ws, _, cx| {
            assert!(ws.render_titlebar_chrome(cx).is_some(), "chrome renders");
        });
        // Focus mode is a deliberately bare surface: no chrome at all.
        cx.dispatch_action(ToggleFocusMode);
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| {
            assert!(ws.focus_mode, "focus mode is on");
            assert!(ws.render_titlebar_chrome(cx).is_none(), "chrome hides");
        });
        cx.dispatch_action(ToggleFocusMode);
        cx.run_until_parked();

        // The toggles drive the same state the keyboard does.
        let before = cx.update(|_, app| ws.read(app).show_knowledge);
        cx.dispatch_action(ToggleKnowledge);
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_ne!(ws.read(app).show_knowledge, before, "knowledge panel toggled");
        });
    }

    #[gpui::test]
    fn about_dialog_toggles_and_reports_the_running_version(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        cx.update(|window, app| {
            let handle = ws.read(app).focus_handle(app);
            window.focus(&handle);
        });
        cx.run_until_parked();
        cx.update(|_, app| assert!(!ws.read(app).show_about, "closed by default"));

        cx.dispatch_action(ToggleAbout);
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| {
            assert!(ws.show_about, "About opens");
            assert!(ws.render_about(cx).is_some(), "the dialog renders");
            // Nothing has answered yet, so the dialog offers a check and
            // no download.
            assert_eq!(
                crate::update::update_status(
                    env!("CARGO_PKG_VERSION"),
                    ws.about_latest.as_ref().map(|t| t.as_ref()),
                    ws.about_checking,
                ),
                crate::update::UpdateStatus::Unknown
            );
        });

        cx.dispatch_action(ToggleAbout);
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| {
            assert!(!ws.show_about, "toggles shut");
            assert!(ws.render_about(cx).is_none(), "and stops rendering");
        });
    }

    #[gpui::test]
    fn graph_and_flux_are_actions_not_only_palette_strings(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        cx.update(|window, app| {
            let handle = ws.read(app).focus_handle(app);
            window.focus(&handle);
        });
        cx.run_until_parked();

        cx.dispatch_action(ToggleFlux);
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(
                app.global::<crate::theme::ThemeState>().settings.flux.enabled,
                "ToggleFlux enables flux, as the palette entry did"
            );
        });

        cx.dispatch_action(ToggleGraph);
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(ws.read(app).graph.is_some(), "ToggleGraph opens the graph");
        });
    }

    #[gpui::test]
    fn flux_toggle_lives_in_the_palette_and_persists(cx: &mut TestAppContext) {
        let home = temp_home();
        let root = tempfile::tempdir().unwrap();
        let (_ws, cx) = open_workspace(cx, root.path());
        cx.update(|window, app| {
            let handle = _ws.read(app).focus_handle(app);
            window.focus(&handle);
        });
        // No ExtensionState global: the toggle must be there anyway.
        cx.dispatch_action(TogglePalette);
        cx.run_until_parked();
        cx.update(|_, app| assert!(_ws.read(app).palette.is_some(), "palette open"));
        cx.simulate_input("Flux");
        cx.run_until_parked();
        cx.dispatch_action(crate::palette::PaletteConfirm);
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(
                app.global::<crate::theme::ThemeState>().settings.flux.enabled,
                "palette toggle enables flux"
            );
        });
        let saved =
            std::fs::read_to_string(crate::settings::config_dir().join("settings.toml")).unwrap();
        assert!(saved.contains("[flux]") && saved.contains("enabled = true"), "{saved}");

        // Toggling again flips it back off.
        cx.dispatch_action(TogglePalette);
        cx.simulate_input("Flux");
        cx.dispatch_action(crate::palette::PaletteConfirm);
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(!app.global::<crate::theme::ThemeState>().settings.flux.enabled);
        });
        drop(home);
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

    #[gpui::test]
    fn navigate_back_and_forward_walk_open_tabs(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let c = root.path().join("c.md");
        std::fs::write(&c, "# c\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        // Give history real depth, then revisit `a` so the visit order
        // (a, b, c, a) diverges from tab order (a, b, c) — this rules out
        // a handler that merely walks adjacent tab indices rather than
        // real history.
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path(&b, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path(&c, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(tab_paths(w, app), vec![Some(a.clone()), Some(b.clone()), Some(c.clone())]);
            assert_eq!(w.tabs[w.active].path(app), Some(a.clone()), "revisit reuses a's tab");
        });

        // Actions dispatched at the focused editor bubble to the workspace,
        // same as the NextTab/PrevTab wiring above.
        cx.dispatch_action(NavigateBack);
        let first_back = cx.update(|_, app| ws.read(app).tabs[ws.read(app).active].path(app));
        assert_eq!(
            first_back,
            Some(c.clone()),
            "first back lands on the previously visited file, not the tab to a's left"
        );

        cx.dispatch_action(NavigateBack);
        let second_back = cx.update(|_, app| ws.read(app).tabs[ws.read(app).active].path(app));
        assert_eq!(
            second_back,
            Some(b.clone()),
            "second back keeps walking backwards through real history"
        );
        assert_ne!(
            first_back, second_back,
            "two consecutive backs must not oscillate between the same two files"
        );

        cx.dispatch_action(NavigateForward);
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(
                w.tabs[w.active].path(app),
                Some(c.clone()),
                "forward moves forward from the second back to the first back's target"
            );
        });
    }

    /// Switching tabs is navigation. When history does not see it, the
    /// cursor stays where the last `open_path` left it, and Back moves
    /// you *forward*: open a, b, c, walk back to a with ⌘⇧[, and the
    /// stack still reads (a, b, c) at c.
    #[gpui::test]
    fn tab_switching_is_recorded_in_history(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let c = root.path().join("c.md");
        std::fs::write(&c, "# c\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        for path in [&a, &b, &c] {
            ws.update_in(cx, |ws, window, cx| ws.open_path(path, window, cx));
        }
        cx.run_until_parked();
        // ⌘⇧[ twice: c → b → a, without touching open_path.
        cx.dispatch_action(PrevTab);
        cx.dispatch_action(PrevTab);
        cx.run_until_parked();
        let active = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, app| ws.read(app).tabs[ws.read(app).active].path(app))
        };
        assert_eq!(active(cx), Some(a.clone()), "precondition: ⌘⇧[ reached a");

        cx.dispatch_action(NavigateBack);
        assert_eq!(active(cx), Some(b.clone()), "back returns to where we just were");
        cx.dispatch_action(NavigateBack);
        assert_eq!(
            active(cx), Some(c.clone()),
            "back keeps walking backwards through the tab switches"
        );
        cx.dispatch_action(NavigateForward);
        assert_eq!(active(cx), Some(b.clone()), "forward retraces the same steps");
    }

    /// Sidebar single-click and keyboard browsing both go through
    /// `open_path_preview`, which recorded nothing at all.
    #[gpui::test]
    fn sidebar_preview_opens_are_recorded_in_history(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, true, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&b, true, window, cx));
        cx.run_until_parked();
        let active = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, app| ws.read(app).tabs[ws.read(app).active].path(app))
        };
        assert_eq!(active(cx), Some(b.clone()), "precondition: previewing b");

        ws.update_in(cx, |ws, window, cx| ws.navigate_back(&NavigateBack, window, cx));
        cx.run_until_parked();
        assert_eq!(active(cx), Some(a.clone()), "back returns to the previewed file");

        // Back and forward are moves *through* history, not new visits:
        // two backs in a row keep walking backwards rather than
        // oscillating between the same two files.
        ws.update_in(cx, |ws, window, cx| ws.navigate_forward(&NavigateForward, window, cx));
        cx.run_until_parked();
        assert_eq!(active(cx), Some(b.clone()), "forward returns to b");
        ws.update_in(cx, |ws, window, cx| ws.navigate_back(&NavigateBack, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.navigate_back(&NavigateBack, window, cx));
        cx.run_until_parked();
        assert_eq!(
            active(cx), Some(a.clone()),
            "navigation must not stack new entries: the history is still just a, b"
        );
    }

    /// Keyboard browsing previews every row the arrows pass over. Those
    /// are glances, not navigations: recording them would push an entry
    /// per keypress and truncate the forward branch, so holding the arrow
    /// key through a vault would destroy Forward and fill Back with files
    /// the user never chose to open. Only a focused open is a visit.
    ///
    /// Needs a third file: with only the two the shared fixture makes,
    /// every row the arrows reach is already the current history entry
    /// and `History::visit`'s dedup hides the bug.
    #[gpui::test]
    fn keyboard_sidebar_browsing_is_not_recorded_in_history(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a.md");
        let b = root.path().join("b.md");
        let c = root.path().join("c.md");
        std::fs::write(&a, "# a\n").unwrap();
        std::fs::write(&b, "# b\n").unwrap();
        std::fs::write(&c, "# c\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, true, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&b, true, window, cx));
        cx.run_until_parked();
        let active = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, app| ws.read(app).tabs[ws.read(app).active].path(app))
        };

        // Step back to a, leaving b on the forward branch.
        ws.update_in(cx, |ws, window, cx| ws.navigate_back(&NavigateBack, window, cx));
        cx.run_until_parked();
        assert_eq!(active(cx), Some(a.clone()), "precondition: back on a");

        // Scrub down to c with the keyboard, exactly as SidebarDown does:
        // an unfocused preview of every row it lands on. c is not in
        // history, so a recorded visit here truncates b off the forward
        // branch.
        for _ in 0..2 {
            ws.update_in(cx, |ws, window, cx| ws.sidebar_move(1, window, cx));
            cx.run_until_parked();
        }
        assert_eq!(active(cx), Some(c.clone()), "precondition: scrubbed onto c");

        ws.update_in(cx, |ws, window, cx| ws.navigate_forward(&NavigateForward, window, cx));
        cx.run_until_parked();
        assert_eq!(
            active(cx), Some(b.clone()),
            "arrow-key browsing must not truncate the forward branch"
        );
    }

    /// A rename moves a file; history still names where it was. Forward
    /// then walks its cursor forward and opens nothing at all.
    #[gpui::test]
    fn history_follows_a_renamed_file(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        for path in [&a, &b] {
            ws.update_in(cx, |ws, window, cx| ws.open_path(path, window, cx));
        }
        cx.run_until_parked();
        let active = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, app| ws.read(app).tabs[ws.read(app).active].path(app))
        };
        ws.update_in(cx, |ws, window, cx| ws.navigate_back(&NavigateBack, window, cx));
        assert_eq!(active(cx), Some(a.clone()), "precondition: back reached a");

        let renamed = root.path().join("b-renamed.md");
        std::fs::rename(&b, &renamed).unwrap();
        ws.update_in(cx, |ws, _, cx| ws.after_path_change(&b, &renamed, cx));
        cx.run_until_parked();

        ws.update_in(cx, |ws, window, cx| ws.navigate_forward(&NavigateForward, window, cx));
        cx.run_until_parked();
        assert_eq!(
            active(cx), Some(renamed),
            "forward must follow the file to its new name, not dead-step"
        );
    }

    /// Opening another folder replaces the workspace. Its history is
    /// about files that are no longer reachable — and under the App
    /// Store sandbox they sit outside the active security-scoped
    /// bookmark, so Back would fail silently.
    #[gpui::test]
    fn switching_workspaces_clears_history(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        for path in [&a, &b] {
            ws.update_in(cx, |ws, window, cx| ws.open_path(path, window, cx));
        }
        cx.run_until_parked();
        let active = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, app| ws.read(app).tabs[ws.read(app).active].path(app))
        };

        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("x.md"), "# x\n").unwrap();
        ws.update_in(cx, |ws, window, cx| ws.open_path(other.path(), window, cx));
        cx.run_until_parked();

        ws.update_in(cx, |ws, window, cx| ws.navigate_back(&NavigateBack, window, cx));
        cx.run_until_parked();
        assert_eq!(
            active(cx), Some(b.clone()),
            "back must not walk into the previous workspace"
        );
    }

    /// A deleted entry is stepped over, not stepped onto: the old code
    /// moved the cursor onto it and then opened nothing (or, worse,
    /// re-activated the stale tab).
    #[gpui::test]
    fn navigation_skips_files_that_no_longer_exist(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let c = root.path().join("c.md");
        std::fs::write(&c, "# c\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        for path in [&a, &b, &c] {
            ws.update_in(cx, |ws, window, cx| ws.open_path(path, window, cx));
        }
        cx.run_until_parked();
        let active = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, app| ws.read(app).tabs[ws.read(app).active].path(app))
        };

        std::fs::remove_file(&b).unwrap();
        ws.update_in(cx, |ws, window, cx| ws.navigate_back(&NavigateBack, window, cx));
        cx.run_until_parked();
        assert_eq!(
            active(cx), Some(a.clone()),
            "back skips the deleted entry and lands on the live one"
        );
        ws.update_in(cx, |ws, window, cx| ws.navigate_forward(&NavigateForward, window, cx));
        cx.run_until_parked();
        assert_eq!(active(cx), Some(c.clone()), "forward skips it too");
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

    /// Every persisted setting is a read-modify-write against *disk*.
    /// The copy in the `ThemeState` global was seeded once at launch
    /// and knows nothing of what has been written since -- recents (and
    /// under the sandbox the security-scoped bookmarks that hang off
    /// them), plugin grants, net-domain grants. Saving that whole
    /// struct back silently reverts all of it; only the one field the
    /// user just changed may travel to disk.
    #[gpui::test]
    fn saving_one_setting_keeps_what_disk_gained_since_launch(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        let dir = crate::settings::config_dir();

        // What a fresh-load writer (record_recent, resolve_consent,
        // enable_previews_for_hovered_site) leaves behind mid-session.
        let stamp = |dir: &Path| {
            let mut s = crate::settings::load(dir);
            s.recent_workspaces = vec!["/opened/after/launch".to_string()];
            s.plugin_grants.insert("demo".into(), vec!["workspace-read".into()]);
            crate::settings::save(dir, &s).unwrap();
        };
        let still_there = |dir: &Path, who: &str| {
            let s = crate::settings::load(dir);
            assert_eq!(s.recent_workspaces, ["/opened/after/launch"], "{who} wiped the recents");
            assert!(s.plugin_grants.contains_key("demo"), "{who} wiped the plugin grants");
        };

        stamp(&dir);
        ws.update_in(cx, |ws, _w, cx| ws.record_default_handler_answer(cx));
        still_there(&dir, "the default-handler banner");
        assert!(crate::settings::load(&dir).default_handler_asked, "its own field did persist");

        stamp(&dir);
        let was = cx.update(|_, app| app.global::<crate::theme::ThemeState>().settings.flux.enabled);
        ws.update_in(cx, |ws, window, cx| ws.toggle_flux(&ToggleFlux, window, cx));
        still_there(&dir, "the flux toggle");
        assert_eq!(crate::settings::load(&dir).flux.enabled, !was, "its own field did persist");

        stamp(&dir);
        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_theme_picker(&ToggleThemePicker, window, cx)
        });
        ws.update_in(cx, |ws, window, cx| ws.theme_picker_down(&ThemePickerDown, window, cx));
        let picked = cx.update(|_, app| {
            let w = ws.read(app);
            let picker = w.theme_picker.as_ref().expect("picker open");
            app.global::<crate::theme::ThemeState>().themes[picker.order[picker.pos]].name.clone()
        });
        ws.update_in(cx, |ws, window, cx| {
            ws.theme_picker_confirm(&ThemePickerConfirm, window, cx)
        });
        still_there(&dir, "the theme picker");
        assert_eq!(crate::settings::load(&dir).light_theme, picked, "its own field did persist");
    }

    // ── edit/preview flip, new file ─────────────────────────────────────

    /// Previewing a code file must render it as code. It used to hand
    /// the source to the CommonMark parser: doc comments became
    /// paragraphs, the source was reflowed, and any four-space-indented
    /// block turned into an indented code block, so a Rust file read as
    /// mangled prose.
    /// A single click is a look; a double click is an edit. Both give
    /// a tab, but only the double click gives an editable one — and
    /// this holds for every file type, not just Markdown.
    /// Looking at a file must not change it. Opening a preview used to
    /// call `Editor::flush`, which runs the save hooks *before* its
    /// dirty check -- a hook that rewrites the document marks it dirty
    /// and the file is written. Browsing the sidebar therefore rewrote
    /// and saved notes nobody had edited, and dirtied the git tree.
    #[gpui::test]
    fn browsing_a_file_does_not_write_it(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let before = std::fs::read_to_string(&a).unwrap();
        let mtime = std::fs::metadata(&a).unwrap().modified().unwrap();

        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, true, window, cx));
        cx.run_until_parked();
        // And again, the way arrowing through a sidebar does.
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, false, window, cx));
        cx.run_until_parked();

        assert_eq!(std::fs::read_to_string(&a).unwrap(), before, "contents untouched");
        assert_eq!(
            std::fs::metadata(&a).unwrap().modified().unwrap(),
            mtime,
            "and the file was not rewritten at all"
        );
    }

    #[gpui::test]
    fn single_click_previews_and_double_click_edits(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let rs = root.path().join("code.rs");
        std::fs::write(&rs, "fn main() {}\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        // Single click: read-only, and non-sticky.
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&rs, true, window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(
                matches!(
                    w.tabs.get(w.active),
                    Some(Tab::Editor { view: EditorView::Preview(_), .. })
                ),
                "a single click opens read-only, code included"
            );
            assert_eq!(w.preview_tab, Some(w.active), "and the tab stays non-sticky");
        });

        // Double click goes through `open_path`: editable, and pinned.
        ws.update_in(cx, |ws, window, cx| ws.open_path(&rs, window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(
                matches!(w.tabs.get(w.active), Some(Tab::Editor { view: EditorView::Edit, .. })),
                "a double click opens for editing"
            );
        });
    }

    /// A link clicked in the rendered preview must navigate. The
    /// reading view had no link handling at all: `markdown.rs` recorded
    /// only *that* a run was a link and dropped the destination, so a
    /// preview could draw links it could never follow — which became
    /// the common case when a single click started opening previews.
    /// A preview reader must know which file it is showing, or a
    /// relative link in it has no base to resolve against and its hover
    /// preview silently shows nothing.
    /// A wiki link clicked in the reading view must open its note. The
    /// reader built every link as `wiki: false`, so `[[Editing]]` was
    /// classified as a relative path, matched no file, and did nothing —
    /// in the view a single click now opens, in the release named
    /// "links that work".
    #[gpui::test]
    fn a_wiki_link_in_the_rendered_preview_opens_its_note(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let stem = b.file_stem().unwrap().to_string_lossy().into_owned();
        std::fs::write(&a, format!("see [[{stem}]]\n")).unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, true, window, cx));
        cx.run_until_parked();

        // The destination carries the wiki marker through parsing.
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { view: EditorView::Preview(reader), .. }) = w.tabs.get(w.active)
            else {
                panic!("expected a preview tab")
            };
            let doc = &reader.read(app).document;
            let crate::markdown::Block::Paragraph(inline) = &doc.blocks[0] else {
                panic!("paragraph")
            };
            assert_eq!(inline.links[0].1, format!("[[{stem}"), "marked as a wiki target");
        });

        ws.update(cx, |ws, cx| ws.follow_from_reader(&format!("[[{stem}"), cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(
                w.tabs[w.active].path(app).as_deref(),
                Some(b.as_path()),
                "the wiki link opened its note"
            );
        });
    }

    #[gpui::test]
    fn a_preview_reader_knows_its_own_file(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, true, window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { view: EditorView::Preview(reader), .. }) = w.tabs.get(w.active)
            else {
                panic!("expected a preview tab")
            };
            assert_eq!(
                reader.read(app).path.as_deref(),
                Some(a.as_path()),
                "the reader carries the file it renders"
            );
        });
    }

    #[gpui::test]
    fn clicking_a_link_in_the_rendered_preview_navigates(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        std::fs::write(&a, format!("see [b]({})\n", b.file_name().unwrap().to_string_lossy()))
            .unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, true, window, cx));
        cx.run_until_parked();

        // The destination survived parsing into the rendered document.
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { view: EditorView::Preview(reader), .. }) = w.tabs.get(w.active)
            else {
                panic!("expected a preview tab")
            };
            let doc = &reader.read(app).document;
            let crate::markdown::Block::Paragraph(inline) = &doc.blocks[0] else {
                panic!("paragraph")
            };
            assert_eq!(inline.links.len(), 1, "the rendered view knows where the link goes");
        });

        // Following it opens the target.
        ws.update(cx, |ws, cx| ws.follow_from_reader("b.md", cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(
                w.tabs[w.active].path(app).as_deref(),
                Some(b.as_path()),
                "the click opened the linked note"
            );
        });
    }

    #[gpui::test]
    fn previewing_a_code_file_renders_code_not_prose(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let rs = root.path().join("sample.rs");
        // Every shape that the Markdown parser used to mangle.
        std::fs::write(
            &rs,
            "//! Doc comment.\n\n#[derive(Debug)]\nstruct S;\n\n    fn indented() {}\n",
        )
        .unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&rs, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.toggle_preview(&TogglePreview, window, cx));
        cx.run_until_parked();

        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { view: EditorView::Preview(reader), .. }) = w.tabs.get(w.active)
            else {
                panic!("expected a preview tab")
            };
            // One code block holding the whole file: nothing should
            // have been parsed as a paragraph, a heading, or an
            // indented code block.
            let blocks = &reader.read(app).document.blocks;
            assert_eq!(blocks.len(), 1, "the file renders as one block: {blocks:#?}");
            let crate::markdown::Block::Code { lang, code, .. } = &blocks[0] else {
                panic!("that block is code, not prose: {:?}", blocks[0])
            };
            assert_eq!(lang.as_deref(), Some("rust"), "highlighted as Rust");
            assert!(code.contains("#[derive(Debug)]"), "the source survives verbatim");
            assert!(code.contains("    fn indented() {}"), "indentation preserved");
        });
    }

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

    // ── pure helpers ────────────────────────────────────────────────────

    #[test]
    fn seti_tint_maps_every_palette_color() {
        let t = crate::theme::Theme::dark();
        assert_eq!(seti_tint(SetiColor::Blue, &t), t.syntax.function);
        assert_eq!(seti_tint(SetiColor::Green, &t), t.syntax.string);
        assert_eq!(seti_tint(SetiColor::Grey, &t), t.fg_muted);
        assert_eq!(seti_tint(SetiColor::Ignore, &t), t.fg_muted);
        assert_eq!(seti_tint(SetiColor::GreyLight, &t), t.fg);
        assert_eq!(seti_tint(SetiColor::Orange, &t), t.syntax.constant);
        assert_eq!(seti_tint(SetiColor::SetiPrimary, &t), t.syntax.constant);
        assert_eq!(seti_tint(SetiColor::Pink, &t), t.syntax.property);
        assert_eq!(seti_tint(SetiColor::Purple, &t), t.syntax.keyword);
        assert_eq!(seti_tint(SetiColor::Red, &t), t.accent);
        assert_eq!(seti_tint(SetiColor::White, &t), t.fg);
        assert_eq!(seti_tint(SetiColor::Yellow, &t), t.syntax.kind);
    }

    #[test]
    fn welcome_file_write_failure_still_returns_path() {
        // A config "dir" that is actually a file: create_dir_all and the
        // write both fail; the function still hands back the would-be path.
        let dir = tempfile::tempdir().unwrap();
        let bogus = dir.path().join("not-a-dir");
        std::fs::write(&bogus, "file").unwrap();
        let p = ensure_welcome_file(&bogus);
        assert_eq!(p, bogus.join("Welcome.md"));
        assert!(!p.exists());
    }

    #[test]
    fn record_recent_tolerates_unwritable_config() {
        let home = temp_home();
        // ~/.supermd exists as a *file*, so settings::save must fail;
        // record_recent swallows the error.
        std::fs::write(home._dir.path().join(".supermd"), "block").unwrap();
        let ws_dir = tempfile::tempdir().unwrap();
        record_recent(ws_dir.path());
    }

    /// A corrupt settings.toml must reach the user, not just an
    /// eprintln! nobody sees in a packaged app -- and it has to happen
    /// on this same startup, because record_recent's load-mutate-save
    /// (triggered by opening this very workspace) is what overwrites
    /// the preserved copy with defaults right afterward.
    #[gpui::test]
    fn corrupt_settings_file_surfaces_a_command_error_at_startup(cx: &mut TestAppContext) {
        let _home = temp_home();
        let config = crate::settings::config_dir();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("settings.toml"), "this is not = valid toml [[[").unwrap();

        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        cx.update(|_, app| {
            let w = ws.read(app);
            let msg = w.command_error.clone().expect("the corruption must surface, not just print");
            assert!(msg.contains("settings.toml.corrupt"), "message should name the recovery file: {msg}");
        });
        assert!(
            config.join("settings.toml.corrupt").exists(),
            "and the original bytes are actually kept on disk"
        );
    }

    // ── startup argument variants ───────────────────────────────────────

    #[gpui::test]
    fn single_file_arg_covers_no_tree_paths(cx: &mut TestAppContext) {
        let _home = temp_home();
        let recent1 = tempfile::tempdir().unwrap();
        let recent2 = tempfile::tempdir().unwrap();
        record_recent(&recent1.path().canonicalize().unwrap());
        record_recent(&recent2.path().canonicalize().unwrap());

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("solo.md");
        std::fs::write(&file, "# solo\n").unwrap();
        let (ws, cx) = open_arg(cx, Some(file.clone()));

        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.tree.is_none(), "file argument opens single-file mode");
            assert_eq!(tab_paths(w, app), vec![Some(file.clone())]);
            assert_eq!(w.startup_recents.len(), 2, "recents loaded at launch");
            assert!(w.git_modified.is_empty(), "no tree, no git dots");
        });

        // Everything that needs a tree is a quiet no-op.
        ws.update_in(cx, |ws, window, cx| {
            ws.focus_sidebar(&FocusSidebar, window, cx);
            ws.sidebar_up(&SidebarUp, window, cx);
            ws.sidebar_down(&SidebarDown, window, cx);
            ws.sidebar_expand(&SidebarExpand, window, cx);
            ws.sidebar_collapse(&SidebarCollapse, window, cx);
            ws.sidebar_open(&SidebarOpen, window, cx);
            ws.toggle_finder(&ToggleFinder, window, cx);
            ws.toggle_search(&ToggleSearch, window, cx);
            ws.new_file(&NewFile, window, cx);
            ws.setup_watcher(cx);
            ws.open_path_preview(&dir.path().to_path_buf(), true, window, cx);
            ws.set_active(99, window, cx);
            ws.close_tab_at(99, window, cx);
            ws.adjust_zoom(Some(2.0), cx); // active tab is not an image
            ws.on_fs_events(std::slice::from_ref(&file), cx); // no tree: no visibility filter
        });
        cx.run_until_parked(); // renders the "No folder open" sidebar + recents
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.finder.is_none());
            assert!(w.search.is_none());
            assert_eq!(tab_paths(w, app).len(), 1, "no-ops left the tabs alone");
        });

        // The recent-workspace rows sit at the bottom of the centered
        // empty-state stack; sweep upward from below it, stopping well
        // above the "Open Folder…" button (which would open an OS dialog
        // the test platform cannot service). A row click installs a tree.
        let (_w, h) = viewport(cx);
        let mut y = h / 2. + 110.;
        while y > h / 2. + 20. {
            click_at(cx, 120., y);
            if cx.update(|_, app| ws.read(app).tree.is_some()) {
                break;
            }
            y -= 6.;
        }
        cx.update(|_, app| {
            assert!(ws.read(app).tree.is_some(), "a recent row click opened the workspace");
        });
        // Back to the no-tree state for the direct open_recent calls.
        ws.update_in(cx, |ws, _, cx| {
            ws.tree = None;
            ws._watcher = None;
            cx.notify();
        });
        cx.run_until_parked();

        // Recents: an out-of-range slot is ignored; slot 0 is the most
        // recently recorded directory and opening it installs a tree.
        ws.update_in(cx, |ws, window, cx| ws.open_recent_ix(7, window, cx));
        cx.update(|_, app| assert!(ws.read(app).tree.is_none()));
        let newest = recent2.path().canonicalize().unwrap();
        ws.update_in(cx, |ws, window, cx| ws.open_recent_ix(0, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.tree.as_ref().map(|t| t.root.clone()), Some(newest));
        });
    }

    #[gpui::test]
    fn missing_file_arg_yields_empty_workspace(cx: &mut TestAppContext) {
        let _home = temp_home();
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.md");
        let (ws, cx) = open_arg(cx, Some(missing));

        cx.run_until_parked(); // renders the empty-state pane
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.tabs.is_empty(), "unreadable file argument opens nothing");
            assert!(w.tree.is_none());
            // Focusable impl hands back the workspace focus handle.
            assert_eq!(w.focus_handle(app), w.focus_handle);
        });

        // Tab-less no-ops.
        ws.update_in(cx, |ws, window, cx| {
            ws.show_changes(&ShowChanges, window, cx);
            ws.toggle_preview(&TogglePreview, window, cx);
            ws.close_tab(&CloseTab, window, cx);
        });
        cx.update(|_, app| assert!(ws.read(app).tabs.is_empty()));
    }

    #[gpui::test]
    fn no_arg_opens_the_editable_welcome_file(cx: &mut TestAppContext) {
        let home = temp_home();
        let (ws, cx) = open_arg(cx, None);
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { editor, .. }) = w.tabs.first() else {
                panic!("welcome should open as an editable tab");
            };
            assert!(editor.read(app).path().ends_with("Welcome.md"));
        });
        assert!(home._dir.path().join(".supermd/Welcome.md").exists());
    }

    #[gpui::test]
    fn unwritable_config_falls_back_to_readonly_welcome(cx: &mut TestAppContext) {
        let home = temp_home();
        // ~/.supermd as a file: the welcome doc cannot be written, so the
        // workspace falls back to the built-in read-only Reader.
        std::fs::write(home._dir.path().join(".supermd"), "block").unwrap();
        let (ws, cx) = open_arg(cx, None);
        cx.run_until_parked(); // renders the Reader (and its outline)
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(tab @ Tab::Reader(reader)) = w.tabs.first() else {
                panic!("fallback should be a Reader tab");
            };
            assert_eq!(tab.title(app), reader.read(app).title);
            assert_eq!(tab.path(app), None, "built-in welcome has no path");
            assert!(w.show_outline, "outline panel renders the reader toc");
        });
    }

    // ── external opens (Finder events, drops) ───────────────────────────

    #[gpui::test]
    fn external_open_queue_routes_dirs_and_files(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        let pending: Arc<Mutex<Vec<crate::PendingOpen>>> = Arc::new(Mutex::new(Vec::new()));
        cx.update(|_, app| watch_external_opens(pending.clone(), app));
        // First poll finds an empty queue and keeps looping.
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(350));
        cx.run_until_parked();

        let other = tempfile::tempdir().unwrap();
        let other_root = other.path().canonicalize().unwrap();
        let missing = root.path().join("gone.md");
        pending.lock().unwrap().extend([
            crate::PendingOpen::Path(other_root.clone()),
            crate::PendingOpen::Path(a.clone()),
            crate::PendingOpen::Path(missing),
        ]);
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(350));
        cx.run_until_parked();

        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(
                w.tree.as_ref().map(|t| t.root.clone()),
                Some(other_root),
                "dropped directory becomes the workspace root"
            );
            assert_eq!(tab_paths(w, app), vec![Some(a.clone())], "file opened; missing filtered");
            assert_eq!(w.preview_tab, None, "external opens are permanent tabs");
        });
    }

    /// Closing the window an external-open watcher was armed on must
    /// not silence Finder opens, "Open With → SuperMD", or
    /// `supermd://` links for the rest of the session. That is exactly
    /// what happened while the watcher was a per-window task: with one
    /// window it could never be observed, and ⌘⇧N made it reachable.
    #[gpui::test]
    fn external_opens_survive_the_window_they_were_armed_on(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let second_root = tempfile::tempdir().unwrap();
        install_test_globals(cx);

        let first = cx
            .update(|app| open_in_new_window(Some(root.path().to_path_buf()), app))
            .expect("first window");
        let second = cx
            .update(|app| open_in_new_window(Some(second_root.path().to_path_buf()), app))
            .expect("second window");
        let pending: Arc<Mutex<Vec<crate::PendingOpen>>> = Arc::new(Mutex::new(Vec::new()));
        cx.update(|app| watch_external_opens(pending.clone(), app));
        cx.run_until_parked();

        // Close the window the app started in.
        cx.update(|app| {
            first.update(app, |_, window, _| window.remove_window()).ok();
        });
        cx.run_until_parked();
        assert!(cx.update(|app| first.read(app).is_err()), "the first window is gone");

        pending
            .lock()
            .unwrap()
            .push(crate::PendingOpen::Path(a.clone()));
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(350));
        cx.run_until_parked();

        cx.update(|app| {
            let ws = second.read(app).expect("second window alive");
            assert!(
                tab_paths(ws, app).iter().flatten().any(|p| p == &a),
                "the open landed in the surviving window: {:?}",
                tab_paths(ws, app)
            );
        });
        assert!(
            pending.lock().unwrap().is_empty(),
            "a delivered batch is not requeued"
        );
    }

    /// A batch that reaches no window is handed back, not dropped --
    /// the old loop took the queue and then threw the contents away if
    /// the update failed.
    #[gpui::test]
    fn an_undeliverable_batch_comes_back_rather_than_vanishing(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        install_test_globals(cx);
        let only = cx
            .update(|app| open_in_new_window(Some(root.path().to_path_buf()), app))
            .expect("window");
        cx.update(|app| {
            only.update(app, |_, window, _| window.remove_window()).ok();
        });
        cx.run_until_parked();

        let batch = vec![crate::PendingOpen::Path(a.clone())];
        let back = cx.update(|app| deliver_external_opens(batch, app));
        let back = back.expect("no window could take it, so it comes back");
        assert!(matches!(back.as_slice(), [crate::PendingOpen::Path(p)] if *p == a));
    }

    // ── diff view (⌘⇧D) ─────────────────────────────────────────────────

    #[gpui::test]
    fn show_changes_toggles_the_diff_view(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        sh_git(root.path(), &["init", "-q"]);
        sh_git(root.path(), &["add", "-A"]);
        sh_git(root.path(), &["commit", "-qm", "init"]);
        std::fs::write(&a, "# a\nplus a new line\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));

        ws.update_in(cx, |ws, window, cx| ws.show_changes(&ShowChanges, window, cx));
        let editor = active_editor(&ws, cx);
        cx.run_until_parked(); // render the diff view
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { view, .. }) = w.tabs.get(w.active) else { panic!() };
            assert!(matches!(view, EditorView::Diff), "⌘⇧D enters the diff view");
            assert!(editor.read(app).diff_active());
        });

        ws.update_in(cx, |ws, window, cx| ws.show_changes(&ShowChanges, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { view, .. }) = w.tabs.get(w.active) else { panic!() };
            assert!(matches!(view, EditorView::Edit), "⌘⇧D again leaves the diff view");
            assert!(!editor.read(app).diff_active());
        });
    }

    // ── preview-open corner cases ───────────────────────────────────────

    #[gpui::test]
    fn preview_open_variants(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let pic = root.path().join("shot.png");
        std::fs::write(&pic, TINY_PNG).unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        // Previewing an unreadable path is a no-op.
        let missing = root.path().join("gone.md");
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path_preview(&missing, true, window, cx)
        });
        cx.update(|_, app| assert!(ws.read(app).tabs.is_empty()));

        // Images preview too (with focus back at the workspace).
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&pic, true, window, cx));
        cx.update(|window, app| {
            let w = ws.read(app);
            assert!(matches!(w.tabs.get(0), Some(Tab::Image { .. })));
            assert_eq!(w.preview_tab, Some(0));
            assert!(w.focus_handle.is_focused(window), "image tabs focus the shell");
        });

        // Activating an existing pinned tab from a different active tab
        // flushes the old one and focuses the target.
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path(&b, window, cx));
        cx.update(|_, app| assert_eq!(ws.read(app).active, 2));
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, true, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.active, 1, "existing pinned tab activated in place");
        });

        // Replacing the preview slot while a different tab is active.
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&b, false, window, cx));
        cx.update(|_, app| assert_eq!(ws.read(app).active, 2));
        ws.update_in(cx, |ws, window, cx| ws.set_active(1, window, cx));
        let c = root.path().join("c.md");
        std::fs::write(&c, "# c\n").unwrap();
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&c, false, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.preview_tab, Some(0), "slot index unchanged");
            assert_eq!(w.tabs[0].path(app), Some(c.clone()), "slot replaced from afar");
            assert_eq!(w.active, 0);
        });
    }

    #[gpui::test]
    fn open_path_ignores_unreadable_files(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        let missing = root.path().join("gone.md");
        ws.update_in(cx, |ws, window, cx| ws.open_path(&missing, window, cx));
        cx.update(|_, app| assert!(ws.read(app).tabs.is_empty()));
    }

    #[gpui::test]
    fn new_file_write_failure_is_swallowed(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        std::fs::remove_dir_all(root.path()).unwrap();
        ws.update_in(cx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
        cx.update(|_, app| {
            assert!(ws.read(app).tabs.is_empty(), "failed create opens nothing")
        });
    }

    // ── watcher lifecycle ───────────────────────────────────────────────

    #[gpui::test]
    fn rewatch_disconnects_the_old_drain_loop(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let root_canon = root.path().canonicalize().unwrap();
        let (ws, cx) = open_workspace(cx, &root_canon);

        ws.update_in(cx, |ws, _, cx| ws.setup_watcher(cx));
        // Replacing the watcher drops the old channel sender; the first
        // drain loop sees Disconnected on its next tick and exits.
        ws.update_in(cx, |ws, _, cx| ws.setup_watcher(cx));
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(250));
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app)._watcher.is_some()));

        // A vanished root cannot be watched; the watcher stays off.
        ws.update_in(cx, |ws, _, cx| {
            if let Some(tree) = &mut ws.tree {
                tree.root = PathBuf::from("/nonexistent/supermd-test-root");
            }
            ws.setup_watcher(cx);
        });
        cx.update(|_, app| assert!(ws.read(app)._watcher.is_none()));
    }

    /// Closing a window must let its `Workspace` actually go. The
    /// watcher drain loop holds a `WeakEntity`; if it ever held a
    /// strong one, every closed window would leak a whole workspace.
    ///
    /// It does **not** check that the loop exits, and the name no
    /// longer says it does: a loop that never exits leaks a task, not
    /// the entity, so removing its `break`s leaves this green. The
    /// name it replaced (`drain_loops_exit_when_the_workspace_goes_away`)
    /// promised that and more — it had no assertion at all, and the
    /// drain parks on a timer, so `run_until_parked` returned whether
    /// or not the loop exited. `rewatch_disconnects_the_old_drain_loop`
    /// is the one that covers exiting.
    ///
    /// The external-open drain is deliberately not covered here: it is
    /// app-level now and outlives any one window — see
    /// `external_opens_survive_the_window_they_were_armed_on`.
    #[gpui::test]
    fn closing_a_window_drops_its_workspace(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let root_canon = root.path().canonicalize().unwrap();
        let (ws, cx) = open_workspace(cx, &root_canon);

        ws.update_in(cx, |ws, _, cx| ws.setup_watcher(cx));
        let weak = ws.downgrade();

        // Queue a real fs event so the loop has work in flight, then
        // tear the window (and workspace) down under it.
        std::fs::write(root_canon.join("late.md"), "# late\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(80));
        cx.update(|window, _| window.remove_window());
        drop(ws);
        for _ in 0..4 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(400));
            cx.run_until_parked();
        }
        assert!(
            weak.upgrade().is_none(),
            "the workspace is gone; no drain loop is still holding it"
        );
        let _ = &root_canon;
    }

    // ── theme picker edges ──────────────────────────────────────────────

    #[gpui::test]
    fn theme_picker_dark_choice_and_save_failure(cx: &mut TestAppContext) {
        let home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        // Guarded entry points without an open picker.
        ws.update_in(cx, |ws, window, cx| {
            ws.theme_picker_apply(0, cx);
            ws.theme_picker_confirm(&ThemePickerConfirm, window, cx);
        });
        cx.update(|_, app| assert!(ws.read(app).theme_picker.is_none()));

        // Jump straight to the first dark theme and confirm it.
        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_theme_picker(&ToggleThemePicker, window, cx)
        });
        let (dark_pos, dark_name) = cx.update(|_, app| {
            let w = ws.read(app);
            let Some(picker) = &w.theme_picker else { panic!("picker open") };
            let state = app.global::<crate::theme::ThemeState>();
            let pos = picker
                .order
                .iter()
                .position(|&i| state.themes[i].theme.is_dark)
                .expect("builtins include a dark theme");
            (pos, state.themes[picker.order[pos]].name.clone())
        });
        ws.update_in(cx, |ws, _, cx| ws.theme_picker_apply(dark_pos, cx));
        ws.update_in(cx, |ws, window, cx| {
            ws.theme_picker_confirm(&ThemePickerConfirm, window, cx)
        });
        cx.update(|_, app| {
            let state = app.global::<crate::theme::ThemeState>();
            assert_eq!(state.settings.dark_theme, dark_name, "dark slot updated");
        });

        // Confirm again with an unwritable config dir: the save error is
        // logged and the picker still closes.
        let config = home._dir.path().join(".supermd");
        std::fs::remove_dir_all(&config).unwrap();
        std::fs::write(&config, "block").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_theme_picker(&ToggleThemePicker, window, cx)
        });
        ws.update_in(cx, |ws, window, cx| {
            ws.theme_picker_confirm(&ThemePickerConfirm, window, cx)
        });
        cx.update(|_, app| assert!(ws.read(app).theme_picker.is_none()));
    }

    // ── mouse-driven overlays ───────────────────────────────────────────

    /// A valid 1×1 PNG so image tabs can actually decode in render tests.
    const TINY_PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1,
        0, 0, 0, 1, 8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84,
        120, 156, 99, 96, 96, 96, 248, 15, 0, 1, 4, 1, 0, 95, 229, 195, 75, 0, 0,
        0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    fn click_at(cx: &mut gpui::VisualTestContext, x: f32, y: f32) {
        cx.simulate_click(gpui::point(px(x), px(y)), gpui::Modifiers::none());
        cx.run_until_parked();
    }

    fn double_click_at(cx: &mut gpui::VisualTestContext, x: f32, y: f32) {
        let position = gpui::point(px(x), px(y));
        cx.simulate_event(gpui::MouseDownEvent {
            position,
            modifiers: gpui::Modifiers::none(),
            button: gpui::MouseButton::Left,
            click_count: 2,
            first_mouse: false,
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position,
            modifiers: gpui::Modifiers::none(),
            button: gpui::MouseButton::Left,
            click_count: 2,
        });
        cx.run_until_parked();
    }

    fn viewport(cx: &mut gpui::VisualTestContext) -> (f32, f32) {
        let size = cx.update(|window, _| window.viewport_size());
        (f32::from(size.width), f32::from(size.height))
    }

    #[gpui::test]
    fn shortcuts_overlay_click_targets(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        let (w, h) = viewport(cx);

        ws.update_in(cx, |ws, window, cx| ws.toggle_shortcuts(&ToggleShortcuts, window, cx));
        cx.run_until_parked();
        // The panel is centered and occludes the middle: clicking it must
        // not close the overlay.
        click_at(cx, w / 2., h / 2.);
        cx.update(|_, app| assert!(ws.read(app).show_shortcuts, "panel click is inert"));
        // The dimmed background closes it.
        click_at(cx, 3., 3.);
        cx.update(|_, app| assert!(!ws.read(app).show_shortcuts, "backdrop click closes"));
    }

    #[gpui::test]
    fn theme_picker_click_targets(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        let (w, h) = viewport(cx);

        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_theme_picker(&ToggleThemePicker, window, cx)
        });
        cx.run_until_parked();
        click_at(cx, 3., 3.);
        cx.update(|_, app| {
            assert!(ws.read(app).theme_picker.is_none(), "backdrop click cancels")
        });

        // Click near the panel's vertical center until a row click moves
        // the selection. The panel is centered, but its height depends on
        // the theme list, so clicks that fall off it cancel the picker —
        // reopen and keep probing closer to the center.
        let mut row_clicked = false;
        for step in 0..40 {
            let open = cx.update(|_, app| ws.read(app).theme_picker.is_some());
            if !open {
                ws.update_in(cx, |ws, window, cx| {
                    ws.toggle_theme_picker(&ToggleThemePicker, window, cx)
                });
                cx.run_until_parked();
            }
            let before = cx.update(|_, app| ws.read(app).theme_picker.as_ref().map(|p| p.pos));
            // 0, +8, -8, +16, -16, … around the panel center.
            let offset = if step % 2 == 0 { (step / 2) as f32 * 8. } else { -(((step + 1) / 2) as f32 * 8.) };
            click_at(cx, w / 2., h / 2. + offset);
            let after = cx.update(|_, app| ws.read(app).theme_picker.as_ref().map(|p| p.pos));
            if after.is_some() && after != before {
                row_clicked = true;
                break;
            }
        }
        assert!(row_clicked, "some probe click landed on a theme row");
    }

    #[gpui::test]
    fn finder_and_search_overlays_have_click_targets(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        let (w, _h) = viewport(cx);

        ws.update_in(cx, |ws, window, cx| ws.toggle_finder(&ToggleFinder, window, cx));
        cx.run_until_parked();
        click_at(cx, w / 2., 125.); // the palette itself: stays open
        cx.update(|_, app| assert!(ws.read(app).finder.is_some(), "panel click is inert"));
        click_at(cx, 5., 5.); // backdrop: dismissed
        cx.update(|_, app| assert!(ws.read(app).finder.is_none()));

        ws.update_in(cx, |ws, window, cx| ws.toggle_search(&ToggleSearch, window, cx));
        cx.run_until_parked();
        click_at(cx, w / 2., 125.);
        cx.update(|_, app| assert!(ws.read(app).search.is_some(), "panel click is inert"));
        click_at(cx, 5., 5.);
        cx.update(|_, app| assert!(ws.read(app).search.is_none()));
    }

    #[gpui::test]
    fn titlebar_tab_clicks_and_update_pill(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        let (w, h) = viewport(cx);

        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&b, false, window, cx));
        ws.update_in(cx, |ws, _, cx| {
            ws.update_available = Some("v99.0.0".into());
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.active, 1);
            assert_eq!(w.preview_tab, Some(1));
        });

        // The tab strip starts just right of the sidebar on macOS; other
        // platforms put the ☰ app-menu button there first, whose popover
        // items would swallow the sweep, so gate the tab sweeps.
        if crate::platform::MACOS {
            // Sweep double-clicks across the strip. Hits land on either a
            // tab body (activates; pins when it is the preview tab) or a
            // × close button (removes the tab — rebuild and keep going).
            let reset = |cx: &mut gpui::VisualTestContext| {
                ws.update_in(cx, |ws, window, cx| {
                    while !ws.tabs.is_empty() {
                        ws.close_tab_at(0, window, cx);
                    }
                    ws.open_path(&a, window, cx);
                    ws.open_path_preview(&b, false, window, cx);
                });
                cx.run_until_parked();
            };
            let mut activated = false;
            let mut pinned = false;
            let mut closed = false;
            let mut x = 244.;
            while x < 900. && !(activated && pinned && closed) {
                let sane = cx.update(|_, app| {
                    let w = ws.read(app);
                    w.tabs.len() == 2 && w.preview_tab == Some(1)
                });
                if !sane {
                    reset(cx);
                }
                double_click_at(cx, x, 17.);
                cx.update(|_, app| {
                    let w = ws.read(app);
                    if w.tabs.len() < 2 {
                        closed = true;
                    } else if w.preview_tab.is_none() {
                        pinned = true;
                    } else if w.active == 0 {
                        activated = true;
                    }
                });
                x += 8.;
            }
            assert!(activated, "a sweep click activated tab 0");
            assert!(pinned, "a double-click pinned the preview tab");
            assert!(closed, "a sweep click hit a close button");
            reset(cx);
        } else {
            // Off macOS the ☰ app-menu button leads the titlebar; toggle
            // it open (rendering the popover) and close via the backdrop.
            let mut x = 244.;
            while x < 280. {
                click_at(cx, x, 17.);
                if cx.update(|_, app| ws.read(app).app_menu_open) {
                    break;
                }
                x += 8.;
            }
            if cx.update(|_, app| ws.read(app).app_menu_open) {
                click_at(cx, w - 40., h - 40.);
                cx.update(|_, app| {
                    assert!(!ws.read(app).app_menu_open, "backdrop click closes the menu")
                });
            }
        }

        // The update pill sits at the right edge of the titlebar.
        let mut x = w - 12.;
        while x > w - 220. {
            click_at(cx, x, 17.);
            if cx.opened_url().is_some() {
                break;
            }
            x -= 6.;
        }
        assert_eq!(
            cx.opened_url().as_deref(),
            Some(crate::update::RELEASES_URL),
            "pill click opens the releases page"
        );
    }

    #[gpui::test]
    fn sidebar_row_clicks_toggle_dirs_preview_and_pin_files(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let sub = root.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("c.md"), "# c\n").unwrap();
        // A git repo with an uncommitted edit renders the modified dot.
        sh_git(root.path(), &["init", "-q"]);
        sh_git(root.path(), &["add", "-A"]);
        sh_git(root.path(), &["commit", "-qm", "init"]);
        std::fs::write(&a, "# a modified\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app).git_modified.contains(&a)));

        // Sweep down the sidebar. Directory hits flip expansion; the
        // first file hit opens a preview.
        let mut dir_clicked = false;
        let mut file_clicked = false;
        let mut y = 40.;
        while y < 400. && !(dir_clicked && file_clicked) {
            let expanded_before =
                cx.update(|_, app| ws.read(app).tree.as_ref().is_some_and(|t| t.is_expanded(&sub)));
            let tabs_before = cx.update(|_, app| ws.read(app).tabs.len());
            click_at(cx, 120., y);
            let expanded_after =
                cx.update(|_, app| ws.read(app).tree.as_ref().is_some_and(|t| t.is_expanded(&sub)));
            let tabs_after = cx.update(|_, app| ws.read(app).tabs.len());
            if expanded_after != expanded_before {
                dir_clicked = true;
            }
            if tabs_after > tabs_before {
                file_clicked = true;
                cx.update(|_, app| {
                    assert_eq!(ws.read(app).preview_tab, Some(0), "single click previews")
                });
            }
            y += 5.;
        }
        assert!(dir_clicked, "some click toggled the directory row");
        assert!(file_clicked, "some click previewed a file row");

        // Double-clicking the previewed file's row pins it. Preview a
        // top-level file so its row stays visible however the folder
        // rows above it get toggled by the sweep.
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path_preview(&a, false, window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(ws.read(app).preview_tab, Some(0)));
        let mut y = 40.;
        while y < 400. {
            double_click_at(cx, 120., y);
            if cx.update(|_, app| ws.read(app).preview_tab).is_none() {
                break;
            }
            y += 5.;
        }
        cx.update(|_, app| {
            assert_eq!(ws.read(app).preview_tab, None, "double-click pins the preview")
        });
    }

    #[gpui::test]
    fn search_toggle_twice_and_non_editor_hit(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let pic = root.path().join("hit.png");
        std::fs::write(&pic, TINY_PNG).unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        // A second ⌘⇧F while the overlay is open dismisses it.
        ws.update_in(cx, |ws, window, cx| ws.toggle_search(&ToggleSearch, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.toggle_search(&ToggleSearch, window, cx));
        cx.update(|_, app| assert!(ws.read(app).search.is_none()));

        // A hit that opens a non-editor tab skips the scroll-to-line step.
        ws.update_in(cx, |ws, window, cx| ws.toggle_search(&ToggleSearch, window, cx));
        let overlay = cx.update(|_, app| {
            let Some((overlay, _)) = &ws.read(app).search else { panic!("search open") };
            overlay.clone()
        });
        cx.update(|_, app| {
            overlay.update(app, |_, cx| {
                cx.emit(crate::search_ui::SearchEvent::Open { path: pic.clone(), line: 1 })
            })
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.search.is_none());
            assert!(matches!(w.tabs.get(w.active), Some(Tab::Image { .. })));
        });
    }

    #[gpui::test]
    fn sidebar_keyboard_moves_across_row_types(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let sub = root.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("c.md"), "# c\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.focus_sidebar(&FocusSidebar, window, cx));

        // ↑ clamps at the top; the row is a directory, so no preview opens.
        ws.update_in(cx, |ws, window, cx| ws.sidebar_up(&SidebarUp, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.sidebar_selected, 0);
            assert!(w.tabs.is_empty(), "landing on a directory previews nothing");
        });

        // ⏎ on a directory row toggles it instead of opening a tab.
        ws.update_in(cx, |ws, window, cx| ws.sidebar_open(&SidebarOpen, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.tree.as_ref().is_some_and(|t| t.is_expanded(&sub)));
            assert!(w.tabs.is_empty());
        });

        // → on a file row is a no-op.
        ws.update_in(cx, |ws, window, cx| ws.sidebar_down(&SidebarDown, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.sidebar_expand(&SidebarExpand, window, cx));
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.sidebar_selected, 1, "expand on a file does not move");
        });
    }

    #[gpui::test]
    fn file_drop_events_reach_the_drop_handler(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        let (w, h) = viewport(cx);
        let position = gpui::point(px(w / 2.), px(h / 2.));

        cx.simulate_event(gpui::FileDropEvent::Entered {
            position,
            paths: Default::default(),
        });
        cx.simulate_event(gpui::FileDropEvent::Pending { position });
        cx.run_until_parked(); // drag_over style branch renders
        cx.simulate_event(gpui::FileDropEvent::Submit { position });
        cx.run_until_parked();
        cx.simulate_event(gpui::FileDropEvent::Exited);
        cx.update(|_, app| {
            // An empty drop routes through open_external_paths untouched.
            assert!(ws.read(app).tabs.is_empty());
        });
    }

    // ── render-path variants ────────────────────────────────────────────

    #[gpui::test]
    fn image_tabs_render_fit_and_zoomed(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let pic = root.path().join("pic.png");
        std::fs::write(&pic, TINY_PNG).unwrap();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.open_path(&pic, window, cx));
        cx.run_until_parked(); // fit-to-window branch
        ws.update_in(cx, |ws, window, cx| ws.zoom_in(&ZoomIn, window, cx));
        cx.run_until_parked(); // scrollable zoomed branch
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Image { zoom, .. }) = w.tabs.get(w.active) else { panic!() };
            assert!(*zoom > 1.0);
            assert!(w.show_outline, "outline stays enabled but renders nothing for images");
        });
    }

    #[gpui::test]
    fn outline_renders_for_editors_and_previews(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let doc = root.path().join("doc.md");
        std::fs::write(
            &doc,
            "# one\ntext\n## two\ntext\n## three\ntext\n### four\ntext\n",
        )
        .unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        let (w, _h) = viewport(cx);

        ws.update_in(cx, |ws, window, cx| ws.open_path(&doc, window, cx));
        cx.run_until_parked(); // editor outline
        // Click across the outline rows (right-hand panel).
        let mut y = 44.;
        while y < 140. {
            click_at(cx, w - 110., y);
            y += 8.;
        }
        cx.update(|_, app| {
            assert_eq!(tab_paths(ws.read(app), app), vec![Some(doc.clone())]);
        });

        ws.update_in(cx, |ws, window, cx| ws.toggle_preview(&TogglePreview, window, cx));
        cx.run_until_parked(); // preview reader + its outline
        cx.update(|_, app| {
            let w = ws.read(app);
            let Some(Tab::Editor { view: EditorView::Preview(preview), .. }) =
                w.tabs.get(w.active)
            else {
                panic!("preview should be active")
            };
            assert!(preview.read(app).toc.len() >= 2);
        });
        // Outline clicks now target the reader (scroll-to-block arm).
        let mut y = 44.;
        while y < 140. {
            click_at(cx, w - 110., y);
            y += 8.;
        }
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(matches!(
                w.tabs.get(w.active),
                Some(Tab::Editor { view: EditorView::Preview(_), .. })
            ));
        });
    }

    #[gpui::test]
    fn editing_a_previewed_buffer_pins_its_tab(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| ws.open_path_preview(&a, true, window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(w.preview_tab, Some(0));
            assert!(
                matches!(w.tabs.get(0), Some(Tab::Editor { view: EditorView::Preview(_), .. })),
                "a single click opens read-only, so there is nothing to type into"
            );
        });

        // Entering edit mode and typing is what pins it.
        ws.update_in(cx, |ws, window, cx| ws.toggle_preview(&TogglePreview, window, cx));
        cx.run_until_parked();
        cx.simulate_input("edited ");
        cx.run_until_parked(); // render notices the dirty buffer and pins it
        cx.update(|_, app| {
            assert_eq!(ws.read(app).preview_tab, None, "typing pins the tab")
        });
    }

    #[gpui::test]
    fn install_banner_buttons(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, _a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        let (w, _h) = viewport(cx);

        let arm = |cx: &mut gpui::VisualTestContext| {
            ws.update_in(cx, |ws, _, cx| {
                ws.install_banner = Some("SuperMD is running from the disk image.".into());
                cx.notify();
            });
            cx.run_until_parked();
        };
        arm(cx);

        // Sweep the banner row right-to-left: "Not now" comes first and
        // clears the banner; further left, "Move to Applications" fails
        // outside a real .app bundle and rewrites the banner message.
        let mut dismissed = false;
        let mut move_failed = false;
        let mut x = w - 8.;
        while x > w - 400. && !(dismissed && move_failed) {
            for y in [40., 46., 52.] {
                let before = cx.update(|_, app| ws.read(app).install_banner.clone());
                if before.is_none() {
                    arm(cx);
                }
                click_at(cx, x, y);
                let after = cx.update(|_, app| ws.read(app).install_banner.clone());
                match &after {
                    None => dismissed = true,
                    Some(msg) if msg.starts_with("Couldn't move") => move_failed = true,
                    _ => {}
                }
            }
            x -= 6.;
        }
        assert!(dismissed, "the Not-now button cleared the banner");
        assert!(move_failed, "the Move button reported the expected failure");
    }

    #[test]
    fn should_offer_default_handler_only_on_the_third_open() {
        assert!(!should_offer_default_handler(1));
        assert!(!should_offer_default_handler(2));
        assert!(should_offer_default_handler(3));
        // A fourth-or-later open must not re-arm an already-answered or
        // already-dismissed offer: the caller only reaches this once
        // per session, but the pure rule itself stays a hard `== 3`.
        assert!(!should_offer_default_handler(4));
    }

    /// Never on the first two Markdown files opened this session --
    /// that is the "demanding to be the default before you've been
    /// used" behaviour the brief calls out by name -- and never at all
    /// off macOS, which has no Markdown handler to become.
    #[gpui::test]
    fn third_markdown_open_triggers_the_default_handler_offer(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, _w, cx| {
            ws.note_markdown_open(&a, cx);
            ws.note_markdown_open(&a, cx);
            assert!(!ws.show_default_handler_offer, "not on the first two opens");
            ws.note_markdown_open(&a, cx);
            assert_eq!(
                ws.show_default_handler_offer,
                crate::platform::MACOS,
                "the third open offers it, but only where there is a handler to become"
            );
        });
    }

    /// The reading view resolves links through the workspace index,
    /// which `make_reader` hands over in one line. Without it the
    /// reader's index stays `None` and every wiki link and every
    /// relative link in a preview reads as a note that does not exist
    /// -- and the only `describe_link` test passes `None`, so it
    /// structurally cannot see the difference.
    #[gpui::test]
    fn the_reading_view_previews_links_through_the_index(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Target.md"), "# Target\n\nthe body\n").unwrap();
        let note = root.path().join("Note.md");
        std::fs::write(&note, "see [[Target]] and [here](Target.md)\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&note, window, cx));
        ws.update_in(cx, |ws, window, cx| ws.toggle_preview(&TogglePreview, window, cx));
        cx.run_until_parked();

        let reader = cx.update(|_, app| {
            let ws = ws.read(app);
            match ws.tabs.get(ws.active) {
                Some(Tab::Editor { view: EditorView::Preview(reader), .. }) => reader.clone(),
                _ => panic!("the active tab should be previewing"),
            }
        });
        cx.update(|_, app| {
            let reader = reader.read(app);
            assert!(
                matches!(reader.describe("[[Target", app), Some(crate::preview::Preview::Note { .. })),
                "the wiki link resolves: {:?}",
                reader.describe("[[Target", app)
            );
            assert!(
                matches!(reader.describe("Target.md", app), Some(crate::preview::Preview::Note { .. })),
                "and so does the relative link: {:?}",
                reader.describe("Target.md", app)
            );
        });
    }

    /// The counter has to be wired to the real open path. Every other
    /// test of this feature calls `note_markdown_open` directly, so
    /// deleting its single call site left the whole thing unreachable
    /// with the suite green.
    #[gpui::test]
    fn opening_markdown_files_normally_reaches_the_offer(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        for name in ["one.md", "two.md", "three.md"] {
            std::fs::write(root.path().join(name), "# x\n").unwrap();
        }
        let (ws, cx) = open_workspace(cx, root.path());
        for name in ["one.md", "two.md"] {
            ws.update_in(cx, |ws, window, cx| {
                ws.open_path(&root.path().join(name), window, cx)
            });
        }
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(!ws.read(app).show_default_handler_offer, "not on the first two");
        });
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(&root.path().join("three.md"), window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(
                ws.read(app).show_default_handler_offer,
                crate::platform::MACOS,
                "opening the third Markdown file the ordinary way raises the offer"
            );
        });
    }

    /// A refusal (or an acceptance) is remembered permanently: once
    /// `default_handler_asked` is set, opening more Markdown files must
    /// never bring the offer back.
    #[gpui::test]
    fn the_offer_never_returns_once_already_asked(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, _w, cx| {
            cx.global_mut::<crate::theme::ThemeState>().settings.default_handler_asked = true;
            for _ in 0..5 {
                ws.note_markdown_open(&a, cx);
            }
            assert!(!ws.show_default_handler_offer, "an answered offer must not come back");
        });
    }

    /// Answering hides the banner and persists the answer to disk, so a
    /// restart does not ask again.
    #[gpui::test]
    fn recording_the_answer_hides_the_banner_and_survives_a_reload(cx: &mut TestAppContext) {
        let _home = temp_home();
        let (root, a, _b) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, _w, cx| {
            ws.note_markdown_open(&a, cx);
            ws.note_markdown_open(&a, cx);
            ws.note_markdown_open(&a, cx);
            ws.record_default_handler_answer(cx);
            assert!(!ws.show_default_handler_offer, "answering dismisses the banner");
            assert!(cx.global::<crate::theme::ThemeState>().settings.default_handler_asked);
        });

        let reloaded = crate::settings::load(&crate::settings::config_dir());
        assert!(reloaded.default_handler_asked, "the answer survives a restart");
    }

    // ── plugin shell: palette commands, templates, exports, consent,
    //    viewers, reload ──────────────────────────────────────────────

    /// Load the fixture plugins into the ExtensionState global and the
    /// contribution tables. The returned guard serializes table-
    /// mutating tests; None = fixtures absent.
    fn with_plugins(
        cx: &mut TestAppContext,
    ) -> Option<std::sync::MutexGuard<'static, ()>> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
        if !dir.join("echo/plugin.wasm").exists() {
            eprintln!("SKIP: fixtures not built (scripts/build_plugins.sh --fixtures)");
            return None;
        }
        let guard = crate::extensions::table_test_guard();
        let mut host = crate::extensions::ExtensionHost::load(&dir);
        crate::extensions::refresh_tables(&mut host);
        cx.update(|cx| {
            cx.set_global(crate::extensions::ExtensionState(Arc::new(Mutex::new(host))));
        });
        Some(guard)
    }

    fn active_editor_text(ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> String {
        cx.update(|_, app| {
            let w = ws.read(app);
            match w.tabs.get(w.active) {
                Some(Tab::Editor { editor, .. }) => editor.read(app).text(),
                _ => panic!("active tab is not an editor"),
            }
        })
    }

    #[gpui::test]
    fn palette_opens_and_plugin_commands_apply(cx: &mut TestAppContext) {
        let _home = temp_home();
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        let (root, a, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();

        // The palette overlay builds its entries from the host tables.
        cx.dispatch_action(TogglePalette);
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app).palette.is_some(), "palette open"));
        cx.dispatch_action(TogglePalette);
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app).palette.is_none(), "palette toggled away"));

        // A plain command inserts at the cursor…
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("echo".into(), "echo.run".into(), window, cx)
        });
        cx.run_until_parked();
        assert!(active_editor_text(&ws, cx).contains("echo:echo.run"));

        // …and the formatter path rewrites the whole document (echo's
        // format-document uppercases).
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("echo".into(), "__format".into(), window, cx)
        });
        cx.run_until_parked();
        let text = active_editor_text(&ws, cx);
        assert!(text.contains("ECHO:ECHO.RUN"), "{text}");
    }

    #[gpui::test]
    fn template_command_materializes_and_opens_the_file(cx: &mut TestAppContext) {
        let _home = temp_home();
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        let (root, _, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("probe".into(), "__template:note".into(), window, cx)
        });
        cx.run_until_parked();
        let journal = std::fs::read_dir(root.path().join("from-template"))
            .unwrap()
            .flatten()
            .next()
            .expect("template file created");
        assert!(journal.file_name().to_string_lossy().starts_with("note-"));
        cx.update(|_, app| {
            let w = ws.read(app);
            let path = w.tabs[w.active].path(app).expect("template tab open");
            assert!(path.starts_with(root.path().join("from-template")));
        });
        // Running it again opens the existing file instead of erroring.
        let before = std::fs::read_dir(root.path().join("from-template")).unwrap().count();
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("probe".into(), "__template:note".into(), window, cx)
        });
        cx.run_until_parked();
        let after = std::fs::read_dir(root.path().join("from-template")).unwrap().count();
        assert_eq!(before, after, "idempotent");
    }

    #[gpui::test]
    fn export_writes_through_the_save_dialog(cx: &mut TestAppContext) {
        let _home = temp_home();
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        let (root, a, _) = workspace_fixture();
        let dest = root.path().join("exported.txt");
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();
        // fetcher's "one" format returns a single file: the flow ends
        // in prompt_for_new_path. The export runs in the background, so
        // park first (prompt becomes pending), then answer it.
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("fetcher".into(), "__export:one".into(), window, cx)
        });
        cx.run_until_parked();
        cx.simulate_new_path_selection({
            let dest = dest.clone();
            move |_| Some(dest.clone())
        });
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# a\n");
    }

    #[gpui::test]
    fn consent_banner_flow_persists_grants(cx: &mut TestAppContext) {
        let _home = temp_home();
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        let (root, a, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();
        // reader declares workspace-read with no grant: the formatter
        // path raises the consent banner instead of running.
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("reader".into(), "__format".into(), window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert_eq!(
                w.consent_request,
                Some(("reader".to_string(), "workspace-read".to_string()))
            );
        });
        ws.update_in(cx, |ws, _, cx| ws.resolve_consent(true, cx));
        cx.run_until_parked();
        let settings = crate::settings::load(&crate::settings::config_dir());
        assert_eq!(settings.plugin_grants["reader"], ["workspace-read"]);
        cx.update(|_, app| assert!(ws.read(app).consent_request.is_none()));

        // A second request denied persists the refusal.
        ws.update_in(cx, |ws, _, cx| {
            ws.handle_plugin_error("probe".into(), "consent required: example.com".into(), cx)
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| ws.resolve_consent(false, cx));
        let settings = crate::settings::load(&crate::settings::config_dir());
        assert_eq!(settings.plugin_grants["probe"], ["denied:net:example.com"]);
    }

    #[gpui::test]
    fn viewer_files_open_rendered_and_toggle_to_source(cx: &mut TestAppContext) {
        let _home = temp_home();
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        let (root, _, _) = workspace_fixture();
        let prb = root.path().join("view-me.prb");
        std::fs::write(&prb, "probe body\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&prb, window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(
                matches!(w.tabs[w.active], Tab::Editor { view: EditorView::Preview(_), .. }),
                "viewer file opens as a rendered preview"
            );
        });
        // ⌘E to source and back to the (re-rendered) view.
        cx.dispatch_action(TogglePreview);
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(matches!(w.tabs[w.active], Tab::Editor { view: EditorView::Edit, .. }));
        });
        cx.dispatch_action(TogglePreview);
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(matches!(
                w.tabs[w.active],
                Tab::Editor { view: EditorView::Preview(_), .. }
            ));
        });
    }

    #[gpui::test]
    fn status_strip_renders_widget_text(cx: &mut TestAppContext) {
        let _home = temp_home();
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        let (root, a, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_path(&a, window, cx));
        cx.run_until_parked();
        // Let the debounced widget refresh land, then draw the strip.
        for _ in 0..10 {
            cx.executor().advance_clock(std::time::Duration::from_millis(200));
            cx.run_until_parked();
        }
        let status = cx.update(|_, app| {
            let w = ws.read(app);
            match w.tabs.get(w.active) {
                Some(Tab::Editor { editor, .. }) => editor.read(app).status(),
                _ => None,
            }
        });
        assert!(status.is_some(), "probe's len widget filled the strip");
        ws.update_in(cx, |_, _, cx| cx.notify());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn template_errors_surface_in_the_command_strip(cx: &mut TestAppContext) {
        let _home = temp_home();
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        // No folder open: templates need a workspace root.
        let (ws, cx) = open_arg(cx, None);
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("probe".into(), "__template:note".into(), window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let msg = ws.read(app).command_error.clone().expect("error strip");
            assert!(msg.contains("folder"), "{msg}");
        });

        // Unknown template id: the plugin's error lands in the strip.
        let (root, _, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("probe".into(), "__template:ghost".into(), window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let msg = ws.read(app).command_error.clone().expect("error strip");
            assert!(msg.contains("ghost") || msg.contains("unknown"), "{msg}");
        });
    }


    #[gpui::test]
    fn install_overlay_flow_installs_from_the_catalog(cx: &mut TestAppContext) {
        let _home = temp_home();
        let _tables = crate::extensions::table_test_guard();
        // A valid plugin zip: the echo fixture's real component under a
        // new name, so the reloaded host actually compiles it.
        let fixture_wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/plugins/echo/plugin.wasm");
        if !fixture_wasm.exists() {
            eprintln!("SKIP: fixtures not built");
            return;
        }
        let wasm_bytes = std::fs::read(&fixture_wasm).unwrap();
        let zip_bytes = {
            use std::io::Write as _;
            let mut buf = std::io::Cursor::new(Vec::new());
            {
                let mut w = zip::ZipWriter::new(&mut buf);
                let opts = zip::write::SimpleFileOptions::default();
                w.start_file("demo/plugin.toml", opts).unwrap();
                w.write_all(b"name=\"demo\"\nversion=\"0.1.0\"\nformats=true\n")
                    .unwrap();
                w.start_file("demo/plugin.wasm", opts).unwrap();
                w.write_all(&wasm_bytes).unwrap();
                w.finish().unwrap();
            }
            buf.into_inner()
        };
        let sha = {
            use sha2::Digest as _;
            format!("{:x}", sha2::Sha256::digest(&zip_bytes))
        };
        let catalog_json = format!(
            r#"{{"catalog_version":1,"plugins":[{{"name":"demo","description":"a demo","version":"0.1.0","capabilities":[],"download":"https://github.com/SuperJackfruitLabs/supermd/releases/download/v0/plugin-demo.zip","sha256":"{sha}"}}]}}"#
        );
        cx.update(|cx| {
            let zip_bytes = zip_bytes.clone();
            let catalog_json = catalog_json.clone();
            cx.set_global(crate::catalog::CatalogFetcher(Arc::new(move |url: &str| {
                if url.ends_with("catalog.json") {
                    Ok(catalog_json.clone().into_bytes())
                } else {
                    Ok(zip_bytes.clone())
                }
            })));
            // an empty host so reload_plugins has a global to swap
            let host = crate::extensions::ExtensionHost::load(std::path::Path::new("/nonexistent"));
            cx.set_global(crate::extensions::ExtensionState(Arc::new(Mutex::new(host))));
        });
        let (root, _, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.run_plugin_command("supermd".into(), "__install".into(), window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, app| assert!(ws.read(app).install_overlay.is_some(), "overlay open"));

        // The App Store build deliberately offers no browsable catalog:
        // `install_ui::catalog_browsable()` is false under `mas`, so the
        // overlay opens with no entries (a downloadable-plugin list reads
        // as a storefront under DPLA 3.3.2(b)). There is nothing to
        // confirm, so the rest of this flow does not exist there — assert
        // that deliberate absence rather than a broken install.
        //
        // This branch was missing and nobody saw it: CI builds the
        // fixtures on Linux only, and runs the `mas` suite on macOS only,
        // so this test has never once executed under `mas` — it takes the
        // "fixtures not built" skip above every time.
        if !crate::install_ui::catalog_browsable() {
            cx.update(|_, app| {
                let overlay = ws.read(app).install_overlay.as_ref().expect("overlay open");
                assert_eq!(
                    overlay.0.read(app).entry_count(),
                    0,
                    "the App Store build lists no installable plugins"
                );
            });
            return;
        }

        cx.dispatch_action(crate::install_ui::InstallConfirm);
        cx.run_until_parked();
        let installed = crate::settings::config_dir().join("plugins/demo/plugin.toml");
        assert!(installed.exists(), "plugin landed in the plugins dir");
        cx.update(|_, app| {
            assert!(ws.read(app).install_overlay.is_none(), "overlay closed");
            let names: Vec<String> = ws
                .read(app)
                .host
                .lock()
                .unwrap()
                .plugins()
                .iter()
                .map(|p| p.name.clone())
                .collect();
            assert_eq!(names, ["demo"], "host reloaded with the new plugin");
        });
    }

    #[test]
    fn install_confirmation_names_the_plugin_and_its_capabilities() {
        let mut entry = crate::catalog::CatalogEntry {
            name: "demo".into(),
            description: "d".into(),
            version: "0.1.0".into(),
            capabilities: vec![],
            download: "https://example.invalid/demo.zip".into(),
            sha256: "0".into(),
        };
        let plain = Workspace::install_confirmation(&entry);
        assert!(plain.contains("\"demo\""), "{plain}");
        entry.capabilities = vec!["net".into()];
        let with_caps = Workspace::install_confirmation(&entry);
        assert!(with_caps.contains("network"), "{with_caps}");
    }

    /// The security property of the supermd:// handoff: a link supplies a
    /// name, never code, and nothing installs without a yes.
    #[gpui::test]
    fn a_supermd_link_asks_before_installing(cx: &mut TestAppContext) {
        let _home = temp_home();
        let catalog_json = r#"{"catalog_version":1,"plugins":[{"name":"demo","description":"a demo","version":"0.1.0","capabilities":["net"],"download":"https://github.com/SuperJackfruitLabs/supermd/releases/download/v0/plugin-demo.zip","sha256":"00"}]}"#;
        cx.update(|cx| {
            cx.set_global(crate::catalog::CatalogFetcher(Arc::new(move |_url: &str| {
                Ok(catalog_json.as_bytes().to_vec())
            })));
        });
        let (root, _, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());

        ws.update_in(cx, |ws, window, cx| {
            ws.request_plugin_install("demo".into(), window, cx)
        });
        cx.run_until_parked();
        let landed = crate::settings::config_dir().join("plugins/demo");
        cx.update(|_, app| {
            let w = ws.read(app);
            let req = w.install_request.as_ref().expect("confirmation raised");
            assert_eq!(req.name, "demo");
        });
        assert!(!landed.exists(), "nothing installed before the user answers");

        // An unknown name never raises one.
        ws.update_in(cx, |ws, window, cx| {
            ws.resolve_install_request(false, window, cx);
            ws.request_plugin_install("nope".into(), window, cx);
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let w = ws.read(app);
            assert!(w.install_request.is_none(), "no confirmation for an unknown name");
            assert!(
                w.command_error.as_ref().is_some_and(|m| m.contains("nope")),
                "the user is told why"
            );
        });
        assert!(!landed.exists());
    }

    /// `refresh_tables` writes the *process-wide* fence/decoration/
    /// inline tables, so a reload in one window re-points every
    /// window's projectors at the new plugin set -- while only the
    /// reloading window's host actually has it. Window B then claims a
    /// fence, its own host answers "no such plugin", and a red error
    /// banner replaces every such block until that window restarts.
    #[gpui::test]
    fn reloading_plugins_refreshes_every_window(cx: &mut TestAppContext) {
        let _home = temp_home();
        // Not `with_plugins`: that installs the ExtensionState global,
        // and `open_arg` then hands every window the *same* host handle,
        // which is exactly the sharing this test has to not have.
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
        if !fixtures.join("echo/plugin.wasm").exists() {
            eprintln!("SKIP: fixtures not built (scripts/build_plugins.sh --fixtures)");
            return;
        }
        let _tables = crate::extensions::table_test_guard();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        // B first, so the live `cx` belongs to A's window -- the
        // reload has to run through the window whose action it is.
        let (ws_b, cx) = open_workspace(cx, b.path());
        let (ws_a, cx) = open_workspace(cx, a.path());
        cx.run_until_parked();

        // The plugin appears on disk only after both windows are open,
        // so neither host has it until something reloads.
        let plugins = crate::settings::config_dir().join("plugins/echo");
        std::fs::create_dir_all(&plugins).unwrap();
        for f in ["plugin.toml", "plugin.wasm"] {
            std::fs::copy(fixtures.join("echo").join(f), plugins.join(f)).unwrap();
        }

        ws_a.update_in(cx, |ws, window, cx| ws.reload_plugins(&ReloadPlugins, window, cx));
        cx.run_until_parked();
        cx.update(|_, app| {
            let names = |ws: &Entity<Workspace>| -> Vec<String> {
                ws.read(app)
                    .host
                    .lock()
                    .unwrap()
                    .plugins()
                    .iter()
                    .map(|p| p.name.clone())
                    .collect()
            };
            assert_eq!(names(&ws_a), ["echo"], "the reloading window");
            assert_eq!(names(&ws_b), ["echo"], "and every other window with it");
            assert_eq!(
                ws_b.read(app).host.lock().unwrap().workspace_root().as_deref(),
                Some(b.path()),
                "each window keeps its own sandbox root"
            );
        });
    }

    #[gpui::test]
    fn reload_plugins_rebuilds_the_host(cx: &mut TestAppContext) {
        let _home = temp_home();
        let Some(_tables) = with_plugins(cx) else {
            return;
        };
        // A plugins dir under the temp HOME with just the echo fixture.
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
        let plugins = crate::settings::config_dir().join("plugins/echo");
        std::fs::create_dir_all(&plugins).unwrap();
        for f in ["plugin.toml", "plugin.wasm"] {
            std::fs::copy(fixtures.join("echo").join(f), plugins.join(f)).unwrap();
        }
        let (root, _, _) = workspace_fixture();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.reload_plugins(&ReloadPlugins, window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let names: Vec<String> = ws
                .read(app)
                .host
                .lock()
                .unwrap()
                .plugins()
                .iter()
                .map(|p| p.name.clone())
                .collect();
            assert_eq!(names, ["echo"], "reload swapped to the temp-HOME plugin set");
        });
        let _ = ws;
    }
}
