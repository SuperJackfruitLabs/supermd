# Extensions Program Roadmap

**Date:** 2026-08-24
**Status:** Program-level spec — each phase gets its own detailed
design + plan + implementation cycle. Phase 1's design:
`2026-08-24-extensions-phase1-design.md`.

## Thesis

SuperMD extends the **document**, not the editor. Extensions are
WebAssembly components (wasmtime) behind a narrow, typed API —
capability-scoped, deny-by-default, language-agnostic. The Zed model,
not the VS Code model: no embedded JS runtime, no webviews, no
unscoped machine access, ever.

## Extension-type taxonomy

**A. Content renderers** (pure; text → visual)
- A1 Block renderers: fence lang → SVG through the projector registry
  and diagram cache. Candidates: graphviz/DOT, typst/LaTeX math,
  PlantUML subset, ABC music notation, chess (FEN/PGN), railroad
  diagrams, WaveDrom, QR codes, CSV→chart, SMILES chemistry,
  calendars.
- A2 Inline renderers: in-line pattern → replacement run (rides the
  display transform). Candidates: inline `$math$`, `:emoji:`
  shortcodes, color chips, unit conversions.
- A3 Span decorators: pattern → extra style spans. Candidates:
  TODO/FIXME highlighting, @mentions, `[[wiki-link]]` styling.

**B. Text transformers** (pure; text → text)
- B1 Commands (palette-invoked): TOC insert/update, table formatter,
  sort lists, smart punctuation, case tools, template/date inserters,
  citation formatting.
- B2 On-save formatters: markdown normalizer, whitespace cleanup,
  reference-link reorganizer.
- B3 Paste processors: HTML→Markdown, rich-text cleanup, CSV→table.

**C. Language support**
- C1 Grammars: tree-sitter grammars compiled to wasm, runtime-loaded
  (GraphQL, Terraform, Prisma, the community grammar universe).
- C2 Themes: already shipped as TOML files — the existing zero-code
  extension format.

**D. Workspace & data** (capability-gated)
- D1 Exporters (fs-write): md→PDF, →HTML site, →EPUB, →docx.
- D2 Importers (fs-read): Notion/Bear/Evernote/Obsidian → md tree.
- D3 Publishers (net): gist/Ghost/blog posting.
- D4 Enrichers (net): paste-URL→titled link, link health checks.

**E. UI surfaces** (declarative-native only; no plugin pixels)
- E1 Panels: daily-note calendar, kanban-from-checkboxes, backlinks.
- E2 Status widgets: word count, reading time, pomodoro.
- E3 Custom viewers: CSV table, JSON tree.

**F. Automation**
- F1 Hooks: on-open/on-save actions.
- F2 Templates: new-file templates (daily notes), naming schemes.

## Capability model

| Capability | Grants | Grant UX |
|---|---|---|
| *(none)* | pure compute: bytes in → bytes out | installs silently |
| `workspace-read` | read files under the open workspace root | one-time per-plugin consent |
| `fs-write` | write only to paths the USER picks via the system dialog — plugins never choose destinations | implicit in the dialog |
| `net:<domain>` | HTTPS to manifest-declared domains only | one-time per-plugin, per-domain consent |
| `ui` | declarative panel/widget contributions rendered natively | one-time consent |

**Never granted:** process spawning, unscoped filesystem, raw
sockets, plugin-drawn pixels/webviews, background execution outside a
host-initiated call. Every call runs under an epoch deadline; a slow
plugin is interrupted, never the app.

## Phases (each = one spec → plan → implementation cycle)

### Phase 1 — Runtime + pure extensions (types A1, B1)
Wasmtime component runtime; `supermd:extension` WIT world; TOML
manifest; `~/.supermd/plugins/` loading; plugin block renderers
joining the projector registry through the diagram cache; a **command
palette (⌘⇧P)** listing plugin commands; `supermd-plugin` template
crate; first-party plugins: **dot** (graphviz via the pure-Rust
`layout` crate) and **toc** (insert/update table of contents).
*Exit criteria:* both plugins ship as `.wasm` built by
`scripts/build_plugins.sh`, render/transform correctly on all three
OSes, a plugin panic or timeout degrades exactly like a mermaid
syntax error, and a third party can build a working plugin from the
template README alone.

### Phase 2 — Pure surface growth (A2, A3, B2, B3) + workspace-read
Inline renderers and span decorators on the display-transform and
span pipelines; on-save formatters and paste processors on the flush
and paste paths; the `workspace-read` capability with consent UX.
First-party: inline math (typst-compiled-to-wasm if viable, else
deferred to its own cycle), TODO-highlight decorator, HTML-paste
cleanup. *Exit criteria:* capability consent flow shipped; a
decorator over a 10k-file workspace cannot block the UI thread.

### Phase 3 — Workspace & data (D1–D4): fs-write + net
Exporters/importers/publishers/enrichers; `fs-write` via
user-picked-path handles; `net:<domain>` via a host-mediated HTTPS
fetch function. First-party: HTML exporter, URL-title enricher.
Third-party target: PDF export, Notion importer. *Exit criteria:* a
plugin with no net capability provably cannot fetch (test), consent
persists in settings and is revocable.

### Phase 4 — Grammars (C1)
tree-sitter-wasm loading in the highlight layer; grammar extensions
declare file extensions + highlight queries; GraphQL ships as the
first-party proof (closing the oldest open item in the project).
*Exit criteria:* a grammar plugin highlights fences and standalone
files with zero core recompile.

### Phase 5 — UI surfaces + automation (E1–E3, F1–F2)
Declarative UI tree (data + layout description, rendered native);
panels/status widgets/custom viewers; hooks and templates.
First-party: word-count widget, daily-note template, CSV viewer.
*Honest note:* this phase is the most likely to be reshaped by what
1–3 teach; the roadmap records direction, not contract.

### Sequenced but independent: registry/marketplace
A git-based extensions index (Zed's model) once third-party authors
exist; in-app browse/install. Deliberately unscheduled until Phase 3
is real.

## Dependency notes

- Phases 2–5 all ride Phase 1's runtime, manifest, and consent
  scaffolding (consent UI lands in Phase 2 with its first consumer).
- Phase 4 is independent of 2–3 and can be pulled forward if grammar
  demand (GraphQL) outweighs data features.
- The command palette built in Phase 1 becomes the surface for every
  later command-shaped extension.
