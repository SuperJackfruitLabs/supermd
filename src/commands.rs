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
    ws::ToggleFlux => { id: "flux", label: "Flux (adaptive theme)",
        keys: ["ctrl-cmd-n"], ctx: None, menu: Some((View, 2)), help: Some(General) },
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
    ws::ToggleGraph => { id: "graph", label: "Graph View", keys: ["cmd-shift-g"],
        ctx: None, menu: Some((Go, 0)), help: Some(General) },
    ws::TogglePalette => { id: "palette", label: "Command Palette…",
        keys: ["cmd-shift-p"], ctx: None, menu: Some((Tools, 0)), help: Some(General) },
    ws::InstallPlugins => { id: "install_plugins", label: "Install Plugins…",
        keys: [], ctx: None, menu: Some((Tools, 1)), help: None },
    ws::OpenPluginsFolder => { id: "plugins_folder", label: "Open Plugins Folder",
        keys: [], ctx: None, menu: Some((Tools, 1)), help: None },
    ws::ReloadPlugins => { id: "reload_plugins", label: "Reload Plugins",
        keys: [], ctx: None, menu: Some((Tools, 1)), help: None },

    // ── Help ───────────────────────────────────────────────────────────
    ws::ToggleShortcuts => { id: "shortcuts", label: "Keyboard Shortcuts",
        keys: ["cmd-/"], ctx: None, menu: Some((Help, 0)), help: Some(General) },
    // On macOS About belongs in the app menu, so it is filtered out of
    // Help there and added to the app menu by `app_menus`.
    ws::ToggleAbout => { id: "about", label: "About SuperMD", keys: [],
        ctx: None, menu: Some((Help, 1)), help: None },
    // No keystroke: the sandboxed build hides ~/.supermd in its container,
    // so this is a discoverability affordance, not a hot path.
    ws::RevealSettingsFolder => { id: "settings_folder", label: "Reveal Settings Folder",
        keys: [], ctx: None, menu: Some((Help, 1)), help: None },

    // ── Editor: menus arrive with the restructure (Task 10) ────────────
    // ── Edit ───────────────────────────────────────────────────────────
    // An Edit menu is not only discoverability: macOS hangs Emoji &
    // Symbols, dictation and substitutions off a conventional one.
    ed::Undo => { id: "undo", label: "Undo", keys: ["cmd-z"],
        ctx: Some("Editor"), menu: Some((Edit, 0)), help: Some(HEditor) },
    ed::Redo => { id: "redo", label: "Redo", keys: ["cmd-shift-z"],
        ctx: Some("Editor"), menu: Some((Edit, 0)), help: Some(HEditor) },
    ed::Cut => { id: "cut", label: "Cut", keys: ["cmd-x"],
        ctx: Some("Editor"), menu: Some((Edit, 1)), help: None },
    ed::Copy => { id: "copy", label: "Copy", keys: ["cmd-c"],
        ctx: Some("Editor"), menu: Some((Edit, 1)), help: None },
    ed::Paste => { id: "paste", label: "Paste", keys: ["cmd-v"],
        ctx: Some("Editor"), menu: Some((Edit, 1)), help: None },
    ed::SelectAll => { id: "select_all", label: "Select All", keys: ["cmd-a"],
        ctx: Some("Editor"), menu: Some((Edit, 1)), help: None },
    ed::OpenFind => { id: "find", label: "Find in File", keys: ["cmd-f"],
        ctx: Some("Editor"), menu: Some((Edit, 2)), help: Some(HEditor) },
    ed::FindNext => { id: "find_next", label: "Find Next", keys: ["cmd-g"],
        ctx: Some("Editor"), menu: Some((Edit, 2)), help: Some(HEditor) },
    ed::FindPrev => { id: "find_prev", label: "Find Previous",
        keys: ["cmd-shift-g"], ctx: Some("Editor"), menu: Some((Edit, 2)),
        help: Some(HEditor) },

    // ── Format ─────────────────────────────────────────────────────────
    ed::ToggleBold => { id: "bold", label: "Bold", keys: ["cmd-b"],
        ctx: Some("Editor"), menu: Some((Format, 0)), help: Some(HEditor) },
    ed::ToggleItalic => { id: "italic", label: "Italic", keys: ["cmd-i"],
        ctx: Some("Editor"), menu: Some((Format, 0)), help: Some(HEditor) },
    ed::ToggleCode => { id: "code", label: "Code", keys: [],
        ctx: Some("Editor"), menu: Some((Format, 0)), help: None },
    ed::ToggleStrike => { id: "strike", label: "Strikethrough", keys: [],
        ctx: Some("Editor"), menu: Some((Format, 0)), help: None },
    ed::InsertLink => { id: "link", label: "Link", keys: [],
        ctx: Some("Editor"), menu: Some((Format, 0)), help: None },
    ed::CycleHeading => { id: "heading", label: "Heading", keys: [],
        ctx: Some("Editor"), menu: Some((Format, 1)), help: None },
    ed::ToggleQuote => { id: "quote", label: "Quote", keys: [],
        ctx: Some("Editor"), menu: Some((Format, 1)), help: None },

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
        // About is a macOS app-menu item there, and a Help item elsewhere.
        .filter(|c| !(c.id == "about" && crate::platform::ABOUT_IN_APP_MENU))
        .collect()
}

