# Mac App Store Distribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `mas` build target that produces a sandboxed, App Store–signed `.pkg` validated and uploaded to internal TestFlight, leaving the Developer ID / DMG build byte-for-byte unchanged in behaviour. Promotion to external TestFlight and App Review is a release decision outside this plan.

**Architecture:** One cargo feature, `mas`, selects three swaps: the plugin host compiles to Pulley bytecode instead of native code (no executable pages, so no JIT entitlements); the wasm grammar surface is compiled out (costing one plugin's GraphQL highlighting); and the plugin installer loses its in-app catalog browser but keeps installing via an open-panel import and a `supermd://` URL handoff. Alongside those, six sandbox papercuts are fixed — security-scoped bookmarks for recent workspaces, `NSFileManager` trashing, `NSWorkspace` reveal, and the removal of three process spawns. All new policy lands in pure, tested modules; the objc2 shim stays under 40 lines.

**Tech Stack:** Rust, GPUI 0.2.2, wasmtime 48 (`pulley`), objc2 0.6.4 / objc2-foundation 0.3.2 / objc2-app-kit 0.3.2, inline `#[cfg(test)]` tests, `cargo test`, `cargo llvm-cov`.

**Spec:** `docs/superpowers/specs/2026-08-31-mac-app-store-design.md`

## Global Constraints

- **Editing/policy logic is pure Rust under tests; the GPUI shell stays thin.** Every new decision in this plan goes in a pure function with inline tests; the shell only drives it.
- **CI enforces a 90% line-coverage floor** (`cargo llvm-cov`). Every task adds tests alongside its code.
- Tests live inline as `#[cfg(test)] mod tests` beside the code they cover.
- **Per-OS decisions live in `src/platform.rs`**, never as scattered `cfg!()`. The one exception this plan permits is `cfg!(feature = "mas")` inside the module that owns the affected policy, because it is a *distribution* decision, not an OS decision.
- **The Developer ID build must not change.** Every behavioural difference is gated on `feature = "mas"`, which is **off by default**. After each task, `cargo test` (no feature flags) must show **zero failures**, and the pass count must not decrease.
- **Baseline is 647 passing tests, 2 ignored** (the two `bench_backends` benchmarks). Each task adds tests, so the pass count grows — a *drop* is a regression. Do not let the ignored count grow without saying why.
- Cargo features are **additive** — you cannot subtract `tree-sitter/wasm` with a feature. The MAS build is therefore `--no-default-features --features mas`, and `grammars` must become a *default* feature (Task 8).
- Adding a `KeyBinding` means bumping the count in `main.rs`'s `every_keybinding_parses_and_binds`, and updating `SHORTCUTS` in `workspace.rs` plus `docs/site/shortcuts.md`.
- `src/seti.rs` is GENERATED — do not edit.
- `site/docs/` is GENERATED — edit `docs/site/*.md` and run `cargo run --example build_docs`.
- macOS deployment target stays 12.0. SuperMD never writes to the user's git repository.
- Release discipline unchanged: bump `version` in `Cargo.toml` **and** commit before tagging.

---

## Current State

Tasks 1's groundwork is **already in the working tree, uncommitted** (added while benchmarking):

- `Cargo.toml` — `"pulley"` added to the wasmtime features.
- `src/extensions.rs` — `ExtensionHost::load_with_target()`, with `load()` delegating `None`; plus `mod bench_backends` holding `pulley_vs_cranelift` and `hardened_runtime_probe`, both `#[ignore]`d.

Task 1 commits that and builds the feature gate on top.

---

## File Structure

| File | Responsibility |
| ---- | -------------- |
| `Cargo.toml` (modify) | `mas` / `grammars` features; macOS-only objc2 deps; `tree-sitter/wasm` made optional. |
| `src/extensions.rs` (modify) | `wasm_target()` picks the backend; `CALL_DEADLINE_TICKS` widens under `mas`. |
| `src/bookmarks.rs` (create) | **Pure**: hex codec, bookmark-entry policy, staleness/pruning, `Resolution`. No objc2. |
| `src/bookmarks_mac.rs` (create) | The objc2 shim only: create / resolve / start / stop. Under 40 lines, `#[cfg(all(target_os = "macos", feature = "mas"))]`. |
| `src/settings.rs` (modify) | `workspace_bookmarks` map; `note_workspace` grows a blob parameter. |
| `src/main.rs` (modify) | `resolve_startup_arg` becomes scope-aware; `supermd://` URLs join the open-event queue. |
| `src/fileops.rs` (modify) | `delete` uses `DeleteMethod::NsFileManager` on macOS. |
| `src/platform.rs` (modify) | `reveal_dir` uses `NSWorkspace` on macOS. |
| `src/highlight.rs` (modify) | Grammar registry behind `feature = "grammars"`; no-op stubs otherwise. |
| `src/catalog.rs` (modify) | `install_plugin_from_bytes` factored out; `entry_by_name` for the URL handoff. |
| `src/install_ui.rs` (modify) | Catalog browse gated off under `mas`; Import… item always present. |
| `src/workspace.rs` (modify) | Import Plugin… command; git-out-of-scope hint; Reveal Settings Folder. |
| `src/update.rs` (modify) | Whole check compiled out under `mas`. |
| `assets/mas.entitlements` (create) | Sandbox entitlements; no `cs.*` keys. |
| `scripts/bundle_mas.sh` (create) | Apple Distribution signing, embedded profile, `productbuild` `.pkg`. |
| `.github/workflows/release.yml` (modify) | A `mas` job beside `macos`. |
| `docs/site/plugins.md`, `docs/site/themes.md` (modify) | Container paths, the two MAS install routes. |

---

### Task 1: The `mas` feature and the Pulley backend

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/extensions.rs:566-580` (`load` / `load_with_target`), `src/extensions.rs:312` (`CALL_DEADLINE_TICKS`)

**Interfaces:**
- Consumes: nothing.
- Produces: cargo feature `mas`; `pub fn extensions::wasm_target() -> Option<&'static str>`; `ExtensionHost::load_with_target(&Path, Option<&str>) -> Self` (already present).

- [ ] **Step 1: Commit the benchmarking groundwork already in the tree**

```bash
git add Cargo.toml src/extensions.rs
git commit -m "test: add pulley/cranelift plugin-host benchmark and target seam"
```

- [ ] **Step 2: Write the failing test**

Add to `mod bench_backends` in `src/extensions.rs`:

```rust
#[test]
fn wasm_target_is_pulley_only_under_mas() {
    if cfg!(feature = "mas") {
        assert_eq!(wasm_target(), Some("pulley64"));
    } else {
        assert_eq!(wasm_target(), None);
    }
}

#[test]
fn mas_widens_the_call_deadline() {
    if cfg!(feature = "mas") {
        assert_eq!(CALL_DEADLINE_TICKS, 16);
    } else {
        assert_eq!(CALL_DEADLINE_TICKS, 4);
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test --release wasm_target_is_pulley -- --nocapture`
Expected: FAIL — `cannot find function 'wasm_target' in this scope`.

- [ ] **Step 4: Add the feature to `Cargo.toml`**

```toml
[features]
default = []
# Mac App Store build: interprets plugin bytecode instead of JIT-compiling
# it, so the bundle needs no hardened-runtime JIT entitlements.
mas = []
```

- [ ] **Step 5: Implement `wasm_target` and widen the deadline**

In `src/extensions.rs`, replace the `CALL_DEADLINE_TICKS` constant:

```rust
/// How long a single plugin call may run before the epoch deadline
/// interrupts it (epoch ticks every 500ms).
///
/// The MAS build interprets Pulley bytecode, measured at 19-29x slower
/// than native codegen, so the same wall clock buys ~24x less work. 16
/// ticks (8s) restores roughly 333ms of native-equivalent budget; the
/// heaviest first-party call is 190ms of Pulley time on a 1MB document.
const EPOCH_TICK_MS: u64 = 500;
const CALL_DEADLINE_TICKS: u64 = if cfg!(feature = "mas") { 16 } else { 4 };
```

And beside `load`:

```rust
/// Compilation target for the plugin host. The App Store build compiles
/// to Pulley bytecode and interprets it — no executable pages, so no
/// `com.apple.security.cs.*` entitlements. Every other build uses
/// native codegen.
pub fn wasm_target() -> Option<&'static str> {
    cfg!(feature = "mas").then_some("pulley64")
}
```

Then make `load` use it:

```rust
    pub fn load(plugins_dir: &Path) -> Self {
        Self::load_with_target(plugins_dir, wasm_target())
    }
```

- [ ] **Step 6: Run the tests both ways**

```bash
cargo test --release wasm_target_is_pulley mas_widens
cargo test --release --features mas wasm_target_is_pulley mas_widens
```
Expected: PASS in both configurations.

- [ ] **Step 7: Prove the entitlements are unnecessary**

```bash
cargo test --release --features mas --no-run
BIN=$(ls -t target/release/deps/supermd-* | grep -v '\.d$' | head -1)
codesign --force --options runtime --sign - "$BIN"
"$BIN" hardened_runtime_probe --ignored --nocapture
```
Expected: PASS — `status_text -> Ok("5 words · 1 min read")` under a
hardened-runtime signature carrying no entitlements. (The same probe
without `--features mas` exits 137, SIGKILL. That contrast is the point.)

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml src/extensions.rs
git commit -m "feat: mas feature selects the pulley interpreter backend"
```

---

### Task 2: Trash without Apple events

**Files:**
- Modify: `src/fileops.rs:78-80`

**Interfaces:**
- Consumes: nothing.
- Produces: `fileops::delete(&Path) -> Result<(), String>` — signature unchanged, macOS backend swapped.

`trash` 5.2.6 defaults to `DeleteMethod::Finder`, which shells out to
`osascript` and sends Apple events to Finder. The sandbox blocks both. The
fix is worth taking on the Developer ID build too: it is faster and skips
the automation consent prompt.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/fileops.rs`:

```rust
#[test]
fn delete_uses_the_file_manager_on_macos() {
    // The Finder backend spawns osascript, which the App Sandbox blocks.
    // This asserts the policy, not the OS call: `delete_method_name`
    // is the seam the shell reads.
    #[cfg(target_os = "macos")]
    assert_eq!(delete_method_name(), "NsFileManager");
    #[cfg(not(target_os = "macos"))]
    assert_eq!(delete_method_name(), "platform-default");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test delete_uses_the_file_manager`
Expected: FAIL — `cannot find function 'delete_method_name'`.

- [ ] **Step 3: Implement**

Replace `fileops::delete`:

```rust
/// Which trash backend this build uses. macOS defaults to Finder
/// (osascript + Apple events), which the App Sandbox blocks, so we pin
/// NSFileManager. Cost: no Finder sound and no "Put Back" entry.
pub fn delete_method_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "NsFileManager"
    } else {
        "platform-default"
    }
}

/// Send a file or folder to the OS trash.
#[cfg(target_os = "macos")]
pub fn delete(path: &Path) -> Result<(), String> {
    use trash::macos::{DeleteMethod, TrashContextExtMacos as _};
    let mut ctx = trash::TrashContext::default();
    ctx.set_delete_method(DeleteMethod::NsFileManager);
    ctx.delete(path).map_err(|e| format!("cannot delete {}: {e}", path.display()))
}

#[cfg(not(target_os = "macos"))]
pub fn delete(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| format!("cannot delete {}: {e}", path.display()))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test fileops`
Expected: PASS, including the existing `TrashFn`-seam tests.

- [ ] **Step 5: Verify by hand**

Run the app, delete a file from the sidebar, confirm it lands in
`~/.Trash` and that no Automation consent prompt appears.

- [ ] **Step 6: Commit**

```bash
git add src/fileops.rs
git commit -m "fix: trash via NSFileManager instead of Finder Apple events"
```

---

### Task 3: `bookmarks.rs` — the pure policy layer

**Files:**
- Create: `src/bookmarks.rs`
- Modify: `src/main.rs` (add `mod bookmarks;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn hex_encode(&[u8]) -> String`; `pub fn hex_decode(&str) -> Option<Vec<u8>>`; `pub enum Resolution { Fresh(PathBuf), Stale(PathBuf), Missing }`; `pub fn prune(&mut BTreeMap<String, String>, &[String])`; `pub fn needs_scope() -> bool`.

Hex rather than base64 keeps the codec a dependency-free pure function that
is trivially covered.

- [ ] **Step 1: Write the failing tests**

Create `src/bookmarks.rs` containing only the doc comment and tests:

```rust
//! Security-scoped bookmark policy. The App Sandbox grants access to
//! what the user picks in an open panel; that grant dies with the
//! process unless it is captured as a bookmark. This module owns the
//! pure half — encoding, pruning, and what a resolution means. The
//! objc2 calls live in `bookmarks_mac.rs`.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn hex_roundtrips_arbitrary_bytes() {
        let bytes: Vec<u8> = (0u8..=255).collect();
        assert_eq!(hex_decode(&hex_encode(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn hex_encode_is_lowercase_and_double_width() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
    }

    #[test]
    fn hex_decode_rejects_odd_length_and_non_hex() {
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
        assert_eq!(hex_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn prune_drops_bookmarks_with_no_matching_recent() {
        let mut map = BTreeMap::from([
            ("/a".to_string(), "00".to_string()),
            ("/b".to_string(), "11".to_string()),
        ]);
        prune(&mut map, &["/a".to_string()]);
        assert_eq!(map.keys().collect::<Vec<_>>(), vec!["/a"]);
    }

    #[test]
    fn prune_keeps_everything_when_all_are_recent() {
        let mut map = BTreeMap::from([("/a".to_string(), "00".to_string())]);
        prune(&mut map, &["/a".to_string(), "/b".to_string()]);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn scope_is_only_needed_in_the_sandboxed_build() {
        assert_eq!(needs_scope(), cfg!(all(target_os = "macos", feature = "mas")));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test bookmarks`
Expected: FAIL — `cannot find function 'hex_encode'`.

- [ ] **Step 3: Implement**

Prepend to `src/bookmarks.rs`:

```rust
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What resolving a stored bookmark produced.
#[derive(Debug, PartialEq)]
pub enum Resolution {
    /// Usable; access has been started.
    Fresh(PathBuf),
    /// Resolved, but macOS flagged it stale — the folder moved. Usable
    /// once, and the caller should re-create the bookmark.
    Stale(PathBuf),
    /// Gone: deleted, on an unmounted volume, or never bookmarked.
    Missing,
}

/// True when this build must capture bookmarks to keep folder access
/// across launches. Only the sandboxed macOS build does.
pub fn needs_scope() -> bool {
    cfg!(all(target_os = "macos", feature = "mas"))
}

/// Lowercase hex. Bookmark blobs are ~1KB of opaque bytes and
/// settings.toml is text, so they are stored hex-encoded.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Inverse of `hex_encode`. None on odd length or a non-hex digit.
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// Drop bookmarks whose path is no longer in the recents list, so the
/// map cannot outgrow the eight entries it mirrors.
pub fn prune(map: &mut BTreeMap<String, String>, recents: &[String]) {
    map.retain(|path, _| recents.iter().any(|r| r == path));
}
```

Add `mod bookmarks;` to `src/main.rs` beside the other module declarations.

- [ ] **Step 4: Run the tests**

Run: `cargo test bookmarks`
Expected: PASS — 6 tests.

- [ ] **Step 5: Commit**

```bash
git add src/bookmarks.rs src/main.rs
git commit -m "feat: pure security-scoped bookmark policy"
```

---

### Task 4: The objc2 bookmark shim

**Files:**
- Create: `src/bookmarks_mac.rs`
- Modify: `Cargo.toml` (macOS target deps), `src/bookmarks.rs` (re-export)

**Interfaces:**
- Consumes: `bookmarks::{Resolution, hex_encode, hex_decode}`.
- Produces: `pub fn create(&Path) -> Option<String>`; `pub fn resolve(&str) -> Resolution`; `pub fn stop(&Path)`.

Keep this file tiny — it is the only unsafe code in the plan and the only
part the 90% coverage floor cannot reach. Everything decidable lives in
Task 3's pure module.

- [ ] **Step 1: Add the dependencies**

In `Cargo.toml`, at the versions already resolved in `Cargo.lock` (so this
adds no downloads):

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6.4"
objc2-foundation = { version = "0.3.2", features = ["NSURL", "NSArray", "NSError", "NSString", "NSData", "NSDictionary"] }
objc2-app-kit = { version = "0.3.2", features = ["NSWorkspace"] }
```

- [ ] **Step 2: Verify they resolve without new downloads**

Run: `cargo tree -p objc2-foundation --depth 0`
Expected: `objc2-foundation v0.3.2` — the version already in the lockfile,
with `Cargo.lock` otherwise unchanged (`git diff --stat Cargo.lock`).

- [ ] **Step 3: Write the shim**

Create `src/bookmarks_mac.rs`:

```rust
//! The objc2 half of security-scoped bookmarks. Deliberately thin: no
//! decisions live here, only the four Foundation calls. Policy is in
//! `bookmarks.rs`, which is pure and fully tested.
#![cfg(all(target_os = "macos", feature = "mas"))]

use crate::bookmarks::{hex_decode, hex_encode, Resolution};
use objc2_foundation::{
    NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions,
};
use std::path::Path;

fn url(path: &Path) -> objc2::rc::Retained<NSURL> {
    NSURL::fileURLWithPath(&objc2_foundation::NSString::from_str(&path.to_string_lossy()))
}

/// Capture the current sandbox grant for `path` as a hex blob.
pub fn create(path: &Path) -> Option<String> {
    let data = url(path)
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::WithSecurityScope,
            None,
            None,
        )
        .ok()?;
    Some(hex_encode(&data.to_vec()))
}

/// Resolve a stored blob and START accessing it. Callers must pair this
/// with `stop` when the workspace closes.
pub fn resolve(blob: &str) -> Resolution {
    let Some(bytes) = hex_decode(blob) else {
        return Resolution::Missing;
    };
    let data = objc2_foundation::NSData::with_bytes(&bytes);
    let mut stale = objc2::runtime::Bool::NO;
    // SAFETY: `stale` is a live local; the call writes at most one Bool.
    let resolved = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &data,
            NSURLBookmarkResolutionOptions::WithSecurityScope,
            None,
            &mut stale,
        )
    };
    let Ok(u) = resolved else {
        return Resolution::Missing;
    };
    if !unsafe { u.startAccessingSecurityScopedResource() } {
        return Resolution::Missing;
    }
    let Some(path) = u.path() else {
        return Resolution::Missing;
    };
    let path = std::path::PathBuf::from(path.to_string());
    if stale.as_bool() {
        Resolution::Stale(path)
    } else {
        Resolution::Fresh(path)
    }
}

