# Extensions Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The wasmtime extension runtime, plugin block renderers, plugin text commands with a new ⌘⇧P palette, the author template, and first-party `dot` + `toc` plugins.

**Architecture:** Host side: `src/extensions.rs` owns wasmtime (component model, `bindgen!` over `plugins/wit/extension.wit`), manifest discovery under `~/.supermd/plugins/`, epoch-deadline calls on the background executor, and a never-fatal load report. Guest side: plain Rust crates in `plugins/` built with `wit-bindgen` + `--target wasm32-wasip2` (components directly; no cargo-component). Block renderers ride the existing projector registry + diagram cache; commands ride a new finder-family palette applying one-undo-group edits through `EditorCore`.

**Tech Stack:** wasmtime 48 (component-model), wit-bindgen (guest), `wasm32-wasip2` target, layout-rs 0.1.3 (dot), existing nucleo/projector/diagram infrastructure.

**Spec:** `docs/superpowers/specs/2026-08-24-extensions-phase1-design.md`

## Global Constraints

- Plugin failures are data, never crashes: load/link errors → report; traps/timeouts → `Err(String)`; enforced by fixture tests (panicking + hanging plugins).
- Every plugin call runs under a 2s epoch deadline, on the background executor.
- Manifest `capabilities` key present → rejected with a forward-compat error.
- Fixture-dependent host tests skip (with a printed notice) when fixtures aren't built; the linux CI job builds fixtures so they always run somewhere.
- TDD for all pure logic; commit per task; full suite green before each commit.

---

### Task 1: WIT + manifest parsing

**Files:** Create `plugins/wit/extension.wit` (exact content from spec Component 1), `src/extensions.rs` (manifest half); modify `src/main.rs` (`mod extensions;`)

**Interfaces — Produces:**

```rust
pub struct CommandInfo { pub id: String, pub title: String }
pub struct PluginMeta { pub name: String, pub version: String,
    pub fences: Vec<String>, pub commands: Vec<CommandInfo>, pub dir: PathBuf }
pub fn parse_manifest(dir: &Path, toml_src: &str) -> Result<PluginMeta, String>;
pub fn discover(plugins_dir: &Path) -> (Vec<PluginMeta>, Vec<(PathBuf, String)>); // (loaded, failures)
```

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn manifest_parses_contributions() {
    let m = parse_manifest(Path::new("/p/dot"), r#"
name = "dot"
version = "0.1.0"
fences = ["dot", "graphviz"]
[[commands]]
id = "dot.about"
title = "About Dot"
"#).unwrap();
    assert_eq!(m.name, "dot");
    assert_eq!(m.fences, ["dot", "graphviz"]);
    assert_eq!(m.commands[0].id, "dot.about");
}

#[test]
fn capabilities_key_is_rejected_forward_compat() {
    let err = parse_manifest(Path::new("/p/x"), "name=\"x\"\nversion=\"0\"\ncapabilities=[\"net\"]\n")
        .unwrap_err();
    assert!(err.contains("capabilities"), "{err}");
}

#[test]
fn discover_collects_good_and_bad() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good");
    std::fs::create_dir_all(&good).unwrap();
    std::fs::write(good.join("plugin.toml"), "name=\"good\"\nversion=\"1\"\nfences=[\"x\"]\n").unwrap();
    std::fs::write(good.join("plugin.wasm"), b"stub").unwrap();
    let bad = dir.path().join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("plugin.toml"), "not toml [").unwrap();
    let (ok, fail) = discover(dir.path());
    assert_eq!(ok.len(), 1);
    assert_eq!(fail.len(), 1);
}

#[test]
fn discover_requires_wasm_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("nowasm");
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("plugin.toml"), "name=\"n\"\nversion=\"1\"\n").unwrap();
    let (ok, fail) = discover(dir.path());
    assert!(ok.is_empty());
    assert_eq!(fail.len(), 1);
}
```

- [ ] **Step 2:** RED → implement with serde (`#[serde(deny_unknown_fields)]` on a `ManifestFile` struct whose fields include `capabilities: Option<toml::Value>` checked explicitly → tailored error). Write the WIT file verbatim from the spec. `rustup target add wasm32-wasip2`.

- [ ] **Step 3:** Green, full suite, commit `feat: extension manifest parsing and WIT interface`.

---

### Task 2: Guest crates — template + fixtures + build script

**Files:** Create `plugins/template/` (Cargo.toml, src/lib.rs, README.md), `plugins/fixtures/echo/`, `plugins/fixtures/panic/`, `plugins/fixtures/hang/`, `scripts/build_plugins.sh`

**Interfaces — Produces:** built components under `dist/plugins/` and `tests/fixtures/plugins/` (`--fixtures` mode); the template crate other tasks copy.

