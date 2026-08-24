# Linux & Windows Builds Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SuperMD at feature parity on Linux and Windows — platform layer, native-feeling chrome, ☰ app menu, deb/tarball + Inno installer/zip packaging, and a three-OS CI/release pipeline.

**Architecture:** A single `src/platform.rs` owns every per-OS decision (keybinding translation, fonts, home dir, macOS flag); window chrome branches only inside `render_titlebar` (custom Min/Max/Close controls tagged with `WindowControlArea` on Windows/Linux-CSD, server-decoration fallback on Linux); packaging is per-OS scripts driven by a restructured three-job release workflow. The three-OS CI matrix lands FIRST — every later task iterates against it, since the dev host is macOS-only.

**Tech Stack:** GPUI 0.2.2 (linux wayland/x11 + windows backends, `WindowControlArea`, `WindowDecorations::Client`, `minimize_window`/`zoom_window`/`remove_window`), cargo-deb, Inno Setup (`iscc`, preinstalled on windows-latest).

**Spec:** `docs/superpowers/specs/2026-08-24-linux-windows-design.md`

## Global Constraints

- Config dir stays `.supermd` under `$HOME` else `%USERPROFILE%` on every OS.
- Bindings stay authored once, in macOS notation, in main.rs; translation happens at bind time via `platform::keybinding`.
- Chrome branches live only in `render_titlebar` + the sidebar strip; nowhere else.
- Windows release binary must not open a console (`windows_subsystem = "windows"`).
- macOS signing/notarization pipeline behavior unchanged.
- After every task: full local suite green, push, and confirm the three-OS CI run before starting the next task (CI is the only non-mac verifier).

---

### Task 1: Three-OS CI matrix (the feedback loop)

**Files:** Modify `.github/workflows/ci.yml`

- [ ] **Step 1:** Rewrite ci.yml:

```yaml
name: CI
on:
  push: { branches: [master] }
  pull_request:
jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [macos-latest, ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Linux build deps
        if: runner.os == 'Linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y libxkbcommon-dev libxkbcommon-x11-dev \
            libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig1-dev \
            libfreetype6-dev
      - run: cargo test
      - run: cargo build
```

- [ ] **Step 2:** Commit `ci: three-OS matrix`, push, `gh run watch` the matrix. Expect the ubuntu job to surface missing apt packages and possibly gpui cfg issues in OUR code (e.g. mac-only APIs used unconditionally — `traffic_light_position` is a plain Option field and fine; `window_control_area` is cross-platform). Fix iteratively — each fix is its own commit — until all three jobs are green. Windows job: expect line-ending or path-separator test issues (`/` vs `\`); fix tests to use `Path::join` where they don't.

- [ ] **Step 3:** Record the final apt list in the spec (replace the "finalized empirically" note). Commit.

---

### Task 2: Platform layer

**Files:** Create `src/platform.rs`; modify `src/main.rs` (mod + binding wrap), `src/settings.rs` (home dir), `src/theme.rs` (fonts), `src/install.rs` (cfg gates), `src/workspace.rs` (SHORTCUTS glyph rendering)

**Interfaces — Produces:**
`platform::keybinding(&str) -> String`; `platform::body_font() -> &'static str`; `platform::mono_font() -> &'static str`; `platform::home_dir() -> PathBuf`; `platform::MACOS: bool`; `platform::shortcut_glyphs(mac: &str) -> String` (for the ⌘/ dialog).

- [ ] **Step 1: Failing tests** (in `src/platform.rs`):

```rust
#[test]
fn keybindings_translate_off_macos() {
    // The translation function itself is platform-independent so it
    // can be tested everywhere; `keybinding()` applies it only off mac.
    assert_eq!(translate("cmd-shift-f"), "ctrl-shift-f");
    assert_eq!(translate("ctrl-cmd-f"), "ctrl-alt-f");
    assert_eq!(translate("cmd-="), "ctrl-=");
    assert_eq!(translate("ctrl-tab"), "ctrl-tab");     // untouched
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
    // pure helper over explicit inputs; the public fn reads env
    assert_eq!(pick_home(Some("/h".into()), Some("C:\\u".into())), PathBuf::from("/h"));
    assert_eq!(pick_home(None, Some("C:\\u".into())), PathBuf::from("C:\\u"));
    assert_eq!(pick_home(None, None), PathBuf::from("."));
}
```

- [ ] **Step 2:** RED → implement:

```rust
pub const MACOS: bool = cfg!(target_os = "macos");

fn translate(binding: &str) -> String {
    binding.replace("ctrl-cmd-", "ctrl-alt-").replace("cmd-", "ctrl-")
}
pub fn keybinding(mac_binding: &str) -> String {
    if MACOS { mac_binding.to_string() } else { translate(mac_binding) }
}

fn translate_glyphs(mac: &str) -> String {
    mac.replace("⌃ ⌘", "Ctrl Alt").replace("⌘", "Ctrl")
        .replace("⇧", "Shift").replace("⌥", "Alt").replace("⌃", "Ctrl")
}
pub fn shortcut_glyphs(mac: &str) -> String {
    if MACOS { mac.to_string() } else { translate_glyphs(mac) }
}

fn pick_home(home: Option<String>, userprofile: Option<String>) -> PathBuf {
    home.or(userprofile).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}
pub fn home_dir() -> PathBuf {
    pick_home(std::env::var("HOME").ok(), std::env::var("USERPROFILE").ok())
}

pub fn body_font() -> &'static str {
    if cfg!(target_os = "macos") { ".SystemUIFont" }
    else if cfg!(target_os = "windows") { "Segoe UI" } else { "DejaVu Sans" }
}
pub fn mono_font() -> &'static str {
    if cfg!(target_os = "macos") { "Menlo" }
    else if cfg!(target_os = "windows") { "Consolas" } else { "DejaVu Sans Mono" }
}
```

- [ ] **Step 3: Wire** — `settings::config_dir` uses `platform::home_dir()`; `Theme::light/dark` use `platform::{body_font, mono_font}` (`.into()`); main.rs: `let kb = |k: &str, a, ctx| KeyBinding::new(&platform::keybinding(k), a, ctx)`? KeyBinding::new takes `&str` — build the Vec with a small local macro or map (mechanical; keep the list literal, wrap each string). SHORTCUTS dialog rendering wraps key labels with `platform::shortcut_glyphs`. install.rs: `#[cfg(target_os = "macos")]` on the real bodies + non-mac stubs (`needs_install(_) -> false`, `bundle_path(_) -> None`, `move_to_applications(_) -> Err("unsupported on this platform".into())`); its path tests get `#[cfg(target_os = "macos")]`.

