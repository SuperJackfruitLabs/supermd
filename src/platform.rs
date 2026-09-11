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

/// SuperMD's bundle identifier, hand-kept in step with the plist
/// templates in `scripts/bundle_macos.sh` and `scripts/bundle_mas.sh`
/// (this binary cannot read either at compile time).
#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "com.superjackfruit.supermd";

/// The Uniform Type Identifier both bundle scripts declare
/// `LSHandlerRank: Owner` for.
#[cfg(target_os = "macos")]
const MARKDOWN_UTI: &str = "net.daringfireball.markdown";

/// Whether SuperMD is currently registered with LaunchServices as the
/// Editor-role default for Markdown files. Read-only query.
#[cfg(target_os = "macos")]
pub fn is_default_markdown_handler() -> bool {
    launch_services::current_editor(MARKDOWN_UTI).as_deref() == Some(BUNDLE_ID)
}

#[cfg(not(target_os = "macos"))]
pub fn is_default_markdown_handler() -> bool {
    false
}

/// Ask LaunchServices to make SuperMD the default handler for Markdown
/// files.
///
/// SPIKE FINDING (2026-09-11, task 6): `LSSetDefaultRoleHandlerForContentType`
/// is refused under the App Sandbox. Verified empirically against a
/// build signed with the `com.apple.security.app-sandbox` entitlement
/// (not the DMG build, and not by reading documentation): the call
/// returns OSStatus -54 (`permErr`) and the LaunchServices database is
/// left untouched, reproducibly, while the identical call from an
/// unsandboxed build returns 0 and the database updates.
/// `NSWorkspace.setDefaultApplication(at:toOpen:)` (macOS 14+) was
/// refused the same way (NSOSStatusErrorDomain -54) in the same test.
/// So: the LaunchServices call is always attempted -- it is correct for
/// the unsandboxed DMG build, and costs nothing to also try from the
/// Mac App Store build in case a future OS stops refusing it -- and a
/// `permErr` is translated into the one thing an App Store user can
/// actually do about it today: the Finder "Open with" / "Change All…"
/// steps this command exists to save most people from finding on their
/// own.
#[cfg(target_os = "macos")]
pub fn request_default_markdown_handler() -> Result<(), String> {
    let status = launch_services::set_default_editor(MARKDOWN_UTI, BUNDLE_ID);
    if status == 0 {
        Ok(())
    } else {
        Err(describe_default_handler_status(status))
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request_default_markdown_handler() -> Result<(), String> {
    Err("not supported".into())
}

/// Turn a nonzero LaunchServices `OSStatus` into a message the user can
/// act on. Pure and platform-independent so it is directly testable
/// everywhere; its only caller is macOS-only.
fn describe_default_handler_status(status: i32) -> String {
    if status == -54 {
        "SuperMD can't set itself as the default automatically here -- \
         the App Sandbox blocks that for Mac App Store apps. In Finder, \
         right-click a Markdown file, choose Get Info, pick SuperMD \
         under \"Open with\", then click \"Change All…\"."
            .to_string()
    } else {
        format!("LaunchServices refused the request (status {status})")
    }
}

/// Minimal LaunchServices/CoreServices FFI: no crate on crates.io
/// wraps `LSCopyDefaultRoleHandlerForContentType` /
/// `LSSetDefaultRoleHandlerForContentType`, so this hand-declares the
/// two functions rather than adding a dependency for them. String
/// marshaling reuses `objc2_foundation::NSString`, already a
/// dependency: NSString and CFString are toll-free bridged, so a live
/// `NSString*` is a valid `CFStringRef`.
#[cfg(target_os = "macos")]
mod launch_services {
    use objc2_foundation::NSString;
    use std::ffi::{c_char, c_void, CStr};

    type CFStringRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCStringPtr(the_string: CFStringRef, encoding: u32) -> *const c_char;
        fn CFStringGetCString(
            the_string: CFStringRef,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(cf: CFStringRef);
    }

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn LSCopyDefaultRoleHandlerForContentType(content_type: CFStringRef, role: u32) -> CFStringRef;
        fn LSSetDefaultRoleHandlerForContentType(
            content_type: CFStringRef,
            role: u32,
            handler_bundle_id: CFStringRef,
        ) -> i32;
    }

    /// `kLSRolesEditor`, from `LaunchServices/LSInfo.h`.
    const ROLE_EDITOR: u32 = 0x0000_0004;
    /// `kCFStringEncodingUTF8`, from `CoreFoundation/CFString.h`.
    const UTF8: u32 = 0x0800_0100;

    fn as_cfstring_ref(s: &NSString) -> CFStringRef {
        (s as *const NSString).cast()
    }

    fn cfstring_to_owned(s: CFStringRef) -> Option<String> {
        unsafe {
            let ptr = CFStringGetCStringPtr(s, UTF8);
            if !ptr.is_null() {
                return Some(CStr::from_ptr(ptr).to_string_lossy().into_owned());
            }
            // No fast-path pointer for this string's internal encoding;
            // CFStringGetCString always works, just with a copy.
            let mut buf = vec![0i8; 512];
            if CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) != 0 {
                Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
            } else {
                None
            }
        }
    }

    /// The bundle identifier LaunchServices has registered as the
    /// Editor-role default for `content_type`, if any.
    pub fn current_editor(content_type: &str) -> Option<String> {
        let content_type = NSString::from_str(content_type);
        unsafe {
            let raw = LSCopyDefaultRoleHandlerForContentType(as_cfstring_ref(&content_type), ROLE_EDITOR);
            if raw.is_null() {
                return None;
            }
            // LSCopy* follows the Create Rule: this call owns the
            // returned reference and must release it.
            let owned = cfstring_to_owned(raw);
            CFRelease(raw);
            owned
        }
    }

    /// Ask LaunchServices to make `bundle_id` the Editor-role default
    /// for `content_type`. Returns the raw OSStatus; 0 is success.
    pub fn set_default_editor(content_type: &str, bundle_id: &str) -> i32 {
        let content_type = NSString::from_str(content_type);
        let bundle_id = NSString::from_str(bundle_id);
        unsafe {
            LSSetDefaultRoleHandlerForContentType(
                as_cfstring_ref(&content_type),
                ROLE_EDITOR,
                as_cfstring_ref(&bundle_id),
            )
        }
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

    /// Never true off macOS, and never panics anywhere.
    #[test]
    fn the_default_handler_query_is_safe_on_every_platform() {
        let answer = is_default_markdown_handler();
        if !MACOS {
            assert!(!answer, "only macOS has a Markdown handler to be");
        }
    }

    #[test]
    fn permission_error_becomes_finder_instructions() {
        let msg = describe_default_handler_status(-54);
        assert!(msg.contains("Get Info"), "got {msg}");
        assert!(msg.contains("Change All"), "got {msg}");
    }

    #[test]
    fn other_ls_errors_include_the_status_code() {
        let msg = describe_default_handler_status(-43);
        assert!(msg.contains("-43"), "got {msg}");
    }

    #[test]
    fn reveal_spawns_no_subprocess_on_macos() {
        // The sandbox forbids spawning /usr/bin/open; NSWorkspace is the
        // sanctioned route. This asserts the policy the shell reads.
        assert_eq!(
            reveal_backend(),
            if cfg!(target_os = "macos") { "NSWorkspace" } else { "spawn" }
        );
    }
}

/// Which mechanism `reveal_dir` uses. macOS must not spawn `open` —
/// the App Sandbox blocks subprocesses.
pub fn reveal_backend() -> &'static str {
    if cfg!(target_os = "macos") { "NSWorkspace" } else { "spawn" }
}

/// Open a directory in the system file manager.
#[cfg(target_os = "macos")]
pub fn reveal_dir(path: &std::path::Path) {
    use objc2_foundation::{NSArray, NSString, NSURL};
    let s = NSString::from_str(&path.to_string_lossy());
    let url = NSURL::fileURLWithPath(&s);
    let urls = NSArray::from_retained_slice(&[url]);
    objc2_app_kit::NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&urls);
}

#[cfg(not(target_os = "macos"))]
pub fn reveal_dir(path: &std::path::Path) {
    let tool = if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
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
