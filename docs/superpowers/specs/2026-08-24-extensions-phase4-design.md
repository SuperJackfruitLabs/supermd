# Extensions Phase 4 Design — Grammar Plugins (C1)

**Date:** 2026-08-24
**Status:** Approved for planning
**Program:** `2026-08-24-extensions-roadmap.md` (Phase 4 of 5)
**Branch:** continues `extensions` (Phases 1–3 unmerged; this phase
builds on their manifest/discovery machinery but NOT on the component
runtime — grammar wasm is a different kind of module)

## Purpose

Load tree-sitter grammars as plugins with **zero core recompile**: a
grammar plugin highlights fenced code blocks and standalone files the
moment it lands in `~/.supermd/plugins/`. GraphQL ships as the
first-party proof, closing the oldest open item in the codebase (the
"GraphQL blocked: ABI 15 vs inkjet's runtime" note in `highlight.rs`).

## Mechanism — tree-sitter's own wasm loading (Zed model)

Enable the `wasm` feature on our existing `tree-sitter = "0.23"`
dependency. It provides:

- `WasmStore::new(engine)` — a sandboxed store for grammar modules.
- `WasmStore::load_language(name, bytes) -> Language` — turns a
  grammar `.wasm` (built by the tree-sitter CLI) into a runtime
  `Language`.
- `Parser::set_wasm_store(store)` / `take_wasm_store()` — a parser
  must hold the store while parsing a wasm-backed language; the API
  moves the store, so the host keeps it in a `Mutex` and takes/puts
  it around each highlight call.

Grammar wasm is a parser module inside tree-sitter's own wasmtime —
NOT a WASI component. It never touches the extension host, has no
imports to grant, and is bounded by tree-sitter's sandbox.

**Recorded cost:** tree-sitter 0.23's wasm feature statically links
`wasmtime-c-api-impl` 24 beside the extension host's wasmtime 48.
Cargo keeps the two majors separate; the price is build time and some
binary size. Consolidation happens when inkjet bumps to a tree-sitter
whose wasm feature aligns with our wasmtime major; until then this is
the accepted price of sandboxed, cross-platform grammar loading (the
alternative — native dylib grammars — sacrifices both).

## Plugin shape — data + parser module, no component wasm

A grammar plugin directory contains `plugin.toml`, `grammar.wasm`,
and `highlights.scm`:

```toml
name = "graphql"
version = "0.1.0"

[[grammars]]
name = "graphql"            # language token; doubles as the fence alias
extensions = ["graphql", "gql"]
```

- `discover()` relaxes its `plugin.wasm` requirement: a manifest with
  `[[grammars]]` may ship `grammar.wasm` + `highlights.scm` instead.
  A plugin may ship both kinds (component surfaces AND grammars); each
  declared grammar requires both files or the plugin fails to load.
- Multiple `[[grammars]]` per plugin are allowed; each names its own
  `grammar.wasm`/`highlights.scm` pair via an optional `files` stem
  (default: `grammar.wasm` / `highlights.scm`; with two-plus grammars
  the stem is required: `files = "graphql"` → `graphql.wasm` +
  `graphql.scm`).
- Query compile errors, bad wasm, or unsupported ABI → per-plugin
  load-report failure, exactly like a bad manifest today.

## Highlight-layer integration — a dynamic registry beside inkjet

A grammar registry with the same shape as the other contribution
tables (RwLock, rebuilt by `refresh_tables` / Reload Plugins):

- `name → HighlightConfiguration` (built from the wasm `Language` +
  `highlights.scm`, configured with the shared `CAPTURE_NAMES` table
  so theme mapping is identical to built-ins).
- `extension → name` for file resolution.

Resolution order in `Languages::highlight()`: built-in extras → 
**plugin registry** → inkjet. Built-ins win on name collision (a
plugin cannot shadow `rust`); a test pins this. `language_for_file`
gains a registry lookup for extensions the static table doesn't know;
its return type widens from `Option<&'static str>` to an owned form
where the call sites need it. Fenced blocks resolve through the same
token, so ```graphql fences highlight with no extra wiring.

Highlighting a wasm-backed language runs on the same paths as today
(once per open/edit, never per frame); the only difference is the
take/put of the `WasmStore` around the parse, and the store lives in
the same global as the registry.

## Building the GraphQL artifact

`tree-sitter build --wasm` needs the CLI plus emscripten — a toolchain
we keep OFF user machines and out of CI. Policy (same philosophy as
the vendored merman):

- `scripts/build_grammar_wasm.sh <grammar-repo-dir> <out.wasm>` wraps
  the CLI invocation and documents the emcc requirement.
- The built `grammar.wasm` artifact is committed under
  `plugins/graphql/` (~200 KB), alongside `highlights.scm` taken from
  the tree-sitter-graphql repo (license note included).
- Regenerating with a 0.23-era CLI produces ABI ≤ 14, which is what
  sidesteps the ABI-15 blocker that stopped the native crate.
- The fixture used by host tests reuses the same committed artifact
  (no second build).

`scripts/build_plugins.sh` copies grammar plugins into dist by file
copy (no cargo build step for them).

## First-party plugin

**`plugins/graphql/`** — manifest above, committed `grammar.wasm`,
`highlights.scm`. Proves: open `schema.graphql` → highlighted; type a
```graphql fence → highlighted; delete the plugin dir + Reload
Plugins → plain text, no error.

## Error handling

Same contract: grammar failures are data. Load-time failures land in
the load report (palette footer). A parse that traps or an
incompatible ABI degrades that language to plain text with an eprintln
— never a crash (wrap the registry path in the same catch_unwind that
guards inkjet's lazy statics today).

## Testing strategy

- Manifest: `[[grammars]]` parsing (extensions, multi-grammar stems);
  grammar plugin without `plugin.wasm` is accepted; declared grammar
  with missing `grammar.wasm` or `highlights.scm` fails that plugin.
- Registry (fixture = the committed graphql artifact): loading yields
  spans for graphql source with zero recompile; `language_for_file`
  resolves `.graphql`/`.gql` through the registry; fence token
  resolves; broken `highlights.scm` → per-plugin failure; built-in
  name collision → built-in wins; reload swaps the registry (drop
  fixture, reload, plain text).
- Fixture-skip pattern matches the existing suite (tests skip with an
  eprintln when the artifact is absent — but since the artifact is
  committed, they run everywhere, including all three CI OSes).
- Existing highlight tests keep pinning the inkjet and extras paths.

## Out of scope (recorded)

Injections/locals queries (highlights only, matching our extras),
grammar-provided indentation/folding/textobjects, per-grammar fence
aliases beyond name+extensions, wasmtime consolidation, a grammar
marketplace (registry phase). Repo policy decided alongside this
phase: first-party plugins stay in-repo; extracting the template and
a git-based extensions index happens with the registry phase.