- [ ] **Step 4:** Suite green locally, commit `feat: platform layer (keys, fonts, home, install gates)`, push, all three CI jobs green.

---

### Task 3: Window chrome per platform

**Files:** Modify `src/main.rs` (window options), `src/workspace.rs` (titlebar/window controls/sidebar strip)

**Interfaces:**
- Consumes: `platform::MACOS`, gpui `window.window_decorations()` (exact getter name verified against vendored source at implementation; `Decorations::{Server, Client{..}}`), `minimize_window/zoom_window/remove_window`, `WindowControlArea::{Min, Max, Close}`.
- Produces: `fn render_window_controls(&self, cx) -> Option<AnyElement>` on Workspace.

- [ ] **Step 1: Window options** — in main.rs: `appears_transparent: true` stays for all (custom titlebar everywhere we can); add `window_decorations: Some(gpui::WindowDecorations::Client)` on Linux only (cfg). `traffic_light_position` mac-only meaningful, harmless elsewhere.

- [ ] **Step 2: Controls widget** — in workspace.rs:

```rust
/// Min/Max/Close buttons for platforms without native overlay
/// controls (Windows always; Linux when CSD granted).
fn render_window_controls(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
    if platform::MACOS { return None; }
    if matches!(window.window_decorations(), gpui::Decorations::Server) { return None; }
    let t = theme(cx);
    let btn = |id: &'static str, glyph: &'static str, area: gpui::WindowControlArea| {
        div().id(id).w(px(44.)).h_full().flex().items_center().justify_center()
            .window_control_area(area)
            .text_size(px(13.)).text_color(t.fg_muted)
            .hover(|s| s.bg(t.hover_bg))
            .child(glyph)
    };
    Some(
        div().h_full().flex_none().flex().flex_row()
            .child(btn("win-min", "–", gpui::WindowControlArea::Min))
            .child(btn("win-max", "□", gpui::WindowControlArea::Max))
            .child(btn("win-close", "✕", gpui::WindowControlArea::Close)
                   /* hover red: override hover bg with diff_deleted_bg */)
            .into_any_element(),
    )
}
```

If gpui's `window_control_area` does not itself trigger the action on
click on a given backend, add explicit `on_mouse_down` fallbacks
calling `window.minimize_window()` / `window.zoom_window()` /
`window.remove_window()` (check the vendored windows/linux backends:
`window_control_area` hit-tests for the OS caption handling on
Windows; Linux CSD needs the explicit calls).

