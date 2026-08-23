mod files;
mod finder;
mod highlight;
mod input;
mod markdown;
mod reader;
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
    CloseTab, NextTab, OpenDialog, PrevTab, ToggleFinder, ToggleOutline, ToggleSidebar, Workspace,
};

actions!(app, [Quit]);

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);

    Application::new().run(move |cx: &mut App| {
        cx.set_global(ActiveTheme(Arc::new(Theme::light())));
        cx.set_global(highlight::SyntaxLanguages(Arc::new(
            highlight::Languages::new(),
        )));

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-o", OpenDialog, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PrevTab, None),
            KeyBinding::new("cmd-shift-]", NextTab, None),
            KeyBinding::new("cmd-shift-[", PrevTab, None),
            KeyBinding::new("cmd-b", ToggleSidebar, None),
            KeyBinding::new("cmd-shift-o", ToggleOutline, None),
            KeyBinding::new("cmd-p", ToggleFinder, None),
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
                    MenuItem::action("Open…", OpenDialog),
                    MenuItem::separator(),
                    MenuItem::action("Close Tab", CloseTab),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
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
                    move |_window, cx| cx.new(|cx| Workspace::new(arg, cx))
                },
            )
            .unwrap();

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
