mod editor;
mod files;
mod finder;
mod highlight;
mod input;
mod markdown;
mod reader;
mod seti;
#[cfg(test)]
mod seti_tests;
mod theme;
mod view;
mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    actions, point, px, size, App, Application, Bounds, Focusable, KeyBinding, Menu, MenuItem,
    SystemMenuType, TitlebarOptions, WindowBounds, WindowOptions,
};

use theme::{apply_system_appearance, ActiveTheme, Theme};
use workspace::{
    CloseTab, NewFile, NextTab, OpenDialog, PrevTab, ToggleFinder, ToggleOutline, TogglePreview,
    ToggleSidebar, Workspace,
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

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);

    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        cx.set_global(ActiveTheme(Arc::new(Theme::light())));
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
                name: "supermd".into(),
                items: vec![
                    MenuItem::os_submenu("Services", SystemMenuType::Services),
                    MenuItem::separator(),
                    MenuItem::action("Quit supermd", Quit),
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
                    MenuItem::separator(),
                    MenuItem::action("Toggle Sidebar", ToggleSidebar),
                    MenuItem::action("Toggle Outline", ToggleOutline),
                    MenuItem::separator(),
                    MenuItem::action("Go to File…", ToggleFinder),
                ],
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
                        title: Some("supermd".into()),
                        ..Default::default()
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
