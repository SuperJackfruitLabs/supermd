// No console window on Windows release builds.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod bookmarks;
mod bookmarks_mac;
mod commands;
mod diagram;
mod diff;
mod editor;
mod extensions;
mod files;
mod fileops;
mod flux;
mod finder;
mod git;
mod graph;
mod highlight;
mod input;
mod knowledge;
mod install;
mod install_ui;
mod markdown;
mod palette;
mod platform;
mod reader;
mod search;
mod catalog;
mod seeding;
mod search_ui;
mod seti;
mod settings;
#[cfg(test)]
mod seti_tests;
mod theme;
mod ui_icons;
mod update;
mod view;
mod workspace;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    actions, point, px, size, App, Application, Bounds, Focusable, KeyBinding, Menu, MenuItem,
    SystemMenuType, TitlebarOptions, WindowBounds, WindowOptions,
};

use theme::{apply_system_appearance, ActiveTheme};
use workspace::Workspace;

actions!(app, [Quit]);

/// Serves the embedded Seti SVGs to gpui's svg renderer.
struct Assets;

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(name) = path
            .strip_prefix("icons/seti/")
            .and_then(|p| p.strip_suffix(".svg"))
        {
            if let Some((_, bytes)) = seti::ICONS.iter().find(|(n, _)| *n == name) {
                return Ok(Some(std::borrow::Cow::Borrowed(*bytes)));
            }
        }
        // Hand-authored chrome icons live beside the generated Seti set.
        if let Some(name) = path
            .strip_prefix("icons/ui/")
            .and_then(|p| p.strip_suffix(".svg"))
        {
            if let Some(bytes) = ui_icons::bytes(name) {
                return Ok(Some(std::borrow::Cow::Borrowed(bytes)));
            }
        }
        Ok(None)
    }

    fn list(&self, _path: &str) -> anyhow::Result<Vec<gpui::SharedString>> {
        Ok(Vec::new())
    }
}

/// `file://` URL → filesystem path (host part tolerated, %XX decoded).
fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let raw = &rest[rest.find('/')?..];
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&raw[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

#[cfg(test)]
mod url_tests {
    #[test]
    fn file_urls_decode_to_paths() {
        assert_eq!(
            super::file_url_to_path("file:///Users/u/My%20Notes"),
            Some(std::path::PathBuf::from("/Users/u/My Notes"))
        );
        assert_eq!(
            super::file_url_to_path("file://localhost/tmp/a.md"),
            Some(std::path::PathBuf::from("/tmp/a.md"))
        );
        assert_eq!(super::file_url_to_path("https://example.com"), None);
    }
}

/// Open Recent menu entries from the settings snapshot: (label, index).
fn recent_menu_items(recents: &[String]) -> Vec<(String, usize)> {
    recents
        .iter()
        .take(8)
        .enumerate()
        .filter(|(_, p)| Path::new(p).is_dir())
        .map(|(i, p)| {
            let name = Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone());
            (name, i)
        })
        .collect()
}

/// Launched bare (Dock/Finder) with reopen enabled: return to the most
/// recent workspace that still exists.
///
/// A sandboxed build gets no Powerbox grant with argv, so a CLI path is
/// unusable — return None and let the workspace open the panel instead
/// of failing to read a folder we appear to have been given.
fn resolve_startup_arg(arg: Option<PathBuf>, settings: &settings::Settings) -> Option<PathBuf> {
    if arg.is_some() {
        return if crate::bookmarks::needs_scope() { None } else { arg };
    }
    if !settings.reopen_last {
        return None;
    }
    settings
        .recent_workspaces
        .iter()
        .find(|p| {
            if crate::bookmarks::needs_scope() {
                // Sandboxed: existence is unknowable without a grant, so
                // a resolvable bookmark IS the existence check.
                settings.workspace_bookmarks.get(*p).is_some_and(|blob| {
                    !matches!(crate::bookmarks::resolve(blob), crate::bookmarks::Resolution::Missing)
                })
            } else {
                // Unsandboxed. Blobs left by a sandboxed build sharing this
                // settings.toml are irrelevant here — the path decides.
                Path::new(p).is_dir()
            }
        })
        .map(PathBuf::from)
}

