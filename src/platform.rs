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
/// macOS puts About in the app menu, above Services; every other
/// platform puts it in Help. The command table asks here rather than
/// carrying a `cfg!()` of its own.
pub const ABOUT_IN_APP_MENU: bool = MACOS;

/// Always the non-macOS rendering, for generating cross-platform docs.
/// (`shortcut_glyphs` returns the macOS form when built on macOS; the
/// docs need both columns whatever the host.)
pub fn translate_glyphs_for_docs(mac: &str) -> String {
    translate_glyphs(mac)
}

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

    #[test]
    fn public_keybinding_translates_only_off_macos() {
        let expected = if MACOS { "cmd-shift-f" } else { "ctrl-shift-f" };
        assert_eq!(keybinding("cmd-shift-f"), expected);
        let expected = if MACOS { "ctrl-cmd-f" } else { "ctrl-alt-f" };
        assert_eq!(keybinding("ctrl-cmd-f"), expected);
        assert_eq!(keybinding("escape"), "escape");
    }

    #[test]
    fn public_shortcut_glyphs_translate_only_off_macos() {
        let expected = if MACOS { "⌘ S" } else { "Ctrl S" };
        assert_eq!(shortcut_glyphs("⌘ S"), expected);
        let expected = if MACOS { "⌃ ⌘ F" } else { "Ctrl Alt F" };
        assert_eq!(shortcut_glyphs("⌃ ⌘ F"), expected);
    }

    #[test]
    fn home_dir_agrees_with_env_fallback_order() {
        let expected = pick_home(
            std::env::var("HOME").ok().filter(|s| !s.is_empty()),
            std::env::var("USERPROFILE").ok().filter(|s| !s.is_empty()),
        );
        assert_eq!(home_dir(), expected);
        assert!(!home_dir().as_os_str().is_empty());
    }
}

/// Open a directory in the system file manager.
pub fn reveal_dir(path: &std::path::Path) {
    let tool = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(tool).arg(path).spawn();
}

/// The installer-planted default-plugins payload, probed relative to
/// the running executable: macOS app bundle Resources, deb lib dir,
/// or a plugins/ dir beside the binary (tarball, Windows). Dev runs
/// (under target/) have no payload and seed nothing.
pub fn bundled_plugins_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    bundled_plugins_dir_for(&exe)
}

fn bundled_plugins_dir_for(exe: &std::path::Path) -> Option<PathBuf> {
    if exe.components().any(|c| c.as_os_str() == "target") {
        return None;
    }
    let dir = exe.parent()?;
    [
        dir.join("../Resources/plugins"),
        dir.join("../lib/supermd/plugins"),
        dir.join("plugins"),
    ]
    .into_iter()
    .find(|p| p.is_dir())
}

#[cfg(test)]
mod bundled_tests {
    use super::*;

    #[test]
    fn probes_each_installer_layout_and_skips_dev() {
        let root = tempfile::tempdir().unwrap();
        // macOS bundle layout
        let mac = root.path().join("SuperMD.app/Contents");
        std::fs::create_dir_all(mac.join("MacOS")).unwrap();
        std::fs::create_dir_all(mac.join("Resources/plugins")).unwrap();
        let found = bundled_plugins_dir_for(&mac.join("MacOS/supermd")).unwrap();
        assert!(found.ends_with("Resources/plugins"));
        // deb layout
        let deb = root.path().join("usr");
        std::fs::create_dir_all(deb.join("bin")).unwrap();
        std::fs::create_dir_all(deb.join("lib/supermd/plugins")).unwrap();
        let found = bundled_plugins_dir_for(&deb.join("bin/supermd")).unwrap();
        assert!(found.ends_with("lib/supermd/plugins"));
        // beside-the-binary layout (tarball / Windows)
        let flat = root.path().join("flat");
        std::fs::create_dir_all(flat.join("plugins")).unwrap();
        let found = bundled_plugins_dir_for(&flat.join("supermd")).unwrap();
        assert!(found.ends_with("plugins"));
        // dev run: a target/ path never seeds even if plugins/ exists
        let dev = root.path().join("proj/target/debug");
        std::fs::create_dir_all(dev.join("plugins")).unwrap();
        assert!(bundled_plugins_dir_for(&dev.join("supermd")).is_none());
        // no payload anywhere
        let bare = root.path().join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        assert!(bundled_plugins_dir_for(&bare.join("supermd")).is_none());
    }
}
