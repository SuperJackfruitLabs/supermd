//! The command table: one declaration per user-facing command, and the
//! pure derivations that project it onto every surface (keybindings,
//! menus, the ☰ popover, the ⌘/ dialog, and the docs).
//!
//! Before this module those surfaces were six independent hand-written
//! lists, and two of them had already drifted apart — the non-macOS ☰
//! popover carried about half the macOS menu bar. Declaring a command
//! once and deriving every surface makes that class of bug structural
//! rather than a matter of remembering.

use gpui::Action;

/// Which menu a command appears under. Placement inside the menu is the
/// `u8` group in `Command::menu`: a change of group emits a separator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuId {
    File,
    Edit,
    Format,
    View,
    Go,
    Tools,
    Help,
}

/// Section of the ⌘/ dialog (and the generated shortcut docs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HelpSection {
    General,
    Editor,
    Preview,
    Sidebar,
}

/// One user-facing command. `action` is the only field carrying a
/// concrete type; everything else is data, which is what lets a single
/// table feed surfaces whose APIs are otherwise incompatible.
pub struct Command {
    /// Stable identifier, used by tests and the palette.
    pub id: &'static str,
    /// Menu and ⌘/ text.
    pub label: &'static str,
    /// macOS-form keystrokes. Empty means menu/palette only. The first
    /// is displayed; every one of them is bound (⌘1 plus the ⌘B alias).
    pub keys: &'static [&'static str],
    /// Key context the binding is scoped to; None binds globally.
    pub context: Option<&'static str>,
    /// Menu placement: which menu, and which separator group within it.
    pub menu: Option<(MenuId, u8)>,
    /// ⌘/ dialog section.
    pub help: Option<HelpSection>,
    /// Boxes a fresh action. Both GPUI entry points take boxed actions,
    /// so one entry can feed keybindings, menus, and click dispatch.
    pub action: fn() -> Box<dyn Action>,
}

/// Modifier prefixes, longest-first so `ctrl-` cannot shadow anything.
const MODIFIERS: [(&str, &str); 4] =
    [("cmd-", "⌘"), ("shift-", "⇧"), ("alt-", "⌥"), ("ctrl-", "⌃")];

/// A macOS-form keystroke as display glyphs: `cmd-shift-d` → `⌘ ⇧ D`.
/// `platform::shortcut_glyphs` converts the result for other platforms,
/// so this stays macOS-form exactly like the keystrokes themselves.
///
/// Modifiers are stripped as prefixes rather than by splitting on `-`:
/// the key itself may *be* a hyphen (`cmd--` is Zoom Out), which
/// splitting would turn into empty segments.
pub fn glyphs(keystroke: &str) -> String {
    let mut rest = keystroke;
    let mut out: Vec<String> = Vec::new();
    'strip: loop {
        for (prefix, glyph) in MODIFIERS {
            if let Some(tail) = rest.strip_prefix(prefix) {
                out.push(glyph.to_string());
                rest = tail;
                continue 'strip;
            }
        }
        break;
    }
    out.push(key_glyph(rest));
    out.join(" ")
}