- [ ] **Step 1: Template crate** — `plugins/template/Cargo.toml`:

```toml
[package]
name = "supermd-plugin-template"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"
```

`src/lib.rs`:

```rust
wit_bindgen::generate!({ path: "../wit", world: "extension" });

struct Plugin;

impl Guest for Plugin {
    fn render_block(_lang: String, _source: String, _theme: Theme) -> Result<String, String> {
        Err("unsupported".into())
    }
    fn run_command(_id: String, _input: CommandInput) -> Result<CommandOutput, String> {
        Err("unsupported".into())
    }
}

export!(Plugin);
```

(Exact generated type names — `Theme`, `CommandInput`, `CommandOutput`, `Guest`, `export!` — adjusted to what wit-bindgen 0.36 emits for the world; resolve at implementation by building this crate first.)

- [ ] **Step 2: Fixtures** (each a copy of the template with one change):
  - `echo`: `render_block` returns `Ok(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><desc>{lang}:{source}</desc></svg>"))`; `run_command` returns `Ok(CommandOutput::InsertAtCursor(format!("echo:{id}")))`. Manifest claims fence `echo-fixture`, command `echo.run`.
  - `panic`: both exports `panic!("fixture panic")`.
  - `hang`: both exports `loop {}`.

- [ ] **Step 3: build script** `scripts/build_plugins.sh`:

```bash
#!/bin/bash
# Build first-party plugins (default) or test fixtures (--fixtures).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET=wasm32-wasip2
if [ "${1:-}" = "--fixtures" ]; then
    OUT="$ROOT/tests/fixtures/plugins"
    CRATES="echo panic hang"
    BASE="$ROOT/plugins/fixtures"
else
    OUT="$ROOT/dist/plugins"
    CRATES="dot toc"
    BASE="$ROOT/plugins"
fi
rustup target add $TARGET 2>/dev/null || true
for name in $CRATES; do
    (cd "$BASE/$name" && cargo build --release --target $TARGET)
    mkdir -p "$OUT/$name"
    cp "$BASE/$name/target/$TARGET/release/"*.wasm "$OUT/$name/plugin.wasm"
    cp "$BASE/$name/plugin.toml" "$OUT/$name/plugin.toml"
done
echo "built: $CRATES -> $OUT"
```