/// The About command, for the macOS app menu.
pub fn about_command() -> &'static Command {
    COMMANDS.iter().find(|c| c.id == "about").expect("about is in the table")
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


fn help_title(section: HelpSection) -> &'static str {
    match section {
        HelpSection::General => "General",
        HelpSection::Editor => "Editor",
        HelpSection::Preview => "Preview & read-only tabs",
        HelpSection::Sidebar => "Sidebar",
    }
}

/// The ⌘/ dialog, and the source for the generated shortcut docs. Only
/// the first (canonical) key is listed: aliases bind but do not clutter.
pub fn help_sections() -> Vec<(&'static str, Vec<(String, &'static str)>)> {
    const ORDER: [HelpSection; 4] = [
        HelpSection::General,
        HelpSection::Editor,
        HelpSection::Preview,
        HelpSection::Sidebar,
    ];
    ORDER
        .iter()
        .filter_map(|&section| {
            let rows: Vec<(String, &'static str)> = COMMANDS
                .iter()
                .filter(|c| c.help == Some(section))
                .filter_map(|c| c.keys.first().map(|k| (glyphs(k), c.label)))
                .collect();
            (!rows.is_empty()).then_some((help_title(section), rows))
        })
        .collect()
}


/// The ☰ popover's contents: the same menus, as flat labelled groups.
/// Off macOS there is no global menu bar, so this *is* the menu.
pub fn popover_groups() -> Vec<(&'static str, Vec<&'static Command>)> {
    MENU_ORDER
        .iter()
        .filter_map(|&id| {
            let cmds = items_for(id);
            (!cmds.is_empty()).then_some((menu_title(id), cmds))
        })
        .collect()
}


/// `docs/site/shortcuts.md`, generated. Both platform columns come from
/// the one macOS-form declaration in the table.
pub fn shortcut_markdown() -> String {
    let mut out = String::from(
        "# Keyboard shortcuts\n\nOn Linux and Windows, read ⌘ as Ctrl.\n\n## The scheme\n\nThe modifier says what a shortcut acts on. New bindings follow the tier that matches their scope, so the map stays learnable as it grows.\n\n| Modifier | Acts on | Examples |\n| -------- | ------- | -------- |\n| ⌘ + letter | the file and the text | ⌘N ⌘O ⌘S ⌘W ⌘F ⌘Z ⌘B ⌘I ⌘P ⌘E ⌘T |\n| ⌘ ⇧ + letter | the workspace | ⌘⇧F search · ⌘⇧P palette · ⌘⇧D changes · ⌘⇧G graph |\n| ⌃ ⌘ + letter | modes and environment | ⌃⌘F focus · ⌃⌘N flux |\n| ⌘ + digit | panels | ⌘1 sidebar · ⌘2 outline · ⌘3 knowledge |\n| unmodified | only inside a surface | sidebar: F2 rename · ⌘⌫ trash |\n\n⌘B also toggles the sidebar, kept from long habit in other editors: with a selection it bolds, and with a bare cursor it falls through to the panel.\n\n",
    );
    for (title, rows) in help_sections() {
        out.push_str(&format!("## {title}\n\n"));
        out.push_str("| macOS | Windows / Linux | Action |\n");
        out.push_str("| ----- | --------------- | ------ |\n");
        for (keys, label) in rows {
            let other = crate::platform::translate_glyphs_for_docs(&keys);
            out.push_str(&format!("| {keys} | {other} | {label} |\n"));
        }
        out.push('\n');
    }
    out
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
    fn help_sections_carry_glyphs_and_labels() {
        let sections = help_sections();
        let general = sections
            .iter()
            .find(|(title, _)| *title == "General")
            .expect("General section exists");
        assert!(
            general.1.iter().any(|(keys, label)| keys == "⌘ P" && *label == "Go to File…"),
            "⌘P is listed under General"
        );
    }

    #[test]
    fn help_lists_only_the_canonical_key_not_aliases() {
        let sections = help_sections();
        let general: Vec<String> = sections
            .iter()
            .find(|(t, _)| *t == "General")
            .expect("General section")
            .1
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        assert!(general.iter().any(|k| k == "⌘ 1"), "canonical panel key shown");
        // ⌘B is the sidebar's alias, so it must not appear a second time
        // under General. It still appears under Editor, for Bold — a
        // different command that legitimately owns the same chord.
        assert!(
            !general.iter().any(|k| k == "⌘ B"),
            "the ⌘B sidebar alias is not listed alongside ⌘1"
        );
        let editor: Vec<String> = sections
            .iter()
            .find(|(t, _)| *t == "Editor")
            .expect("Editor section")
            .1
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        assert!(editor.iter().any(|k| k == "⌘ B"), "Bold keeps ⌘B in Editor");
    }

    /// The defect this module exists to kill: the Linux/Windows ☰
    /// popover was a separate hand-written list carrying about half the
    /// macOS menu bar. Both now project the same grouping.
    #[test]
    fn the_popover_shows_exactly_what_the_menu_bar_shows() {
        let built = menus(&[]);
        let bar: Vec<&str> = built
            .iter()
            .flat_map(|m| {
                m.items.iter().filter_map(|i| match i {
                    gpui::MenuItem::Action { name, .. } => Some(name.as_ref()),
                    _ => None,
                })
            })
            .collect();
        let popover: Vec<&str> = popover_groups()
            .iter()
            .flat_map(|(_, cmds)| cmds.iter().map(|c| c.label))
            .collect();
        assert_eq!(bar, popover, "popover and menu bar list the same commands");
    }

    #[test]
    fn shortcut_markdown_documents_the_modifier_scheme() {
        let md = shortcut_markdown();
        assert!(md.contains("## The scheme"), "the scheme is documented");
        // Each tier is named, so a future binding has a principled home.
        for tier in ["the file and the text", "the workspace", "modes", "panels"] {
            assert!(md.contains(tier), "scheme is missing the {tier:?} tier");
        }
    }

    #[test]
    fn shortcut_markdown_renders_a_table_per_section() {
        let md = shortcut_markdown();
        assert!(md.starts_with("# Keyboard shortcuts"), "has a title");
        assert!(md.contains("## General"), "has the General section");
        assert!(
            md.contains("| ⌘ P | Ctrl P | Go to File… |"),
            "a row renders both platform columns"
        );
    }

    /// The docs source is generated from the table. If this fails, run
    /// `UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table`, commit
    /// the result, then `cargo run --example build_docs`.
    #[test]
    fn shortcut_docs_match_the_table() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/site/shortcuts.md");
        let generated = shortcut_markdown();
        if std::env::var("UPDATE_DOCS").is_ok() {
            std::fs::write(path, &generated).expect("write shortcuts.md");
            return;
        }
        let on_disk = std::fs::read_to_string(path).expect("read shortcuts.md");
        // Git checks this file out with CRLF on Windows, so compare the
        // content rather than the line endings — the generator always
        // emits LF, which is what gets committed.
        assert_eq!(
            on_disk.replace("\r\n", "\n"),
            generated,
            "docs/site/shortcuts.md is stale; rerun with UPDATE_DOCS=1"
        );
    }

    /// A command with no menu entry, no ⌘/ row and no keystroke exists
    /// but no user can find it. This is the property the old
    /// `assert_eq!(bindings.len(), N)` counter could never express — it
    /// stayed green while 44 bindings moved and three panels rebound.
    #[test]
    fn every_command_is_reachable_from_some_surface() {
        let unreachable: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.menu.is_none() && c.help.is_none() && c.keys.is_empty())
            .map(|c| c.id)
            .collect();
        assert!(unreachable.is_empty(), "unreachable commands: {unreachable:?}");
    }

    #[test]
    fn the_edit_menu_exists_and_carries_the_clipboard_verbs() {
        let edit: Vec<&str> = items_for(MenuId::Edit).iter().map(|c| c.label).collect();
        for expected in ["Undo", "Redo", "Cut", "Copy", "Paste", "Select All"] {
            assert!(edit.contains(&expected), "Edit menu is missing {expected}");
        }
    }

    #[test]
    fn the_format_menu_carries_every_marker_toggle() {
        let fmt: Vec<&str> = items_for(MenuId::Format).iter().map(|c| c.label).collect();
        for expected in
            ["Bold", "Italic", "Code", "Strikethrough", "Link", "Heading", "Quote"]
        {
            assert!(fmt.contains(&expected), "Format menu is missing {expected}");
        }
    }

    #[test]
    fn about_lands_in_the_app_menu_on_macos_and_help_elsewhere() {
        let help: Vec<&str> = items_for(MenuId::Help).iter().map(|c| c.id).collect();
        if crate::platform::ABOUT_IN_APP_MENU {
            assert!(!help.contains(&"about"), "macOS hangs About off the app menu");
        } else {
            assert!(help.contains(&"about"), "elsewhere About lives in Help");
        }
        // Either way it exists and is reachable.
        assert_eq!(about_command().label, "About SuperMD");
    }

    #[test]
    fn graph_view_reaches_a_menu_and_a_shortcut() {
        let graph = COMMANDS.iter().find(|c| c.id == "graph").expect("graph in table");
        assert_eq!(graph.keys, &["cmd-shift-g"]);
        assert_eq!(graph.menu.map(|(m, _)| m), Some(MenuId::Go));
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
