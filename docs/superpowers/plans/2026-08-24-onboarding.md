# Onboarding Experience Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Frictionless download-to-first-folder: dressed DMG, move-to-Applications offer, welcome tour document, file associations + open events, recents/reopen-last, drag-and-drop open.

**Architecture:** Pure logic lands in two small modules (`src/install.rs` path predicates, `settings.rs` recents) with tests; macOS integration flows through existing seams (`bundle_macos.sh` plist/staging, `Application::on_open_urls` feeding the same launch-target slot as the CLI arg, workspace `on_drop::<ExternalPaths>`); the move-to-Applications offer is a workspace banner, not a modal.

**Tech Stack:** GPUI 0.2.2 (`on_open_urls`, `ExternalPaths`, `.drag_over`), `ditto`/`open` CLI for the self-install, pure-Python DMG background generator.

**Spec:** `docs/superpowers/specs/2026-08-24-onboarding-design.md`

## Global Constraints

- No modal dialogs; the install offer is a dismissible banner.
- Settings stay backward-compatible: new keys all `#[serde(default)]`.
- Recents: max 8 stored, 5 shown; most recent first; dedupe by path.
- All failure paths silent or in-banner — onboarding code never crashes the app.
- TDD for pure logic; GPUI shell verified by compile + manual smoke (repo convention).
- Commit per task; full suite green before each commit.

---

### Task 1: Settings — recents + reopen_last

**Files:** Modify `src/settings.rs`

**Interfaces — Produces:**
`Settings { reopen_last: bool /* default true */, recent_workspaces: Vec<String>, .. }`;
`Settings::note_workspace(&mut self, path: &Path)` — dedupe, push-front, truncate 8.
Tasks 4 and 5 consume both.

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn note_workspace_dedupes_and_caps() {
    let mut s = Settings::default();
    assert!(s.reopen_last);
    for i in 0..10 {
        s.note_workspace(Path::new(&format!("/w/{i}")));
    }
    assert_eq!(s.recent_workspaces.len(), 8);
    assert_eq!(s.recent_workspaces[0], "/w/9");
    s.note_workspace(Path::new("/w/5"));
    assert_eq!(s.recent_workspaces[0], "/w/5");
    assert_eq!(s.recent_workspaces.iter().filter(|p| *p == "/w/5").count(), 1);
}

#[test]
fn old_settings_files_still_parse() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.toml"), "light_theme = \"Paper\"\n").unwrap();
    let s = load(dir.path());
    assert_eq!(s.light_theme, "Paper");
    assert!(s.reopen_last);
    assert!(s.recent_workspaces.is_empty());
}
```

- [ ] **Step 2:** RED → implement:

```rust
pub reopen_last: bool,               // Default impl: true
pub recent_workspaces: Vec<String>,  // Default impl: vec![]

pub fn note_workspace(&mut self, path: &Path) {
    let p = path.to_string_lossy().into_owned();
    self.recent_workspaces.retain(|x| *x != p);
    self.recent_workspaces.insert(0, p);
    self.recent_workspaces.truncate(8);
}
```

(`Default` is hand-written today — extend it; serde `#[serde(default)]` on the struct already covers the new fields.)

- [ ] **Step 3:** Green, full suite, commit `feat: recents and reopen-last in settings`.

---

### Task 2: Install detection — `src/install.rs`

**Files:** Create `src/install.rs`; modify `src/main.rs` (`mod install;`)

**Interfaces — Produces:**
`install::needs_install(exe: &Path) -> bool`;
`install::bundle_path(exe: &Path) -> Option<PathBuf>` (nearest `.app` ancestor);
`install::move_to_applications(bundle: &Path) -> Result<(), String>` (ditto + open + intended for caller to quit). Task 6 consumes all three.

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn detects_dmg_and_translocated_paths() {
    assert!(needs_install(Path::new("/Volumes/SuperMD/SuperMD.app/Contents/MacOS/supermd")));
    assert!(needs_install(Path::new(
        "/private/var/folders/x/AppTranslocation/9F41/d/SuperMD.app/Contents/MacOS/supermd"
    )));
    assert!(!needs_install(Path::new("/Applications/SuperMD.app/Contents/MacOS/supermd")));
    assert!(!needs_install(Path::new("/Users/u/Projects/supermd/target/release/supermd")));
}

