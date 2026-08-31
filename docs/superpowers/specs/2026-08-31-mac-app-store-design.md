# Mac App Store Distribution — Design

**Status:** proposed, not yet approved for build
**Date:** 2026-08-31

## Problem

SuperMD ships today as a Developer ID–signed, notarized, stapled DMG
(`scripts/bundle_macos.sh` + the `macos` job in `.github/workflows/release.yml`).
That track is complete and works. The Mac App Store is a *separate*
distribution target with its own signing identity, its own packaging
format, and — the part that actually costs engineering — the App Sandbox
and the App Review guidelines.

This spec records what the App Store requires, which of SuperMD's design
decisions collide with it, and what the collisions cost. It exists so the
plan (`docs/superpowers/plans/2026-08-31-mac-app-store.md`) can argue from
measured facts rather than assumptions.

## Goal

A second, parallel build target — `--features mas` — that produces a
sandboxed, App Store–signed `.pkg`, with the Developer ID build's behaviour
completely unchanged when the feature is off.

## Non-goals

- Replacing the Developer ID / DMG track. Both ship. The DMG remains the
  primary distribution and keeps every capability.
- iOS or iPadOS. GPUI has no iOS backend; this is not on the table.
- Any change to the on-disk format. Plain CommonMark stays the source of
  truth, sandbox or not.

## What already exists

Nothing in this list needs work — it is recorded so the plan does not
re-derive it:

- Hardened runtime signing with a Developer ID identity, notarization with
  retry, and stapling (`release.yml`).
- `Info.plist` with bundle id `com.superjackfruitlabs.supermd`,
  `LSMinimumSystemVersion` 12.0, icons, and `CFBundleDocumentTypes` for
  Markdown / plain text / source / folder.
- A signed, dressed DMG.
- Plugin seeding from `Contents/Resources/plugins` (`src/seeding.rs`).

## Blocker 1 — executable memory

### There are two wasm JITs, not one

| Site | Runtime | Purpose |
| ---- | ------- | ------- |
| `src/extensions.rs:568` | wasmtime 48 + cranelift | the plugin host |
| `src/highlight.rs:178` | **wasmtime 24.0.13** via `wasmtime-c-api-impl`, pulled in by `tree-sitter`'s `wasm` feature | third-party tree-sitter grammar plugins |

`assets/entitlements.plist` therefore requires both
`com.apple.security.cs.allow-jit` and
`com.apple.security.cs.allow-unsigned-executable-memory`. The latter is a
Hardened Runtime exception that App Review scrutinises and commonly
refuses; more fundamentally, code compiled to native is "executable code"
under guideline 2.5.2 and is banned outright.

### Measured: Pulley solves it for the plugin host

wasmtime 48 exposes a `pulley` feature (verified in the crate's
`Cargo.toml`); `pulley-interpreter 48.0.0` was already resolved in
`Cargo.lock`. Cranelift still compiles, but targets Pulley bytecode rather
than aarch64, and a Rust interpreter loop executes it — **no executable
pages at all**.

Benchmarked with `ExtensionHost` against all 14 built first-party plugins,
release profile, aarch64, median of 20 iterations after warmup
(`src/extensions.rs`, `mod bench_backends`):

| Workload | cranelift (native) | pulley64 (interp) | Ratio |
| -------- | -----------------: | ----------------: | ----: |
| compile all 14 plugins | 361.9 ms | **328.4 ms** | 0.91× *(faster)* |
| word-count status, 3 KB | 9.3 µs | 178.6 µs | 19× |
| word-count status, 125 KB | 301 µs | 7.2 ms | 24× |
| calc render_inline | 3.3 µs | 66.2 µs | 20× |
| tidy format, 3 KB | 27 µs | 606 µs | 22× |
| tidy format, 125 KB | 947 µs | 22.6 ms | 24× |
| tidy format, 1 MB | 7.5 ms | 190 ms | 25× |
| toc on_save, 125 KB | 38.4 µs | 1.1 ms | 29× |

