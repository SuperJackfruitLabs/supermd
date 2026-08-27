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