/// Release the scoped grant. Unbalanced `resolve` calls leak kernel
/// scoped-resource slots, so every open workspace stops on close.
pub fn stop(path: &Path) {
    unsafe { url(path).stopAccessingSecurityScopedResource() };
}
```

- [ ] **Step 4: Add non-MAS stubs so callers need no `cfg`**

Append to `src/bookmarks.rs`:

```rust
/// Capture the sandbox grant for `path`, if this build needs one.
#[cfg(not(all(target_os = "macos", feature = "mas")))]
pub fn create(_path: &std::path::Path) -> Option<String> {
    None
}

/// Resolve a stored grant. Unsandboxed builds never stored one.
#[cfg(not(all(target_os = "macos", feature = "mas")))]
pub fn resolve(_blob: &str) -> Resolution {
    Resolution::Missing
}

/// Release a scoped grant. A no-op where there are no grants.
#[cfg(not(all(target_os = "macos", feature = "mas")))]
pub fn stop(_path: &std::path::Path) {}

#[cfg(all(target_os = "macos", feature = "mas"))]
pub use crate::bookmarks_mac::{create, resolve, stop};
```

Add `mod bookmarks_mac;` to `src/main.rs`.

- [ ] **Step 5: Write the stub test**

Add to `mod tests` in `src/bookmarks.rs`:

```rust
#[test]
fn unsandboxed_builds_capture_no_grants() {
    if !needs_scope() {
        assert_eq!(create(std::path::Path::new("/tmp")), None);
        assert_eq!(resolve("00"), Resolution::Missing);
        stop(std::path::Path::new("/tmp")); // must not panic
    }
}
```

- [ ] **Step 6: Build and test both configurations**

```bash
cargo test bookmarks
cargo build --features mas
```
Expected: tests PASS; the `mas` build compiles the shim without warnings.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/bookmarks.rs src/bookmarks_mac.rs src/main.rs
git commit -m "feat: security-scoped bookmark shim for the sandboxed build"
```