/// The non-modifier tail of a keystroke, as it should read on screen.
fn key_glyph(key: &str) -> String {
    match key {
        "enter" => "⏎".to_string(),
        "backspace" => "⌫".to_string(),
        "delete" => "⌦".to_string(),
        "escape" => "esc".to_string(),
        "tab" => "Tab".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "pageup" => "PgUp".to_string(),
        "pagedown" => "PgDn".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        // A bare hyphen is the minus key; use the typographic sign so it
        // cannot be mistaken for a separator.
        "-" => "−".to_string(),
        other if other.chars().count() == 1 => other.to_uppercase(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

/// Declares the table. Each line reads as one fact about one command;
/// the macro captures the concrete action type in a fn pointer, so the
/// table stays heterogeneous without boxing at declaration time.
macro_rules! commands {
    ($($action:expr => {
        id: $id:expr,
        label: $label:expr,
        keys: [$($key:expr),* $(,)?],
        ctx: $ctx:expr,
        menu: $menu:expr,
        help: $help:expr $(,)?
    }),* $(,)?) => {
        pub const COMMANDS: &[Command] = &[$(
            Command {
                id: $id,
                label: $label,
                keys: &[$($key),*],
                context: $ctx,
                menu: $menu,
                help: $help,
                action: || Box::new($action),
            }
        ),*];
    };
}

use crate::editor as ed;
use crate::reader as rd;
use crate::workspace as ws;
use HelpSection::{Editor as HEditor, General, Preview, Sidebar as HSidebar};
use MenuId::*;

commands! {
    // ── File ───────────────────────────────────────────────────────────
    ws::NewFile => { id: "new_file", label: "New File", keys: ["cmd-n"],
        ctx: None, menu: Some((File, 0)), help: Some(General) },
    ws::OpenDialog => { id: "open", label: "Open…", keys: ["cmd-o"],
        ctx: None, menu: Some((File, 0)), help: Some(General) },
    ed::SaveNow => { id: "save", label: "Save Now", keys: ["cmd-s"],
        ctx: Some("Editor"), menu: Some((File, 1)), help: Some(General) },
    ws::CloseTab => { id: "close_tab", label: "Close Tab", keys: ["cmd-w"],
        ctx: None, menu: Some((File, 2)), help: Some(General) },

    // ── View ───────────────────────────────────────────────────────────
    ws::TogglePreview => { id: "toggle_preview", label: "Toggle Edit/Preview",
        keys: ["cmd-e"], ctx: None, menu: Some((View, 0)), help: Some(General) },
    ws::ShowChanges => { id: "show_changes", label: "Show Changes",
        keys: ["cmd-shift-d"], ctx: None, menu: Some((View, 0)), help: Some(General) },
    ws::ToggleSidebar => { id: "toggle_sidebar", label: "Toggle Sidebar",
        keys: ["cmd-1", "cmd-b"], ctx: None, menu: Some((View, 1)), help: Some(General) },
    ws::ToggleOutline => { id: "toggle_outline", label: "Toggle Outline",
        keys: ["cmd-2"], ctx: None, menu: Some((View, 1)), help: Some(General) },
    ws::ToggleKnowledge => { id: "toggle_knowledge", label: "Knowledge Panel",
        keys: ["cmd-3"], ctx: None, menu: Some((View, 1)), help: Some(General) },
    ws::ToggleFocusMode => { id: "focus_mode", label: "Focus Mode",
        keys: ["ctrl-cmd-f"], ctx: None, menu: Some((View, 2)), help: Some(General) },
    ws::ZoomIn => { id: "zoom_in", label: "Zoom In", keys: ["cmd-="],
        ctx: None, menu: Some((View, 3)), help: Some(General) },
    ws::ZoomOut => { id: "zoom_out", label: "Zoom Out", keys: ["cmd--"],
        ctx: None, menu: Some((View, 3)), help: None },
    ws::ZoomReset => { id: "zoom_reset", label: "Actual Size", keys: ["cmd-0"],
        ctx: None, menu: Some((View, 3)), help: None },
    ws::ToggleThemePicker => { id: "theme", label: "Theme…", keys: ["cmd-t"],
        ctx: None, menu: Some((View, 4)), help: Some(General) },

    // ── Go ─────────────────────────────────────────────────────────────
    ws::ToggleFinder => { id: "go_to_file", label: "Go to File…", keys: ["cmd-p"],
        ctx: None, menu: Some((Go, 0)), help: Some(General) },
    ws::ToggleSearch => { id: "search_workspace", label: "Search in Workspace…",
        keys: ["cmd-shift-f"], ctx: None, menu: Some((Go, 0)), help: Some(General) },
    ws::FocusSidebar => { id: "focus_sidebar", label: "Focus Sidebar", keys: [],
        ctx: None, menu: Some((Go, 1)), help: None },
    ws::NextTab => { id: "next_tab", label: "Next Tab", keys: ["cmd-shift-]", "ctrl-tab"],
        ctx: None, menu: Some((Go, 1)), help: Some(General) },
    ws::PrevTab => { id: "prev_tab", label: "Previous Tab",
        keys: ["cmd-shift-[", "ctrl-shift-tab"],
        ctx: None, menu: Some((Go, 1)), help: Some(General) },
    ed::FollowLink => { id: "follow_link", label: "Follow Link",
        keys: ["cmd-enter"], ctx: Some("Editor"), menu: Some((Go, 2)),
        help: Some(HEditor) },

    // ── Tools ──────────────────────────────────────────────────────────
    ws::TogglePalette => { id: "palette", label: "Command Palette…",
        keys: ["cmd-shift-p"], ctx: None, menu: Some((Tools, 0)), help: Some(General) },
    ws::OpenPluginsFolder => { id: "plugins_folder", label: "Open Plugins Folder",
        keys: [], ctx: None, menu: Some((Tools, 1)), help: None },
    ws::ReloadPlugins => { id: "reload_plugins", label: "Reload Plugins",
        keys: [], ctx: None, menu: Some((Tools, 1)), help: None },

    // ── Help ───────────────────────────────────────────────────────────
    ws::ToggleShortcuts => { id: "shortcuts", label: "Keyboard Shortcuts",
        keys: ["cmd-/"], ctx: None, menu: Some((Help, 0)), help: Some(General) },

    // ── Editor: menus arrive with the restructure (Task 10) ────────────
    ed::ToggleBold => { id: "bold", label: "Bold", keys: ["cmd-b"],
        ctx: Some("Editor"), menu: None, help: Some(HEditor) },
    ed::ToggleItalic => { id: "italic", label: "Italic", keys: ["cmd-i"],
        ctx: Some("Editor"), menu: None, help: Some(HEditor) },
    ed::OpenFind => { id: "find", label: "Find in File", keys: ["cmd-f"],
        ctx: Some("Editor"), menu: None, help: Some(HEditor) },
    ed::FindNext => { id: "find_next", label: "Find Next", keys: ["cmd-g"],
        ctx: Some("Editor"), menu: None, help: Some(HEditor) },
    ed::FindPrev => { id: "find_prev", label: "Find Previous",
        keys: ["cmd-shift-g"], ctx: Some("Editor"), menu: None, help: Some(HEditor) },

    // ── Sidebar (context-scoped; no menu placement) ────────────────────
    ws::SidebarNewFile => { id: "sidebar_new_file", label: "New File Here",
        keys: ["cmd-n"], ctx: Some("Sidebar"), menu: None, help: Some(HSidebar) },
    ws::SidebarNewFolder => { id: "sidebar_new_folder", label: "New Folder Here",
        keys: ["cmd-shift-n"], ctx: Some("Sidebar"), menu: None, help: Some(HSidebar) },
    ws::SidebarRename => { id: "sidebar_rename", label: "Rename", keys: ["f2"],
        ctx: Some("Sidebar"), menu: None, help: Some(HSidebar) },
    ws::SidebarDelete => { id: "sidebar_delete", label: "Delete to Trash",
        keys: ["cmd-backspace"], ctx: Some("Sidebar"), menu: None, help: Some(HSidebar) },
    ws::SidebarMoveTo => { id: "sidebar_move", label: "Move to Folder…",
        keys: ["cmd-shift-m"], ctx: Some("Sidebar"), menu: None, help: Some(HSidebar) },

    // ── Reader (context-scoped) ────────────────────────────────────────
    rd::ScrollUp => { id: "reader_up", label: "Scroll Up", keys: ["up"],
        ctx: Some("Reader"), menu: None, help: Some(Preview) },
    rd::ScrollDown => { id: "reader_down", label: "Scroll Down",
        keys: ["down"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
    rd::PageUp => { id: "reader_pageup", label: "Page Up",
        keys: ["pageup"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
    rd::PageDown => { id: "reader_pagedown", label: "Page Down",
        keys: ["pagedown"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
    rd::ScrollTop => { id: "reader_top", label: "Go to Start",
        keys: ["home"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
    rd::ScrollBottom => { id: "reader_bottom", label: "Go to End",
        keys: ["end"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
}


/// Every declared key as a real binding. `KeyBinding::load` takes the
/// boxed action, which is what lets one table feed a surface whose
/// public helper (`KeyBinding::new`) demands a concrete type.
pub fn bindings() -> Vec<gpui::KeyBinding> {
    COMMANDS
        .iter()
        .flat_map(|cmd| {
            cmd.keys.iter().map(move |key| {
                let predicate = cmd.context.map(|ctx| {
                    std::rc::Rc::new(
                        gpui::KeyBindingContextPredicate::parse(ctx)
                            .expect("command context predicate parses"),
                    )
                });
                gpui::KeyBinding::load(
                    &crate::platform::keybinding(key),
                    (cmd.action)(),
                    predicate,
                    false,
                    None,
                    &gpui::DummyKeyboardMapper,
                )
                .expect("command keystroke parses")
            })
        })
        .collect()
}


/// Menu-bar order, and the order the ☰ popover renders its groups.
pub const MENU_ORDER: [MenuId; 7] = [
    MenuId::File,
    MenuId::Edit,
    MenuId::Format,
    MenuId::View,
    MenuId::Go,
    MenuId::Tools,
    MenuId::Help,
];

pub fn menu_title(id: MenuId) -> &'static str {
    match id {
        MenuId::File => "File",
        MenuId::Edit => "Edit",
        MenuId::Format => "Format",
        MenuId::View => "View",
        MenuId::Go => "Go",
        MenuId::Tools => "Tools",
        MenuId::Help => "Help",
    }
}

/// Commands in one menu, in declaration order — which the table keeps
/// grouped, so group indices come out ascending.
pub fn items_for(id: MenuId) -> Vec<&'static Command> {
    COMMANDS
        .iter()
        .filter(|c| matches!(c.menu, Some((m, _)) if m == id))
        .collect()
}

/// The menu bar. `recents` fills the Open Recent submenu.
pub fn menus(recents: &[String]) -> Vec<gpui::Menu> {
    MENU_ORDER
        .iter()
        .filter_map(|&id| {
            let cmds = items_for(id);
            let first = cmds.first()?;
            let mut items: Vec<gpui::MenuItem> = Vec::new();
            let mut group = first.menu.expect("filtered on menu").1;
            for cmd in cmds {
                let g = cmd.menu.expect("filtered on menu").1;
                if g != group {
                    items.push(gpui::MenuItem::Separator);
                    group = g;
                }
                items.push(gpui::MenuItem::Action {
                    name: cmd.label.into(),
                    action: (cmd.action)(),
                    os_action: None,
                });
                // Open Recent hangs off the File menu's open entry.
                if cmd.id == "open" && !recents.is_empty() {
                    items.push(gpui::MenuItem::Submenu(gpui::Menu {
                        name: "Open Recent".into(),
                        items: recent_items(recents),
                    }));
                }
            }
            Some(gpui::Menu { name: menu_title(id).into(), items })
        })
        .collect()
}

/// The Open Recent submenu. The eight slots are distinct action types,
/// so this stays a match rather than a table entry.
fn recent_items(recents: &[String]) -> Vec<gpui::MenuItem> {
    recents
        .iter()
        .take(8)
        .enumerate()
        .map(|(ix, name)| match ix {
            0 => gpui::MenuItem::action(name.clone(), ws::OpenRecent0),
            1 => gpui::MenuItem::action(name.clone(), ws::OpenRecent1),
            2 => gpui::MenuItem::action(name.clone(), ws::OpenRecent2),
            3 => gpui::MenuItem::action(name.clone(), ws::OpenRecent3),
            4 => gpui::MenuItem::action(name.clone(), ws::OpenRecent4),
            5 => gpui::MenuItem::action(name.clone(), ws::OpenRecent5),
            6 => gpui::MenuItem::action(name.clone(), ws::OpenRecent6),
            _ => gpui::MenuItem::action(name.clone(), ws::OpenRecent7),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_entries_carry_their_keys_and_display_form() {
        let sidebar = COMMANDS
            .iter()
            .find(|c| c.id == "toggle_sidebar")
            .expect("toggle_sidebar is in the table");
        assert_eq!(sidebar.label, "Toggle Sidebar");
        // ⌘1 is canonical; ⌘B is the retained convention alias.
        assert_eq!(sidebar.keys, &["cmd-1", "cmd-b"]);
        assert_eq!(glyphs(sidebar.keys[0]), "⌘ 1");
        assert!(sidebar.context.is_none(), "panel toggles bind globally");
    }

    #[test]
    fn every_command_has_a_unique_id() {
        let mut ids: Vec<&str> = COMMANDS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate command id in the table");
    }

    #[test]
    fn bindings_expand_every_alias_key() {
        let total: usize = COMMANDS.iter().map(|c| c.keys.len()).sum();
        assert_eq!(bindings().len(), total, "one binding per declared key");
    }

    #[test]
    fn no_two_commands_claim_one_key_in_the_same_context() {
        let mut seen: Vec<(&str, Option<&str>)> = Vec::new();
        for cmd in COMMANDS {
            for key in cmd.keys {
                let slot = (*key, cmd.context);
                assert!(
                    !seen.contains(&slot),
                    "{} collides on {key:?} in context {:?}",
                    cmd.id,
                    cmd.context
                );
                seen.push(slot);
            }
        }
    }

    /// Tab cycling has two chords each. They were nearly lost when the
    /// table took over: a filter keyed on the action name alone dropped
    /// ⌃Tab because ⌘⇧] already claimed NextTab.
    #[test]
    fn tab_cycling_keeps_both_of_its_chords() {
        let keys = |id: &str| {
            COMMANDS.iter().find(|c| c.id == id).expect("in table").keys
        };
        assert_eq!(keys("next_tab"), &["cmd-shift-]", "ctrl-tab"]);
        assert_eq!(keys("prev_tab"), &["cmd-shift-[", "ctrl-shift-tab"]);
    }

    #[test]
    fn cmd_b_is_shared_across_contexts_on_purpose() {
        // ToggleSidebar (global) and ToggleBold (Editor) both bind ⌘B; the
        // editor handler propagates a cursor-only press so the sidebar
        // still toggles. Cross-context sharing must stay legal, so this
        // records the exception the collision test above permits.
        let holders: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.keys.contains(&"cmd-b"))
            .map(|c| c.id)
            .collect();
        assert_eq!(holders, vec!["toggle_sidebar", "bold"]);
    }

    #[test]
    fn items_for_a_menu_come_back_in_group_order() {
        let view = items_for(MenuId::View);
        assert!(!view.is_empty(), "the View menu has entries");
        let groups: Vec<u8> = view.iter().map(|c| c.menu.unwrap().1).collect();
        let mut sorted = groups.clone();
        sorted.sort_unstable();
        assert_eq!(groups, sorted, "entries are grouped in order");
    }

    #[test]
    fn menu_titles_cover_every_variant() {
        for id in [
            MenuId::File, MenuId::Edit, MenuId::Format,
            MenuId::View, MenuId::Go, MenuId::Tools, MenuId::Help,
        ] {
            assert!(!menu_title(id).is_empty(), "{id:?} has a title");
        }
    }

    #[test]
    fn separators_fall_between_groups_never_at_the_edges() {
        let built = menus(&[]);
        let view = built
            .iter()
            .find(|m| m.name.as_ref() == "View")
            .expect("View menu is built");
        assert!(
            !matches!(view.items.first(), Some(gpui::MenuItem::Separator)),
            "no leading separator"
        );
        assert!(
            !matches!(view.items.last(), Some(gpui::MenuItem::Separator)),
            "no trailing separator"
        );
        let seps = view
            .items
            .iter()
            .filter(|i| matches!(i, gpui::MenuItem::Separator))
            .count();
        assert_eq!(seps, 4, "View has five groups, so four separators");
    }

    #[test]
    fn glyphs_render_macos_modifier_symbols() {
        assert_eq!(glyphs("cmd-n"), "⌘ N");
        assert_eq!(glyphs("cmd-shift-d"), "⌘ ⇧ D");
        assert_eq!(glyphs("ctrl-cmd-f"), "⌃ ⌘ F");
        assert_eq!(glyphs("alt-left"), "⌥ ←");
    }

    /// `cmd--` is Zoom Out: the key itself is a hyphen, so naive
    /// splitting on '-' yields empty segments and loses the key.
    #[test]
    fn glyphs_survive_a_hyphen_as_the_key() {
        assert_eq!(glyphs("cmd--"), "⌘ −");
        assert_eq!(glyphs("cmd-="), "⌘ =");
    }

    #[test]
    fn glyphs_name_the_non_letter_keys() {
        assert_eq!(glyphs("enter"), "⏎");
        assert_eq!(glyphs("cmd-backspace"), "⌘ ⌫");
        assert_eq!(glyphs("pageup"), "PgUp");
        assert_eq!(glyphs("f2"), "F2");
    }
}
