# Extensions Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Inline renderers, declarative decorators, formatters, paste processors, the `workspace-read` capability with consent, Reload Plugins, and three first-party plugins (emoji, tidy, todo-marks).

**Architecture:** WIT 0.2 world beside 0.1 with a dual-binding host (per-plugin fallback). Inline renderers ride the existing display-transform: cache hits become `StyleKind::InlineReplace(String)` spans → `Action::Replace` directives (Action gains owned text), so the reveal rule is inherited; misses stay raw and resolve via a background drainer. Decorators are host-side regex → theme-token overlays in `line_attrs` (no wasm). Formatters/paste ride the Phase-1 command apply path. `workspace-read` = read-only WASI preopen, consent banner, grants persisted in settings.

**Tech Stack:** wasmtime 48 dual `bindgen!`, `regex` crate (host-side patterns), wasmtime-wasi preopens, existing InlineCache-style global caching.

**Spec:** `docs/superpowers/specs/2026-08-24-extensions-phase2-design.md`

## Global Constraints

- Phase 1 (0.1) plugins keep working unchanged — proven by leaving the panic/hang fixtures on 0.1.
- The keystroke path (restyle/line_attrs/display) never calls wasm; only caches.
- Plugin failures are data (inherited contract); saving never blocks on a formatter.
- Branch: `extensions-phase1` (stacked on Phase 1, unmerged).
- TDD; full suite green + commit per task.

---

### Task 1: WIT 0.2 + manifest additions + dual-binding host

**Files:** Create `plugins/wit-v2/extension.wit` (0.2 world = 0.1 exports + render-inline/format-document/process-paste); modify `src/extensions.rs`, `plugins/template/*` (bump to 0.2 with stubs)

**Interfaces — Produces:**

```rust
// manifest additions on PluginMeta:
pub struct InlineRule { pub id: String, pub pattern: String }
pub struct DecorationRule { pub pattern: String, pub style: String } // accent|muted|strong|highlight
pub struct PluginMeta { …existing…,
    pub inline: Vec<InlineRule>, pub decorations: Vec<DecorationRule>,
    pub formats: bool, pub paste: bool, pub capabilities: Vec<String> }
// host additions:
impl ExtensionHost {
    pub fn render_inline(&mut self, plugin: &str, pattern_id: &str, matched: &str) -> Result<String, String>;
    pub fn format_document(&mut self, plugin: &str, document: &str) -> Result<String, String>;
    pub fn process_paste(&mut self, plugin: &str, text: &str) -> Result<Option<String>, String>;
}
// compiled decoration/inline tables (host-side regex, built at load;
// invalid regex → per-plugin load failure):
pub struct CompiledDecoration { pub regex: regex::Regex, pub style: String }
pub fn decoration_table() -> …snapshot like fence_table…
pub struct CompiledInline { pub plugin: String, pub id: String, pub regex: regex::Regex }
pub fn inline_table() -> …snapshot…
```

- [ ] **Step 1: Failing manifest tests** — inline/decorations/formats/paste/capabilities parse; `capabilities=["net"]` still rejected, `["workspace-read"]` accepted; invalid decoration regex → `discover` failure for that plugin.
- [ ] **Step 2:** RED → extend `ManifestFile` + `parse_manifest` (+ regex validation), `cargo add regex` (already transitively present via grep-regex — add direct).
- [ ] **Step 3: Dual binding** — second `bindgen!` in a `v2` module (path `plugins/wit-v2/extension.wit`); `LoadedPlugin.instance` becomes `enum Bound { V1(Store, Extension), V2(Store, v2::Extension) }`; instantiation tries V2 first, falls back to V1; V1 plugins return Err("requires a 0.2 plugin") from the three new host methods. Template's WIT path switches to `../wit-v2`, stubs for the three new exports.
- [ ] **Step 4: Fixture updates** — `echo` upgraded to 0.2: render-inline returns `Ok(format!("[{matched}]"))`, format-document uppercases, process-paste reverses `Some`; panic/hang stay 0.1 (compat proof). Host tests: echo's three new calls round-trip; panic (a 0.1 plugin) still errors cleanly on render_block AND on render_inline ("requires 0.2").
- [ ] **Step 5:** `build_plugins.sh --fixtures`, suite green, commit `feat: WIT 0.2 world with dual-binding host`.

---

### Task 2: Declarative decorators

**Files:** Modify `src/editor/mod.rs` (`line_attrs` overlay), `src/theme.rs` only if a token helper is cleaner inline

- [ ] **Step 1: Failing test** (pure, in editor tests): a `decoration_overlay(line_text, line_range, table, t) -> Vec<(Range<usize>, Hsla)>` helper:

```rust
#[test]
fn decorations_match_and_map_styles() {
    let table = vec![CompiledDecoration {
        regex: regex::Regex::new(r"\b(TODO|FIXME)\b").unwrap(),
        style: "accent".into(),
    }];
    let t = Theme::light();
    let hits = decoration_overlay("a TODO here", 100..111, &table, &t);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, 102..106); // absolute byte range
    assert_eq!(hits[0].1, t.accent);
}
```