---

### Task 5: Persist and restore workspace grants

**Files:**
- Modify: `src/settings.rs:14` (struct), `:69-75` (`note_workspace`)
- Modify: `src/main.rs:136-148` (`resolve_startup_arg`), `:293`
- Modify: `src/workspace.rs:733` (workspace open)

**Interfaces:**
- Consumes: `bookmarks::{create, resolve, prune, needs_scope, Resolution}`.
- Produces: `Settings::workspace_bookmarks: BTreeMap<String, String>`; `Settings::note_workspace(&mut self, &Path, Option<String>)`; `fn resolve_startup_arg(Option<PathBuf>, &Settings) -> Option<PathBuf>` (signature unchanged, policy changed).

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/settings.rs`:

```rust
#[test]
fn note_workspace_stores_and_prunes_bookmarks() {
    let mut s = Settings::default();
    for i in 0..9 {
        s.note_workspace(Path::new(&format!("/w/{i}")), Some(format!("{i:02x}")));
    }
    // Eight recents cap, and the bookmark map never outgrows them.
    assert_eq!(s.recent_workspaces.len(), 8);
    assert_eq!(s.workspace_bookmarks.len(), 8);
    assert!(!s.workspace_bookmarks.contains_key("/w/0"));
    assert_eq!(s.workspace_bookmarks.get("/w/8"), Some(&"08".to_string()));
}

#[test]
fn note_workspace_without_a_bookmark_leaves_the_map_alone() {
    let mut s = Settings::default();
    s.note_workspace(Path::new("/w/a"), None);
    assert_eq!(s.recent_workspaces, vec!["/w/a".to_string()]);
    assert!(s.workspace_bookmarks.is_empty());
}

