# Command Table & Menus Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace six hand-maintained descriptions of SuperMD's command set with one table, then restructure the menus it now drives.

**Architecture:** A new pure module `src/commands.rs` holds one `Command` per user-facing action, with a single type-carrying field (`action: fn() -> Box<dyn Action>`). Keybindings, the macOS menu bar, the Linux/Windows ☰ popover, the ⌘/ dialog, and the generated docs all become pure projections of that table. Stage 1 is a behaviour-preserving refactor; stage 2 then restructures the menus, which becomes a table edit.

**Tech Stack:** Rust, GPUI 0.2.2, inline `#[cfg(test)]` tests, `cargo test`, `cargo llvm-cov`.

**Spec:** `docs/superpowers/specs/2026-08-26-shortcuts-menus-chrome-design.md`

## Global Constraints

- Editing/policy logic is pure Rust under tests; the GPUI shell stays thin. `commands.rs` must have no GPUI *shell* dependency beyond the `Action`/`KeyBinding`/`Menu` types it produces.
- CI enforces a **90% line-coverage floor** (`cargo llvm-cov --fail-under-lines 90`); the project currently sits at 94.4%.
- Keystrokes are authored in **macOS form** (`cmd-shift-d`) and translated by `platform::keybinding()`. Never write `ctrl-` forms into the table.
- Per-OS decisions live in `src/platform.rs`, never as scattered `cfg!()`.
- `src/seti.rs` is GENERATED — do not edit.
- Tests live inline as `#[cfg(test)] mod tests` beside the code they cover.
- The ⌘B overload is deliberate and must survive: `ToggleSidebar` globally, `editor::ToggleBold` in the `Editor` context, with the editor handler propagating a cursor-only press (`main.rs:269`).
- This plan covers **stages 1 and 2 of 6** from the spec. Stages 3–6 (shortcut rebinds, About dialog, chrome, status bar) get their own plans once the table exists.

---

## File Structure

| File | Responsibility |
| ---- | -------------- |
| `src/commands.rs` (create) | The `Command` struct, `MenuId`, `HelpSection`, the `commands!` macro, the table itself, and the five pure derivations. |
| `src/main.rs` (modify) | `app_keybindings()` and `app_menus()` shrink to calls into `commands`. |
| `src/workspace.rs` (modify) | `SHORTCUTS` deleted; ☰ popover renders from the table; palette built-ins for graph/flux/install become actions. |
| `src/platform.rs` (modify) | `translate_glyphs_for_docs` for the docs' non-macOS column. |
| `docs/site/shortcuts.md` (generated) | Regenerated from `shortcut_markdown()`; a golden-file test fails on drift. |

`MenuId::About` and the About dialog belong to stage 4 and are deliberately
absent here — this plan's `MenuId` has no `About` variant.

---

### Task 1: The `Command` table and glyph derivation

**Files:**
- Create: `src/commands.rs`
- Modify: `src/main.rs` (add `mod commands;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub struct Command { id, label, keys, context, menu, help, action }`; `pub enum MenuId { File, Edit, Format, View, Go, Tools, Help }`; `pub enum HelpSection { General, Editor, Preview, Sidebar }`; `pub fn glyphs(keystroke: &str) -> String`; `pub const COMMANDS: &[Command]`.

- [ ] **Step 1: Write the failing test**

Create `src/commands.rs` containing only the test module:

```rust
//! The command table: one declaration per user-facing command, and the
//! pure derivations that project it onto every surface (keybindings,
//! menus, the ☰ popover, the ⌘/ dialog, and the docs).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_render_macos_modifier_symbols() {
        assert_eq!(glyphs("cmd-n"), "⌘ N");
        assert_eq!(glyphs("cmd-shift-d"), "⌘ ⇧ D");
        assert_eq!(glyphs("ctrl-cmd-f"), "⌃ ⌘ F");
        assert_eq!(glyphs("alt-left"), "⌥ ←");
    }

    #[test]
    fn glyphs_name_the_non_letter_keys() {
        assert_eq!(glyphs("enter"), "⏎");
        assert_eq!(glyphs("cmd-backspace"), "⌘ ⌫");
        assert_eq!(glyphs("pageup"), "PgUp");
        assert_eq!(glyphs("f2"), "F2");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test commands::`
Expected: FAIL — `cannot find function 'glyphs' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add above the test module in `src/commands.rs`:

```rust
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
/// table feed surfaces with incompatible APIs.
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
    /// so one table entry can feed keybindings, menus and dispatch.
    pub action: fn() -> Box<dyn Action>,
}