- [ ] **Step 2:** RED → implement (style tokens: accent→t.accent, muted→t.fg_muted, strong→t.fg_strong, highlight→t.find_match_bg-as-color; unknown token → skipped). Wire into `line_attrs` after style spans, before find matches, skipping when `line_kinds` says Code and when inside fence spans; color applies to `a.color` (highlight applies to `a.bg`).
- [ ] **Step 3:** Suite, commit `feat: declarative decoration overlays`.

---

### Task 3: Inline renderers

**Files:** Modify `src/editor/display.rs` (Action owned text + InlineReplace arm), `src/editor/spans.rs` (StyleKind::InlineReplace + inline pass), `src/extensions.rs` (InlineCache + drainer), `src/editor/mod.rs` (restyle integration)

**Interfaces — Produces:**
`StyleKind::InlineReplace(String)`; `extensions::InlineCache` global (`get(plugin,id,matched) -> Option<Result<String,()>>`, `enqueue(…)`); `extensions::drain_inline_queue(cx)` background task started at app init; `spans::inline_pass(text, spans, table, lookup) -> Vec<StyleSpan>` (pure — lookup injected as `&dyn Fn(&str,&str,&str) -> Option<String>`).

- [ ] **Step 1: display.rs refactor** — `Action::Replace { text: &'static str, … }` → `text: String` (mechanical; existing arms `.to_string()`); add `StyleKind::InlineReplace(s)` arm producing `Action::Replace { text: s.clone(), toggle: None }` when fully on-line. Existing display tests updated only where the literal type changed. New test: InlineReplace span hides raw + shows replacement; cursor inside reveals raw (reuse the checkbox test pattern at display.rs:629-651).
- [ ] **Step 2: spans pass** (failing test first):

```rust
#[test]
fn inline_pass_replaces_hits_skips_code_and_misses() {
    let text = "say :tada: not `:tada:` ok\n";
    let spans = markdown_spans(text);   // gives InlineCode span for the backticks
    let table = vec![CompiledInline { plugin: "emoji".into(), id: "e".into(),
        regex: regex::Regex::new(r":([a-z]+):").unwrap() }];
    let hit = |_: &str, _: &str, m: &str| (m == ":tada:").then(|| "🎉".to_string());
    let extra = inline_pass(text, &spans, &table, &hit);
    assert_eq!(extra.len(), 1, "code span excluded, only the first match");
    assert!(matches!(&extra[0].kind, StyleKind::InlineReplace(s) if s == "🎉"));
    assert_eq!(extra[0].range, 4..10);
}
```

Misses (lookup None) produce nothing but are reported: `inline_pass` also returns misses `Vec<(String,String,String)>` — adjust signature to `-> (Vec<StyleSpan>, Vec<Miss>)`.
- [ ] **Step 3:** RED → implement; exclusion = overlap with InlineCode/FenceContent/FenceDelimiter spans.
- [ ] **Step 4: Cache + drainer** — `InlineCache` (HashMap + LRU cap 4096, like DiagramCache; stores `Ok(replacement)`/`Err(())` for permanent failures); `restyle` calls `inline_pass` with a cache-lookup closure and hands misses to `extensions::enqueue_inline(misses)`; a startup `cx.spawn` loop drains the queue every 100 ms through the host on the background executor, fills the cache, and pokes open editors (`cx.refresh_windows()` + editors restyle on notify — reuse the diagram completion pattern; restyle trigger: mark editors dirty via a global generation counter checked in render → simplest: `cx.refresh_windows()` plus editors compare an `InlineCache::generation()` snapshot in `reproject`, restyling when it advanced).
- [ ] **Step 5:** Suite, commit `feat: inline renderers with cache-only keystroke path`.

---

### Task 4: Formatters + paste processors

**Files:** Modify `src/workspace.rs` (palette entries + format command flow), `src/editor/mod.rs` (paste hook, flush hook), `src/settings.rs` (`format_on_save: bool` default false)

- [ ] **Step 1:** Settings key (test: default false, round-trip).
- [ ] **Step 2: Palette** — palette entry list gains synthetic entries `Format: <plugin>` (`id="__format"`) for `formats=true` plugins; `run_plugin_command` special-cases `__format`: background `format_document(document)` with generation check (snapshot `editor.read(cx).command_snapshot().0`; before applying compare current text — mismatch → strip "document changed while formatting; run again").
- [ ] **Step 3: Paste hook** — in `Editor::paste`, before `insert_str`: iterate `extensions::paste_plugins()` snapshot (load-order names) — synchronous host call `process_paste`; first `Ok(Some(t))` replaces the text; `Err`/`None` continue. (Snapshot fn added beside fence_table.)
- [ ] **Step 4: Format-on-save** — in `flush`, when `format_on_save` and a formats plugin exists: same background format + generation-checked apply, then write; any failure → save original immediately (never block). Test at the pure level: generation-check helper `fn apply_if_unchanged(snapshot: &str, current: &str, formatted: String) -> Option<String>`.
- [ ] **Step 5:** Suite, commit `feat: formatters and paste processors`.