#[test]
fn settings_without_bookmarks_still_parse() {
    // Forward/backward compatibility: an old settings.toml has no
    // workspace_bookmarks key.
    let s: Settings = toml::from_str("reopen_last = true\n").unwrap();
    assert!(s.workspace_bookmarks.is_empty());
}
```

Add to `mod tests` in `src/main.rs`:

```rust
#[test]
fn sandboxed_builds_treat_a_cli_path_as_unscoped() {
    let settings = settings::Settings { reopen_last: false, ..Default::default() };
    let arg = Some(PathBuf::from("/some/dir"));
    let got = resolve_startup_arg(arg.clone(), &settings);
    if crate::bookmarks::needs_scope() {
        // No Powerbox grant comes with argv; the workspace prompts.
        assert_eq!(got, None);
    } else {
        assert_eq!(got, arg);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test note_workspace_stores sandboxed_builds_treat`
Expected: FAIL — `note_workspace` takes 1 argument; no field `workspace_bookmarks`.

- [ ] **Step 3: Extend `Settings`**

In `src/settings.rs`, add the field to the struct (after `recent_workspaces`):

```rust
    /// Security-scoped bookmark blobs for `recent_workspaces`, keyed by
    /// path. Only the sandboxed macOS build writes these; every other
    /// build leaves the map empty and reopens by path.
    pub workspace_bookmarks: std::collections::BTreeMap<String, String>,
```

Add `workspace_bookmarks: Default::default(),` to `Default::default()`, and
replace `note_workspace`:

```rust
    /// Record a just-opened workspace: dedupe, push front, cap at 8.
    /// `bookmark` is the hex-encoded scoped grant, when this build
    /// captures one.
    pub fn note_workspace(&mut self, path: &Path, bookmark: Option<String>) {
        let p = path.to_string_lossy().into_owned();
        self.recent_workspaces.retain(|x| *x != p);
        self.recent_workspaces.insert(0, p.clone());
        self.recent_workspaces.truncate(8);
        if let Some(blob) = bookmark {
            self.workspace_bookmarks.insert(p, blob);
        }
        crate::bookmarks::prune(&mut self.workspace_bookmarks, &self.recent_workspaces);
    }
```

- [ ] **Step 4: Make startup scope-aware**

In `src/main.rs`, replace `resolve_startup_arg`:

```rust
/// Launched bare (Dock/Finder) with reopen enabled: return to the most
/// recent workspace that still exists.
///
/// A sandboxed build gets no Powerbox grant with argv, so a CLI path is
/// unusable — return None and let the workspace open the panel instead
/// of failing to read a folder we appear to have been given.
fn resolve_startup_arg(arg: Option<PathBuf>, settings: &settings::Settings) -> Option<PathBuf> {
    if arg.is_some() {
        return if crate::bookmarks::needs_scope() { None } else { arg };
    }
    if !settings.reopen_last {
        return None;
    }
    settings
        .recent_workspaces
        .iter()
        .find(|p| {
            settings
                .workspace_bookmarks
                .get(*p)
                .map(|blob| {
                    !matches!(crate::bookmarks::resolve(blob), crate::bookmarks::Resolution::Missing)
                })
                .unwrap_or_else(|| !crate::bookmarks::needs_scope() && Path::new(p).is_dir())
        })
        .map(PathBuf::from)
}
```

- [ ] **Step 5: Capture the grant on open**

In `src/workspace.rs`, at the `note_workspace` call site, pass a freshly
created bookmark:

```rust
            let blob = crate::bookmarks::create(path);
            settings.note_workspace(path, blob);
```

Update every other `note_workspace` caller to pass `None`.

- [ ] **Step 6: Run the whole suite**

Run: `cargo test`
Expected: PASS, and the previously existing `note_workspace_dedupes_and_caps`
still passes after its call sites gain the second argument.

- [ ] **Step 7: Commit**

```bash
git add src/settings.rs src/main.rs src/workspace.rs
git commit -m "feat: persist workspace access as security-scoped bookmarks"
```

---

### Task 6: Sandbox-safe reveal, the settings folder, and the git hint

**Files:**
- Modify: `src/platform.rs:157-167` (`reveal_dir`)
- Modify: `src/workspace.rs` (Reveal Settings Folder command; git-out-of-scope hint)
- Modify: `src/commands.rs` (one table row)

**Interfaces:**
- Consumes: `objc2_app_kit::NSWorkspace`, `settings::config_dir()`.
- Produces: `platform::reveal_backend() -> &'static str`; `platform::reveal_dir(&Path)` (signature unchanged); `workspace::git_scope_hint(bool, bool) -> Option<&'static str>`.

Three affordances the sandbox forces, all shell-level and all reviewed together: `reveal_dir` must stop spawning `open`; `~/.supermd/` moves into the container and needs a way to reach it; and Show Changes goes quiet when the repo root sits outside the granted scope.

`reveal_dir` spawns `open`, which the sandbox blocks. `NSWorkspace` is the
sanctioned route and works in every macOS build, so this is not gated on
`mas`.

- [ ] **Step 1: Write the failing test**

Add to `src/platform.rs`'s test module:

```rust
#[test]
fn reveal_spawns_no_subprocess_on_macos() {
    // The sandbox forbids spawning /usr/bin/open; NSWorkspace is the
    // sanctioned route. This asserts the policy the shell reads.
    assert_eq!(reveal_backend(), if cfg!(target_os = "macos") { "NSWorkspace" } else { "spawn" });
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test reveal_spawns_no_subprocess`
Expected: FAIL — `cannot find function 'reveal_backend'`.

- [ ] **Step 3: Implement**

```rust
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
    let url = unsafe { NSURL::fileURLWithPath(&s) };
    let urls = NSArray::from_retained_slice(&[url]);
    unsafe { objc2_app_kit::NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&urls) };
}

#[cfg(not(target_os = "macos"))]
pub fn reveal_dir(path: &std::path::Path) {
    let tool = if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
    let _ = std::process::Command::new(tool).arg(path).spawn();
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test platform`
Expected: PASS.

- [ ] **Step 5: Write the failing test for the git-scope hint**

Add to `src/workspace.rs`'s test module:

```rust
#[test]
fn git_scope_hint_only_fires_when_sandboxed_and_baseline_is_missing() {
    // (has_baseline, in_repo_subdir) -> hint
    assert_eq!(git_scope_hint(true, true), None);
    assert_eq!(git_scope_hint(true, false), None);
    assert_eq!(git_scope_hint(false, false), None);
    assert_eq!(
        git_scope_hint(false, true),
        Some("the git repository is outside the opened folder"),
    );
}
```

- [ ] **Step 6: Run to verify it fails**

Run: `cargo test git_scope_hint_only_fires`
Expected: FAIL — `cannot find function 'git_scope_hint'`.

- [ ] **Step 7: Implement the hint and the Reveal Settings Folder command**

In `src/workspace.rs`:

```rust
    /// Why Show Changes found no baseline. Under the sandbox, `gix::discover`
    /// walks upward out of the granted scope and fails silently; say so
    /// rather than implying the folder has no history.
    fn git_scope_hint(has_baseline: bool, repo_root_above_workspace: bool) -> Option<&'static str> {
        (!has_baseline && repo_root_above_workspace)
            .then_some("the git repository is outside the opened folder")
    }

    fn reveal_settings_folder(&mut self, _: &RevealSettingsFolder, _: &mut Window, _: &mut Context<Self>) {
        let dir = crate::settings::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        crate::platform::reveal_dir(&dir);
    }
```

Add the `RevealSettingsFolder` action and one `commands.rs` row (menu:
Help, **no keystroke**, so `every_keybinding_parses_and_binds` is
unaffected). Show the hint wherever Show Changes reports "no baseline".

- [ ] **Step 8: Run the suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 9: Verify by hand**

Trigger reveal; Finder should open with the folder selected and no `open`
process should appear in Activity Monitor. Then run Reveal Settings Folder
and confirm it lands on `~/.supermd` (or the container path under `mas`).

- [ ] **Step 10: Commit**

```bash
git add src/platform.rs src/workspace.rs src/commands.rs
git commit -m "fix: NSWorkspace reveal, settings-folder command, git scope hint"
```

---

### Task 7: Remove the remaining process spawns under `mas`

**Files:**
- Modify: `src/update.rs:44-56`
- Modify: `src/install.rs:31-45`
- Modify: `src/workspace.rs:392`, `:2285`, `:2326` (update affordance)

**Interfaces:**
- Consumes: nothing.
- Produces: `update::checks_enabled() -> bool`; `install::needs_install(&Path) -> bool` returns false under `mas`.

The App Store owns updates, and a self-moving app violates 2.4.5(ii).
Both features compile out; the pure helpers (`is_newer`, `parse_tag`,
`update_status`) stay so their tests keep contributing coverage.

- [ ] **Step 1: Write the failing tests**

Add to `src/update.rs`'s test module:

```rust
#[test]
fn update_checks_are_off_in_the_app_store_build() {
    assert_eq!(checks_enabled(), !cfg!(feature = "mas"));
}

#[test]
fn fetch_returns_none_without_checks() {
    if !checks_enabled() {
        assert_eq!(fetch_latest_tag(), None);
    }
}
```

Add to `src/install.rs`'s test module:

```rust
#[test]
fn app_store_builds_never_offer_to_move_themselves() {
    if cfg!(feature = "mas") {
        assert!(!needs_install(std::path::Path::new("/Users/x/Downloads/SuperMD.app")));
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --features mas update_checks_are_off app_store_builds_never`
Expected: FAIL — `cannot find function 'checks_enabled'`.

- [ ] **Step 3: Implement**

In `src/update.rs`:

```rust
/// The App Store distributes updates itself, and the sandbox blocks the
/// curl subprocess this check uses. Off in the MAS build.
pub fn checks_enabled() -> bool {
    !cfg!(feature = "mas")
}

/// Blocking: fetch the latest release tag (e.g. "v0.0.5"). Runs on the
/// background executor; any failure is a silent None.
pub fn fetch_latest_tag() -> Option<String> {
    if !checks_enabled() {
        return None;
    }
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "-m", "10", "-H", "User-Agent: supermd", LATEST_API])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_tag(&String::from_utf8_lossy(&out.stdout))
}
```

In `src/install.rs`, guard `needs_install`:

```rust
pub fn needs_install(exe: &Path) -> bool {
    if cfg!(feature = "mas") {
        return false; // the App Store installs into /Applications itself
    }
    // ... existing body unchanged
}
```

In `src/workspace.rs`, wrap the update-pill spawn sites so no timer runs:

```rust
            if crate::update::checks_enabled() {
                // ... existing spawn
            }
```

- [ ] **Step 4: Run both configurations**

```bash
cargo test
cargo test --features mas
```
Expected: PASS in both.

- [ ] **Step 5: Prove no production spawn survives under `mas`**

```bash
grep -rn 'Command::new' src/ | grep -v '#\[cfg(test)\]'
```
Expected: only `update.rs` (unreachable when `checks_enabled()` is false),
`install.rs` (unreachable when `needs_install` is false), and
`platform.rs`'s non-macOS branch.

- [ ] **Step 6: Commit**

```bash
git add src/update.rs src/install.rs src/workspace.rs
git commit -m "feat: disable update checks and self-install in the mas build"
```

---

### Task 8: Grammar surface — cut the second JIT

**Files:**
- Modify: `Cargo.toml` (`grammars` feature; `tree-sitter` wasm made optional)
- Modify: `src/highlight.rs:152-250` (registry), and its callers in `src/workspace.rs`
- Modify: `plugins/graphql/plugin.toml` (documented as MAS-unavailable)

**Interfaces:**
- Consumes: nothing.
- Produces: cargo feature `grammars` (default on); `highlight::grammars_enabled() -> bool`; `highlight::load_plugin_grammars` and `plugin_grammar_for_extension` become no-ops without it.

`tree_sitter::wasmtime::Engine` is wasmtime **24**, which predates usable
Pulley and offers no `Config` hook, so it cannot be interpreted. Cargo
features are additive, so `tree-sitter/wasm` must move into a *default*
feature that the MAS build declines.

- [ ] **Step 1: Write the failing test**

Add to `src/highlight.rs`'s test module:

```rust
#[test]
fn grammar_plugins_are_absent_without_the_feature() {
    assert_eq!(grammars_enabled(), cfg!(feature = "grammars"));
    if !grammars_enabled() {
        assert!(load_plugin_grammars(&[]).is_empty());
        assert_eq!(plugin_grammar_for_extension("graphql"), None);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test grammar_plugins_are_absent`
Expected: FAIL — `cannot find function 'grammars_enabled'`.

- [ ] **Step 3: Restructure the features**

In `Cargo.toml`:

```toml
tree-sitter = { version = "0.23" }

[features]
default = ["grammars"]
# Third-party tree-sitter grammar plugins, loaded as wasm. Pulls
# wasmtime 24 via tree-sitter's C API, which JITs and therefore cannot
# ship in the sandboxed App Store build.
grammars = ["tree-sitter/wasm"]
mas = []
```

- [ ] **Step 4: Gate the registry**

In `src/highlight.rs`, add:

```rust
/// Whether this build can load third-party tree-sitter grammar plugins.
/// The MAS build cannot: tree-sitter's wasm support is wasmtime 24, which
/// JIT-compiles and has no interpreter backend.
pub fn grammars_enabled() -> bool {
    cfg!(feature = "grammars")
}
```

Wrap the existing `SendStore`, `GrammarRegistry`, `GRAMMARS`,
`load_plugin_grammars`, `plugin_grammar_for_extension` and
`plugin_highlight` bodies in `#[cfg(feature = "grammars")]`, and add
no-op counterparts:

```rust
#[cfg(not(feature = "grammars"))]
pub fn load_plugin_grammars(
    specs: &[(String, std::path::PathBuf, crate::extensions::GrammarInfo)],
) -> Vec<(String, String)> {
    specs
        .iter()
        .map(|(p, ..)| (p.clone(), "grammar plugins are unavailable in this build".to_string()))
        .collect()
}

#[cfg(not(feature = "grammars"))]
pub fn plugin_grammar_for_extension(_ext: &str) -> Option<String> {
    None
}

#[cfg(not(feature = "grammars"))]
fn plugin_highlight(_name: &str, _code: &str) -> Option<Vec<(Range<usize>, u8)>> {
    None
}
```

- [ ] **Step 5: Verify wasmtime 24 is gone from the MAS build**

```bash
cargo tree --no-default-features --features mas -i wasmtime@24.0.13
```
Expected: `error: package ID specification ... did not match any packages`
— i.e. the second JIT is no longer linked. (With default features it still
resolves, which is correct.)

- [ ] **Step 6: Run both configurations**

```bash
cargo test
cargo test --no-default-features --features mas
```
Expected: PASS. Built-in highlighting (inkjet `all_languages`) is
unaffected in both — spot-check by opening a `.rs` file in each build.

- [ ] **Step 7: Decide GraphQL's fate — verified, not assumed**

inkjet 0.11.1 does **not** bundle a GraphQL grammar (confirmed: no
`graphql` in its manifest), so `plugins/graphql` is the only source of
GraphQL highlighting and it stops working under `mas`. Two acceptable
outcomes — pick one and record it here:

**(a) Accept the loss.** GraphQL fences render as plain text in the App
Store build. Add to `plugins/graphql/plugin.toml`:

```toml
# Grammar plugins load as wasm through tree-sitter's wasmtime 24, which
# JIT-compiles; unavailable in the sandboxed App Store build.
```

**Decision: (a), accept the loss.** Verified 2026-08-31: the only crate is
`tree-sitter-graphql` 0.2.1, whose `parser.c` declares `LANGUAGE_VERSION 15`
while `tree-sitter` 0.23.2's `api.h` accepts 13-14 (`TREE_SITTER_LANGUAGE_VERSION`
/ `..._MIN_COMPATIBLE_...`); it also dev-depends on tree-sitter 0.25.3. (b)
would require the forbidden tree-sitter upgrade.

**(b) Link a grammar statically.** Only if a `tree-sitter-graphql` release
targets tree-sitter 0.23 / grammar ABI 14 — check first:

```bash
cargo add --dry-run tree-sitter-graphql
```

If the ABI matches, add it as a `#[cfg(not(feature = "grammars"))]`
dependency and register it in `highlight.rs` beside the inkjet languages,
reusing `plugins/graphql/highlights.scm`. If it does not match, take (a) —
do not attempt a tree-sitter upgrade inside this plan.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml src/highlight.rs plugins/graphql/plugin.toml
git commit -m "feat: make wasm grammar plugins an optional default feature"
```

---

### Task 9: Install tier 1 — Import Plugin… from a file

**Files:**
- Modify: `src/catalog.rs:153-181` (factor out), add `install_plugin_from_bytes`
- Modify: `src/workspace.rs` (the Import command + open panel)
- Modify: `src/commands.rs` (the command table entry)

**Interfaces:**
- Consumes: `catalog::validate_plugin_zip`.
- Produces: `pub fn catalog::install_plugin_from_bytes(&[u8], &str, &Path) -> Result<(), String>`; `pub fn catalog::plugin_name_from_zip(&[u8]) -> Result<String, String>`.

This is the floor: the user downloads a plugin themselves and picks it in
an open panel, so the app never downloads code and 2.4.5(iv) does not
engage. It ships in **every** build, not just `mas`.

- [ ] **Step 1: Write the failing tests**

Add to `mod install_tests` in `src/catalog.rs`:

```rust
#[test]
fn installs_a_plugin_from_local_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let zip = good_zip(); // existing helper: one "demo" plugin
    install_plugin_from_bytes(&zip, "demo", dir.path()).unwrap();
    assert!(dir.path().join("demo/plugin.toml").exists());
}

#[test]
fn refuses_to_overwrite_an_installed_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let zip = good_zip();
    install_plugin_from_bytes(&zip, "demo", dir.path()).unwrap();
    let err = install_plugin_from_bytes(&zip, "demo", dir.path()).unwrap_err();
    assert!(err.contains("already installed"), "{err}");
}