(Each guest crate is its own workspace-excluded crate — add `plugins/`, `tests/fixtures/plugins/` to the root `.gitignore`? No: crates committed, built wasm gitignored. Add `[workspace]` empty table to each guest Cargo.toml so the root cargo doesn't try to build them for the host target.)

- [ ] **Step 4:** `bash scripts/build_plugins.sh --fixtures` succeeds locally (this validates the wit-bindgen + wasip2 toolchain end to end before any host code). Commit `feat: plugin template, fixtures, and build script`.

---

### Task 3: Wasmtime host

**Files:** Modify `src/extensions.rs`, `Cargo.toml` (wasmtime)

**Interfaces — Produces:**

```rust
pub struct ExtensionHost { /* engine, metas, failures, instances */ }
impl ExtensionHost {
    pub fn load(plugins_dir: &Path) -> Self;          // discover + engine setup
    pub fn plugins(&self) -> &[PluginMeta];
    pub fn failures(&self) -> &[(PathBuf, String)];
    pub fn render_block(&mut self, plugin: &str, lang: &str, source: &str,
        theme: &crate::diagram::DiagramTheme) -> Result<String, String>;
    pub fn run_command(&mut self, plugin: &str, id: &str,
        document: &str, sel: std::ops::Range<usize>) -> Result<CommandOutput, String>;
}
pub enum CommandOutput { ReplaceDocument(String), ReplaceSelection(String), InsertAtCursor(String) }
```

Global wrapper: `ExtensionState(Arc<Mutex<ExtensionHost>>)` as a gpui Global (calls happen on the background executor holding the mutex briefly).

- [ ] **Step 1:** `cargo add wasmtime@48 --no-default-features --features runtime,component-model,cranelift,parallel-compilation` (trim to the set that compiles; record final).

- [ ] **Step 2: Failing tests** (skip pattern shown once, used by all):

```rust
fn fixtures_dir() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
    d.join("echo/plugin.wasm").exists().then_some(d)
}

#[test]
fn echo_renderer_roundtrips() {
    let Some(dir) = fixtures_dir() else {
        eprintln!("SKIP: fixtures not built (scripts/build_plugins.sh --fixtures)");
        return;
    };
    let mut host = ExtensionHost::load(&dir);
    assert!(host.failures().is_empty(), "{:?}", host.failures());
    let svg = host
        .render_block("echo", "echo-fixture", "hello", &crate::diagram::DiagramTheme::default_light())
        .unwrap();
    assert!(svg.contains("echo-fixture:hello"));
}

#[test]
fn panicking_plugin_returns_err_and_recovers() {
    let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
    let mut host = ExtensionHost::load(&dir);
    let e = host.render_block("panic", "x", "y", &crate::diagram::DiagramTheme::default_light());
    assert!(e.is_err());
    // host still works for other plugins afterward
    assert!(host.render_block("echo", "l", "s", &crate::diagram::DiagramTheme::default_light()).is_ok());
}

#[test]
fn hanging_plugin_hits_deadline() {
    let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
    let mut host = ExtensionHost::load(&dir);
    let t0 = std::time::Instant::now();
    let e = host.render_block("hang", "x", "y", &crate::diagram::DiagramTheme::default_light());
    assert!(e.is_err());
    assert!(t0.elapsed() < std::time::Duration::from_secs(10));
}
```

- [ ] **Step 3:** RED → implement: `bindgen!({ path: "plugins/wit/extension.wit", world: "extension" })` host side; `Engine` with `epoch_interruption(true)`; a `std::thread` ticking `engine.increment_epoch()` every 500ms; each call sets `store.set_epoch_deadline(4)` (≈2s); instantiate lazily per plugin, drop + reinstantiate after trap. Theme conversion `DiagramTheme → wit Theme`.

- [ ] **Step 4:** Green (with fixtures built), full suite, commit `feat: wasmtime extension host with epoch deadlines`.

---

### Task 4: Plugin block renderers through the registry

**Files:** Modify `src/editor/projector.rs` (PluginBlockProjector), `src/diagram.rs` (cache key + plugin render path), `src/main.rs` (init ExtensionState global from `settings::config_dir().join("plugins")`)

**Interfaces:**
- Consumes: `ExtensionState`, `fence_infos`, diagram cache.
- Produces: fences claimed by plugin manifests render as widgets; `diagram::plugin_diagram_state(plugin, lang, source, width, cx) -> DiagramState` mirroring `diagram_state` but calling the host for SVG (same cache, key's source_hash folds in `plugin/name@version`).

- [ ] **Step 1: Failing test** (discovery, pure): `PluginBlockProjector::discover` with an injected claim table:

```rust
#[test]
fn plugin_fences_claimed_by_manifest_table() {
    let src = "```echo-fixture\nhi\n```\n";
    let lines = lines_of(src);
    let blocks = crate::editor::blocks::blocks(src);
    let table = vec![("echo".to_string(), vec!["echo-fixture".to_string()])];
    let claims = plugin_claims(src, &blocks, &lines, &table);
    assert_eq!(claims.len(), 1);
    let p = claims[0].payload.downcast_ref::<PluginBlockPayload>().unwrap();
    assert_eq!(p.plugin, "echo");
}
```

(`plugin_claims` is a pure helper the projector calls with the table read from `ExtensionState`; first plugin claiming a lang wins, mermaid excluded — Diagram projector runs earlier in the registry anyway.)

- [ ] **Step 2:** RED → implement; `render` mirrors DiagramProjector (Ready/Pending/Failed states) via `plugin_diagram_state`. Registry order: [Table, Image, Diagram, PluginBlock].

- [ ] **Step 3:** The projector needs the claim table at discover time (no cx in discover): store a snapshot of `(plugin, fences)` in a `OnceLock`/global refreshed at ExtensionState init — `extensions::fence_table() -> Vec<(String, Vec<String>)>`.

- [ ] **Step 4:** Green, full suite, manual smoke deferred to Task 7 (needs the dot plugin). Commit `feat: plugin block renderers via projector registry`.

---

### Task 5: Command palette (⌘⇧P)

**Files:** Create `src/palette.rs` (entity, finder-family); modify `src/workspace.rs` (wiring, ☰/menu/SHORTCUTS), `src/main.rs` (bindings), `src/editor/core.rs` only if a one-shot grouped-edit helper is missing

**Interfaces:**
- Consumes: `ExtensionState` (plugins + run_command), nucleo `score_candidates` pattern, `EditorCore::replace_range`/undo grouping.
- Produces: `Palette` entity emitting `PaletteEvent::Run { plugin: String, id: String } | Dismissed`; workspace action `TogglePalette` on `cmd-shift-p`; `Workspace::apply_command_output(output, window, cx)`.

- [ ] **Step 1: Failing test** (pure filter): reuse finder's `score_candidates` over command titles:

```rust
#[test]
fn palette_filters_by_title() {
    let cmds = vec!["Insert Table of Contents".to_string(), "About Dot".to_string()];
    let (order, _) = crate::finder::score_candidates("toc", &cmds);
    assert_eq!(order, vec![0]);
}
```

(If already covered by finder tests, assert via a thin `palette::filter` wrapper so the palette's contract has its own test.)

- [ ] **Step 2:** Implement `Palette`: input + uniform_list of `(plugin, CommandInfo)` rows (title strong, plugin name muted right), failures from `host.failures()` as dimmed unclickable rows; Enter/click → emit Run; Escape → Dismissed. Key context "Palette" with up/down/enter/escape bindings (mirror Finder's).

- [ ] **Step 3:** Workspace wiring (finder overlay pattern): on Run → dismiss → if active tab is an editable Editor in `Edit` view: spawn background `run_command` with `(document, selection)` snapshot; on Ok apply via `apply_command_output` (break_undo_group before and after; ReplaceDocument = replace 0..len; ReplaceSelection = replace sel; InsertAtCursor = insert at head), then notify; on Err show a 3s transient strip above the content (reuse install-banner styling, auto-clear via spawn timer). Read-only views: strip saying "Commands need an editable tab".

- [ ] **Step 4:** Bindings (`cmd-shift-p` via platform::keybinding), View menu + ☰ + SHORTCUTS rows, plus "Open Plugins Folder" item (File menu + ☰) that `create_dir_all` + `cx.open_url(file://…)`? — open via `open`/`xdg-open`/`explorer` per platform: add `platform::reveal_dir(path)` shelling to the right tool.

- [ ] **Step 5:** Green, full suite, commit `feat: command palette with plugin commands`.

---

### Task 6: toc plugin

**Files:** Create `plugins/toc/` (copy of template + logic)

- [ ] **Step 1:** Pure logic *inside the guest crate* with normal tests (runs on host target via `cargo test` in that crate):

```rust
pub fn build_toc(document: &str) -> String {
    // ATX headings only; skip fenced code blocks; indent by level-1;
    // links use GitHub-style slugs (lowercase, alnum + hyphens).
}
pub fn update_between_markers(document: &str, toc: &str) -> Result<String, String> {
    // replace content between <!-- toc --> and <!-- /toc -->,
    // Err with guidance when markers absent/mismatched.
}
#[test] fn toc_skips_fences_and_indents() { /* heading in fence ignored; ## indents */ }
#[test] fn update_replaces_marker_content() { /* roundtrip */ }
#[test] fn update_without_markers_errs() { /* guidance text */ }
```

- [ ] **Step 2:** Wire exports: `run_command("toc.insert")` → InsertAtCursor(build_toc(doc)); `"toc.update"` → ReplaceDocument(update_between_markers?); manifest with both commands. `cargo test` green in the crate; `build_plugins.sh` builds it.

- [ ] **Step 3:** Manual smoke: drop into `~/.supermd/plugins/`, run from palette. Commit `feat: toc plugin (first-party)`.

---

### Task 7: dot plugin

**Files:** Create `plugins/dot/`

- [ ] **Step 1:** Guest crate with `layout-rs = "0.1.3"`; `render_block`: parse via layout-rs's dot parser → SVG writer; then theme post-process (string-replace layout-rs's default `fill="#…"`/`stroke` colors: white→theme.background, black→theme.text/border strokes, honoring `dark`). Non-`dot` lang → Err. Pure test in-crate:

```rust
#[test]
fn digraph_renders_with_labels() {
    let svg = render("digraph { a -> b; a [label=\"Start\"]; }").unwrap();
    assert!(svg.contains("Start"));
}
```

(If layout-rs fails to compile for wasm32-wasip2, fallback recorded in spec: vendor its layout core. Verify FIRST with a `cargo check --target wasm32-wasip2` before writing logic.)

- [ ] **Step 2:** Manifest claims `["dot", "graphviz"]`. Build, drop into plugins dir, smoke a ` ```dot ` fence end to end (screenshot). Commit `feat: dot plugin (first-party)`.

---

### Task 8: Release wiring + docs

**Files:** Modify `.github/workflows/release.yml` (linux job builds plugins → `supermd-plugins.zip` artifact; ci.yml linux job adds `bash scripts/build_plugins.sh --fixtures` before `cargo test`), `README.md`, `docs/HISTORY.md`, `plugins/template/README.md` (final author walkthrough)

- [ ] **Step 1:** CI: linux test job gains fixture build (fixtures then exercise the host tests in CI); release linux job zips `dist/plugins` → upload as `supermd-plugins.zip`; publish job includes it.
- [ ] **Step 2:** README: "Extensions" section (what plugins are, install = drop folder into `~/.supermd/plugins/`, author = template link, capability promise). HISTORY row. Template README final pass (build command, install path, troubleshooting link errors).
- [ ] **Step 3:** Full suite + fixtures + release build + combined smoke (dot fence renders; toc via palette; panic fixture shows error strip not crash). Push, CI green on all three OSes. Offer v0.0.7.
