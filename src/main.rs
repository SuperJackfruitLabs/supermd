mod diff;
mod editor;
mod files;
mod finder;
mod git;
mod highlight;
mod input;
mod install;
mod markdown;
mod reader;
mod search;
mod search_ui;
mod seti;
mod settings;
#[cfg(test)]
mod seti_tests;
mod theme;
mod update;
mod view;
mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    actions, point, px, size, App, Application, Bounds, Focusable, KeyBinding, Menu, MenuItem,
    SystemMenuType, TitlebarOptions, WindowBounds, WindowOptions,
};

use theme::{apply_system_appearance, ActiveTheme};
use workspace::{
    CloseTab, NewFile, NextTab, OpenDialog, PrevTab, ToggleFinder, ToggleFocusMode, ToggleOutline,
    TogglePreview, ToggleSidebar, Workspace,
};

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

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);

    // Files/folders arriving via macOS open events (double-click, Dock
    // drop, `open -a`). Drained by the workspace's poll loop.
    let pending_opens: Arc<std::sync::Mutex<Vec<PathBuf>>> = Arc::default();

    let app = Application::new().with_assets(Assets);
    app.on_open_urls({
        let pending = pending_opens.clone();
        move |urls| {
            let mut lock = pending.lock().unwrap();
            for url in urls {
                if let Some(path) = file_url_to_path(&url) {
                    lock.push(path);
                }
            }
        }
    });
    app.run(move |cx: &mut App| {
        let mut themes = theme::builtin_themes();
        themes.extend(theme::load_custom_themes(&settings::themes_dir()));
        let theme_state = theme::ThemeState {
            themes,
            settings: settings::load(&settings::config_dir()),
            system_dark: false,
        };
        cx.set_global(ActiveTheme(theme_state.resolve()));
        cx.set_global(theme_state);
        cx.set_global(highlight::SyntaxLanguages(Arc::new(
            highlight::Languages::new(),
        )));
        cx.set_global(editor::SessionBackups(Arc::new(std::sync::Mutex::new(
            editor::autosave::BackupRegistry::new(
                editor::autosave::BackupRegistry::default_dir(),
            ),
        ))));

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-o", OpenDialog, None),
            KeyBinding::new("cmd-n", NewFile, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PrevTab, None),
            KeyBinding::new("cmd-shift-]", NextTab, None),
            KeyBinding::new("cmd-shift-[", PrevTab, None),
            KeyBinding::new("cmd-b", ToggleSidebar, None),
            KeyBinding::new("cmd-shift-o", ToggleOutline, None),
            KeyBinding::new("cmd-p", ToggleFinder, None),
            KeyBinding::new("cmd-e", TogglePreview, None),
            KeyBinding::new("cmd-shift-d", workspace::ShowChanges, None),
            KeyBinding::new("escape", workspace::ShowChanges, Some("DiffView")),
            KeyBinding::new("cmd-shift-f", workspace::ToggleSearch, None),
            KeyBinding::new("ctrl-cmd-f", ToggleFocusMode, None),
            KeyBinding::new("up", search_ui::SearchUp, Some("Search")),
            KeyBinding::new("down", search_ui::SearchDown, Some("Search")),
            KeyBinding::new("enter", search_ui::SearchConfirm, Some("Search")),
            KeyBinding::new("escape", search_ui::SearchDismiss, Some("Search")),
            KeyBinding::new("cmd-1", workspace::FocusSidebar, None),
            KeyBinding::new("cmd-/", workspace::ToggleShortcuts, None),
            KeyBinding::new("cmd-t", workspace::ToggleThemePicker, None),
            KeyBinding::new("up", workspace::ThemePickerUp, Some("ThemePicker")),
            KeyBinding::new("down", workspace::ThemePickerDown, Some("ThemePicker")),
            KeyBinding::new("enter", workspace::ThemePickerConfirm, Some("ThemePicker")),
            KeyBinding::new("escape", workspace::ThemePickerCancel, Some("ThemePicker")),
            KeyBinding::new("cmd-=", workspace::ZoomIn, None),
            KeyBinding::new("cmd--", workspace::ZoomOut, None),
            KeyBinding::new("cmd-0", workspace::ZoomReset, None),
            KeyBinding::new("escape", workspace::ToggleShortcuts, Some("Shortcuts")),
            // Sidebar navigation (while the sidebar is focused)
            KeyBinding::new("up", workspace::SidebarUp, Some("Sidebar")),
            KeyBinding::new("down", workspace::SidebarDown, Some("Sidebar")),
            KeyBinding::new("right", workspace::SidebarExpand, Some("Sidebar")),
            KeyBinding::new("left", workspace::SidebarCollapse, Some("Sidebar")),
            KeyBinding::new("enter", workspace::SidebarOpen, Some("Sidebar")),
            // Text input (any focused TextInput)
            KeyBinding::new("backspace", input::Backspace, Some("TextInput")),
            KeyBinding::new("delete", input::Delete, Some("TextInput")),
            KeyBinding::new("left", input::Left, Some("TextInput")),
            KeyBinding::new("right", input::Right, Some("TextInput")),
            KeyBinding::new("shift-left", input::SelectLeft, Some("TextInput")),
            KeyBinding::new("shift-right", input::SelectRight, Some("TextInput")),
            KeyBinding::new("cmd-a", input::SelectAll, Some("TextInput")),
            KeyBinding::new("cmd-left", input::Home, Some("TextInput")),
            KeyBinding::new("cmd-right", input::End, Some("TextInput")),
            KeyBinding::new("home", input::Home, Some("TextInput")),
            KeyBinding::new("end", input::End, Some("TextInput")),
            KeyBinding::new("cmd-v", input::Paste, Some("TextInput")),
            KeyBinding::new("cmd-c", input::Copy, Some("TextInput")),
            KeyBinding::new("cmd-x", input::Cut, Some("TextInput")),
            KeyBinding::new("ctrl-cmd-space", input::ShowCharacterPalette, Some("TextInput")),
            // Editor
            KeyBinding::new("left", editor::MoveLeft, Some("Editor")),
            KeyBinding::new("right", editor::MoveRight, Some("Editor")),
            KeyBinding::new("up", editor::MoveUp, Some("Editor")),
            KeyBinding::new("down", editor::MoveDown, Some("Editor")),
            KeyBinding::new("shift-left", editor::SelectLeft, Some("Editor")),
            KeyBinding::new("shift-right", editor::SelectRight, Some("Editor")),
            KeyBinding::new("shift-up", editor::SelectUp, Some("Editor")),
            KeyBinding::new("shift-down", editor::SelectDown, Some("Editor")),
            KeyBinding::new("alt-left", editor::MoveWordLeft, Some("Editor")),
            KeyBinding::new("alt-right", editor::MoveWordRight, Some("Editor")),
            KeyBinding::new("alt-shift-left", editor::SelectWordLeft, Some("Editor")),
            KeyBinding::new("alt-shift-right", editor::SelectWordRight, Some("Editor")),
            KeyBinding::new("cmd-left", editor::LineStart, Some("Editor")),
            KeyBinding::new("cmd-right", editor::LineEnd, Some("Editor")),
            KeyBinding::new("cmd-shift-left", editor::SelectLineStart, Some("Editor")),
            KeyBinding::new("cmd-shift-right", editor::SelectLineEnd, Some("Editor")),
            KeyBinding::new("home", editor::LineStart, Some("Editor")),
            KeyBinding::new("end", editor::LineEnd, Some("Editor")),
            KeyBinding::new("cmd-up", editor::DocStart, Some("Editor")),
            KeyBinding::new("cmd-down", editor::DocEnd, Some("Editor")),
            KeyBinding::new("pageup", editor::PageUp, Some("Editor")),
            KeyBinding::new("pagedown", editor::PageDown, Some("Editor")),
            KeyBinding::new("backspace", editor::Backspace, Some("Editor")),
            KeyBinding::new("delete", editor::Delete, Some("Editor")),
            KeyBinding::new("alt-backspace", editor::DeleteWordLeft, Some("Editor")),
            KeyBinding::new("enter", editor::Newline, Some("Editor")),
            KeyBinding::new("tab", editor::InsertTab, Some("Editor")),
            KeyBinding::new("cmd-z", editor::Undo, Some("Editor")),
            KeyBinding::new("cmd-shift-z", editor::Redo, Some("Editor")),
            KeyBinding::new("cmd-a", editor::SelectAll, Some("Editor")),
            KeyBinding::new("cmd-c", editor::Copy, Some("Editor")),
            KeyBinding::new("cmd-x", editor::Cut, Some("Editor")),
            KeyBinding::new("cmd-v", editor::Paste, Some("Editor")),
            KeyBinding::new("cmd-s", editor::SaveNow, Some("Editor")),
            KeyBinding::new("cmd-f", editor::OpenFind, Some("Editor")),
            KeyBinding::new("cmd-g", editor::FindNext, Some("Editor")),
            KeyBinding::new("cmd-shift-g", editor::FindPrev, Some("Editor")),
            KeyBinding::new("enter", editor::FindNext, Some("FindBar")),
            KeyBinding::new("shift-enter", editor::FindPrev, Some("FindBar")),
            KeyBinding::new("escape", editor::CloseFind, Some("FindBar")),
            // Finder overlay
            KeyBinding::new("up", finder::FinderUp, Some("Finder")),
            KeyBinding::new("down", finder::FinderDown, Some("Finder")),
            KeyBinding::new("ctrl-p", finder::FinderUp, Some("Finder")),
            KeyBinding::new("ctrl-n", finder::FinderDown, Some("Finder")),
            KeyBinding::new("enter", finder::FinderConfirm, Some("Finder")),
            KeyBinding::new("escape", finder::FinderDismiss, Some("Finder")),
        ]);

        cx.set_menus(vec![
            Menu {
                name: "SuperMD".into(),
                items: vec![
                    MenuItem::os_submenu("Services", SystemMenuType::Services),
                    MenuItem::separator(),
                    MenuItem::action("Quit SuperMD", Quit),
                ],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("New File", NewFile),
                    MenuItem::action("Open…", OpenDialog),
                    MenuItem::separator(),
                    MenuItem::action("Close Tab", CloseTab),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Toggle Edit/Preview", TogglePreview),
                    MenuItem::action("Show Changes", workspace::ShowChanges),
                    MenuItem::action("Focus Mode", ToggleFocusMode),
                    MenuItem::action("Theme…", workspace::ToggleThemePicker),
                    MenuItem::separator(),
                    MenuItem::action("Toggle Sidebar", ToggleSidebar),
                    MenuItem::action("Toggle Outline", ToggleOutline),
                    MenuItem::separator(),
                    MenuItem::action("Go to File…", ToggleFinder),
                    MenuItem::action("Search in Workspace…", workspace::ToggleSearch),
                ],
            },
            Menu {
                name: "Help".into(),
                items: vec![MenuItem::action(
                    "Keyboard Shortcuts",
                    workspace::ToggleShortcuts,
                )],
            },
        ]);

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