#[test]
fn reads_the_plugin_name_out_of_the_zip() {
    assert_eq!(plugin_name_from_zip(&good_zip()).unwrap(), "demo");
}

#[test]
fn rejects_a_zip_whose_manifest_name_disagrees() {
    let dir = tempfile::tempdir().unwrap();
    assert!(install_plugin_from_bytes(&good_zip(), "other", dir.path()).is_err());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test installs_a_plugin_from_local_bytes`
Expected: FAIL — `cannot find function 'install_plugin_from_bytes'`.

- [ ] **Step 3: Factor the install path out of `install_plugin`**

In `src/catalog.rs`:

```rust
/// Validate and install an already-fetched plugin archive. Shared by
/// the catalog install (which fetches first) and the local Import
/// command (where the user supplied the bytes).
pub fn install_plugin_from_bytes(
    bytes: &[u8],
    name: &str,
    plugins_dir: &std::path::Path,
) -> Result<(), String> {
    let destination = plugins_dir.join(name);
    if destination.exists() {
        return Err(format!("{name} is already installed"));
    }
    validate_plugin_zip(bytes, name)?;
    let staging = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    archive.extract(staging.path()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(plugins_dir).map_err(|e| e.to_string())?;
    let staged = staging.path().join(name);
    if std::fs::rename(&staged, &destination).is_err() {
        copy_tree(&staged, &destination).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The single top-level directory name inside a plugin archive — the
/// plugin's name, which `validate_plugin_zip` then cross-checks against
/// the manifest.
pub fn plugin_name_from_zip(bytes: &[u8]) -> Result<String, String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut roots: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|e| e.to_string())?;
        let root = file.name().split('/').next().unwrap_or_default().to_string();
        if !root.is_empty() && !roots.contains(&root) {
            roots.push(root);
        }
    }
    match roots.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err("archive must contain exactly one plugin directory".to_string()),
    }
}
```

Then rewrite `install_plugin` to reuse it:

```rust
pub fn install_plugin(
    entry: &CatalogEntry,
    plugins_dir: &std::path::Path,
    fetch: &Fetcher,
) -> Result<(), String> {
    if !url_allowed(&entry.download) {
        return Err(format!("download URL is not from the SuperMD repo: {}", entry.download));
    }
    if plugins_dir.join(&entry.name).exists() {
        return Err(format!("{} is already installed", entry.name));
    }
    let bytes = fetch(&entry.download)?;
    if sha256_hex(&bytes) != entry.sha256 {
        return Err("download did not match the catalog checksum".to_string());
    }
    install_plugin_from_bytes(&bytes, &entry.name, plugins_dir)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test catalog`
Expected: PASS, including the pre-existing `install_plugin` tests — the
refactor must not change their behaviour.

- [ ] **Step 5: Wire the Import command**

In `src/workspace.rs`, add a handler that opens a file panel and installs:

```rust
    fn import_plugin(&mut self, _: &ImportPlugin, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let result = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    let name = crate::catalog::plugin_name_from_zip(&bytes)?;
                    crate::catalog::install_plugin_from_bytes(
                        &bytes,
                        &name,
                        &crate::settings::config_dir().join("plugins"),
                    )
                    .map(|_| name)
                });
            this.update(cx, |this, cx| {
                this.show_command_error(
                    match result {
                        Ok(name) => format!("Installed {name} — restart to load it"),
                        Err(e) => e,
                    },
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }
```

Add the `ImportPlugin` action and its `commands.rs` table row (menu:
Tools, no keybinding — so the keybinding count test does **not** change).

- [ ] **Step 6: Run the whole suite**

Run: `cargo test`
Expected: PASS. If `every_keybinding_parses_and_binds` fails, the command
was given a keystroke it should not have.

- [ ] **Step 7: Commit**

```bash
git add src/catalog.rs src/workspace.rs src/commands.rs
git commit -m "feat: import a plugin from a local archive"
```

---

### Task 10: Install tier 2 — the `supermd://` handoff

**Files:**
- Modify: `src/catalog.rs` (add `entry_by_name`, `parse_install_url`)
- Modify: `src/main.rs:149-157` (`queue_open_urls`)
- Modify: `src/install_ui.rs` (gate the catalog browse under `mas`)
- Modify: `scripts/bundle_macos.sh`, `scripts/bundle_mas.sh` (`CFBundleURLTypes`)

**Interfaces:**
- Consumes: `catalog::{parse_catalog, install_plugin, ureq_fetcher}`.
- Produces: `pub fn catalog::parse_install_url(&str) -> Option<String>`; `pub fn catalog::entry_by_name(&[CatalogEntry], &str) -> Option<&CatalogEntry>`; `enum main::PendingOpen { Path(PathBuf), InstallPlugin(String) }`.

The Drafts pattern: the catalog lives on the website, the user clicks
Install there, and a URL scheme hands one plugin name to the app, which
confirms before installing. No in-app storefront, so §3.3.2(b) is satisfied.

- [ ] **Step 1: Write the failing tests**

Add to `src/catalog.rs`:

```rust
#[test]
fn parses_a_plugin_name_out_of_an_install_url() {
    assert_eq!(parse_install_url("supermd://install-plugin?name=calc"), Some("calc".into()));
}

#[test]
fn rejects_install_urls_that_are_not_ours() {
    assert_eq!(parse_install_url("https://evil.example.com/?name=calc"), None);
    assert_eq!(parse_install_url("supermd://open?name=calc"), None);
    assert_eq!(parse_install_url("supermd://install-plugin"), None);
}

#[test]
fn rejects_a_name_with_path_separators() {
    assert_eq!(parse_install_url("supermd://install-plugin?name=../evil"), None);
    assert_eq!(parse_install_url("supermd://install-plugin?name=a/b"), None);
}

#[test]
fn finds_a_catalog_entry_by_name() {
    let entries = vec![entry_for(&good_zip())]; // existing helpers; name "demo"
    assert!(entry_by_name(&entries, "demo").is_some());
    assert!(entry_by_name(&entries, "nope").is_none());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test parses_a_plugin_name_out_of`
Expected: FAIL — `cannot find function 'parse_install_url'`.

- [ ] **Step 3: Implement**

```rust
/// Extract the plugin name from a `supermd://install-plugin?name=X`
/// handoff. Anything else — a foreign scheme, a different host, a name
/// with path separators — is rejected outright; the name is then looked
/// up in the pinned catalog, so only known plugins can ever install.
pub fn parse_install_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("supermd://install-plugin?")?;
    let name = rest
        .split('&')
        .find_map(|kv| kv.strip_prefix("name="))?
        .trim()
        .to_string();
    let safe = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then_some(name)
}

/// Look one catalog entry up by name.
pub fn entry_by_name<'a>(entries: &'a [CatalogEntry], name: &str) -> Option<&'a CatalogEntry> {
    entries.iter().find(|e| e.name == name)
}
```

- [ ] **Step 4: Route the URL through the existing open-event queue**

In `src/main.rs`, widen the pending queue:

```rust
/// Something a macOS open event asked us to do, drained by the
/// workspace's poll loop.
pub enum PendingOpen {
    Path(PathBuf),
    /// `supermd://install-plugin?name=X` — the website's Install link.
    InstallPlugin(String),
}

