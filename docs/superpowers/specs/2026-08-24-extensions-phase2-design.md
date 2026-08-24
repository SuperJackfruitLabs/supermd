# Extensions Phase 2 Design — Pure Surface Growth + workspace-read

**Date:** 2026-08-24
**Status:** Approved for planning
**Program:** `2026-08-24-extensions-roadmap.md` (Phase 2 of 5)
**Branch:** continues `extensions-phase1` (Phase 1 unmerged; this
phase builds directly on its runtime)

## Purpose

Grow the pure extension surface — inline renderers, span decorators,
document formatters, paste processors — and ship the first real
capability (`workspace-read`) with its consent UX. Plus quality of
life earned in Phase 1: a Reload Plugins command.

## WIT evolution — `supermd:extension@0.2.0`

A new world adds four exports; the host binds **both** 0.1 and 0.2
(two `bindgen!` blocks, per-plugin fallback at instantiation), so
Phase 1 plugins keep working unchanged:

```wit
world extension {  // @0.2.0 — includes the 0.1 exports plus:
    /// Replace one inline pattern match (cached by matched text).
    export render-inline: func(pattern-id: string, matched: string)
        -> result<string, string>;

    /// Normalize a whole document ("Format Document" / on-save).
    export format-document: func(document: string)
        -> result<string, string>;

    /// Transform pasted text; none = leave paste unchanged.
    export process-paste: func(text: string)
        -> result<option<string>, string>;

    /// Phase-2 capability probe (workspace-read): host preopens the
    /// workspace root read-only when granted.
    // (no new export needed — WASI preopens carry the capability)
}
```

Template gains stubs for all; the manifest declares which surfaces a
plugin actually uses so the host never calls unused exports.

## Manifest additions

```toml
# Inline renderers: regex with exactly one match consumed whole.
[[inline]]
id = "emoji"
pattern = ":([a-z0-9_+-]+):"

# Declarative decorators — NO wasm involved; pure manifest.
[[decorations]]
pattern = "\\b(TODO|FIXME|NOTE)\\b"
style = "accent"        # fixed palette: accent | muted | strong | highlight

# Formatter registration (palette command auto-added: "Format: <name>").
formats = true

# Paste processor registration.
paste = true

capabilities = ["workspace-read"]   # now accepted; consent-gated
```

Regexes compile host-side at load (`regex` crate); invalid patterns →
load-report failure for that plugin. `capabilities` other than
`workspace-read` still rejected forward-compat.

## Inline renderers (A2) — text replacements only (honest limit)

Inline results ride the display transform as `Replacement` segments
(the checkbox machinery), so the reveal rule is inherited: cursor in
the span shows raw text. **Phase 2 renders text→text only** (emoji,
unit conversions); inline *images* (math) need inline boxes in line
layout and are explicitly deferred to the math cycle.

Latency design — restyle runs per keystroke and must never call wasm:
- A global `InlineCache: (plugin, pattern-id, matched) → Result<String, ()>`
  (capped LRU, like DiagramCache).
- `spans.rs` gains an inline pass: host-side regexes find matches;
  cache hits become Replacement directives; misses render nothing
  (raw text stays) and enqueue.
- A background drainer resolves misses via the plugin and triggers
  restyle on completion. Deterministic content ⇒ near-100% hit rate
  after first sight.
- Replacements never apply inside fences or inline code (span check).

## Span decorators (A3) — declarative, zero wasm

`[[decorations]]` are pure manifest data: host-compiled regex → style
token from a fixed palette mapped to theme colors. Applied in
`line_attrs` as an overlay (same layering as find matches), skipping
code fences. No plugin call, no latency, no cache. Programmable
decorators (wasm-computed spans) are recorded as deferred — the
declarative form covers TODO/mention/wiki-link styling without them.

## Formatters (B2)

- Every `formats = true` plugin contributes a palette command
  "Format: <plugin>" running `format-document` through the existing
  command apply path (one undo group, ReplaceDocument semantics with
  a generation check: snapshot text; if the buffer changed while the
  plugin ran, the result is discarded with a strip message).
