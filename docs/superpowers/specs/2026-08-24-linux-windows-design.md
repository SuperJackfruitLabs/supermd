# Linux & Windows Builds Design

**Date:** 2026-08-24
**Status:** Approved for planning

## Purpose

Ship SuperMD on Linux and Windows at feature parity with macOS:
native window chrome per platform, platform-correct keybindings,
fonts, and config paths, real packages (deb + tarball, Windows
installer + zip), and a three-OS CI/release pipeline. Windows code
signing is explicitly deferred (additive later via CI secrets, like
macOS notarization was).

## Feasibility (verified)

GPUI 0.2.2 ships Linux (Wayland + X11, default features) and Windows
backends; `WindowControlArea::{Drag, Close, Max, Min}` exists for
custom-chrome hit-testing (including Windows snap layouts), and Linux
supports `WindowDecorations::Client` with a runtime
`window.decorations()` fallback signal. All app dependencies (gix,
ignore, grep-*, notify, nucleo, merman, resvg, ropey, inkjet) are
cross-platform.

## Component 1: Platform layer — `src/platform.rs` (new)

Every per-OS decision lives here; the rest of the code asks.

```rust
/// Translate a macOS-authored keybinding for the current platform.
/// mac: identity. Linux/Windows: "ctrl-cmd-" → "ctrl-alt-",
/// then "cmd-" → "ctrl-". Existing literal "ctrl-" bindings pass
/// through untouched (no double-mapping; translation runs on the
/// original string only).
pub fn keybinding(mac_binding: &str) -> String;

/// Body/mono font families for the current OS:
/// macOS: ".SystemUIFont" / "Menlo"
/// Windows: "Segoe UI" / "Consolas"
/// Linux: "DejaVu Sans" / "DejaVu Sans Mono"
pub fn body_font() -> &'static str;
pub fn mono_font() -> &'static str;

/// Home directory: $HOME, else %USERPROFILE% (Windows), else ".".
pub fn home_dir() -> PathBuf;

/// True only on macOS (used to branch titlebar layout, menus,
/// install offer).
pub const MACOS: bool = cfg!(target_os = "macos");
```

- `main.rs` wraps every `KeyBinding::new(k, …)` with
  `platform::keybinding(k)` (one mechanical pass; bindings stay
  authored in macOS notation in one place). The SHORTCUTS dialog
  renders glyphs from the same translation (⌘→Ctrl, ⌃⌘→Ctrl+Alt,
  ⌥→Alt, ⇧→Shift on non-mac).
- `theme.rs` `Theme::light()/dark()` take families from
  `platform::{body_font, mono_font}`; `diagram.rs` keeps its dot-name
  substitution (non-mac names pass straight through).
- `settings.rs::config_dir()` uses `platform::home_dir()`; the
  directory stays `.supermd` on every OS.
- `install.rs` bodies become `#[cfg(target_os = "macos")]`;
  non-mac stubs: `needs_install → false`,
  `move_to_applications → Err("unsupported")`.
- `update.rs` unchanged (curl ships with modern Windows and
  effectively all Linux distros; failures were already silent).

## Component 2: Window chrome

- **macOS:** unchanged (transparent titlebar, traffic-light strip,
  hidden-sidebar inset).
- **Windows:** transparent titlebar retained; the tab bar's right end
  gains our own minimize / maximize / close buttons (theme-colored,
  hover states; close hovers red) tagged
  `window_control_area(Min/Max/Close)`; the drag filler keeps
  `WindowControlArea::Drag`. No traffic-light inset or sidebar strip.
- **Linux:** window opens requesting `WindowDecorations::Client`; at
  render time `window.decorations()` decides: `Client{..}` → same
  custom buttons as Windows; `Server` → no buttons, system titlebar,
  and the app row starts at the window top. Best-effort across
  compositors, recorded as such.
- Window-control actions use gpui's window methods (the exact
  minimize/zoom/close calls resolved against the vendored source at
  implementation; they exist — the Windows backend's own caption
  handling depends on them).
- The workspace layout branches on `platform::MACOS` +
  `window.decorations()` only inside `render_titlebar` and the
  sidebar strip — nowhere else.

## Component 3: App menu on non-mac