/// Queue work from open-event URLs (`file://` opens and `supermd://`
/// plugin-install handoffs) for the workspace poll loop.
fn queue_open_urls(pending: &std::sync::Mutex<Vec<PendingOpen>>, urls: Vec<String>) {
    let mut lock = pending.lock().unwrap();
    for url in urls {
        if let Some(path) = file_url_to_path(&url) {
            lock.push(PendingOpen::Path(path));
        } else if let Some(name) = catalog::parse_install_url(&url) {
            lock.push(PendingOpen::InstallPlugin(name));
        }
    }
}
```

Update the drain site in `src/workspace.rs` to match on `PendingOpen`, and
for `InstallPlugin` show a confirmation listing the plugin's declared
capabilities before calling `catalog::install_plugin`.

- [ ] **Step 5: Register the URL scheme**

Add to the `Info.plist` heredoc in **both** `scripts/bundle_macos.sh` and
`scripts/bundle_mas.sh` (Task 11):

```xml
    <key>CFBundleURLTypes</key>
    <array>
      <dict>
        <key>CFBundleURLName</key><string>com.superjackfruitlabs.supermd.install</string>
        <key>CFBundleURLSchemes</key><array><string>supermd</string></array>
      </dict>
    </array>
```

- [ ] **Step 6: Gate the in-app catalog browse**

In `src/install_ui.rs`, guard the overlay's *catalog listing* — not the
overlay itself, which still hosts Import…:

```rust
    /// The App Store build shows no browsable catalog: a list of
    /// downloadable plugins reads as a storefront for other code under
    /// DPLA 3.3.2(b). Install arrives via Import… or a supermd:// link.
    fn catalog_browsable() -> bool {
        !cfg!(feature = "mas")
    }