**The 19–29× slowdown does not reach the user.** Every plugin surface
already runs on `cx.background_executor()`:

- `status_text` — `src/editor/mod.rs:699`, behind a 500 ms debounce
- `format_document` / `export_document` / `run_command` — `src/workspace.rs:1390`
- `render_view` — `src/workspace.rs:881`
- `render_template` — `src/workspace.rs:1352`
- inline rules — the drainer in `extensions::start_inline_drainer`

Compile time slightly *improves*, so startup does not regress.

### Verified: the entitlements actually become unnecessary

The test binary was ad-hoc signed with the hardened runtime and **no
entitlements** (`flags=0x10002(adhoc,runtime)`), then run:

- cranelift → **SIGKILL, exit 137** — the "CODESIGNING: Invalid Page" kill
  already documented in `assets/entitlements.plist`.
- pulley64 → **runs clean**, `status_text -> Ok("5 words · 1 min read")`.

Reproduce with `bench_backends::hardened_runtime_probe` (see the plan,
Task 1). The `pulley` feature alone is sufficient; `all-arch` is not needed
and would only bloat the binary.

### D1 — the epoch budget shrinks 24× in real terms

`CALL_DEADLINE_TICKS = 4` (≈2 s) stays 2 s of wall clock, but now buys
roughly 83 ms of native-equivalent work. First-party plugins have ~10×
headroom (190 ms for a 1 MB document). A third-party plugin doing 150 ms of
native work today would become ~3.6 s and trap. **Decision:** raise the
deadline under `mas` rather than silently breaking heavy plugins.

### D2 — tree-sitter grammar plugins are cut under `mas`

wasmtime 24 predates usable Pulley, and `tree_sitter::wasmtime::Engine::default()`
offers no `Config` hook. Upgrading past tree-sitter 0.23 is blocked by
inkjet 0.11's pin and grammar ABI 14 — too large a ripple for one plugin.

Exactly **one** first-party plugin uses a wasm grammar
(`plugins/graphql/plugin.toml`). All inkjet built-in languages are
statically linked (`all_languages`), so the editor's own highlighting is
untouched.

**Decision:** under `mas`, drop `tree-sitter/wasm` and compile out the
grammar registry.

inkjet 0.11.1 does **not** bundle a GraphQL grammar (verified — no
`graphql` in its manifest), so `plugins/graphql` is the sole source of
GraphQL highlighting and it goes dark in the MAS build. The plan's Task 8
carries an explicit either/or: accept that loss, or statically link
`tree-sitter-graphql` *if* a release targeting tree-sitter 0.23 / grammar
ABI 14 exists. A tree-sitter upgrade is out of scope either way.

The larger loss is the third-party grammar *extension point*.

## Blocker 2 — App Review and downloaded code

### The three texts that apply

- **2.5.2** — apps "may not download, install, or execute code which
  introduces or changes features or functionality of the app".
- **2.4.5(iv)**, macOS-specific and stricter — apps "may not download or
  install standalone apps, kexts, additional code, or resources to add
  functionality or significantly change the app from what we see during the
  review process."
- **Developer Program License Agreement §3.3.2**, the carve-out —
  *"Interpreted code may be downloaded to an Application but only so long as
  such code: (a) does not change the primary purpose of the Application by
  providing features or functionality that are inconsistent with the
  intended and advertised purpose of the Application as submitted to the App
  Store, (b) does not create a store or storefront for other code or
  applications, and (c) does not bypass signing, sandbox, or other security
  features of the OS."*

Blocker 1 and Blocker 2 solve together: under cranelift the plugins are
compiled to native and are flatly banned; under Pulley they are bytecode
processed by an interpreter — the same category as the JavaScript, Lua and
Python that §3.3.2 explicitly permits.

Scoring SuperMD against the carve-out:

| Condition | Verdict |
| --------- | ------- |
| (a) primary purpose | **Passes.** A Markdown editor stays a Markdown editor. |
| (b) no store or storefront | **Fails today.** The "Install Plugins…" overlay (`src/install_ui.rs`) is literally a browsable storefront for other code. |
| (c) does not bypass sandbox | **Passes emphatically.** Capability gating keyed off the manifest, read-only preopen, no sockets, no processes, hard timeout. |

### Precedents

- [Drafts](https://apps.apple.com/us/app/drafts/id1435957248?mt=12) ships on
  the Mac App Store and installs community JavaScript actions from a web
  directory: the user browses `actions.getdrafts.com` in a *browser*, clicks
  Install, a URL scheme hands off to the app, and the app prompts to
  confirm. There is no in-app store.
- [Nova is deliberately absent from the Mac App Store](https://www.macstories.net/reviews/nova-review-panics-code-editor-demonstrates-why-mac-like-design-matters/)
  because its extensions rely on arbitrary third-party *executables* that
  sandboxing forbids. SuperMD's plugins spawn no processes and open no
  sockets, which is the distinction that matters.

### D3 — three install tiers, ranked by risk

1. **Import Plugin… via `NSOpenPanel`** — near-zero risk. The user
   downloads a plugin themselves; the app never downloads code, so
   2.4.5(iv) does not engage. This is the floor: third-party plugins
   survive regardless of how review rules on tier 2.
2. **`supermd://install-plugin` URL handoff** from the existing site — the
   Drafts pattern. Low-to-medium risk, live precedent.
3. **In-app browsable catalog** (today's `install_ui.rs` overlay) — highest
   risk; reads as a storefront under §3.3.2(b) and as downloading code to
   add functionality under 2.4.5(iv). **Cut under `mas`.**

`src/catalog.rs` already does org-pinned URL allowlisting and sha256
verification, so the security machinery for tiers 1 and 2 exists. Only the
*initiation point* moves. Structured this way, a rejection of tier 2 costs
the one-click convenience, not the plugin system.

### D4 — the updater and the self-installer are cut under `mas`

- `src/update.rs:45` spawns **`curl`** to poll GitHub releases. Subprocess
  spawning is blocked by the sandbox, and pointing users at an external
  binary download is exactly what 2.4.5(iv) forbids. The App Store owns
  updates.
- `src/install.rs:41` spawns a process to offer moving the app into
  `/Applications`. The App Store installs there already, and a self-moving
  app violates 2.4.5(ii).

## Blocker 3 — the App Sandbox

`com.apple.security.app-sandbox` is mandatory and everything below follows
from it.

### What already works

- `cx.prompt_for_paths` (`src/workspace.rs:825`, `:1504`) is `NSOpenPanel`,
  so Powerbox grants scope automatically.
- Files opened from Finder arrive scoped via `CFBundleDocumentTypes`.
- The `notify` watcher (FSEvents) and `gix` reads both work inside a
  granted scope.
- `main.rs`'s `queue_open_urls` already handles `file://` open events — the
  natural hook for a custom URL scheme.

### What breaks

| # | Site | Breakage | Fix |
| - | ---- | -------- | --- |
| B1 | `src/settings.rs:14`, `src/main.rs:139` | `recent_workspaces` holds bare paths; reopen-last fails every launch | security-scoped bookmarks |
| B2 | `src/fileops.rs:79` | `trash::delete` defaults to `DeleteMethod::Finder`, which shells out to `osascript` and sends Apple events — blocked | `DeleteMethod::NsFileManager` |
| B3 | `src/platform.rs:166` | `reveal_dir` spawns `open` — blocked | `NSWorkspace::activateFileViewerSelectingURLs` |
| B4 | `src/main.rs:293` | a CLI path argument carries no scope | treat as a suggestion; open the panel pre-pointed there |
| B5 | `src/git.rs:26` | `gix::discover` walks *upward* out of scope | already degrades to "no baseline"; add a status hint (plan Task 6) |
| B6 | `src/platform.rs:64` | `home_dir()` resolves to the container, moving `~/.supermd/` | no path change; a Reveal Settings Folder command (plan Task 6) + docs |

The four production process spawns are `update.rs:45` (curl),
`install.rs:41` (move-to-Applications), and `platform.rs:166` (reveal).
Every `Command::new("git")` in the tree is inside `#[cfg(test)]` and does
not ship — verified.

### D5 — bookmark storage shape

`recent_workspaces: Vec<String>` stays as-is for backward compatibility. A
new `workspace_bookmarks: BTreeMap<String, String>` maps path → hex-encoded
bookmark blob, `#[serde(default)]` so old and new `settings.toml` files
interoperate in both directions. Hex rather than base64 keeps the encoder a
dependency-free pure function.

Scope is started only for the *currently open* workspace and stopped on
close — never for all eight recents, which would burn kernel scoped-resource
slots.

### D6 — objc2 is already resolved

`objc2 0.6.4`, `objc2-foundation 0.3.2` and `objc2-app-kit 0.3.2` are
already in `Cargo.lock` transitively via gpui. Promoting them to
macOS-target dependencies adds no downloads. Verified API surface:

- `NSURL::bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error`
- `NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error`
- `NSURL::startAccessingSecurityScopedResource() -> bool`
- `NSURL::stopAccessingSecurityScopedResource()`
- `NSURLBookmarkCreationOptions::WithSecurityScope` (`1<<11`)
- `NSURLBookmarkResolutionOptions::WithSecurityScope` (`1<<10`)
- `NSWorkspace::activateFileViewerSelectingURLs(&NSArray<NSURL>)`

## Entitlements

The `mas` build's entitlements are a different file, not an edit of
`assets/entitlements.plist` (which the Developer ID build keeps using):

```
com.apple.security.app-sandbox                  true
com.apple.security.files.user-selected.read-write  true
com.apple.security.files.bookmarks.app-scope    true
com.apple.security.network.client               true
```

Both `com.apple.security.cs.*` keys are **absent** — verified unnecessary
under Pulley.

## Packaging

- Apple Distribution certificate, not Developer ID.
- Mac App Store provisioning profile embedded at
  `Contents/embedded.provisionprofile`.
- `productbuild --component`, signed with *3rd Party Mac Developer
  Installer*, uploaded via Transporter or `xcrun altool --upload-app`.
- No notarization step — the App Store does its own.
- `CFBundleVersion` must strictly increase per upload.
  `scripts/bundle_macos.sh:31` currently sets it equal to the marketing
  version, so a re-upload of the same tag would be rejected.

## Accepted losses

| Loss | Severity |
| ---- | -------- |
| Plugin execution 19–29× slower (all off the UI thread, ≤190 ms for 1 MB) | negligible |
| Third-party tree-sitter grammars, plus GraphQL highlighting unless a static grammar is linked | small |
| In-app plugin *browsing* (install itself survives via tiers 1 and 2) | moderate |
| Finder "Put Back" and the trash sound on delete | cosmetic |
| `supermd <path>` from a terminal becomes a prompt | small |
| Show Changes when a subdirectory of a repo is opened | small |
| `~/.supermd/` moves into the container | needs docs + a Reveal command |
| No in-app update check (the App Store owns updates) | none |

## Open risks

1. **Review may reject the plugin system outright** on 2.4.5(iv), despite
   §3.3.2. Mitigation: tier 1 (open-panel import) never downloads code, so
   the fallback build is one cfg away.
2. **`runtime_shaders`** compiles Metal shaders at launch via
   `newLibraryWithSource`. This goes through the out-of-process Metal
   compiler and should not need JIT entitlements, but it has **not** been
   verified under the sandbox. Task 11 verifies it before submission.
3. **Private API usage in gpui** would fail App Store Connect validation.
   Unknown until a real upload is attempted; `altool --validate-app`
   catches it early.

## Effort

Roughly 6–11 engineering days across the 12 tasks in the plan, plus review
turnaround, which is not under our control.