- [ ] **Step 3: Layout branches** — `render_titlebar`: append `render_window_controls` after the update pill; the `!show_sidebar` traffic-light inset becomes `platform::MACOS &&` guarded. Sidebar top drag strip: on non-mac keep the strip (drag area is still wanted) but without the traffic-light height assumption — keep h34 drag strip everywhere (harmless, consistent). Empty-state strip same.

- [ ] **Step 4:** Local suite + compile; push; CI green on all three (this validates the non-mac cfg branches compile — behavior verified on hardware later).  Commit `feat: window chrome for windows and linux CSD`.

---

### Task 4: ☰ app menu (non-mac)

**Files:** Modify `src/workspace.rs`, `src/main.rs` (register `ToggleAppMenu` binding `ctrl-shift-m`? No — menu opens by click only; action optional. Keep click-only.)

**Interfaces:**
- Produces: `Workspace.app_menu_open: bool`; ☰ button at the left of the tab bar (non-mac only); popover overlay listing actions.

- [ ] **Step 1:** State + button: `app_menu_open: bool` (init false). In `render_titlebar`, before the tabs and only when `!platform::MACOS`: a `☰` button (same styling as window controls, 40px) toggling `app_menu_open`.

- [ ] **Step 2:** Popover: rendered in the root overlay section (pattern of theme picker): anchored top-left (absolute, top 40, left 8, w 260), panel bg, border, shadow, column of rows `(label, shortcut, action)`:

```
New File            Ctrl N
Open…               Ctrl O
── Open Recent ──
<startup_recents names, up to 5>
──
Search in Workspace Ctrl Shift F
Show Changes        Ctrl Shift D
Toggle Preview      Ctrl E
Theme…              Ctrl T
Shortcuts           Ctrl /
```

Each row `on_click`: set `app_menu_open = false` then
`window.dispatch_action(NewFile.boxed_clone(), cx)`-style dispatch
(or call the handler directly like existing code paths — use direct
handler calls: `self.new_file(&NewFile, window, cx)` etc., matching
how sidebar rows call `open_path`). Shortcut labels via
`platform::shortcut_glyphs`. Outside-click overlay dismisses (occlude
+ on_mouse_down like finder overlay).

- [ ] **Step 3:** Compile locally (dead code on mac — gate the render with `!platform::MACOS` but keep code compiled on all targets to avoid rot; the button just never renders on mac). Suite green, push, CI green. Commit `feat: app menu popover for linux/windows`.

---

### Task 5: Linux packaging

**Files:** Create `scripts/make_icon_png.py`, `assets/linux/supermd.desktop`, `scripts/bundle_linux.sh`; modify `Cargo.toml` (`[package.metadata.deb]`)

