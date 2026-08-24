# Extensions Phase 5 Design — UI Surfaces + Automation (E2, E3, F1, F2)

**Date:** 2026-08-24
**Status:** Approved for planning
**Program:** `2026-08-24-extensions-roadmap.md` (Phase 5 of 5)
**Branch:** continues `extensions` (Phases 1–4 unmerged; merge decision
follows this phase)

## Purpose

Ship the last roadmap surfaces that fit the proven model — custom
viewers, status widgets, templates, and a save hook — without
inventing plugin-drawn UI. The guiding lesson from Phases 1–4:
surfaces that reuse existing render machinery ship fast and safe.
Panels (E1) are explicitly deferred (see Out of scope).

## WIT evolution — `supermd:extension@0.4.0`

A fourth world (fourth `bindgen!`, `Bound::V4`, fallback chain
V4→V3→V2→V1). It carries every 0.3 export plus:

```wit
world extension {  // @0.4.0 — the 0.3 world plus:
    /// Render a non-markdown file to MARKDOWN; the host shows it with
    /// the existing Reader. This is the whole viewer model.
    export render-view: func(filename: string, content: string)
        -> result<string, string>;

    /// One-line status text for the active document ("1,234 words").
    export status-text: func(document: string)
        -> result<string, string>;

    record template-context {
        /// "2026-08-24"
        date: string,
        /// "16:52"
        time: string,
        /// "Monday"
        weekday: string,
        /// Workspace root folder name, "" when no folder is open.
        workspace: string,
    }
    record template-file {
        /// Relative path under the workspace root (host-validated).
        filename: string,
        content: string,
    }
    /// Materialize a template ("New: Daily Note").
    export render-template: func(id: string, context: template-context)
        -> result<template-file, string>;

    /// Pre-save transform; none = save unchanged. Runs on every save
    /// for plugins declaring hooks = ["save"] — NOT gated by the
    /// format_on_save setting.
    export on-save: func(path: string, document: string)
        -> result<option<string>, string>;
}
```

Older worlds get the usual readable "requires a 0.4 plugin" error on
the new surfaces. Existing plugins keep working unchanged; first-party
plugins that gain 0.4 surfaces (toc) move their wit path to 0.4.

## Manifest additions

```toml
# Custom viewer: claims file extensions for rendered view.
[[viewers]]
extensions = ["csv", "tsv"]

# Status widget (text-only).
[[widgets]]
id = "words"

# Template: palette entry "New: <name>".
[[templates]]
id = "daily"
name = "Daily Note"

# Save hook events; only "save" is understood this phase.
hooks = ["save"]
```

Unknown hook events reject the plugin at load (forward-compat, same
policy as capabilities). All four surfaces are component surfaces
(`needs_component` includes them).

## Viewers (E3) — markdown projections, ⌘E to source

- Resolution: a new RwLock viewer table maps extension → plugin
  (first plugin wins on overlap, load order; recorded). Grammar
  plugins take precedence for the EDITOR path; viewers only change
  what opening the file shows.
- Opening a file whose extension a viewer claims: the host reads the
  file, calls `render-view` on the background executor, and opens a
  Reader tab from the returned markdown (the Reader already handles
  tables, headings, fences, diagrams, theming). While rendering or on
  failure, fall back to the plain source editor — a broken viewer
  never hides the file.
- ⌘E on a viewer tab toggles to the raw source editor and back
  (re-rendering on toggle so edits show).
- Views are read-only projections; interactivity inside a view is out
  of scope this phase.

## Status widgets (E2) — one new core strip

- A thin status strip at the bottom of the editor pane, styled like
  the rest of the chrome (muted, small), rendered ONLY when at least
  one widget plugin is loaded and the active tab is an editor.
- Refresh: debounced ~500 ms after edits, and on tab switch. Calls run
  on the background executor; results cached per (plugin, document
  generation). A failing or slow widget contributes nothing — the
  strip never blocks typing and never shows errors.
- Multiple widgets concatenate with " · " in plugin load order.

## Templates (F2) — palette-driven, workspace-only

- Each `[[templates]]` entry adds palette entry "New: <name>"
  (id-routed like "__format"/"__export:").
- Host builds `template-context` from the current local date/time
  (wasm has no clock) and workspace name, calls `render-template`,
  validates `filename` with the Phase 3 path rules (relative, no `..`)
  plus "must have a workspace open", then:
  - file exists → just open it (idempotent daily notes);
  - else create parent dirs + file, refresh the tree, open the tab.
- No capability needed: user-invoked, host-validated, workspace-only
  writes.

## Save hook (F1) — always-on pre-save transform

- Plugins with `hooks = ["save"]` run `on-save(path, document)` in the
  flush path AFTER the optional format_on_save formatter, each under
  the usual epoch deadline, with the same generation guard
  (`apply_if_unchanged`): if the buffer moved while the hook ran, the
  hook result is discarded and the original saves.
- `Some(new_text)` replaces the document (one undo group) and saves;
  `None` or error → save proceeds unchanged. Saving never blocks on a
  plugin beyond the deadline.
- Multiple hook plugins chain in load order, each seeing the previous
  result.
- Deferred: `on-open` (no first-party use case yet; recorded).

## First-party plugins

- **`plugins/word-count/`** — widget: "1,234 words · 6 min read"
  (words = whitespace-split count outside fenced code; 200 wpm).
- **`plugins/daily-note/`** — template `daily`:
  `journal/<date>.md` with a `# <weekday>, <date>` header and an
  empty checklist section.
- **`plugins/csv-view/`** — viewer for csv/tsv: sniffs the delimiter
  (tab first, then comma), escapes pipes, emits a markdown table
  capped at 500 rows with a "… N more rows" tail note. Non-tabular
  content → error (host falls back to source editor).
- **`plugins/toc/`** — gains `hooks = ["save"]` + `on-save` that
  refreshes `<!-- toc -->`…`<!-- /toc -->` blocks (returns None when
  no markers present), moving to the 0.4 wit path.
- Fixture: echo moves to 0.4 and echoes all four new surfaces so host
  tests can drive them.

## Error handling

Unchanged contract: plugin failures are data. Viewer failure → source
editor; widget failure → absent text; template failure → command-error
strip; hook failure → save proceeds unchanged. Nothing new can crash
or hang the app (epoch deadlines cover all four surfaces).

## Testing strategy

- Manifest: viewers/widgets/templates/hooks parse; unknown hook event
  rejects; surfaces imply `needs_component`.
- Host (echo fixture): all four surfaces roundtrip; v3-and-older get
  readable errors on them.
- Viewer: extension table resolution (first wins); csv-view in-crate
  (delimiter sniff, pipe escaping, row cap, non-tabular error).
- Widget: cache keyed on generation (stale results dropped);
  concatenation order.
- Template: path validation reuse (`../evil` rejected; absolute
  rejected); exists→open semantics (pure fs test); context date
  formatting.
- Hook: chain order; generation guard discards stale hook results;
  toc in-crate marker-refresh tests extended for the hook path.

## Out of scope (recorded)

Panels (E1: kanban, backlinks, calendar) — need interactive native UI
contributions; deferred to a dedicated cycle once these surfaces prove
the model. `on-open` hooks. Interactive viewers (sorting, links back
to source rows). Clickable widget actions. Pomodoro-style widgets
needing timers (no background execution for plugins). Viewer editing
affordances beyond the ⌘E source toggle.
