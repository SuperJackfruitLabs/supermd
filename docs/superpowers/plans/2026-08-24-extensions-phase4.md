# Extensions Phase 4 Implementation Plan — Grammar Plugins

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Grammar plugins (tree-sitter wasm + highlights.scm) highlight fences and standalone files with zero core recompile; GraphQL ships as the first-party proof.

**Architecture:** tree-sitter 0.23's `wasm` feature (already enabled in Cargo.toml — the feasibility spike is done) provides `WasmStore` for loading grammar `.wasm` at runtime inside tree-sitter's own sandbox. A grammar registry (RwLock global in `highlight.rs`, rebuilt by `refresh_tables`) sits between the built-in extras and inkjet. The GraphQL artifact is built once with the CLI+emcc and committed.

**Tech Stack:** tree-sitter 0.23.2 (`wasm` feature → wasmtime-c-api-impl 24 beside the host's wasmtime 48; needs `cmake` at build time — present on all GitHub CI runners), tree-sitter-cli 0.23.2 + emscripten (one-time artifact build, dev machine only), grammar source: bkegley/tree-sitter-graphql via npm.

**Spec:** `docs/superpowers/specs/2026-08-24-extensions-phase4-design.md`

## Global Constraints

- Zero core recompile to add a grammar: drop dir + Reload Plugins.
- Built-in languages always win name collisions with plugin grammars.
- Grammar failures are data: load failures → load report; parse-time trouble → plain text + eprintln, never a crash.
- The committed `grammar.wasm` must be ABI ≤ 14 (regenerate parser.c with CLI 0.23.2 before building wasm).
- Grammar plugins need no `plugin.wasm`; each `[[grammars]]` entry requires its wasm+scm pair.
- The wasm feature stays: `tree-sitter = { version = "0.23", features = ["wasm"] }` (already in Cargo.toml).
- Tests run against the committed artifact on all OSes — no fixture build step, no emcc in CI.

## Verified API facts (from the spike)

- `tree_sitter::wasmtime::Engine` is re-exported (wasmtime 24); `WasmStore::new(&engine)`, `WasmStore::load_language(&mut self, name, &[u8]) -> Result<Language, WasmError>`.
- `Parser::set_wasm_store(&mut self, WasmStore)` moves the store in; `Parser::take_wasm_store(&mut self) -> Option<WasmStore>` gets it back.
- `tree_sitter_highlight::Highlighter::parser(&mut self) -> &mut Parser` (inkjet re-exports the crate as `inkjet::tree_sitter_highlight`).
- `HighlightConfiguration::new(language, name, highlights, "", "")` then `.configure(CAPTURE_NAMES)` — same as the extras path.

---

### Task 1: GraphQL artifact + build script

Artifact creation (generated code — TDD exception; its proof is Task 3's load test). Toolchain already installed: emcc 6.0.8 (`/opt/homebrew/bin/emcc`), tree-sitter-cli 0.23.2.

**Files:**
- Create: `scripts/build_grammar_wasm.sh`
- Create: `plugins/graphql/plugin.toml`
- Create: `plugins/graphql/grammar.wasm` (built, committed)
- Create: `plugins/graphql/highlights.scm` (from the grammar repo, with license header)

- [ ] **Step 1: The script**

```bash
#!/bin/bash
# Build a tree-sitter grammar to wasm for SuperMD grammar plugins.
# One-time developer tool — the built artifact is committed; users and
# CI never need this. Requires: tree-sitter-cli 0.23.x (ABI 14) and
# emscripten (emcc) on PATH.
#   scripts/build_grammar_wasm.sh <grammar-src-dir> <out-dir>
# Regenerates parser.c at the CLI's ABI, then builds the wasm module.
set -euo pipefail
SRC="$1"; OUT="$2"
(cd "$SRC" && tree-sitter generate && tree-sitter build --wasm -o "$OUT/grammar.wasm")
echo "built: $OUT/grammar.wasm"
```

- [ ] **Step 2: Fetch grammar source and build**

```bash
cd <scratchpad>
npm pack tree-sitter-graphql@1.0.0 && tar xzf tree-sitter-graphql-1.0.0.tgz
mkdir -p <repo>/plugins/graphql
bash <repo>/scripts/build_grammar_wasm.sh package <repo>/plugins/graphql
```

(If `tree-sitter generate` balks at the old grammar.js, fall back to building from the shipped `src/parser.c` — but then verify the ABI: `tree-sitter build --wasm` compiles whatever parser.c is present; a "language version" load error in Task 3 means regeneration is mandatory. If emcc's first run wants a config, run `emcc --generate-config` once.)

- [ ] **Step 3: highlights.scm** — fetch `queries/highlights.scm` from the bkegley/tree-sitter-graphql GitHub repo (the npm tarball has no queries dir). Prepend a comment header naming the source repo and its license. If the repo has no highlights.scm, write a minimal one by hand (~20 lines: types, fields, keywords, strings, comments — captures from `CAPTURE_NAMES`: `type`, `property`, `keyword`, `string`, `comment`, `constant`).

- [ ] **Step 4: Manifest**

```toml
name = "graphql"
version = "0.1.0"

[[grammars]]
name = "graphql"
extensions = ["graphql", "gql"]
```

- [ ] **Step 5: Sanity + commit** — `file plugins/graphql/grammar.wasm` shows a WebAssembly binary; `git add -f plugins/graphql/grammar.wasm` if any gitignore rule catches `.wasm` (check `.gitignore`); commit "feat: graphql grammar artifact and build script".

---

### Task 2: Manifest `[[grammars]]` + discovery relaxation

**Files:**
- Modify: `src/extensions.rs` (ManifestFile, PluginMeta, parse_manifest, discover, manifest_tests)

**Interfaces:**
- Produces: `pub struct GrammarInfo { pub name: String, pub extensions: Vec<String>, pub files: Option<String> }` and `PluginMeta.grammars: Vec<GrammarInfo>`; helper `pub fn grammar_paths(meta_dir: &Path, g: &GrammarInfo, count: usize) -> Result<(PathBuf, PathBuf), String>` returning (wasm, scm) paths (default names when `count == 1` and `files` is None; `files` stem required otherwise).

- [ ] **Step 1: Failing tests** (manifest_tests)

```rust
    #[test]
    fn grammars_parse_and_relax_wasm_requirement() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("plugin.toml"),
            "name=\"g\"\nversion=\"0\"\n[[grammars]]\nname=\"graphql\"\nextensions=[\"graphql\",\"gql\"]\n",
        )
        .unwrap();
        std::fs::write(p.join("grammar.wasm"), b"\0asm").unwrap();
        std::fs::write(p.join("highlights.scm"), "(comment) @comment").unwrap();
        let (ok, fail) = discover(dir.path());
        assert_eq!(fail.len(), 0, "{fail:?}");
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].grammars[0].name, "graphql");
        assert_eq!(ok[0].grammars[0].extensions, ["graphql", "gql"]);
    }

    #[test]
    fn grammar_missing_files_fails_discover() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("plugin.toml"),
            "name=\"g\"\nversion=\"0\"\n[[grammars]]\nname=\"x\"\nextensions=[\"x\"]\n",
        )
        .unwrap();
        // no grammar.wasm / highlights.scm
        let (ok, fail) = discover(dir.path());
        assert!(ok.is_empty());
        assert_eq!(fail.len(), 1);
        assert!(fail[0].1.contains("grammar"), "{}", fail[0].1);
    }

    #[test]
    fn multi_grammar_requires_files_stem() {
        let two = r#"
name = "g"
version = "0"
[[grammars]]
name = "a"
extensions = ["a"]
[[grammars]]
name = "b"
extensions = ["b"]
"#;
        let err = parse_manifest(Path::new("/p/g"), two).unwrap_err();
        assert!(err.contains("files"), "{err}");
    }
```

- [ ] **Step 2: Verify FAIL** — `cargo test manifest_tests` (unknown field `grammars`).

- [ ] **Step 3: Implement**

```rust
#[derive(Clone, Debug, serde::Deserialize)]
pub struct GrammarInfo {
    pub name: String,
    pub extensions: Vec<String>,
    /// Filename stem for this grammar's wasm/scm pair; required when a
    /// plugin ships more than one grammar.
    #[serde(default)]
    pub files: Option<String>,
}
```

- `ManifestFile` + `PluginMeta` gain `grammars: Vec<GrammarInfo>` (serde default; copied through in parse_manifest).
- In `parse_manifest`, after the capability loop: if `file.grammars.len() > 1 && file.grammars.iter().any(|g| g.files.is_none())` → `Err("plugins with multiple grammars must set files = \"<stem>\" on each")`.
- Path helper:

```rust
/// (wasm, scm) paths for a grammar declaration.
pub fn grammar_paths(dir: &Path, g: &GrammarInfo) -> (PathBuf, PathBuf) {
    match &g.files {
        Some(stem) => (dir.join(format!("{stem}.wasm")), dir.join(format!("{stem}.scm"))),
        None => (dir.join("grammar.wasm"), dir.join("highlights.scm")),
    }
}
```

- In `discover`'s per-dir closure, replace the flat `plugin.wasm` check:

```rust
            .and_then(|meta| {
                let needs_component = meta.grammars.is_empty()
                    || !meta.commands.is_empty()
                    || !meta.fences.is_empty()
                    || !meta.inline.is_empty()
                    || meta.formats
                    || meta.paste
                    || !meta.exports.is_empty();
                if needs_component && !dir.join("plugin.wasm").exists() {
                    return Err("plugin.wasm missing".to_string());
                }
                for g in &meta.grammars {
                    let (wasm, scm) = grammar_paths(&dir, g);
                    if !wasm.exists() || !scm.exists() {
                        return Err(format!(
                            "grammar `{}` needs {} and {}",
                            g.name,
                            wasm.file_name().unwrap().to_string_lossy(),
                            scm.file_name().unwrap().to_string_lossy(),
                        ));
                    }
                }
                Ok(meta)
            });
```

(Decorations-only plugins still require plugin.wasm today — the todo-marks template stub — keep that behavior by NOT listing decorations in `needs_component`… todo-marks DOES ship a stub wasm, so listing or not listing changes nothing for existing plugins; leave decorations out so a future manifest-only decorator + grammar plugin works.)

- [ ] **Step 4: GREEN + full suite** — `cargo test` all green (existing `discover_requires_wasm_file` still passes: its manifest has no grammars).

- [ ] **Step 5: Commit** — "feat: manifest [[grammars]] entries and grammar-aware discovery".

---

### Task 3: Grammar registry in the highlight layer

**Files:**
- Modify: `src/highlight.rs` (registry statics, loader, highlight() hook)
- Modify: `src/extensions.rs` (`refresh_tables` calls the loader; load failures merge into the host's failure list)

**Interfaces:**
- Produces (in `crate::highlight`):
  - `pub fn load_plugin_grammars(specs: &[(String, PathBuf, GrammarInfo)]) -> Vec<(String, String)>` — (plugin name, plugin dir, grammar decl) in; (plugin name, error) out for failures. Loads each wasm+scm into the registry, replacing the whole registry contents (reload semantics).
  - `pub fn plugin_grammar_for_extension(ext: &str) -> Option<String>`
  - `pub(crate) fn plugin_highlight(name: &str, code: &str) -> Option<Vec<(Range<usize>, u8)>>` — None when the registry has no such grammar.
- Consumes: `GrammarInfo`, `grammar_paths` from Task 2.

- [ ] **Step 1: Failing tests** (new `mod grammar_tests` in highlight.rs; they use the committed artifact and run everywhere)

```rust
#[cfg(test)]
mod grammar_tests {
    use super::*;
    use std::path::PathBuf;

    fn graphql_spec() -> Option<(String, PathBuf, crate::extensions::GrammarInfo)> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/graphql");
        dir.join("grammar.wasm").exists().then(|| {
            (
                "graphql".to_string(),
                dir,
                crate::extensions::GrammarInfo {
                    name: "graphql".into(),
                    extensions: vec!["graphql".into(), "gql".into()],
                    files: None,
                },
            )
        })
    }

    #[test]
    fn graphql_plugin_grammar_highlights() {
        let Some(spec) = graphql_spec() else { panic!("graphql artifact missing — run Task 1") };
        let failures = load_plugin_grammars(&[spec]);
        assert!(failures.is_empty(), "{failures:?}");
        let src = "type Query {\n  hero(episode: Episode): Character\n}\n";
        let spans = Languages::new().highlight("graphql", src);
        assert!(!spans.is_empty(), "no spans for graphql");
        assert_eq!(plugin_grammar_for_extension("gql"), Some("graphql".to_string()));
        assert_eq!(plugin_grammar_for_extension("nope"), None);
    }

    #[test]
    fn broken_query_reports_failure_not_panic() {
        let Some((plugin, dir, _)) = graphql_spec() else { panic!("artifact missing") };
        let tmp = tempfile::tempdir().unwrap();
        std::fs::copy(dir.join("grammar.wasm"), tmp.path().join("grammar.wasm")).unwrap();
        std::fs::write(tmp.path().join("highlights.scm"), "(nonexistent_node_xyz) @x").unwrap();
        let failures = load_plugin_grammars(&[(
            plugin,
            tmp.path().to_path_buf(),
            crate::extensions::GrammarInfo { name: "g2".into(), extensions: vec![], files: None },
        )]);
        assert_eq!(failures.len(), 1, "{failures:?}");
    }

    #[test]
    fn builtins_win_name_collisions() {
        let Some((plugin, dir, _)) = graphql_spec() else { panic!("artifact missing") };
        // register the graphql wasm under the name "rust"
        let failures = load_plugin_grammars(&[(
            plugin,
            dir,
            crate::extensions::GrammarInfo { name: "rust".into(), extensions: vec![], files: None },
        )]);
        assert!(failures.is_empty(), "{failures:?}");
        let spans = Languages::new().highlight("rust", "fn main() { let x = 1; }");
        // inkjet's rust grammar must still be the one answering: a real
        // rust span set includes the `fn` keyword capture; the graphql
        // grammar would produce nothing sensible (likely empty).
        assert!(!spans.is_empty());
        load_plugin_grammars(&[]); // reset for other tests
    }

    #[test]
    fn empty_registry_reload_clears() {
        let Some(spec) = graphql_spec() else { panic!("artifact missing") };
        assert!(load_plugin_grammars(&[spec]).is_empty());
        load_plugin_grammars(&[]);
        assert_eq!(plugin_grammar_for_extension("graphql"), None);
        assert!(Languages::new().highlight("graphql", "type Query { a: B }").is_empty());
    }
}
```

NOTE on test isolation: these tests mutate a process-global registry and run in one binary with threaded tests. Give every test a distinct grammar NAME where possible, and end registry-clearing tests with a reset. If cross-test flakiness appears, merge them into one `#[test]` running scenarios sequentially — correctness over granularity.

- [ ] **Step 2: Verify FAIL** — missing functions.

- [ ] **Step 3: Implement the registry**

In `highlight.rs`:

```rust
/// Plugin grammars: name → compiled highlight config. The WasmStore
/// must sit in the parser during a parse (the API moves it), so it
/// lives beside the registry and is taken/put around each highlight.
struct GrammarRegistry {
    engine: tree_sitter::wasmtime::Engine,
    store: Option<tree_sitter::WasmStore>,
    grammars: Vec<(String, HighlightConfiguration)>,
    extensions: Vec<(String, String)>, // extension → grammar name
}

static GRAMMARS: std::sync::Mutex<Option<GrammarRegistry>> = std::sync::Mutex::new(None);

pub fn load_plugin_grammars(
    specs: &[(String, std::path::PathBuf, crate::extensions::GrammarInfo)],
) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    let engine = tree_sitter::wasmtime::Engine::default();
    let mut store = match tree_sitter::WasmStore::new(&engine) {
        Ok(s) => s,
        Err(e) => {
            *GRAMMARS.lock().unwrap() = None;
            return specs.iter().map(|(p, ..)| (p.clone(), format!("wasm store: {e}"))).collect();
        }
    };
    let mut grammars = Vec::new();
    let mut extensions = Vec::new();
    for (plugin, dir, g) in specs {
        let (wasm_path, scm_path) = crate::extensions::grammar_paths(dir, g);
        let result = (|| -> Result<HighlightConfiguration, String> {
            let bytes = std::fs::read(&wasm_path).map_err(|e| e.to_string())?;
            let language =
                store.load_language(&g.name, &bytes).map_err(|e| format!("grammar wasm: {e}"))?;
            let query = std::fs::read_to_string(&scm_path).map_err(|e| e.to_string())?;
            let mut config = HighlightConfiguration::new(language, &g.name, &query, "", "")
                .map_err(|e| format!("highlights.scm: {e}"))?;
            config.configure(CAPTURE_NAMES);
            Ok(config)
        })();
        match result {
            Ok(config) => {
                grammars.push((g.name.clone(), config));
                for ext in &g.extensions {
                    extensions.push((ext.clone(), g.name.clone()));
                }
            }
            Err(e) => failures.push((plugin.clone(), format!("grammar `{}`: {e}", g.name))),
        }
    }
    *GRAMMARS.lock().unwrap() = if grammars.is_empty() && extensions.is_empty() {
        None
    } else {
        Some(GrammarRegistry { engine, store: Some(store), grammars, extensions })
    };
    failures
}

pub fn plugin_grammar_for_extension(ext: &str) -> Option<String> {
    let guard = GRAMMARS.lock().unwrap();
    let reg = guard.as_ref()?;
    reg.extensions.iter().find(|(e, _)| e == ext).map(|(_, n)| n.clone())
}

/// Highlight through a plugin grammar; None = not a plugin grammar.
fn plugin_highlight(name: &str, code: &str) -> Option<Vec<(Range<usize>, u8)>> {
    let mut guard = GRAMMARS.lock().unwrap();
    let reg = guard.as_mut()?;
    if !reg.grammars.iter().any(|(n, _)| n == name) {
        return None;
    }
    let store = reg.store.take()?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut highlighter = RawHighlighter::new();
        highlighter.parser().set_wasm_store(store).ok();
        let config = &reg.grammars.iter().find(|(n, _)| n == name).unwrap().1;
        let spans = highlighter
            .highlight(config, code.as_bytes(), None, |_| None)
            .map(|events| collect_spans(events.flatten()))
            .unwrap_or_default();
        (highlighter.parser().take_wasm_store(), spans)
    }));
    match result {
        Ok((store_back, spans)) => {
            reg.store = store_back;
            Some(spans)
        }
        Err(_) => {
            eprintln!("supermd: plugin grammar '{name}' failed; degrading to plain text");
            // store lost with the panicked parser; rebuild on next reload
            Some(Vec::new())
        }
    }
}
```

Hook into `Languages::highlight`, after the extras branch and BEFORE inkjet — with the collision rule enforced by checking inkjet first:

```rust
        // Plugin grammars answer only when no built-in claims the
        // name (built-ins win collisions).
        let builtin = Language::from_token(canonical).is_some()
            || self.extras.iter().any(|(n, _)| *n == canonical);
        if !builtin {
            if let Some(spans) = plugin_highlight(canonical, code) {
                return spans;
            }
        }
```

(Place this after the extras `if let` and before `Language::from_token(canonical)`'s `else { return Vec::new() }` — restructure minimally: compute `builtin` first; keep the existing flow otherwise. The `graphql` token currently maps into `Language::from_token`? NO — inkjet has no graphql; `from_token("graphql")` is None today, so plugin graphql wins. The collision test registers "rust", where `from_token` is Some → built-in wins.)

- [ ] **Step 4: Wire refresh_tables** (extensions.rs):

```rust
    let grammar_specs: Vec<(String, std::path::PathBuf, GrammarInfo)> = metas
        .iter()
        .flat_map(|m| m.grammars.iter().map(|g| (m.name.clone(), m.dir.clone(), g.clone())))
        .collect();
    let grammar_failures = crate::highlight::load_plugin_grammars(&grammar_specs);
```

`refresh_tables` currently takes `&ExtensionHost` — grammar failures should surface in the load report. Change `refresh_tables(host: &ExtensionHost)` to `refresh_tables(host: &mut ExtensionHost)` and push failures into `host.failures` (add a small `pub fn note_failure(&mut self, plugin: &str, error: String)` that pushes `(self.dir_of(plugin), error)` — or simpler: failures vec accepts `(PathBuf::from(plugin_name), error)`; the palette footer only shows dir file_name + message, so pushing `(PathBuf::from(plugin), error)` reads fine). Update the call sites (main.rs startup, reload_plugins in workspace.rs).

- [ ] **Step 5: GREEN** — `cargo test grammar_tests` then full suite.

- [ ] **Step 6: Commit** — "feat: plugin grammar registry in the highlight layer".

---

### Task 4: File resolution + editor provider widening

**Files:**
- Modify: `src/highlight.rs` (`language_for_file` → `Option<String>`, registry fallback; mapping_tests updated)
- Modify: `src/reader.rs` (`language_for_path` → `Option<String>`)
- Modify: `src/editor/mod.rs` (`Provider::Code(String)`, three touch points at lines ~192/255/319)

**Interfaces:**
- Produces: `pub fn language_for_file(path: &Path) -> Option<String>` — static table first, then `plugin_grammar_for_extension`.

- [ ] **Step 1: Failing test** (mapping_tests):

```rust
    #[test]
    fn plugin_grammar_resolves_files() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/graphql");
        assert!(dir.join("grammar.wasm").exists(), "run Task 1 first");
        super::load_plugin_grammars(&[(
            "graphql".into(),
            dir,
            crate::extensions::GrammarInfo {
                name: "graphql".into(),
                extensions: vec!["graphql".into(), "gql".into()],
                files: None,
            },
        )]);
        assert_eq!(f("schema.graphql"), Some("graphql".to_string()));
        assert_eq!(f("q.gql"), Some("graphql".to_string()));
        super::load_plugin_grammars(&[]);
    }
```

The `f` helper and every existing assertion change from `Some("x")` to `Some("x".to_string())` — mechanical (or keep `f` returning the Option<String> and compare with `.as_deref()`; pick whichever reads cleaner and apply consistently).

- [ ] **Step 2: Verify FAIL** (type mismatch / missing fallback).

- [ ] **Step 3: Implement**

- `language_for_file`: change signature to `Option<String>`; the static match arms wrap in `.to_string()` at the single return point:

```rust
pub fn language_for_file(path: &std::path::Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    match name { /* filename arms return Some("...".to_string()) */ _ => {} }
    let ext = path.extension()?.to_str()?;
    if let Some(known) = static_language_for_ext(ext) {
        return Some(known.to_string());
    }
    plugin_grammar_for_extension(ext)
}
```

Extract the big ext match into `fn static_language_for_ext(ext: &str) -> Option<&'static str>` (pure move, no edits to the arms).

- `reader::language_for_path` → `Option<String>`.
- `Provider::Code(&'static str)` → `Code(String)`; at line ~192 `Provider::Code(lang)` now receives String; at ~255/319 `spans::code_spans(&text, lang, langs)` — pass `&lang` / adjust to `lang.as_str()` per compiler.

- [ ] **Step 4: GREEN + full suite** (regressions: mapping_tests, reader/editor code-file tests).

- [ ] **Step 5: Commit** — "feat: plugin grammars resolve standalone files".

---

### Task 5: Reload wiring, dist packaging, docs, smoke test, wrap-up

**Files:**
- Modify: `scripts/build_plugins.sh` (copy `graphql` into dist by file copy — no cargo build)
- Modify: `plugins/template/README.md` (grammar plugin section)
- Modify: `src/main.rs` / `src/workspace.rs` (refresh_tables signature fallout from Task 3, if not already done there)

- [ ] **Step 1: dist packaging** — in the non-fixtures branch of `build_plugins.sh`, after the cargo loop:

```bash
# Grammar plugins ship committed artifacts — plain copy, no build.
for name in graphql; do
    mkdir -p "$OUT/$name"
    cp "$ROOT/plugins/$name/plugin.toml" "$ROOT/plugins/$name/grammar.wasm" \
       "$ROOT/plugins/$name/highlights.scm" "$OUT/$name/"
done
```

- [ ] **Step 2: Template README** — add a "Grammar plugins" section: dir layout (plugin.toml + grammar.wasm + highlights.scm, no plugin.wasm needed), the `[[grammars]]` manifest shape, the build_grammar_wasm.sh pointer, and the collision rule (built-ins win).

- [ ] **Step 3: Full suite + build** — `cargo test && cargo build`; `bash scripts/build_plugins.sh` shows graphql in dist.

- [ ] **Step 4: Smoke test** (macOS): copy dist graphql plugin into `~/.supermd/plugins/`, launch the dev build on a folder containing a `schema.graphql` (write one in the scratchpad workspace), screenshot: the file renders highlighted; a ```graphql fence in a markdown doc highlights. Kill stale supermd instances FIRST (`pkill -f supermd`) — earlier smoke tests hit a stale-instance trap.

- [ ] **Step 5: Commit + push** — "feat: graphql grammar plugin ships in dist; docs"; push branch. CI note: no workflow change needed (cmake preinstalled on all runners; tests use the committed artifact). Watch the next CI run on the eventual PR for the windows wasmtime-c-api-24 build in particular.

## Self-Review Notes

- Spec coverage: manifest/discovery ✔ (Task 2), registry + resolution order + collision ✔ (Task 3), file resolution + fences ✔ (Tasks 3–4 — fences resolve through `highlight(lang)` which hits the registry), artifact + script ✔ (Task 1), reload ✔ (Task 3 Step 4 + registry replace semantics tested), error handling ✔ (broken-scm test, catch_unwind in plugin_highlight), dist/docs ✔ (Task 5). Exit criterion "zero core recompile" is literal: Tasks 2–4 build the loader once; the graphql plugin itself is data.
- Registry global mutability across tests: flagged with a mitigation (distinct names, reset calls, merge-if-flaky).
- Compiler-guided points: exact placement of the `builtin` check in `highlight()`, `set_wasm_store` error type, `code_spans` borrow forms — all small and named inline.
- Store-loss-after-panic is accepted (degrades to plain text until next reload) and logged.