`cx.set_menus` stays macOS-only. On Linux/Windows the tab bar's left
end gets a **☰ button** opening a popover (same overlay pattern as
the theme picker): New File, Open…, Open Recent (startup snapshot),
Search in Workspace, Show Changes, Toggle Preview, Theme…, Shortcuts,
each dispatching the existing actions and showing its translated
shortcut. Escape or outside-click dismisses. No nested submenus —
Open Recent items render inline under a divider.

## Component 4: Packaging

- **Linux — `scripts/bundle_linux.sh`:** builds release, stages
  `supermd` + `assets/linux/supermd.desktop` (Name=SuperMD,
  MimeType=text/markdown;text/plain, Exec=supermd %F,
  Icon=supermd) + PNG icons (128/512, generated from the existing
  icon pipeline via a new `scripts/make_icon_png.py` reusing
  make_icon.py's renderer) + `install.sh` (copies to ~/.local) →
  `supermd-linux-<arch>.tar.gz`.
- **Linux — deb:** `cargo-deb` metadata in Cargo.toml
  (`[package.metadata.deb]`: assets for binary, .desktop, icons;
  section editors; depends auto). CI runs `cargo deb` on the ubuntu
  job → `supermd_<ver>_<arch>.deb`.
- **Windows — `scripts/bundle_windows.ps1`:** cargo build --release;
  stage exe + generate `supermd.ico` (new `scripts/make_icon_ico.py`,
  same renderer, ICO container) → portable
  `supermd-windows-<arch>.zip`.
- **Windows — installer:** `scripts/windows/supermd.iss` (Inno
  Setup; `iscc` is preinstalled on windows-latest): installs to
  {autopf}\SuperMD, Start Menu shortcut, optional `.md`/`.markdown`
  association (HKA registry via [Registry] section), uninstaller →
  `SuperMD-Setup-<ver>.exe`. Unsigned for now; a future signing step
  slots between compile and upload.
- Windows binary must not open a console: `#![windows_subsystem =
  "windows"]` attribute (cfg_attr) at the top of main.rs.

## Component 5: CI & release

- **ci.yml:** matrix {macos-latest, ubuntu-latest, windows-latest} ×
  (cargo test + cargo build). Ubuntu installs GPUI build deps via
  apt: libxkbcommon-dev, libxkbcommon-x11-dev, libwayland-dev,
  libx11-xcb-dev, libxcb1-dev, libfontconfig1-dev, libfreetype6-dev
  (list finalized empirically against CI on the first run — expected
  to iterate).
- **release.yml:** three build jobs (mac job unchanged with
  sign/notarize; linux job → tar.gz + deb; windows job → zip +
  setup exe), artifacts uploaded, then a final `publish` job creates
  the release with every artifact attached (replaces the current
  in-job `gh release create`).
- **Site:** the download CTA becomes three links (macOS primary,
  Linux, Windows) pointing at the latest-release assets; note text
  updated.

## Explicit limits (v1, recorded)

- Opening a file from Explorer/Files opens a new app instance
  (single-instance IPC is a follow-up).
- Linux CSD is best-effort per compositor; Server-decoration fallback
  is the safety net.
- Windows builds unsigned (SmartScreen click-through) until a
  certificate lands.
- No global menu bar off macOS — the ☰ popover and shortcuts carry
  the same actions.
- Verified by CI build + full test suite on all three OSes; first
  interactive smoke on real Linux/Windows hardware is manual.

## Testing strategy

- `platform.rs`: pure tests — binding translation (cmd→ctrl,
  ctrl-cmd→ctrl-alt, literal ctrl untouched, alt/shift preserved),
  home-dir env fallback, font names per cfg.
- SHORTCUTS glyph translation: pure test on the formatting helper.
- All existing suites must stay green on all three CI OSes — this is
  the port's regression net (config-dir tests already use tempdirs;
  git tests use the git CLI, present on all runners).
- Chrome/menu/popover: compile + CI; interactive smoke deferred to
  real hardware.

## Out of scope

Windows/Linux code signing, AppImage/Flatpak/Snap, single-instance
IPC, per-platform auto-update beyond the existing pill, ARM Linux
builds (x86_64 first; the script is arch-parameterized for later).