/// A macOS-form keystroke as display glyphs: `cmd-shift-d` → `⌘ ⇧ D`.
/// `platform::shortcut_glyphs` converts the result for other platforms,
/// so this stays macOS-form exactly like the keystrokes themselves.
pub fn glyphs(keystroke: &str) -> String {
    keystroke
        .split('-')
        .map(|part| match part {
            "cmd" => "⌘".to_string(),
            "shift" => "⇧".to_string(),
            "alt" => "⌥".to_string(),
            "ctrl" => "⌃".to_string(),
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
            other if other.len() == 1 => other.to_uppercase(),
            other => {
                let mut c = other.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().chain(c).collect(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
```

Add `mod commands;` to `src/main.rs` beside the other `mod` declarations (alphabetical order, after `mod catalog;`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test commands::`
Expected: PASS, 2 tests.

- [ ] **Step 5: Commit**

```bash
git add src/commands.rs src/main.rs
git commit -m "feat: command table types and glyph derivation"
```

---

### Task 2: The `commands!` macro and the table

**Files:**
- Modify: `src/commands.rs`

**Interfaces:**
- Consumes: `Command`, `MenuId`, `HelpSection` from Task 1.
- Produces: `pub const COMMANDS: &[Command]` — every global and Workspace-context command currently in `app_keybindings()`, plus the menu-only entries.

**Note:** This table covers only commands that appear in menus, the ⌘/ dialog, or bind globally. Overlay-internal navigation (`PaletteUp`, `FinderDown`, `SidebarUp`, `ThemePicker*`, `Search*`, `Install*`, and the `Editor`/`TextInput`/`Reader`/`FindBar` movement bindings) stays in `app_keybindings()` as a separate literal list — those are not user-facing *commands*, they are surface mechanics, and putting them in the table would bloat it without serving any derivation. Task 3 keeps both lists.

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/commands.rs`:

```rust
    #[test]
    fn table_entries_carry_their_action_and_display_form() {
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test commands::`
Expected: FAIL — `cannot find value 'COMMANDS' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/commands.rs`, above the tests:

```rust
/// Declares the table. Each line reads as one fact about one command;
/// the macro captures the concrete action type in a fn pointer so the
/// table can stay heterogeneous without boxing at declaration time.
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

use crate::workspace as ws;
use MenuId::*;
use HelpSection::{Editor as HEditor, General, Preview, Sidebar as HSidebar};

commands! {
    // ── File ───────────────────────────────────────────────────────
    ws::NewFile => { id: "new_file", label: "New File", keys: ["cmd-n"],
        ctx: None, menu: Some((File, 0)), help: Some(General) },
    ws::OpenDialog => { id: "open", label: "Open…", keys: ["cmd-o"],
        ctx: None, menu: Some((File, 0)), help: Some(General) },
    crate::editor::SaveNow => { id: "save", label: "Save Now", keys: ["cmd-s"],
        ctx: Some("Editor"), menu: Some((File, 1)), help: Some(General) },
    ws::CloseTab => { id: "close_tab", label: "Close Tab", keys: ["cmd-w"],
        ctx: None, menu: Some((File, 2)), help: Some(General) },

    // ── View ───────────────────────────────────────────────────────
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
        ctx: None, menu: Some((View, 3)), help: Some(General) },
    ws::ZoomReset => { id: "zoom_reset", label: "Actual Size", keys: ["cmd-0"],
        ctx: None, menu: Some((View, 3)), help: Some(General) },
    ws::ToggleThemePicker => { id: "theme", label: "Theme…", keys: ["cmd-t"],
        ctx: None, menu: Some((View, 4)), help: Some(General) },

    // ── Go ─────────────────────────────────────────────────────────
    ws::ToggleFinder => { id: "go_to_file", label: "Go to File…", keys: ["cmd-p"],
        ctx: None, menu: Some((Go, 0)), help: Some(General) },
    ws::ToggleSearch => { id: "search_workspace", label: "Search in Workspace…",
        keys: ["cmd-shift-f"], ctx: None, menu: Some((Go, 0)), help: Some(General) },
    ws::NextTab => { id: "next_tab", label: "Next Tab", keys: ["cmd-shift-]"],
        ctx: None, menu: Some((Go, 1)), help: Some(General) },
    ws::PrevTab => { id: "prev_tab", label: "Previous Tab", keys: ["cmd-shift-["],
        ctx: None, menu: Some((Go, 1)), help: Some(General) },
    crate::editor::FollowLink => { id: "follow_link", label: "Follow Link",
        keys: ["cmd-enter"], ctx: Some("Editor"), menu: Some((Go, 2)),
        help: Some(HEditor) },

    // ── Tools ──────────────────────────────────────────────────────
    ws::TogglePalette => { id: "palette", label: "Command Palette…",
        keys: ["cmd-shift-p"], ctx: None, menu: Some((Tools, 0)), help: Some(General) },
    ws::OpenPluginsFolder => { id: "plugins_folder", label: "Open Plugins Folder",
        keys: [], ctx: None, menu: Some((Tools, 1)), help: None },
    ws::ReloadPlugins => { id: "reload_plugins", label: "Reload Plugins",
        keys: [], ctx: None, menu: Some((Tools, 1)), help: None },

    // ── Help ───────────────────────────────────────────────────────
    ws::ToggleShortcuts => { id: "shortcuts", label: "Keyboard Shortcuts",
        keys: ["cmd-/"], ctx: None, menu: Some((Help, 0)), help: Some(General) },

    // ── Editor: formatting and find (menus arrive in Task 8) ───────
    crate::editor::ToggleBold => { id: "bold", label: "Bold", keys: ["cmd-b"],
        ctx: Some("Editor"), menu: None, help: Some(HEditor) },
    crate::editor::ToggleItalic => { id: "italic", label: "Italic", keys: ["cmd-i"],
        ctx: Some("Editor"), menu: None, help: Some(HEditor) },
    crate::editor::OpenFind => { id: "find", label: "Find in File", keys: ["cmd-f"],
        ctx: Some("Editor"), menu: None, help: Some(HEditor) },
    crate::editor::FindNext => { id: "find_next", label: "Find Next", keys: ["cmd-g"],
        ctx: Some("Editor"), menu: None, help: Some(HEditor) },
    crate::editor::FindPrev => { id: "find_prev", label: "Find Previous",
        keys: ["cmd-shift-g"], ctx: Some("Editor"), menu: None, help: Some(HEditor) },

    // ── Sidebar (context-scoped; no menu placement) ────────────────
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
    ws::FocusSidebar => { id: "focus_sidebar", label: "Focus Sidebar", keys: [],
        ctx: None, menu: None, help: None },

    // ── Reader (context-scoped) ────────────────────────────────────
    crate::reader::ScrollUp => { id: "reader_up", label: "Scroll Up", keys: ["up"],
        ctx: Some("Reader"), menu: None, help: Some(Preview) },
    crate::reader::ScrollDown => { id: "reader_down", label: "Scroll Down",
        keys: ["down"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
    crate::reader::PageUp => { id: "reader_pageup", label: "Page Up",
        keys: ["pageup"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
    crate::reader::PageDown => { id: "reader_pagedown", label: "Page Down",
        keys: ["pagedown"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
    crate::reader::ScrollTop => { id: "reader_top", label: "Go to Start",
        keys: ["home"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
    crate::reader::ScrollBottom => { id: "reader_bottom", label: "Go to End",
        keys: ["end"], ctx: Some("Reader"), menu: None, help: Some(Preview) },
}
```

Note the panel keys already reflect the spec's ⌘1/⌘2/⌘3 change; the old
⌘⇧O / ⌘⇧K bindings are gone and ⌘1 no longer means focus-sidebar.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test commands::`
Expected: PASS, 4 tests.

- [ ] **Step 5: Commit**

```bash
git add src/commands.rs
git commit -m "feat: the command table"
```

---

### Task 3: `bindings()` — drive keybindings from the table

**Files:**
- Modify: `src/commands.rs`, `src/main.rs:150-292` (`app_keybindings`)

**Interfaces:**
- Consumes: `COMMANDS` from Task 2.
- Produces: `pub fn bindings() -> Vec<KeyBinding>`.

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/commands.rs`:

```rust
    #[test]
    fn bindings_expand_every_alias_key() {
        let all = bindings();
        // ToggleSidebar declares two keys, so it contributes two bindings.
        let sidebar_keys = COMMANDS
            .iter()
            .find(|c| c.id == "toggle_sidebar")
            .unwrap()
            .keys
            .len();
        assert_eq!(sidebar_keys, 2);
        let total: usize = COMMANDS.iter().map(|c| c.keys.len()).sum();
        assert_eq!(all.len(), total, "one binding per declared key");
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

    #[test]
    fn cmd_b_is_shared_across_contexts_on_purpose() {
        // ToggleSidebar (global) and ToggleBold (Editor) both bind ⌘B; the
        // editor handler propagates a cursor-only press. Cross-context
        // sharing must stay legal, so this documents the exception.
        let holders: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.keys.contains(&"cmd-b"))
            .map(|c| c.id)
            .collect();
        assert_eq!(holders, vec!["toggle_sidebar", "bold"]);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test commands::`
Expected: FAIL — `cannot find function 'bindings' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/commands.rs`:

```rust
use gpui::{KeyBinding, KeyBindingContextPredicate};
use std::rc::Rc;

/// Every declared key as a real binding. `KeyBinding::load` takes the
/// boxed action, which is what lets one table feed a surface whose
/// public helper (`KeyBinding::new`) demands a concrete type.
pub fn bindings() -> Vec<KeyBinding> {
    COMMANDS
        .iter()
        .flat_map(|cmd| {
            cmd.keys.iter().map(move |key| {
                let predicate = cmd.context.map(|ctx| {
                    Rc::new(
                        KeyBindingContextPredicate::parse(ctx)
                            .expect("command context predicate parses"),
                    )
                });
                KeyBinding::load(
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
```

In `src/main.rs`, delete from `app_keybindings()` every binding whose
action now lives in the table (the global ones, the Workspace-context
ones, the six `reader::*`, the five sidebar file-op ones, `editor::SaveNow`,
`editor::FollowLink`, `editor::ToggleBold`, `editor::ToggleItalic`,
`editor::OpenFind`, `editor::FindNext`, `editor::FindPrev`), then prepend
the table's bindings:

```rust
fn app_keybindings() -> Vec<KeyBinding> {
    let mut bindings = commands::bindings();
    bindings.extend(vec![
        // Surface mechanics: overlay navigation and text movement. These
        // are not user-facing commands, so they stay a literal list.
        // TextInput: Left Right Home End cmd-left cmd-right shift-left
        // shift-right Backspace Delete Cut Copy Paste SelectAll
        // ShowCharacterPalette
        // Editor movement/editing: MoveLeft MoveRight MoveUp MoveDown
        // SelectLeft SelectRight SelectUp SelectDown MoveWordLeft
        // MoveWordRight SelectWordLeft SelectWordRight LineStart LineEnd
        // SelectLineStart SelectLineEnd DocStart DocEnd PageUp PageDown
        // Backspace Delete DeleteWordLeft Newline InsertTab Outdent
        // DismissCompletion
        // Overlay navigation: FindBar(3) Palette(4) Finder(6) Search(4)
        // InstallOverlay(4) ThemePicker(4) Shortcuts(1) GraphView(1)
        // DiffView(1)
        // Sidebar navigation: SidebarUp SidebarDown SidebarOpen
        // SidebarExpand SidebarCollapse SidebarEditCommit SidebarEditCancel
        //
        // Every one of these is copied across verbatim from the current
        // list; none of them changes.
        KeyBinding::new(&platform::keybinding("left"), input::Left, Some("TextInput")),
        // …etc, verbatim…
    ]);
    bindings
}
```

Update the count assertion in `every_keybinding_parses_and_binds` to the
new total (run the test once to read the actual number from the failure).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test commands:: && cargo test every_keybinding_parses_and_binds`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/commands.rs src/main.rs
git commit -m "feat: keybindings derive from the command table"
```

---

### Task 4: `menus()` — drive the macOS menu bar from the table

**Files:**
- Modify: `src/commands.rs`, `src/main.rs:293-355` (`app_menus`)

**Interfaces:**
- Consumes: `COMMANDS`, `MenuId`.
- Produces: `pub fn menus(recents: &[String]) -> Vec<Menu>`; `pub fn menu_title(id: MenuId) -> &'static str`; `pub fn items_for(id: MenuId) -> Vec<&'static Command>`.

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/commands.rs`:

```rust
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
        let menus = menus(&[]);
        let view = menus
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
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test commands::`
Expected: FAIL — `cannot find function 'items_for' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/commands.rs`:

```rust
use gpui::{Menu, MenuItem};

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

/// Commands in one menu, in declaration order (which the table keeps
/// grouped, so group indices come out ascending).
pub fn items_for(id: MenuId) -> Vec<&'static Command> {
    COMMANDS
        .iter()
        .filter(|c| matches!(c.menu, Some((m, _)) if m == id))
        .collect()
}

/// The menu bar. `recents` fills the Open Recent submenu.
pub fn menus(recents: &[String]) -> Vec<Menu> {
    const ORDER: [MenuId; 7] = [
        MenuId::File, MenuId::Edit, MenuId::Format,
        MenuId::View, MenuId::Go, MenuId::Tools, MenuId::Help,
    ];
    ORDER
        .iter()
        .filter_map(|&id| {
            let cmds = items_for(id);
            if cmds.is_empty() {
                return None;
            }
            let mut items: Vec<MenuItem> = Vec::new();
            let mut group = cmds[0].menu.unwrap().1;
            for cmd in cmds {
                let g = cmd.menu.unwrap().1;
                if g != group {
                    items.push(MenuItem::Separator);
                    group = g;
                }
                items.push(MenuItem::Action {
                    name: cmd.label.into(),
                    action: (cmd.action)(),
                    os_action: None,
                });
                // Open Recent hangs off the File menu's open entry.
                if cmd.id == "open" && !recents.is_empty() {
                    items.push(MenuItem::Submenu(Menu {
                        name: "Open Recent".into(),
                        items: recent_items(recents),
                    }));
                }
            }
            Some(Menu { name: menu_title(id).into(), items })
        })
        .collect()
}

fn recent_items(recents: &[String]) -> Vec<MenuItem> {
    recents
        .iter()
        .take(8)
        .enumerate()
        .map(|(ix, name)| match ix {
            0 => MenuItem::action(name.clone(), crate::workspace::OpenRecent0),
            1 => MenuItem::action(name.clone(), crate::workspace::OpenRecent1),
            2 => MenuItem::action(name.clone(), crate::workspace::OpenRecent2),
            3 => MenuItem::action(name.clone(), crate::workspace::OpenRecent3),
            4 => MenuItem::action(name.clone(), crate::workspace::OpenRecent4),
            5 => MenuItem::action(name.clone(), crate::workspace::OpenRecent5),
            6 => MenuItem::action(name.clone(), crate::workspace::OpenRecent6),
            _ => MenuItem::action(name.clone(), crate::workspace::OpenRecent7),
        })
        .collect()
}
```

Replace the body of `app_menus` in `src/main.rs`:

```rust
fn app_menus(recents: &[String]) -> Vec<Menu> {
    let mut menus = vec![Menu {
        name: "SuperMD".into(),
        items: vec![
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Quit SuperMD", Quit),
        ],
    }];
    menus.extend(commands::menus(recents));
    menus
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test commands::`
Expected: PASS, 10 tests.

- [ ] **Step 5: Commit**

```bash
git add src/commands.rs src/main.rs
git commit -m "feat: the macOS menu bar derives from the command table"
```

---

### Task 5: `help_sections()` — drive the ⌘/ dialog from the table

**Files:**
- Modify: `src/commands.rs`, `src/workspace.rs:101-158` (delete `SHORTCUTS`), and the ⌘/ render site

**Interfaces:**
- Consumes: `COMMANDS`, `HelpSection`, `glyphs`.
- Produces: `pub fn help_sections() -> Vec<(&'static str, Vec<(String, &'static str)>)>` — `(section title, [(glyphs, label)])`.

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/commands.rs`:

```rust
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
        let rows: Vec<&String> =
            sections.iter().flat_map(|(_, rows)| rows.iter().map(|(k, _)| k)).collect();
        assert!(rows.iter().any(|k| k.as_str() == "⌘ 1"), "canonical panel key shown");
        assert!(
            !rows.iter().any(|k| k.as_str() == "⌘ B" ),
            "the ⌘B alias is not listed twice"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test commands::`
Expected: FAIL — `cannot find function 'help_sections' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/commands.rs`:

```rust
fn help_title(section: HelpSection) -> &'static str {
    match section {
        HelpSection::General => "General",
        HelpSection::Editor => "Editor",
        HelpSection::Preview => "Preview & read-only tabs",
        HelpSection::Sidebar => "Sidebar",
    }
}

/// The ⌘/ dialog, and the source for the generated shortcut docs. Only
/// the first (canonical) key is listed; aliases bind but do not clutter.
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
            (!rows.is_empty()).then(|| (help_title(section), rows))
        })
        .collect()
}
```

In `src/workspace.rs`, delete the `SHORTCUTS` const (lines 101-158) and
change the ⌘/ render site to iterate the derivation instead:

```rust
// Was: SHORTCUTS.iter().map(|(section, rows)| …)
crate::commands::help_sections().into_iter().map(|(section, rows)| {
    let rows = rows.into_iter().map(|(keys, label)| {
        let keys = crate::platform::shortcut_glyphs(&keys);
        // …existing per-row rendering, unchanged, using `keys` and `label`…
    });
    // …existing per-section rendering, unchanged…
})
```

The row and section *rendering* is untouched; only the data source moves.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test commands:: && cargo test shortcuts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/commands.rs src/workspace.rs
git commit -m "feat: the shortcut dialog derives from the command table"
```

---

### Task 6: `popover_items()` — one menu structure on every platform

**Files:**
- Modify: `src/commands.rs`, `src/workspace.rs:3728-3900` (the ☰ popover)

**Interfaces:**
- Consumes: `menus()`, `items_for()`.
- Produces: `pub fn popover_groups() -> Vec<(&'static str, Vec<&'static Command>)>`.

This is the task that closes the defect: the popover stops being a
separate hand-written list and renders the same grouping as the menu bar.

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/commands.rs`:

```rust
    #[test]
    fn the_popover_shows_exactly_what_the_menu_bar_shows() {
        // The Linux/Windows ☰ popover and the macOS menu bar must never
        // diverge again: both project the same grouping.
        let bar: Vec<&str> = menus(&[])
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test commands::`
Expected: FAIL — `cannot find function 'popover_groups' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/commands.rs`:

```rust
/// The ☰ popover's contents: the same menus, as flat labelled groups.
pub fn popover_groups() -> Vec<(&'static str, Vec<&'static Command>)> {
    const ORDER: [MenuId; 7] = [
        MenuId::File, MenuId::Edit, MenuId::Format,
        MenuId::View, MenuId::Go, MenuId::Tools, MenuId::Help,
    ];
    ORDER
        .iter()
        .filter_map(|&id| {
            let cmds = items_for(id);
            (!cmds.is_empty()).then(|| (menu_title(id), cmds))
        })
        .collect()
}
```

In `src/workspace.rs`, replace the popover's hand-written `.child(item(…))`
chain with iteration over `popover_groups()`. Each row renders the label
and `platform::shortcut_glyphs(&commands::glyphs(key))` for its first key,
and on click dispatches the action rather than calling a method:

```rust
.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
    this.app_menu_open = false;
    window.dispatch_action((cmd.action)(), cx);
}))
```

Group titles reuse the popover's existing `divider()` helper plus a muted
label row:

```rust
for (title, cmds) in crate::commands::popover_groups() {
    list = list.child(divider()).child(
        div()
            .px_3()
            .py(px(3.))
            .text_size(px(10.))
            .text_color(t.fg_muted)
            .child(title),
    );
    for cmd in cmds {
        let keys = cmd.keys.first().map(|k| {
            crate::platform::shortcut_glyphs(&crate::commands::glyphs(k))
        });
        list = list.child(popover_row(cmd, keys, cx));
    }
}
```

`popover_row` is the existing `item(…)` closure, adjusted to take a
`&'static Command` and an `Option<String>` instead of two `&'static str`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test commands:: && cargo test app_menu`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/commands.rs src/workspace.rs
git commit -m "fix: the ☰ popover renders the same menus as the menu bar"
```

---

### Task 7: Generated shortcut docs (golden-file test)

**Files:**
- Modify: `src/commands.rs`, `docs/site/shortcuts.md`

**Interfaces:**
- Consumes: `help_sections()`.
- Produces: `pub fn shortcut_markdown() -> String`, plus a test that fails when `docs/site/shortcuts.md` drifts from it.

**Why a test, not the example:** `supermd` is a **binary-only crate** — it
has no `[lib]` target, which is why `examples/build_docs.rs` is entirely
self-contained and imports nothing from `src/`. An example therefore
*cannot* call `commands::shortcut_markdown()`. Rather than restructure the
crate into a library for this, the check lives where it belongs: a
golden-file test. That is strictly better than a generator step anyway —
drift becomes a **failing test** instead of something a contributor has to
remember to re-run. `cargo run --example build_docs` then renders the
already-correct markdown into `site/docs/` exactly as it does today.

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/commands.rs`:

```rust
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
    /// `UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table` and commit
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
        assert_eq!(
            on_disk, generated,
            "docs/site/shortcuts.md is stale; rerun with UPDATE_DOCS=1"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test commands::shortcut`
Expected: FAIL — `cannot find function 'shortcut_markdown' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/commands.rs`:

```rust
/// `docs/site/shortcuts.md`, generated. Both platform columns come from
/// the one macOS-form declaration in the table.
pub fn shortcut_markdown() -> String {
    let mut out = String::from(
        "# Keyboard shortcuts\n\nOn Linux and Windows, read ⌘ as Ctrl.\n\n",
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
```

In `src/platform.rs`, expose the existing private `translate_glyphs` so the
docs always render the non-macOS column regardless of the host OS:

```rust
/// Always the non-macOS rendering, for generating cross-platform docs.
/// (`shortcut_glyphs` returns the macOS form when built on macOS; the
/// docs need both columns on every host.)
pub fn translate_glyphs_for_docs(mac: &str) -> String {
    translate_glyphs(mac)
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
UPDATE_DOCS=1 cargo test commands::shortcut_docs_match_the_table
cargo test commands::shortcut
cargo run --example build_docs
```
Expected: the first regenerates `docs/site/shortcuts.md`; the second passes;
the third re-renders `site/docs/`.

- [ ] **Step 5: Commit**

```bash
git add src/commands.rs src/platform.rs docs/site/shortcuts.md site/docs
git commit -m "feat: shortcut docs generate from the command table"
```

---

### Task 8: Reachability invariant — the test that would have caught this

**Files:**
- Modify: `src/commands.rs`

**Interfaces:**
- Consumes: `COMMANDS`.
- Produces: no new API; replaces the count-based change detector with a property.

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/commands.rs`:

```rust
    #[test]
    fn every_command_is_reachable_from_some_surface() {
        // A command with no menu entry, no ⌘/ row and no keystroke is
        // invisible: it exists but no user can find it. This is the
        // property that the old `assert_eq!(bindings.len(), N)` counter
        // could never express.
        let unreachable: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.menu.is_none() && c.help.is_none() && c.keys.is_empty())
            .map(|c| c.id)
            .collect();
        assert!(unreachable.is_empty(), "unreachable commands: {unreachable:?}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test commands::every_command_is_reachable`
Expected: FAIL — reports `["focus_sidebar"]`, which has no keys, no menu and no help row.

- [ ] **Step 3: Write minimal implementation**

`focus_sidebar` is genuinely unreachable after ⌘1 became a toggle. Give it
a home in the Go menu rather than deleting it (it is still useful for
keyboard users who want focus without toggling):

```rust
    ws::FocusSidebar => { id: "focus_sidebar", label: "Focus Sidebar", keys: [],
        ctx: None, menu: Some((Go, 1)), help: None },
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test commands::`
Expected: PASS, all tests.

- [ ] **Step 5: Full verification and commit**

```bash
cargo test
cargo build
git add src/commands.rs
git commit -m "test: every command must be reachable from some surface"
```

Expected: 615+ tests pass, build clean.

---

### Task 9: New actions for Format, Graph, Flux and Install

**Files:**
- Modify: `src/workspace.rs:22-73` (the `actions!` list), `src/workspace.rs:1155-1200` and `:1342-1452` (palette built-ins), `src/editor/mod.rs:45-55` (the editor `actions!` list)

**Interfaces:**
- Consumes: the palette's existing `__graph` / `__flux` / `__install` handlers.
- Produces: `workspace::ToggleGraph`, `workspace::ToggleFlux`, `workspace::InstallPlugins`, and `editor::ToggleCode`, `ToggleStrike`, `ToggleLink`, `ToggleHeading`, `ToggleQuote`.

- [ ] **Step 1: Write the failing test**

Add to `src/workspace.rs`'s test module:

```rust
    #[gpui::test]
    fn graph_and_flux_actions_replace_their_palette_string_ids(cx: &mut TestAppContext) {
        let fx = tempfile::tempdir().unwrap();
        std::fs::write(fx.path().join("a.md"), "# A\n").unwrap();
        let (ws, cx) = open_workspace(cx, fx.path());
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
        cx.update(|_, app| assert!(ws.read(app).graph.is_some(), "ToggleGraph opens the graph"));
    }
```

`open_workspace(cx, &Path) -> (Entity<Workspace>, &mut VisualTestContext)`
is the existing helper at `src/workspace.rs:4153`; it wraps `open_arg` and
installs the theme/highlight/session globals the workspace needs.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test graph_and_flux_actions`
Expected: FAIL — `cannot find value 'ToggleFlux' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add `ToggleGraph`, `ToggleFlux`, `InstallPlugins` to the `actions!` list in
`src/workspace.rs`. Add handler methods that call the *same* code the
palette's string-id arms call, then have those arms delegate:

```rust
    fn toggle_flux(&mut self, _: &ToggleFlux, _w: &mut Window, cx: &mut Context<Self>) {
        // Body moved verbatim from the `id == "__flux"` arm.
        cx.update_global::<crate::theme::ThemeState, _>(|state, _| {
            state.settings.flux.enabled = !state.settings.flux.enabled;
            state.flux_blend = crate::flux::current_blend(&state.settings.flux);
        });
        let settings = cx.global::<crate::theme::ThemeState>().settings.clone();
        let _ = crate::settings::save(&crate::settings::config_dir(), &settings);
        cx.notify();
    }
```

The palette arm becomes `if id == "__flux" { self.toggle_flux(&ToggleFlux, window, cx); return; }`
so the palette keeps working while the action becomes the real entry point.
Repeat for `__graph` → `toggle_graph`, `__install` → `install_plugins`.

Add `ToggleCode`, `ToggleStrike`, `ToggleLink`, `ToggleHeading`, `ToggleQuote`
to the editor `actions!` list, each dispatching to the `formatting.rs`
function the selection toolbar already calls, and register them with
`.on_action(cx.listener(Self::…))` alongside `toggle_bold`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/workspace.rs src/editor/mod.rs
git commit -m "feat: actions for graph, flux, install and the formatting toggles"
```

---

### Task 10: The menu restructure

**Files:**
- Modify: `src/commands.rs` (the table only)

**Interfaces:**
- Consumes: the actions from Task 9.
- Produces: the Edit, Format, Go and Tools menus populated.

This is the payoff: because every surface derives from the table, the
restructure is a table edit and nothing else.

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/commands.rs`:

```rust
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
        for expected in ["Bold", "Italic", "Code", "Strikethrough", "Link", "Heading", "Quote"] {
            assert!(fmt.contains(&expected), "Format menu is missing {expected}");
        }
    }

    #[test]
    fn graph_view_reaches_a_menu_and_a_shortcut() {
        let graph = COMMANDS.iter().find(|c| c.id == "graph").expect("graph in table");
        assert_eq!(graph.keys, &["cmd-shift-g"]);
        assert_eq!(graph.menu.map(|(m, _)| m), Some(MenuId::Go));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test commands::`
Expected: FAIL — `Edit menu is missing Undo`.

- [ ] **Step 3: Write minimal implementation**

Add to the table in `src/commands.rs`:

```rust
    // ── Edit ───────────────────────────────────────────────────────
    crate::editor::Undo => { id: "undo", label: "Undo", keys: ["cmd-z"],
        ctx: Some("Editor"), menu: Some((Edit, 0)), help: Some(HEditor) },
    crate::editor::Redo => { id: "redo", label: "Redo", keys: ["cmd-shift-z"],
        ctx: Some("Editor"), menu: Some((Edit, 0)), help: Some(HEditor) },
    crate::editor::Cut => { id: "cut", label: "Cut", keys: ["cmd-x"],
        ctx: Some("Editor"), menu: Some((Edit, 1)), help: None },
    crate::editor::Copy => { id: "copy", label: "Copy", keys: ["cmd-c"],
        ctx: Some("Editor"), menu: Some((Edit, 1)), help: None },
    crate::editor::Paste => { id: "paste", label: "Paste", keys: ["cmd-v"],
        ctx: Some("Editor"), menu: Some((Edit, 1)), help: None },
    crate::editor::SelectAll => { id: "select_all", label: "Select All",
        keys: ["cmd-a"], ctx: Some("Editor"), menu: Some((Edit, 1)), help: None },
    // find entries move from menu: None to the Edit menu, group 2
    // (edit the existing `find`, `find_next`, `find_prev` rows).

    // ── Format ─────────────────────────────────────────────────────
    // `bold` and `italic` change menu: None → Some((Format, 0)).
    crate::editor::ToggleCode => { id: "code", label: "Code", keys: [],
        ctx: Some("Editor"), menu: Some((Format, 0)), help: None },
    crate::editor::ToggleStrike => { id: "strike", label: "Strikethrough",
        keys: [], ctx: Some("Editor"), menu: Some((Format, 0)), help: None },
    crate::editor::ToggleLink => { id: "link", label: "Link", keys: [],
        ctx: Some("Editor"), menu: Some((Format, 0)), help: None },
    crate::editor::ToggleHeading => { id: "heading", label: "Heading", keys: [],
        ctx: Some("Editor"), menu: Some((Format, 1)), help: None },
    crate::editor::ToggleQuote => { id: "quote", label: "Quote", keys: [],
        ctx: Some("Editor"), menu: Some((Format, 1)), help: None },

    // ── Go / View / Tools additions ────────────────────────────────
    ws::ToggleGraph => { id: "graph", label: "Graph View", keys: ["cmd-shift-g"],
        ctx: None, menu: Some((Go, 0)), help: Some(General) },
    ws::ToggleFlux => { id: "flux", label: "Flux (adaptive theme)",
        keys: ["ctrl-cmd-n"], ctx: None, menu: Some((View, 2)), help: Some(General) },
    ws::InstallPlugins => { id: "install_plugins", label: "Install Plugins…",
        keys: [], ctx: None, menu: Some((Tools, 1)), help: None },
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test`
Expected: PASS — including the popover/menu-bar parity test from Task 6,
which now proves the new menus reach Linux and Windows too.

- [ ] **Step 5: Regenerate docs, verify coverage, commit**

```bash
cargo run --example build_docs
cargo llvm-cov --summary-only --fail-under-lines 90
git add -A
git commit -m "feat: Edit, Format, Go and Tools menus"
```

Expected: coverage ≥ 90%; `docs/site/shortcuts.md` and `site/docs/` reflect
the new commands automatically.

---

## Verification

After Task 10:

```bash
cargo test          # expect 630+ passing, 0 failed
cargo build         # clean; the 4 pre-existing warnings only
cargo llvm-cov --summary-only --fail-under-lines 90
```

Manual check on Linux: press ☰ and confirm the popover now shows all seven
menu groups rather than eight loose items — this is the defect the plan set
out to close, and it is only visible off macOS.

## What this plan does not cover

Stages 3–6 of the spec, each getting its own plan:

- **Stage 3** — the shortcut scheme written into the docs prose (the four rebinds themselves land in Task 2 of this plan).
- **Stage 4** — the About dialog and the Help/app-menu About entries.
- **Stage 5** — the UI icon set, titlebar panel toggles, sidebar `+`, Show Changes button.
- **Stage 6** — the status bar (droppable).