#[test]
fn bundle_path_finds_app_ancestor() {
    assert_eq!(
        bundle_path(Path::new("/Volumes/S/SuperMD.app/Contents/MacOS/supermd")),
        Some(PathBuf::from("/Volumes/S/SuperMD.app"))
    );
    assert_eq!(bundle_path(Path::new("/usr/bin/thing")), None);
}
```

- [ ] **Step 2:** RED → implement (predicates: any component `== "AppTranslocation"` or path starts with `/Volumes/`; `bundle_path`: ancestors().find(extension == "app")). `move_to_applications`:

```rust
pub fn move_to_applications(bundle: &Path) -> Result<(), String> {
    let dest = Path::new("/Applications/SuperMD.app");
    let run = |cmd: &str, args: &[&str]| -> Result<(), String> {
        let out = std::process::Command::new(cmd).args(args).output()
            .map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    };
    run("ditto", &[&bundle.to_string_lossy(), &dest.to_string_lossy()])?;
    run("open", &[&dest.to_string_lossy()])
}
```

- [ ] **Step 3:** Green, full suite, commit `feat: install-location detection and self-move helper`.

---

### Task 3: Welcome tour + HISTORY

**Files:** Rewrite `WELCOME.md`; create `docs/HISTORY.md` (receives the phase table verbatim).

- [ ] **Step 1:** Move the phase/roadmap table and dev-status content from `WELCOME.md` to `docs/HISTORY.md`.

- [ ] **Step 2:** Write the tour (final copy, editable-first, in this order): title "Welcome to SuperMD"; "This page is a real document — edit anything."; a task list with two toggleable checkboxes; a "click into this **bold** text" reveal demo line; a 2×3 live table; a small rust fence; a shortcut crib (⌘P, ⌘⇧F, ⌘⇧D, ⌘E, ⌘T, ⌘/); closing "Open a folder (⌘O) — or just drop one on this window."

- [ ] **Step 3:** `cargo run`, eyeball the welcome tab. Full suite, commit `docs: welcome tour document; move phase history`.

---

### Task 4: Open events + file associations

**Files:** Modify `src/main.rs`, `scripts/bundle_macos.sh`, `src/workspace.rs`

**Interfaces:**
- Produces: `Workspace::open_external_paths(&mut self, paths: Vec<PathBuf>, window, cx)` — dirs → `open_path` (workspace root, recents noted); files → permanent tabs. Task 5 reuses recents notes; Task 6's drop handler reuses this method.

- [ ] **Step 1: plist** — in `bundle_macos.sh` heredoc add:

```xml
<key>CFBundleDocumentTypes</key>
<array>
  <dict>
    <key>CFBundleTypeName</key><string>Markdown Document</string>
    <key>CFBundleTypeRole</key><string>Editor</string>
    <key>LSHandlerRank</key><string>Owner</string>
    <key>LSItemContentTypes</key>
    <array><string>net.daringfireball.markdown</string></array>
    <key>CFBundleTypeExtensions</key>
    <array><string>md</string><string>markdown</string><string>mdown</string><string>mdx</string></array>
  </dict>
  <dict>
    <key>CFBundleTypeName</key><string>Text Document</string>
    <key>CFBundleTypeRole</key><string>Editor</string>
    <key>LSHandlerRank</key><string>Alternate</string>
    <key>LSItemContentTypes</key>
    <array><string>public.plain-text</string><string>public.source-code</string></array>
  </dict>
  <dict>
    <key>CFBundleTypeName</key><string>Folder</string>
    <key>CFBundleTypeRole</key><string>Viewer</string>
    <key>LSHandlerRank</key><string>Alternate</string>
    <key>LSItemContentTypes</key>
    <array><string>public.folder</string></array>
  </dict>
</array>
```

- [ ] **Step 2: on_open_urls** — in `main()` before `run`: parse `file://` URLs (percent-decode via simple `%XX` loop or the `url`-free approach: `gpui` hands strings; decode with a small helper), stash into a `Mutex<Vec<PathBuf>>` shared with the app closure; cold start: first dir (or file) becomes the launch target exactly like the CLI arg; warm (window exists): forward through a global holding the workspace entity handle → `open_external_paths`.

- [ ] **Step 3:** Implement `open_external_paths` (dirs first, then files; nonexistent paths skipped).

- [ ] **Step 4:** Build a bundle (`bash scripts/bundle_macos.sh 0.0.0-dev`), `open dist/supermd.app`, then `open -a <bundle> README.md` and a folder; verify both arrive. Full suite, commit `feat: file associations and macOS open events`.

---

### Task 5: Recents wiring + reopen-last + empty-state list + menu