```

- [ ] **Step 7: Run and verify by hand**

```bash
cargo test
cargo test --no-default-features --features mas
```
Then build the app, and from Terminal:
`open "supermd://install-plugin?name=calc"` — the app should raise a
confirmation, not install silently.

- [ ] **Step 8: Commit**

```bash
git add src/catalog.rs src/main.rs src/workspace.rs src/install_ui.rs scripts/bundle_macos.sh
git commit -m "feat: install plugins via a supermd:// handoff from the website"
```

---

### Task 11: Entitlements and the MAS bundle script

**Files:**
- Create: `assets/mas.entitlements`, `scripts/bundle_mas.sh`
- Modify: `scripts/bundle_macos.sh:31` (`CFBundleVersion`)

**Interfaces:**
- Consumes: the `mas` feature from Task 1, `grammars` from Task 8.
- Produces: `dist/SuperMD-mas.pkg`.

- [ ] **Step 1: Write the entitlements**

Create `assets/mas.entitlements`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <!-- Mandatory for the Mac App Store. -->
    <key>com.apple.security.app-sandbox</key>
    <true/>
    <!-- Workspaces and exports the user picks in an open/save panel. -->
    <key>com.apple.security.files.user-selected.read-write</key>
    <true/>
    <!-- Reopen-last-workspace across launches (see src/bookmarks.rs). -->
    <key>com.apple.security.files.bookmarks.app-scope</key>
    <true/>
    <!-- Plugin catalog + the host-mediated fetch plugins use for data. -->
    <key>com.apple.security.network.client</key>
    <true/>
    <!-- NOTE: no com.apple.security.cs.* keys. The mas build interprets
         Pulley bytecode and maps no executable pages; verified with
         extensions::bench_backends::hardened_runtime_probe. -->
</dict>
</plist>
```

- [ ] **Step 2: Write the bundle script**

Create `scripts/bundle_mas.sh` (mirroring `bundle_macos.sh`, diverging at
the feature flags, identity, entitlements, and packaging):

```bash
#!/bin/bash
# Build the sandboxed App Store package into dist/.
# Usage: scripts/bundle_mas.sh <version>
# Requires: APP_IDENTITY   ("Apple Distribution: ...")
#           INSTALLER_IDENTITY ("3rd Party Mac Developer Installer: ...")
#           PROFILE        (path to the .provisionprofile)
set -euo pipefail

VERSION="${1:?version required}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"
APP="$DIST/SuperMD.app"

echo "building sandboxed release binary…"
cargo build --release --no-default-features --features mas \
    --manifest-path "$ROOT/Cargo.toml"

rm -rf "$APP" "$DIST"/SuperMD-mas*.pkg
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/supermd" "$APP/Contents/MacOS/supermd"
cp "$ROOT/assets/icon.icns" "$APP/Contents/Resources/icon.icns"

# CFBundleVersion must strictly increase per upload; the App Store
# rejects a re-upload that reuses one. Callers pass a build number.
BUILD="${BUILD_NUMBER:-$(date +%Y%m%d%H%M)}"

# Copy the Info.plist heredoc verbatim from scripts/bundle_macos.sh
# (lines 24-70: CFBundleName through NSSupportsAutomaticGraphicsSwitching),
# changing exactly three things:
#   1. <key>CFBundleVersion</key><string>${BUILD}</string>   (was ${VERSION})
#   2. append the CFBundleURLTypes block added in Task 10
#   3. nothing else - bundle id, icon, document types and the 12.0
#      minimum are shared with the Developer ID build on purpose.

# All 14 plugins ride inside the bundle — the MAS build has no catalog
# browser, so anything not shipped is unreachable until the user imports it.
mkdir -p "$APP/Contents/Resources/plugins"
cp -R "$ROOT/dist/plugins/." "$APP/Contents/Resources/plugins/"

cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"

codesign --force --options runtime --timestamp \
    --entitlements "$ROOT/assets/mas.entitlements" \
    --sign "$APP_IDENTITY" "$APP"

productbuild --component "$APP" /Applications \
    --sign "$INSTALLER_IDENTITY" \
    "$DIST/SuperMD-mas-${VERSION}.pkg"

echo "done: $DIST/SuperMD-mas-${VERSION}.pkg"
```