- `format_on_save = false` settings key (global default OFF): when
  enabled, flush runs the first formatter before writing, same
  generation check; failures skip formatting and save anyway — saving
  never blocks on a plugin.

## Paste processors (B3)

The paste handler consults `paste = true` plugins in load order:
first `Some(replacement)` wins. Synchronous call under the existing
2s epoch cap (paste is an explicit action; typical transforms are
sub-millisecond). Errors or `None` → original text pastes. Plain text
only — clipboard HTML flavor access depends on gpui's clipboard API
and is checked at plan time; if unavailable, HTML→md stays deferred.

## workspace-read capability + consent

- Manifest `capabilities = ["workspace-read"]` accepted.
- Grants persist in settings: `plugin_grants = { "<name>" = ["workspace-read"] }`.
- Consent UX: first call needing the capability → the plugin's calls
  return Err("awaiting consent") and a banner appears (install-banner
  styling): "Plugin <name> wants to read files in this workspace —
  [Allow] [Deny]". Allow persists the grant and reloads the plugin's
  instance; Deny persists a refusal (asked once).
- Mechanics: granted plugins get their store's WasiCtx built with a
  **read-only preopen of the workspace root** (`DirPerms::READ`,
  file perms read-only) mounted at `/workspace`. No grant → no
  preopen (Phase 1 zero-grant ctx unchanged). Plugins see the
  workspace at a stable path; everything else stays invisible.
- Enforced by fixture tests: a file-reading fixture plugin fails
  without the grant and succeeds with it, and cannot read outside the
  preopen either way.

## Reload Plugins

Palette/menu command: re-runs discovery + compilation into a fresh
`ExtensionHost`, swaps the global, rebuilds the fence table snapshot
(now a mutable `RwLock` instead of `OnceLock`), clears the diagram
and inline caches, restyles open editors. Errors land in the load
report as usual.

## First-party plugins (Phase 2)

- **`plugins/emoji/`** — inline renderer: `:tada:` → 🎉 over a
  bundled shortcode table (~1500 entries, generated once from the
  GitHub gemoji list into Rust source).
- **`plugins/tidy/`** — formatter + paste processor: smart quotes and
  dashes, collapse 3+ blank lines, trim trailing whitespace; paste
  side additionally detects TSV/CSV-shaped text and converts to a
  markdown table.
- **`plugins/todo-marks/`** — manifest-only decorator plugin (no
  meaningful wasm; the template stub compiles): TODO/FIXME/NOTE in
  `accent`. Proves the zero-code plugin path.
- Fixture: `fixtures/reader` (workspace-read probe used by capability
  tests).

## Error handling

Same contract as Phase 1: plugin failures are data. New surfaces
inherit it — inline miss stays raw text; formatter failure leaves the
document untouched with a strip; paste failure pastes the original;
consent-denied reads are Errs inside the plugin, not crashes.

## Testing strategy

- Manifest: inline/decorations/formats/paste parsing; invalid regex →
  per-plugin load failure; workspace-read accepted, others rejected.
- Inline: pure pass over a doc with cache injected (hit → Replacement
  directive placed, miss → raw + enqueued); fence/inline-code
  exclusion; reveal rule via existing display tests' pattern.
- Decorators: overlay spans land in `line_attrs` (theme token
  mapping), fences excluded.
- Formatter: generation-check discards stale results; tidy's rules
  unit-tested in-crate (quotes, dashes, blank-line collapse, CSV
  detection width edge cases).
- Paste: first-Some-wins ordering; None passthrough; error
  passthrough.
- Capability: reader fixture denied/granted/path-escape tests; grant
  persistence round-trip in settings.
- Reload: swapped host serves a newly-dropped fixture without
  restart (test drives `ExtensionHost` reload directly).

## Out of scope (recorded)

Inline images/math (own cycle), programmable decorators, clipboard
HTML flavor (pending gpui API check), fs-write/net (Phase 3),
grammars (Phase 4), UI surfaces (Phase 5).