**Files:** Modify `src/workspace.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `Settings::note_workspace`, `settings::{load, save, config_dir}`.
- Produces: workspace unit actions `OpenRecent0..OpenRecent7` (indices into the launch-time snapshot `Workspace.startup_recents: Vec<PathBuf>`); recents recorded on every successful dir open.

- [ ] **Step 1: Record** — in `open_path`'s dir branch and `Workspace::new`'s dir arm: load settings, `note_workspace(root)`, save. (Load-modify-save keeps theme choices intact.)

- [ ] **Step 2: Reopen-last** — `main()`: when `arg.is_none()`, read settings; if `reopen_last`, first existing `recent_workspaces` entry becomes the launch arg (welcome tab still shown when there are no recents — first run unchanged).

- [ ] **Step 3: Empty-state recents** — under the "Open Folder…" button: up to 5 existing recent paths as rows (folder file_name in `fg`, parent dir in `fg_muted`, click → `open_path`); hint line "…or drop a folder here".

- [ ] **Step 4: Menu** — `impl_actions!`-style action with payload is heavier than needed: instead register 8 unit actions `OpenRecent0..OpenRecent7` (macro-free, simple), each opening `self.startup_recents[i]` — a `Vec<PathBuf>` snapshot stored on the workspace at construction. File menu gains "Open Recent" submenu listing the snapshot's folder names.

- [ ] **Step 5:** Full suite, manual smoke (quit/relaunch → last workspace reopens; empty state shows recents when launched with `SUPERMD_NO_REOPEN=1`? — no: test first-run by temporarily moving `~/.supermd/settings.toml` aside). Commit `feat: recent workspaces, reopen last, empty-state recents`.

---

### Task 6: Drag-drop + move-to-Applications banner

**Files:** Modify `src/workspace.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `install::{needs_install, bundle_path, move_to_applications}`, `open_external_paths` (Task 4).
- Produces: `Workspace.install_banner: Option<SharedString>` (message; None = hidden).

- [ ] **Step 1: Drop target** — on the workspace root div:

```rust
.on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
    this.open_external_paths(paths.paths().to_vec(), window, cx);
}))
.drag_over::<gpui::ExternalPaths>(|style, _, _, _| style.border_2().border_color(/* accent */))
```

(Adjust to the exact `drag_over` closure signature; an inset accent border on the root while hovering.)

- [ ] **Step 2: Banner state** — `Workspace::new`: if `install::needs_install(&std::env::current_exe().unwrap_or_default())`, set `install_banner: Some("SuperMD is running from the disk image.".into())`. Render (when Some) as a slim strip between titlebar and content in the right column: message left; [Move to Applications] accent button; [Not now] muted button (sets None).

- [ ] **Step 3: Move handler** — on click: `bundle_path(current_exe)` → `move_to_applications` → `cx.quit()` on Ok; on Err(e) replace banner text with `format!("Couldn't move: {e}")`.

- [ ] **Step 4:** Full suite; manual smoke of drop (drag a folder from Finder onto the window). Banner smoke: temporarily hardcode `needs_install → true`, eyeball, revert. Commit `feat: drag-drop open and move-to-Applications banner`.

---

### Task 7: DMG dressing

**Files:** Create `scripts/make_dmg_bg.py`, `assets/dmg/DS_Store` (authored locally), modify `scripts/bundle_macos.sh`

- [ ] **Step 1: Background** — `make_dmg_bg.py` (pure Python, PNG writer lifted from `make_icon.py`): 660×420 (@2x 1320×840), Jackfruit paper `#fdfbf6`, "SuperMD" wordmark text is out of reach without fonts — instead draw the app-icon rounded-square + block-M motif (reuse `make_icon.py` SDF fns) at left position, a subtle arrow (triangle + rect SDF) center, pointing right; writes `assets/dmg/bg@2x.png`.

- [ ] **Step 2: Staging in bundle script** — build `dist/dmg-staging/`: `SuperMD.app` (renamed copy of the built app for display), `Applications` symlink (`ln -s /Applications`), `.background/bg@2x.png`, `.DS_Store` from `assets/dmg/DS_Store` when that file exists. `hdiutil create -volname SuperMD -srcfolder dist/dmg-staging`.

- [ ] **Step 3: Author DS_Store once (local machine)** — build a RW image from the staging dir (`hdiutil create -format UDRW`), mount, `osascript` Finder: icon view, 660×420 window, background picture `.background/bg@2x.png`, icon size 96, position app at {165,210} and Applications at {495,210}; close, detach, mount read-only, copy `.DS_Store` out to `assets/dmg/DS_Store`, commit it. (One-time; CI never runs Finder scripting.)

- [ ] **Step 4:** Local `bash scripts/bundle_macos.sh 0.0.0-dev`, mount `dist/supermd-0.0.0-dev.dmg`, eyeball layout. Note: the app inside the DMG is named `SuperMD.app` — verify `codesign`/notarize steps in `release.yml` still reference the right paths (they operate on the bundle before staging, unaffected; `spctl` targets the DMG).

- [ ] **Step 5:** Full suite, commit `feat: dressed DMG with Applications symlink and background`.

---

### Task 8: Docs + finish

- [ ] **Step 1:** README: installation section ("drag to Applications; SuperMD offers to move itself if you skip it"), Open Recent/reopen-last, drag-drop, double-click .md.
- [ ] **Step 2:** Full suite + release build + combined manual smoke.
- [ ] **Step 3:** Commit `docs: onboarding in README`, push. Offer the user a v0.0.5 tag (release is their call).