/// Something a macOS open event asked us to do, drained by the
/// workspace's poll loop.
#[derive(Debug, PartialEq)]
pub enum PendingOpen {
    Path(PathBuf),
    /// `supermd://install-plugin?name=X` — the website's Install link.
    InstallPlugin(String),
}

/// Queue work from open-event URLs (`file://` opens and `supermd://`
/// plugin-install handoffs) for the workspace poll loop.
fn queue_open_urls(pending: &std::sync::Mutex<Vec<PendingOpen>>, urls: Vec<String>) {
    let mut lock = pending.lock().unwrap();
    for url in urls {
        if let Some(path) = file_url_to_path(&url) {
            lock.push(PendingOpen::Path(path));
        } else if let Some(name) = catalog::parse_install_url(&url) {
            lock.push(PendingOpen::InstallPlugin(name));
        }
    }
}

/// Every application key binding. Separated from `run` so tests can
/// prove each keystroke string parses on all platforms.
fn app_keybindings() -> Vec<KeyBinding> {
    // Every user-facing command declares its own keys in `commands`.
    // What remains here is surface mechanics: overlay navigation and
    // text movement, which are not commands and appear in no menu.
    let mut bindings = commands::bindings();
    bindings.extend(vec![
            KeyBinding::new(&platform::keybinding("cmd-q"), Quit, None),
            KeyBinding::new(&platform::keybinding("up"), palette::PaletteUp, Some("Palette")),
            KeyBinding::new(&platform::keybinding("down"), palette::PaletteDown, Some("Palette")),
            KeyBinding::new(&platform::keybinding("enter"), palette::PaletteConfirm, Some("Palette")),
            KeyBinding::new(&platform::keybinding("escape"), palette::PaletteDismiss, Some("Palette")),
            KeyBinding::new(&platform::keybinding("up"), install_ui::InstallUp, Some("InstallOverlay")),
            KeyBinding::new(&platform::keybinding("down"), install_ui::InstallDown, Some("InstallOverlay")),
            KeyBinding::new(&platform::keybinding("enter"), install_ui::InstallConfirm, Some("InstallOverlay")),
            KeyBinding::new(&platform::keybinding("escape"), install_ui::InstallDismiss, Some("InstallOverlay")),
            KeyBinding::new(&platform::keybinding("up"), search_ui::SearchUp, Some("Search")),
            KeyBinding::new(&platform::keybinding("down"), search_ui::SearchDown, Some("Search")),
            KeyBinding::new(&platform::keybinding("enter"), search_ui::SearchConfirm, Some("Search")),
            KeyBinding::new(&platform::keybinding("escape"), search_ui::SearchDismiss, Some("Search")),
            KeyBinding::new(&platform::keybinding("up"), workspace::ThemePickerUp, Some("ThemePicker")),
            KeyBinding::new(&platform::keybinding("down"), workspace::ThemePickerDown, Some("ThemePicker")),
            KeyBinding::new(&platform::keybinding("enter"), workspace::ThemePickerConfirm, Some("ThemePicker")),
            KeyBinding::new(&platform::keybinding("escape"), workspace::ThemePickerCancel, Some("ThemePicker")),
            // Sidebar file operations (while the sidebar is focused)
            KeyBinding::new(&platform::keybinding("enter"), workspace::SidebarEditCommit, Some("SidebarEdit")),
            KeyBinding::new(&platform::keybinding("escape"), workspace::SidebarEditCancel, Some("SidebarEdit")),
            KeyBinding::new(&platform::keybinding("escape"), workspace::GraphDismiss, Some("GraphView")),
            // Sidebar navigation (while the sidebar is focused)
            KeyBinding::new(&platform::keybinding("up"), workspace::SidebarUp, Some("Sidebar")),
            KeyBinding::new(&platform::keybinding("down"), workspace::SidebarDown, Some("Sidebar")),
            KeyBinding::new(&platform::keybinding("right"), workspace::SidebarExpand, Some("Sidebar")),
            KeyBinding::new(&platform::keybinding("left"), workspace::SidebarCollapse, Some("Sidebar")),
            KeyBinding::new(&platform::keybinding("enter"), workspace::SidebarOpen, Some("Sidebar")),
            // Text input (any focused TextInput)
            KeyBinding::new(&platform::keybinding("backspace"), input::Backspace, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("delete"), input::Delete, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("left"), input::Left, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("right"), input::Right, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("shift-left"), input::SelectLeft, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("shift-right"), input::SelectRight, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("cmd-a"), input::SelectAll, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("cmd-left"), input::Home, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("cmd-right"), input::End, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("home"), input::Home, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("end"), input::End, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("cmd-v"), input::Paste, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("cmd-c"), input::Copy, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("cmd-x"), input::Cut, Some("TextInput")),
            KeyBinding::new(&platform::keybinding("ctrl-cmd-space"), input::ShowCharacterPalette, Some("TextInput")),
            // Editor
            KeyBinding::new(&platform::keybinding("left"), editor::MoveLeft, Some("Editor")),
            KeyBinding::new(&platform::keybinding("right"), editor::MoveRight, Some("Editor")),
            KeyBinding::new(&platform::keybinding("up"), editor::MoveUp, Some("Editor")),
            KeyBinding::new(&platform::keybinding("down"), editor::MoveDown, Some("Editor")),
            KeyBinding::new(&platform::keybinding("shift-left"), editor::SelectLeft, Some("Editor")),
            KeyBinding::new(&platform::keybinding("shift-right"), editor::SelectRight, Some("Editor")),
            KeyBinding::new(&platform::keybinding("shift-up"), editor::SelectUp, Some("Editor")),
            KeyBinding::new(&platform::keybinding("shift-down"), editor::SelectDown, Some("Editor")),
            KeyBinding::new(&platform::keybinding("alt-left"), editor::MoveWordLeft, Some("Editor")),
            KeyBinding::new(&platform::keybinding("alt-right"), editor::MoveWordRight, Some("Editor")),
            KeyBinding::new(&platform::keybinding("alt-shift-left"), editor::SelectWordLeft, Some("Editor")),
            KeyBinding::new(&platform::keybinding("alt-shift-right"), editor::SelectWordRight, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-left"), editor::LineStart, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-right"), editor::LineEnd, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-shift-left"), editor::SelectLineStart, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-shift-right"), editor::SelectLineEnd, Some("Editor")),
            KeyBinding::new(&platform::keybinding("home"), editor::LineStart, Some("Editor")),
            KeyBinding::new(&platform::keybinding("end"), editor::LineEnd, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-up"), editor::DocStart, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-down"), editor::DocEnd, Some("Editor")),
            KeyBinding::new(&platform::keybinding("pageup"), editor::PageUp, Some("Editor")),
            KeyBinding::new(&platform::keybinding("pagedown"), editor::PageDown, Some("Editor")),
            // Read-only surfaces (⌘E preview, viewer tabs, welcome).
            KeyBinding::new(&platform::keybinding("backspace"), editor::Backspace, Some("Editor")),
            KeyBinding::new(&platform::keybinding("delete"), editor::Delete, Some("Editor")),
            KeyBinding::new(&platform::keybinding("alt-backspace"), editor::DeleteWordLeft, Some("Editor")),
            KeyBinding::new(&platform::keybinding("enter"), editor::Newline, Some("Editor")),
            KeyBinding::new(&platform::keybinding("tab"), editor::InsertTab, Some("Editor")),
            KeyBinding::new(&platform::keybinding("shift-tab"), editor::Outdent, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-z"), editor::Undo, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-shift-z"), editor::Redo, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-a"), editor::SelectAll, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-c"), editor::Copy, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-x"), editor::Cut, Some("Editor")),
            KeyBinding::new(&platform::keybinding("cmd-v"), editor::Paste, Some("Editor")),
            // With a selection cmd-b bolds; the handler propagates a
            // cursor-only press so ToggleSidebar still fires.
            KeyBinding::new(&platform::keybinding("escape"), editor::DismissCompletion, Some("Editor")),
            // These reuse a command's action in a different context, so
            // they are surface mechanics rather than commands: Enter in
            // the find bar, and Escape to leave the diff or the ⌘/ sheet.
            KeyBinding::new(&platform::keybinding("enter"), editor::FindNext, Some("FindBar")),
            KeyBinding::new(&platform::keybinding("shift-enter"), editor::FindPrev, Some("FindBar")),
            KeyBinding::new(&platform::keybinding("escape"), workspace::ShowChanges, Some("DiffView")),
            KeyBinding::new(&platform::keybinding("escape"), workspace::ToggleShortcuts, Some("Shortcuts")),
            KeyBinding::new(&platform::keybinding("escape"), editor::CloseFind, Some("FindBar")),
            // Finder overlay
            KeyBinding::new(&platform::keybinding("up"), finder::FinderUp, Some("Finder")),
            KeyBinding::new(&platform::keybinding("down"), finder::FinderDown, Some("Finder")),
            KeyBinding::new(&platform::keybinding("ctrl-p"), finder::FinderUp, Some("Finder")),
            KeyBinding::new(&platform::keybinding("ctrl-n"), finder::FinderDown, Some("Finder")),
            KeyBinding::new(&platform::keybinding("enter"), finder::FinderConfirm, Some("Finder")),
            KeyBinding::new(&platform::keybinding("escape"), finder::FinderDismiss, Some("Finder")),
    ]);
    bindings
}

