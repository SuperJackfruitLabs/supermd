//! Per-OS decisions in one place: keybinding translation, fonts, the
//! home directory, and the macOS flag. Everything else asks here
//! instead of sprinkling cfg!() through the codebase.

use std::path::PathBuf;

pub const MACOS: bool = cfg!(target_os = "macos");

/// Platform-independent core of `keybinding` (testable everywhere):
/// macOS-authored bindings become ctrl-based. Order matters —
/// ctrl-cmd first so it doesn't double-translate.
fn translate(binding: &str) -> String {
    binding
        .replace("ctrl-cmd-", "ctrl-alt-")
        .replace("cmd-", "ctrl-")
}

/// Translate a macOS-authored keybinding for the current platform.
pub fn keybinding(mac_binding: &str) -> String {
    if MACOS {
        mac_binding.to_string()
    } else {
        translate(mac_binding)
    }
}

/// Core of `shortcut_glyphs` (testable everywhere).
fn translate_glyphs(mac: &str) -> String {
    mac.replace("⌃ ⌘", "Ctrl Alt")
        .replace("⌘", "Ctrl")
        .replace("⇧", "Shift")
        .replace("⌥", "Alt")
        .replace("⌃", "Ctrl")
}

/// Shortcut labels for the ⌘/ dialog on the current platform.
pub fn shortcut_glyphs(mac: &str) -> String {
    if MACOS {
        mac.to_string()
    } else {
        translate_glyphs(mac)
    }
}

fn pick_home(home: Option<String>, userprofile: Option<String>) -> PathBuf {
    home.or(userprofile)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// $HOME, else %USERPROFILE% (Windows), else ".".
pub fn home_dir() -> PathBuf {
    pick_home(
        std::env::var("HOME").ok().filter(|s| !s.is_empty()),
        std::env::var("USERPROFILE").ok().filter(|s| !s.is_empty()),
    )
}

pub fn body_font() -> &'static str {
    if cfg!(target_os = "macos") {
        ".SystemUIFont"
    } else if cfg!(target_os = "windows") {
        "Segoe UI"
    } else {
        "DejaVu Sans"
    }
}

pub fn mono_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keybindings_translate_off_macos() {
        assert_eq!(translate("cmd-shift-f"), "ctrl-shift-f");
        assert_eq!(translate("ctrl-cmd-f"), "ctrl-alt-f");
        assert_eq!(translate("cmd-="), "ctrl-=");
        assert_eq!(translate("ctrl-tab"), "ctrl-tab");
        assert_eq!(translate("alt-backspace"), "alt-backspace");
        assert_eq!(translate("escape"), "escape");
    }

    #[test]
    fn shortcut_glyphs_translate_off_macos() {
        assert_eq!(translate_glyphs("⌘ ⇧ F"), "Ctrl Shift F");
        assert_eq!(translate_glyphs("⌃ ⌘ F"), "Ctrl Alt F");
        assert_eq!(translate_glyphs("⌥ ⌫"), "Alt ⌫");
        assert_eq!(translate_glyphs("⏎"), "⏎");
    }

    #[test]
    fn home_dir_prefers_home_then_userprofile() {
        assert_eq!(
            pick_home(Some("/h".into()), Some("C:\\u".into())),
            PathBuf::from("/h")
        );
        assert_eq!(pick_home(None, Some("C:\\u".into())), PathBuf::from("C:\\u"));
        assert_eq!(pick_home(None, None), PathBuf::from("."));
    }

    #[test]
    fn fonts_are_nonempty_per_platform() {
        assert!(!body_font().is_empty());
        assert!(!mono_font().is_empty());
    }
}
