# Extensions Phase 1 Design — Runtime + Pure Extensions

**Date:** 2026-08-24
**Status:** Approved for planning
**Program:** `2026-08-24-extensions-roadmap.md` (Phase 1 of 5)

## Purpose

The WebAssembly extension runtime and its first two extension types —
block renderers and text commands, both pure functions — plus the
command palette that surfaces commands, a template crate for authors,
and two first-party plugins (dot, toc) proving the loop end to end.

## Component 1: WIT interface — `plugins/wit/extension.wit`

```wit
package supermd:extension@0.1.0;

interface types {
    /// Palette entry contributed by a plugin.
    record command-info { id: string, title: string }

    /// The active theme, for renderers that produce themed output.
    record theme {
        background: string,   // #rrggbb
        surface: string,
        primary: string,
        text: string,
        muted: string,
        border: string,
        font-body: string,
        dark: bool,
    }

    record command-input {
        document: string,
        selection-start: u32,  // byte offsets into document
        selection-end: u32,
    }

    variant command-output {
        replace-document(string),
        replace-selection(string),
        insert-at-cursor(string),
    }
}

world extension {
    use types.{theme, command-input, command-output, command-info};

    /// Render a claimed fenced block to SVG.
    export render-block: func(lang: string, source: string, theme: theme)
        -> result<string, string>;

    /// Run a contributed command.
    export run-command: func(id: string, input: command-input)
        -> result<command-output, string>;
}
```

Both exports are required by the world; the template provides
`Err("unsupported")` stubs so renderer-only or command-only plugins
stay trivial. Fence claims and command lists live in the manifest,
not the wasm, so the host never instantiates a plugin just to
enumerate its contributions.

## Component 2: Manifest — `plugin.toml`

```toml
name = "dot"                    # unique id, kebab-case
version = "0.1.0"
description = "Graphviz DOT diagrams as live blocks"
authors = ["SuperJackfruitLabs"]

# Contributions (all optional):
fences = ["dot", "graphviz"]    # block-renderer claims

[[commands]]
id = "toc.insert"
title = "Insert Table of Contents"
```

Reserved for later phases (present in the parser, rejected in Phase
1 with a clear error): `capabilities = [...]`.

## Component 3: Runtime host — `src/extensions.rs` (new)

- `wasmtime` with the component model; `wasmtime::component::bindgen!`
  over the WIT above generates typed bindings. Version pinned at
  implementation. The wasip2 standard library requires core WASI
  interfaces even for pure code, so the linker provides WASI with a
  ZERO-GRANT context: no preopened directories, no env, no args, no
  network — only stderr is inherited so plugin panics are debuggable.
  The capability surface is empty in every way that matters.
- Discovery: scan `~/.supermd/plugins/*/plugin.toml` at startup; each
  dir needs `plugin.toml` + `plugin.wasm`. Parse failures and link
  failures are collected (never fatal) into a load report:
  `Vec<(plugin_dir, Result<PluginMeta, String>)>`, logged to stderr
  and shown in the palette as a dimmed "failed: <name>" row.
- `ExtensionHost` (gpui Global): engine, and per-plugin
  lazy-instantiated component + store. Instantiation on first call.
- Every call runs with **epoch interruption**: a 2-second deadline
  ticked by a background thread; timeout or trap returns
  `Err(String)` to the caller. One misbehaving plugin can never hang
  the app; a trapped store is dropped and re-instantiated on next
  use.
- Calls run on the background executor (the UI never blocks on a
  plugin), returning through the same channels as diagram renders.

```rust
pub struct PluginMeta { pub name: String, pub version: String,
    pub fences: Vec<String>, pub commands: Vec<CommandInfo>, pub dir: PathBuf }

impl ExtensionHost {
    pub fn plugins(&self) -> &[PluginMeta];
    pub fn failures(&self) -> &[(PathBuf, String)];
    /// Blocking; call from the background executor only.
    pub fn render_block(&self, plugin: &str, lang: &str, source: &str,
        theme: &crate::diagram::DiagramTheme) -> Result<String, String>;
    pub fn run_command(&self, plugin: &str, id: &str, input: CommandInput)
        -> Result<CommandOutput, String>;
}
```

