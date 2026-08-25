# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

SuperMD is a native, GPU-rendered Markdown editor in Rust on GPUI (Zed's UI framework). Hybrid WYSIWYG: plain CommonMark on disk, always; syntax markers hide until the cursor touches them.

## Commands

```sh
cargo run -- .                 # open current directory as a workspace
cargo test                     # full suite
cargo test <name>              # single test (substring match on test fn name)
cargo llvm-cov                 # coverage summary (CI enforces a 90% line floor)
bash scripts/build_plugins.sh            # build first-party plugins → dist/plugins
bash scripts/build_plugins.sh --fixtures # build test-fixture plugins (required for extension-host tests)
```

Plugin builds need `rustup target add wasm32-wasip2`. On Linux, install the system deps listed in README.md before building. CI runs `cargo test` + `cargo build` on macOS/Linux/Windows; changes under `site/**`, `docs/**`, and `README.md` skip app CI entirely. `WELCOME.md` is NOT ignored — it is `include_str!`'d into the binary and render tests assert on it.

Two change-detector conventions: `main.rs` has a keybinding-count test (`every_keybinding_parses_and_binds`) — adding a `KeyBinding` means bumping the count, and updating the ⌘/ dialog (`SHORTCUTS` in workspace.rs) plus `docs/site/shortcuts.md`, which are hand-synced. Releases: bump `version` in Cargo.toml (plus lockfile) and commit BEFORE tagging — cargo-deb reads it; the DMG/exe take the tag and will mask the mistake.

## Architecture

The core rule of the codebase: **editing logic is pure Rust under tests; the GPUI shell stays thin.** Tests live inline as `#[cfg(test)]` modules next to the code they cover, and the 90% coverage floor is enforced in CI — new logic goes in a pure, testable module with the GPUI layer only driving it.

**Byte offsets are the universal currency.** All buffer positions, spans, selections, and movement functions use byte offsets into the source text (`src/editor/buffer.rs`, a ropey rope).

### Editor pipeline (src/editor/)

Source text flows through these stages each render:

1. `core.rs` — `EditorCore`, the tested facade: buffer + one selection + undo history. The GPUI shell drives only this.
2. `spans.rs` — source text → style spans (byte ranges over raw source; markers included).
3. `display.rs` — the hybrid-WYSIWYG transform: the ONE place the "buffer offset == rendered offset" invariant deliberately breaks (hiding `**`, list bullets, etc.). Anything mapping between display and buffer coordinates goes through here.
4. `blocks.rs` + `projection.rs` + `projector.rs` — cross-line widgets (tables, images, code fences, diagrams). Projectors *claim* line ranges; `projection.rs` alone owns the reveal rule (a claim renders as a widget only while the cursor is outside it).
5. `mod.rs` — the GPUI shell: one logical line per virtualized list item, IME-correct input via `EntityInputHandler`, editor actions.

Other editor modules, all pure with the shell driving them: `movement.rs` (horizontal movement is pure; vertical lives in the view layer because it needs wrapped-line geometry), `formatting.rs` (⌘B/⌘I and the selection toolbar's toggles, each one contiguous replacement + post-edit selection), `lists.rs` (Enter-continuation and Tab-indent analysis), `table_edit.rs` (cell navigation, pipe alignment, cursor mapping through alignment), `paste_image.rs` (asset naming/links for clipboard-image paste), `autosave.rs` (policy is a pure state machine over injected time; backups skip content matching git HEAD), `find.rs`. The `[[` completion popup and follow-link live in `mod.rs` but lean on `knowledge.rs`.

### App shell (src/)

- `workspace.rs` — the root view: sidebar, tabs, document pane, outline. Largest file; most UI wiring lands here.
- `main.rs` — app entry, menus, keybindings, asset source.
- `markdown.rs` → `reader.rs` → `view.rs` — the read-only path: CommonMark → block model → GPUI elements (pretty preview).
- `platform.rs` — the single home for per-OS decisions (keybindings, fonts, paths). Ask it instead of sprinkling `cfg!()`.
- `knowledge.rs` — the knowledge index: link/tag extraction, wiki-stem resolution, backlinks, and rename-time link rewriting. In-memory only (files are the truth); built at workspace-open, kept warm by the watcher via `on_fs_events`, shared as the `KnowledgeState` global. `graph.rs` renders it: deterministic force layout (no randomness — frames and tests reproduce) plus the local one-hop layout.
- `fileops.rs` — sidebar file operations policy (rename/move/create/trash, overwrite refusal, tab retargeting). Workspace renames funnel through `after_path_change`, which retargets tabs AND rewrites knowledge links — new "something moved" behavior belongs there.
- `theme.rs` / `settings.rs` — themes (TOML, `~/.supermd/themes/`), persistent settings (`~/.supermd/settings.toml`). `flux.rs` layers time-of-day adaptation on top: NOAA solar math + kelvin warming, applied inside `ThemeState::resolve()`; a minute timer in main.rs drives the blend. New theme colors must be threaded through `Theme::map_colors` or flux warming misses them.
- `highlight.rs` — tree-sitter highlighting via inkjet; runs on open/edit, never per frame.
- `git.rs` + `diff.rs` — Show Changes. `git.rs` is read-only gix access (errors degrade to "no baseline", never a write to the repo); `diff.rs` is a pure diff engine with documented strip-invariants.
- `diagram.rs` — mermaid → SVG (merman) → PNG (resvg) on the background executor; the UI only reads the cache. `merman-render` is vendored in `vendor/` with a patch (see Cargo.toml) — drop when upstream ships a compiling release.
- `search.rs`/`search_ui.rs`, `finder.rs`, `palette.rs` — overlay family (⌘⇧F, ⌘P, ⌘⇧P); engines are pure, overlays drive them from the background executor.
- `seti.rs` — GENERATED by `scripts/vendor_seti.py`, do not edit; its tests live in the hand-written `seti_tests.rs` so regeneration never destroys them.

### Plugin system

Plugins are WebAssembly components (`plugin.wasm` + `plugin.toml`) in `~/.supermd/plugins/`. `src/extensions.rs` is the wasmtime host: manifest discovery and capability enforcement — no filesystem without per-plugin consent (`workspace-read`, read-only preopen), no sockets ever (`net` is a host-mediated fetch with per-domain consent, HTTPS-only, size/time budgets), no processes, 2s compute timeout; enforcement keys off the manifest declaration, not the wasm. WIT interfaces are versioned in `plugins/wit`, `wit-v2` … `wit-v4`. Fourteen first-party plugins live in `plugins/` — eight seeded on first launch via `src/seeding.rs`, the rest installable in-app; each doubles as the working example for its surface, and new ones start from `plugins/template/`. `plugins/fixtures/` holds adversarial test plugins (panic, hang, fetcher…) that host tests need built first. `catalog.json` + `src/catalog.rs`/`install.rs` handle the "Install Plugins…" flow with sha256-pinned downloads.

### Design constraints

- The plain-text Markdown file is the source of truth — no proprietary format ever hits disk.
- Code files get a "viewer-plus" editor (highlighting, gutter, auto-indent) — deliberately no LSP.
- SuperMD never writes to the user's git repository.

### Design docs

`docs/superpowers/plans/` and `docs/superpowers/specs/` hold the per-phase implementation plans and specs — the history of why the architecture looks the way it does. `docs/BACKLOG.md` is the groomed list of deferred work and explicit non-goals — check it before proposing features (some "missing" things were decided against).

The user docs are generated: `docs/site/*.md` + `nav.toml` are the sources, `cargo run --example build_docs` renders them into `site/docs/` (committed). Edit sources, then regenerate — never edit `site/docs/` by hand.