---

### Task 5: workspace-read + consent

**Files:** Modify `src/extensions.rs` (grants, preopen ctx, pending-consent), `src/settings.rs` (`plugin_grants: BTreeMap<String, Vec<String>>`), `src/workspace.rs` (consent banner), `plugins/fixtures/reader/` (new)

- [ ] **Step 1:** Settings grants map (test: round-trip, default empty).
- [ ] **Step 2: Reader fixture** — 0.2 plugin, `format_document` ignores input and returns `std::fs::read_to_string("/workspace/probe.txt")` result (stringified). Manifest: `capabilities=["workspace-read"]`, `formats=true`.
- [ ] **Step 3: Host** — `ExtensionHost::load(plugins_dir)` gains `set_workspace_root(Option<PathBuf>)` + `set_grants(BTreeMap…)`; instantiation: if plugin declares workspace-read AND granted AND root set → WasiCtx with `preopened_dir(root, "/workspace", DirPerms::READ, FilePerms::READ)`; declared-but-ungranted → instantiation succeeds with zero-grant ctx and calls return `Err("awaiting consent for workspace-read")` *before* touching wasm (host-side check). Failing tests:

```rust
#[test] fn reader_denied_without_grant() { /* Err contains "consent" */ }
#[test] fn reader_reads_probe_with_grant() { /* grant + root with probe.txt → Ok contains file body */ }
#[test] fn reader_cannot_escape_preopen() { /* fixture variant path "../outside.txt" → error string, not content */ }
```

(The escape test uses a second command id in the reader fixture reading `/workspace/../outside.txt`.)
- [ ] **Step 4: Consent UX** — workspace: when a command/format run returns the awaiting-consent error, show a persistent banner (install-banner styling): "Plugin <name> wants to read files in this workspace — [Allow] [Deny]"; Allow → persist grant in settings, call `ExtensionState.reload_grants(...)` (drops instances so the next call gets the preopen), re-run nothing (user retries); Deny → persist `"denied:workspace-read"`, banner never returns.
- [ ] **Step 5:** Suite + fixtures, commit `feat: workspace-read capability with consent`.

---

### Task 6: Reload Plugins

**Files:** Modify `src/extensions.rs` (tables → `RwLock`, `reload()`), `src/workspace.rs` + `src/main.rs` (command + menu rows)

- [ ] **Step 1:** Tables (`FENCE_TABLE` etc.) become `RwLock<Vec<…>>` with `set_*` overwriting; `pub fn reload(plugins_dir, workspace_root, grants) -> ExtensionHost` composing load + table refresh; caches cleared (`DiagramCache`/`InlineCache` clear methods). Test: host-level — drop a new fixture dir into a tempdir after first load, reload, assert the new plugin serves.
- [ ] **Step 2:** Workspace command "Reload Plugins" (☰ + File menu + palette synthetic entry `__reload`): swaps the global, restyles open editors, shows a 3s strip "Plugins reloaded: N".
- [ ] **Step 3:** Suite, commit `feat: reload plugins without restart`.

---

### Task 7: First-party plugins + docs

**Files:** Create `plugins/emoji/`, `plugins/tidy/`, `plugins/todo-marks/`, `scripts/gen_emoji_table.py`; modify `scripts/build_plugins.sh` (CRATES="dot toc emoji tidy"; todo-marks copies template wasm), README, HISTORY

- [ ] **Step 1: emoji** — `gen_emoji_table.py` downloads the gemoji JSON once and emits `src/table.rs` (`pub static TABLE: &[(&str, &str)]`, committed); crate: `render_inline("e", ":tada:")` → table lookup; manifest `[[inline]] id="e" pattern=":([a-z0-9_+-]+):"`. In-crate tests: known shortcode, unknown → Err (stays raw).
- [ ] **Step 2: tidy** — formatter: smart quotes/dashes outside code fences, collapse 3+ blank lines, trim trailing spaces; paste: TSV/CSV detection (≥2 lines, consistent column count ≥2) → markdown table, else None. In-crate tests for each rule + CSV edge (quoted commas: keep simple — split on tab first, comma only when no tabs; quoted-comma CSV returns None honestly).
- [ ] **Step 3: todo-marks** — template stub wasm + manifest-only `[[decorations]]` (TODO/FIXME/NOTE → accent). Build script handles it (own crate copy of template).
- [ ] **Step 4:** Install all locally, combined smoke: `:tada:` renders 🎉 and reveals raw on cursor; TODO glows accent; Format: tidy fixes quotes; paste CSV becomes a table; consent banner on a reader-fixture call; Reload picks up a dropped plugin. Screenshot for the record.
- [ ] **Step 5:** README extensions section grows the new surfaces; HISTORY row; suite; commit `feat: emoji, tidy, todo-marks plugins`; push branch.