- [ ] **Step 1: PNG icons** — `make_icon_png.py`: import the SDF renderer from make_icon.py (refactor make_icon.py's `pixel`/`write_png` into importable functions guarded by `if __name__ == "__main__"`), write `assets/linux/supermd-128.png` and `supermd-512.png`. Run it; commit the PNGs.

- [ ] **Step 2: .desktop**:

```ini
[Desktop Entry]
Type=Application
Name=SuperMD
Comment=Markdown that gets out of the way
Exec=supermd %F
Icon=supermd
Terminal=false
Categories=Utility;TextEditor;Development;
MimeType=text/markdown;text/plain;
StartupWMClass=supermd
```

- [ ] **Step 3: bundle_linux.sh** — release build; stage `supermd`, `supermd.desktop`, icons, and an `install.sh` (copies binary to `~/.local/bin`, desktop file to `~/.local/share/applications`, icons to `~/.local/share/icons/hicolor/{128x128,512x512}/apps/supermd.png`, runs `update-desktop-database` if present); `tar czf supermd-linux-$(uname -m).tar.gz`.

- [ ] **Step 4: cargo-deb metadata** in Cargo.toml:

```toml
[package.metadata.deb]
name = "supermd"
section = "editors"
assets = [
  ["target/release/supermd", "usr/bin/", "755"],
  ["assets/linux/supermd.desktop", "usr/share/applications/", "644"],
  ["assets/linux/supermd-128.png", "usr/share/icons/hicolor/128x128/apps/supermd.png", "644"],
  ["assets/linux/supermd-512.png", "usr/share/icons/hicolor/512x512/apps/supermd.png", "644"],
]
```

- [ ] **Step 5:** Local suite green (scripts don't run on mac; shellcheck-read them). Commit `feat: linux packaging (tar.gz, deb metadata, desktop entry)`; the artifacts themselves are exercised by Task 7's release workflow.

---

### Task 6: Windows packaging

**Files:** Modify `src/main.rs` (subsystem attr); create `scripts/make_icon_ico.py`, `scripts/windows/supermd.iss`, `scripts/bundle_windows.ps1`

- [ ] **Step 1:** Top of main.rs:

```rust
// No console window on Windows release builds.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
```

- [ ] **Step 2: ICO** — `make_icon_ico.py`: reuse the shared renderer; ICO container holding 16/32/48/256 PNG entries (ICO-with-PNG format: 6-byte header + directory entries + PNG blobs — pure struct packing). Write `assets/windows/supermd.ico`, commit. Embed in the exe via a `build.rs` gated to windows using the `winresource` crate (`cargo add --build winresource`), setting the icon and product name.

- [ ] **Step 3: supermd.iss**:

```ini
[Setup]
AppName=SuperMD
AppVersion={#AppVersion}
AppPublisher=SuperJackfruitLabs
DefaultDirName={autopf}\SuperMD
DefaultGroupName=SuperMD
OutputBaseFilename=SuperMD-Setup-{#AppVersion}
UninstallDisplayIcon={app}\supermd.exe
ChangesAssociations=yes
[Files]
Source: "..\..\target\release\supermd.exe"; DestDir: "{app}"
[Icons]
Name: "{group}\SuperMD"; Filename: "{app}\supermd.exe"
[Tasks]
Name: "mdassoc"; Description: "Associate .md and .markdown files with SuperMD"
[Registry]
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "SuperMD.md"; ValueData: ""; Flags: uninsdeletevalue; Tasks: mdassoc
Root: HKA; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueType: string; ValueName: "SuperMD.md"; ValueData: ""; Flags: uninsdeletevalue; Tasks: mdassoc
Root: HKA; Subkey: "Software\Classes\SuperMD.md"; ValueType: string; ValueName: ""; ValueData: "Markdown Document"; Flags: uninsdeletekey; Tasks: mdassoc
Root: HKA; Subkey: "Software\Classes\SuperMD.md\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\supermd.exe,0"; Tasks: mdassoc
Root: HKA; Subkey: "Software\Classes\SuperMD.md\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\supermd.exe"" ""%1"""; Tasks: mdassoc
```

- [ ] **Step 4: bundle_windows.ps1** — `cargo build --release`; `Compress-Archive target/release/supermd.exe → supermd-windows-x64.zip`; `iscc /DAppVersion=$env:VERSION scripts/windows/supermd.iss`.

- [ ] **Step 5:** Local suite green (build.rs no-ops off windows). Commit `feat: windows packaging (installer, zip, embedded icon)`; push; CI green (windows job now also exercises build.rs + subsystem attr).

---

### Task 7: Release workflow restructure

**Files:** Modify `.github/workflows/release.yml`

- [ ] **Step 1:** Restructure to four jobs:
  - `macos`: today's steps minus `gh release create`; ends with `actions/upload-artifact` of `dist/supermd-*.dmg`.
  - `linux` (ubuntu-latest): apt deps (Task 1 list) + `cargo install cargo-deb` + `bash scripts/bundle_linux.sh "$VERSION"` + `cargo deb` → upload tar.gz + deb.
  - `windows` (windows-latest): `pwsh scripts/bundle_windows.ps1` → upload zip + Setup exe.
  - `publish` (needs all three): `actions/download-artifact`, then `gh release create "$GITHUB_REF_NAME" <all files> --title "SuperMD $GITHUB_REF_NAME" --generate-notes`.

- [ ] **Step 2:** Commit `ci: three-platform release pipeline`. (Exercised for real at the next tag; a dry-run via `workflow_dispatch` trigger is added so the pipeline can be tested without tagging.)

---

### Task 8: Site + docs

**Files:** Modify `site/index.html`, `README.md`, `docs/HISTORY.md`

- [ ] **Step 1:** Site CTA: primary button stays "Download for macOS"; beneath it a small line: "Also new: <a>Linux (.deb / tar)</a> · <a>Windows installer</a> — first releases, feedback welcome." All three link to the latest-release page. Footer note gains "macOS · Linux · Windows".
- [ ] **Step 2:** README: platforms section (macOS signed/notarized; Linux deb/tar; Windows installer unsigned for now with the SmartScreen sentence); building-on-linux apt list from Task 1. HISTORY row.
- [ ] **Step 3:** Suite, commit `docs: linux and windows availability`, push, CI green. Offer the user a v0.0.6 tag to exercise the full pipeline.