/// The application menu bar; `recents` fills the Open Recent submenu.
fn app_menus(recents: &[String]) -> Vec<Menu> {
    // The app menu is macOS-shaped and has no table entries; everything
    // else is derived, so the menu bar and the ☰ popover cannot drift.
    let mut menus = vec![Menu {
        name: "SuperMD".into(),
        items: vec![
            MenuItem::Action {
                name: commands::about_command().label.into(),
                action: (commands::about_command().action)(),
                os_action: None,
            },
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Quit SuperMD", Quit),
        ],
    }];
    menus.extend(commands::menus(recents));
    menus
}

fn main() {
    let startup_settings = settings::load(&settings::config_dir());
    let arg = resolve_startup_arg(
        std::env::args().nth(1).map(PathBuf::from),
        &startup_settings,
    );

    // Files/folders arriving via macOS open events (double-click, Dock
    // drop, `open -a`). Drained by the workspace's poll loop.
    let pending_opens: Arc<std::sync::Mutex<Vec<PendingOpen>>> = Arc::default();

    let app = Application::new().with_assets(Assets);
    app.on_open_urls({
        let pending = pending_opens.clone();
        move |urls| queue_open_urls(&pending, urls)
    });
    app.run(move |cx: &mut App| {
        let mut themes = theme::builtin_themes();
        themes.extend(theme::load_custom_themes(&settings::themes_dir()));
        let loaded_settings = settings::load(&settings::config_dir());
        let flux_blend = flux::current_blend(&loaded_settings.flux);
        let theme_state = theme::ThemeState {
            themes,
            settings: loaded_settings,
            system_dark: false,
            flux_blend,
        };
        cx.set_global(ActiveTheme(theme_state.resolve()));
        cx.set_global(theme_state);
        // Flux ticks once a minute; through a transition that is one
        // step per tick, which reads as a smooth fade.
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(60))
                    .await;
                let alive = cx.update(|cx| {
                    let state = cx.global_mut::<theme::ThemeState>();
                    let blend = flux::current_blend(&state.settings.flux);
                    if (blend - state.flux_blend).abs() > 0.001 {
                        state.flux_blend = blend;
                        theme::refresh_active_theme(cx);
                        for window in cx.windows() {
                            window
                                .update(cx, |_, window, _| window.refresh())
                                .ok();
                        }
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.set_global(highlight::SyntaxLanguages(Arc::new(
            highlight::Languages::new(),
        )));
        // Seed the installer-bundled default plugins on first run
        // (user deletions and modifications are respected).
        if let Some(bundled) = platform::bundled_plugins_dir() {
            seeding::run_seeding(&bundled, &settings::config_dir().join("plugins"));
        }
        // Extension host: discover + compile plugins, snapshot the
        // contribution tables for pure discovery contexts.
        {
            let mut host =
                extensions::ExtensionHost::load(&settings::config_dir().join("plugins"));
            extensions::refresh_tables(&mut host);
            for (dir, err) in host.failures() {
                eprintln!("supermd: plugin failed: {}: {err}", dir.display());
            }
            host.set_grants(startup_settings.plugin_grants.clone());
            if let Some(dir) = arg.as_ref().filter(|p| p.is_dir()) {
                host.set_workspace_root(Some(dir.clone()));
            }
            cx.set_global(extensions::ExtensionState(Arc::new(std::sync::Mutex::new(host))));
        }
        cx.set_global(editor::SessionBackups(Arc::new(std::sync::Mutex::new(
            editor::autosave::BackupRegistry::new(
                editor::autosave::BackupRegistry::default_dir(),
            ),
        ))));

        cx.set_global(catalog::CatalogFetcher(catalog::ureq_fetcher()));

        extensions::start_inline_drainer(cx);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys(app_keybindings());

        cx.set_menus(app_menus(&startup_settings.recent_workspaces));

        let bounds = Bounds {
            origin: point(px(100.), px(60.)),
            size: size(px(1200.), px(800.)),
        };
        let window = cx
            .open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("SuperMD".into()),
                        // Client-side decorations: we draw the top bar,
                        // native traffic lights overlay it.
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(12.), px(10.))),
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    // Linux: ask for client-side decorations; we draw
                    // our own window controls when the compositor
                    // grants them (Decorations::Server is the fallback).
                    window_decorations: if cfg!(target_os = "linux") {
                        Some(gpui::WindowDecorations::Client)
                    } else {
                        None
                    },
                    ..Default::default()
                },
                {
                    let arg = arg.clone();
                    move |_window, cx| {
                        cx.new(|cx| {
                            let mut workspace = Workspace::new(arg, cx);
                            workspace.setup_watcher(cx);
                            workspace
                        })
                    }
                },
            )
            .unwrap();

        // Flush every dirty editor before the app exits.
        cx.on_app_quit(move |cx| {
            window
                .update(cx, |workspace, _window, cx| workspace.flush_all(cx))
                .ok();
            async {}
        })
        .detach();

        window
            .update(cx, |workspace, window, cx| {
                workspace.watch_external_opens(pending_opens.clone(), window, cx);
                apply_system_appearance(window.appearance(), cx);
                window
                    .observe_window_appearance(|window, cx| {
                        apply_system_appearance(window.appearance(), cx);
                        window.refresh();
                    })
                    .detach();
                window.focus(&workspace.focus_handle(cx));
                cx.activate(true);
            })
            .unwrap();
    });
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    use gpui::AssetSource;

    #[test]
    fn assets_serve_embedded_seti_icons_only() {
        let (known, bytes) = seti::ICONS[0];
        let served = Assets.load(&format!("icons/seti/{known}.svg")).unwrap();
        assert_eq!(served.as_deref(), Some(bytes));
        assert_eq!(Assets.load("icons/seti/definitely-missing.svg").unwrap(), None);
        assert_eq!(Assets.load("other/path.svg").unwrap(), None);
        assert!(Assets.list("anything").unwrap().is_empty());
    }

    #[test]
    fn recent_menu_items_filter_label_and_cap() {
        let dirs: Vec<tempfile::TempDir> =
            (0..10).map(|_| tempfile::tempdir().unwrap()).collect();
        let mut recents: Vec<String> = dirs
            .iter()
            .map(|d| d.path().to_string_lossy().into_owned())
            .collect();
        recents.insert(2, "/nonexistent/gone".to_string());
        let items = recent_menu_items(&recents);
        // take(8) runs before the existence filter: 8 slots, one dead entry.
        assert_eq!(items.len(), 7);
        assert!(items.iter().all(|(_, ix)| *ix != 2));
        let expected = dirs[0].path().file_name().unwrap().to_string_lossy();
        assert_eq!(items[0].0, expected);
        assert_eq!(items[0].1, 0);
    }

    #[test]
    fn startup_arg_prefers_explicit_then_recent_when_reopen_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let recents = vec![
            "/nonexistent/x".to_string(),
            dir.path().to_string_lossy().into_owned(),
        ];
        let on = settings::Settings {
            reopen_last: true,
            recent_workspaces: recents.clone(),
            ..Default::default()
        };
        let off = settings::Settings {
            reopen_last: false,
            recent_workspaces: recents,
            ..Default::default()
        };
        let explicit = PathBuf::from("/explicit/arg");
        // A sandboxed build has no grant for either an argv path or a
        // bookmark-less recent, so both collapse to None there.
        let (want_explicit, want_recent) = if crate::bookmarks::needs_scope() {
            (None, None)
        } else {
            (Some(explicit.clone()), Some(dir.path().to_path_buf()))
        };
        assert_eq!(resolve_startup_arg(Some(explicit), &on), want_explicit);
        assert_eq!(
            resolve_startup_arg(None, &on),
            want_recent,
            "first existing recent wins"
        );
        assert_eq!(resolve_startup_arg(None, &off), None);
    }

    #[test]
    fn unsandboxed_reopen_ignores_bookmarks_left_by_a_sandboxed_build() {
        // One settings.toml, two builds: the MAS build stores blobs the
        // Developer ID build cannot resolve. Reopen must still work.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        let settings = settings::Settings {
            reopen_last: true,
            recent_workspaces: vec![path.clone()],
            workspace_bookmarks: std::collections::BTreeMap::from([(path, "00".to_string())]),
            ..Default::default()
        };
        let want = (!crate::bookmarks::needs_scope()).then(|| dir.path().to_path_buf());
        assert_eq!(resolve_startup_arg(None, &settings), want);
    }

    #[test]
    fn sandboxed_builds_treat_a_cli_path_as_unscoped() {
        let settings = settings::Settings { reopen_last: false, ..Default::default() };
        let arg = Some(PathBuf::from("/some/dir"));
        let got = resolve_startup_arg(arg.clone(), &settings);
        if crate::bookmarks::needs_scope() {
            // No Powerbox grant comes with argv; the workspace prompts.
            assert_eq!(got, None);
        } else {
            assert_eq!(got, arg);
        }
    }

    #[test]
    fn open_urls_queue_only_decodable_file_urls() {
        let pending = std::sync::Mutex::new(Vec::new());
        queue_open_urls(
            &pending,
            vec![
                "file:///tmp/a%20b.md".to_string(),
                "https://example.com/nope".to_string(),
                "file:///tmp/c.md".to_string(),
            ],
        );
        assert_eq!(
            *pending.lock().unwrap(),
            vec![
                PendingOpen::Path(PathBuf::from("/tmp/a b.md")),
                PendingOpen::Path(PathBuf::from("/tmp/c.md")),
            ]
        );
    }

    #[test]
    fn open_urls_queue_plugin_install_handoffs() {
        let pending = std::sync::Mutex::new(Vec::new());
        queue_open_urls(
            &pending,
            vec![
                "supermd://install-plugin?name=calc".to_string(),
                // A foreign scheme and a traversal attempt both drop.
                "supermd://install-plugin?name=../evil".to_string(),
                "otherapp://install-plugin?name=calc".to_string(),
            ],
        );
        assert_eq!(
            *pending.lock().unwrap(),
            vec![PendingOpen::InstallPlugin("calc".to_string())]
        );
    }

    #[test]
    fn menu_bar_structure_and_recent_submenu_mapping() {
        let dirs: Vec<tempfile::TempDir> =
            (0..8).map(|_| tempfile::tempdir().unwrap()).collect();
        let recents: Vec<String> = dirs
            .iter()
            .map(|d| d.path().to_string_lossy().into_owned())
            .collect();
        let menus = app_menus(&recents);
        let names: Vec<&str> = menus.iter().map(|m| m.name.as_ref()).collect();
        assert_eq!(
            names,
            ["SuperMD", "File", "Edit", "Format", "View", "Go", "Tools", "Help"]
        );
        // Every recent slot (0..8) maps through its OpenRecentN arm.
        let file_menu = &menus[1];
        let recent = file_menu
            .items
            .iter()
            .find_map(|item| {
                let MenuItem::Submenu(menu) = item else { return None };
                Some(menu)
            })
            .expect("Open Recent submenu");
        assert_eq!(recent.items.len(), 8);
        let view = menus
            .iter()
            .find(|m| m.name.as_ref() == "View")
            .expect("View menu");
        assert!(view.items.len() >= 8, "View menu holds the toggles");
    }

    #[gpui::test]
    fn every_keybinding_parses_and_binds(cx: &mut gpui::TestAppContext) {
        // No count assertion: it was a weak detector, staying green while
        // 44 bindings moved into the table and three panels rebound. The
        // properties that matter live in `commands`: no same-context key
        // collisions, and every command reachable from some surface.
        let bindings = app_keybindings();
        assert!(bindings.len() > 100, "the binding table is populated");
        // KeyBinding::new panics on malformed keystrokes at construction;
        // binding proves the whole table is accepted by the dispatcher.
        cx.update(|cx| cx.bind_keys(app_keybindings()));
    }
}