## Component 4: Plugin block renderers — projector + cache reuse

A `PluginBlockProjector` registered after Diagram in the projector
registry: discovery claims closed fences whose lang matches any
loaded plugin's `fences` (first plugin claiming a lang wins;
collisions logged). Rendering reuses the *entire* diagram pipeline —
the plugin produces SVG, then resvg rasterization, the global
`DiagramCache` (key gains the plugin name+version), the
Pending/Ready/Failed widget states, and the dissolve contract are all
shared. A plugin syntax error looks exactly like a mermaid syntax
error: slim strip + raw source.

## Component 5: Command palette — ⌘⇧P

A finder-family overlay (list-only, no preview pane; 480px wide):
rows are plugin commands (`title` + dimmed plugin name), filtered by
the existing nucleo scorer as you type; Enter runs the selected
command against the active editor: `command-input` built from the
buffer + selection; output applied as one undo group
(`replace-document` / `replace-selection` / `insert-at-cursor`),
via the existing `EditorCore::replace_range`. Errors surface as a
transient strip in the palette. Read-only tabs (Reader/Diff/preview)
show commands disabled. Binding: `cmd-shift-p` (translated per
platform); menu + ☰ + SHORTCUTS entries. The palette lists only
plugin commands in Phase 1; core actions may join later.

## Component 6: Author experience

- `plugins/template/` — a `cargo component` crate: WIT bindings,
  both exports stubbed, README walking through build → drop into
  `~/.supermd/plugins/` → see it work. Building requires
  `cargo-component` + the `wasm32-wasip2` target (documented; exact
  target/tooling pinned at implementation against current
  cargo-component).
- `scripts/build_plugins.sh` builds every first-party plugin to
  `dist/plugins/<name>/` and is wired into the release workflow so
  releases attach `supermd-plugins.zip`.
- "Open Plugins Folder" command in the File/☰ menu (creates the dir
  if absent).

## Component 7: First-party plugins

- **`plugins/dot/`** — DOT/graphviz block renderer on the pure-Rust
  `layout` crate (SVG writer built in). Theme applied by
  post-processing the emitted SVG's default colors to the theme
  palette (background, text, lines). If `layout` fails to build for
  the wasm target, fallback plan: vendor its layout core (MIT).
- **`plugins/toc/`** — `toc.insert`: scans the document's headings
  (own tiny ATX parser — plugins don't get our internals), builds an
  indented link list, inserts at cursor; `toc.update`: replaces the
  content between `<!-- toc -->` markers, or errs with guidance when
  markers are absent.

## Error handling (the contract, stated once)

Plugin failures are data, never crashes: parse/link failures → load
report; traps/timeouts → `Err` → Failed widget state or palette
strip; a plugin returning invalid SVG → resvg's error → same path as
mermaid. The host process never dies because of a plugin — enforced
by tests that load a deliberately-broken and a deliberately-hanging
fixture plugin.

## Testing strategy

- WIT/manifest: parse valid + invalid manifests; reserved
  `capabilities` key rejected with the forward-compat message.
- Host: fixture wasm components checked into `tests/fixtures/`
  (built once by `build_plugins.sh --fixtures`): an echo renderer, a
  panicking plugin, an infinite-loop plugin (asserts the epoch
  deadline fires), a WASI-importing plugin (asserts readable link
  error).
- Projector: plugin fence claims through the existing registry tests'
  pattern; collision (two plugins claim `dot`) → first wins + logged.
- toc: pure logic tested as a normal Rust crate before wasm packing
  (insert, update-between-markers, missing-markers error).
- dot: golden test — known graph renders SVG containing node labels;
  themed colors present.
- Palette: nucleo filtering pure-tested; apply-output paths tested at
  the EditorCore level (one undo group).
- CI: plugin build script runs in the linux job; fixture-dependent
  tests are skipped when fixtures are absent locally
  (`build_plugins.sh --fixtures` documented in CONTRIBUTING note).

## Out of scope (Phase 1)

Everything in roadmap Phases 2–5 (inline renderers, decorators,
formatters, paste processors, all capabilities, grammars, UI
surfaces, hooks); a registry/marketplace; plugin settings; hot
reload (restart to pick up changes — a "Reload Plugins" command may
land in Phase 2).