`chmod +x scripts/bundle_mas.sh`.

- [ ] **Step 3: Fix the Developer ID build number too**

In `scripts/bundle_macos.sh`, make `CFBundleVersion` monotonic rather than
equal to the marketing version:

```bash
BUILD="${BUILD_NUMBER:-$(date +%Y%m%d%H%M)}"
# ... <key>CFBundleVersion</key><string>${BUILD}</string>
```

- [ ] **Step 4: Build and inspect**

```bash
bash scripts/build_plugins.sh
bash scripts/bundle_mas.sh 0.0.14
codesign -d --entitlements - dist/SuperMD.app
```
Expected: the four sandbox keys, and **no** `com.apple.security.cs.*`.

- [ ] **Step 5: Verify the sandbox at runtime**

Install the `.pkg` locally and confirm, in order: the app launches; the
open panel grants a workspace; the workspace reopens after a restart
(bookmarks); a plugin renders a fence (Pulley under sandbox); delete sends
a file to the Trash; reveal opens Finder. Then check
`~/Library/Containers/com.superjackfruitlabs.supermd/Data/.supermd/` exists.

Also confirm **open risk 2** from the spec: GPUI's `runtime_shaders`
compiles Metal shaders at launch. If the window renders, it passed.

- [ ] **Step 6: Validate, then upload to TestFlight — do not submit yet**

The spec's staged rollout (see its *Staged rollout* section) says the first
build goes to TestFlight, not to App Review. Rungs 1 and 2 need no human
review, so they answer the runtime questions before anyone at Apple sees
the plugin system.

Rung 1 — validate and upload:

```bash
xcrun altool --validate-app -f dist/SuperMD-mas-0.0.14.pkg \
    -t macos --apiKey "$KEY_ID" --apiIssuer "$ISSUER_ID"
xcrun altool --upload-app -f dist/SuperMD-mas-0.0.14.pkg \
    -t macos --apiKey "$KEY_ID" --apiIssuer "$ISSUER_ID"
```
Expected: no errors. Private-API usage in gpui, if any, surfaces here —
this is the whole of open risk 3, retired before a single tester installs.

Rung 2 — internal TestFlight (**no review of any kind**): in App Store
Connect, add the processed build to an internal tester group (up to 100
team members holding an App Store Connect role). Install it through the
TestFlight app on a Mac that has never run a dev build, and walk Step 5's
checklist again there. A clean Step 5 locally is not the same evidence: an
Apple-installed, TestFlight-delivered build is the first time the container,
the provisioning profile, and the sandbox are all real at once.

Do **not** promote to an external group or submit for App Review from this
task — that is a release decision, and the plan ends at a validated build.

- [ ] **Step 7: Commit**

```bash
git add assets/mas.entitlements scripts/bundle_mas.sh scripts/bundle_macos.sh
git commit -m "build: sandboxed App Store bundle and entitlements"
```

---

### Task 12: CI and documentation

**Files:**
- Modify: `.github/workflows/release.yml`, `.github/workflows/ci.yml`
- Modify: `docs/site/plugins.md`, `docs/site/themes.md`
- Regenerate: `site/docs/`

**Interfaces:**
- Consumes: `scripts/bundle_mas.sh`.
- Produces: a `mas` release job; regenerated docs.

- [ ] **Step 1: Build the MAS configuration in CI**

In `.github/workflows/ci.yml`, add to the macOS job so the feature
combination cannot rot:

```yaml
      - name: Build the App Store configuration
        run: cargo build --no-default-features --features mas
      - name: Test the App Store configuration
        run: cargo test --no-default-features --features mas
```

- [ ] **Step 2: Add the release job**

In `.github/workflows/release.yml`, beside `macos`:

```yaml
  mas:
    name: Build App Store package
    runs-on: macos-latest
    if: ${{ secrets.MAS_CERT_P12 != '' }}
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Import Apple Distribution certificate
        env:
          CERT_P12: ${{ secrets.MAS_CERT_P12 }}
          CERT_PASSWORD: ${{ secrets.MAS_CERT_PASSWORD }}
          INSTALLER_P12: ${{ secrets.MAS_INSTALLER_P12 }}
          PROFILE_B64: ${{ secrets.MAS_PROFILE }}
        run: |
          # Import both identities into a scratch keychain, exactly as
          # the `macos` job does (create-keychain, set-keychain-settings,
          # unlock, import, set-key-partition-list, list-keychains), once
          # for MAS_CERT_P12 -> APP_IDENTITY ("Apple Distribution") and
          # once for MAS_INSTALLER_P12 -> INSTALLER_IDENTITY
          # ("3rd Party Mac Developer Installer"). Then:
          echo "$PROFILE_B64" | base64 -d > mas.provisionprofile
          echo "PROFILE=$PWD/mas.provisionprofile" >> "$GITHUB_ENV"
      - name: Build plugins
        run: bash scripts/build_plugins.sh
      - name: Bundle
        run: |
          VERSION="${GITHUB_REF_NAME#v}"
          BUILD_NUMBER="${GITHUB_RUN_NUMBER}" bash scripts/bundle_mas.sh "$VERSION"
      - uses: actions/upload-artifact@v4
        with:
          name: mas
          path: dist/SuperMD-mas-*.pkg
          if-no-files-found: error
```

The `.pkg` is **not** attached to the GitHub Release — it goes to App Store
Connect via Transporter. Leave it out of the `publish` job's globs.

- [ ] **Step 3: Update the docs sources**

In `docs/site/plugins.md`, add a section covering the two MAS install
routes (Import… and the website's Install link) and noting that grammar
plugins are unavailable in the App Store build. In `docs/site/themes.md`,
note that App Store installs read themes from
`~/Library/Containers/com.superjackfruitlabs.supermd/Data/.supermd/themes/`,
reachable via the Reveal Settings Folder command.

- [ ] **Step 4: Regenerate**

Run: `cargo run --example build_docs`
Expected: `site/docs/` updates. Never hand-edit it.

- [ ] **Step 5: Final full verification**

```bash
cargo test
cargo test --no-default-features --features mas
cargo llvm-cov --summary-only
```
Expected: both suites green; coverage at or above the 90% floor.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows docs/site site/docs
git commit -m "ci: build and test the App Store configuration; document it"
```

---

## Post-plan: what remains outside our control

1. **App Review may reject the plugin system** on 2.4.5(iv) despite DPLA
   §3.3.2. If it does, set `catalog_browsable()` and the `supermd://`
   handler to `false` under `mas` — Task 9's Import… path survives, and
   third-party plugins keep working.
2. **The rollout ladder past rung 2.** Rung 3 is external TestFlight, whose
   first build per version goes through **Beta App Review** (~24h) — the
   cheap, private probe of the 2.4.5(iv) question above. Only after that
   does rung 4, full submission, make sense. External testing additionally
   needs a beta app description and beta review notes.
3. **App Store Connect metadata** — screenshots, privacy policy URL,
   privacy nutrition labels ("Data Not Collected"), age rating, export
   compliance (`ITSAppUsesNonExemptEncryption=false`; HTTPS only). Not
   engineering work, but it blocks rung 3 onward.
4. **`docs/BACKLOG.md`** should gain a line pointing at this plan so the
   decision history stays findable.
