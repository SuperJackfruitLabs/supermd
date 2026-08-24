# Website Documentation Implementation Plan — supermd.app/docs

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ten markdown docs pages rendered to `site/docs/` by a small in-repo generator, in the landing page's hand-made style, covering users and plugin authors.

**Architecture:** `docs/site/*.md` + `docs/site/nav.toml` → `examples/build_docs.rs` (pulldown-cmark, existing dep) → committed `site/docs/<slug>/index.html` + patched sitemap. No CI step; Cloudflare Pages serves files. App CI ignores `site/**` and `docs/**` (already landed on master).

**Tech Stack:** pulldown-cmark 0.13, toml (existing dep), hand-written HTML shell mirroring `site/index.html`'s palette.

**Spec:** `docs/superpowers/specs/2026-08-24-website-docs-design.md`

## Global Constraints

- Branch `website-docs` off master; PR when done (docs-only changes skip app CI by design — the generator example still compiles under `cargo test`).
- Task-first, user-friendly voice: no internal jargon ("projector registry", phase numbers), no unexplained abbreviations. Shortcut keys shown for macOS with Windows/Linux equivalents (⌘→Ctrl, ⌥→Alt).
- Pages are plain CommonMark; internal links use `<name>.md` form and the generator rewrites them to `/docs/<slug>/`.
- Generated HTML is self-contained: inline CSS only, no external assets, light/dark via `prefers-color-scheme` using the landing page's exact palette variables.
- Generator tests live in `examples/build_docs.rs` (`#[cfg(test)]`); run with `cargo test --example build_docs` (documented in the file header). Plain `cargo test` still proves the example compiles.
- Content accuracy is sourced from the repo: SHORTCUTS table in `src/workspace.rs:91`, WELCOME.md, `plugins/template/README.md`, the phase specs. Do not invent features.

---

### Task 1: Generator + nav manifest (logic first, TDD)

**Files:**
- Create: `docs/site/nav.toml`
- Create: `examples/build_docs.rs`
- Create: `docs/site/index.md` (stub with real front section so the generator has one true page to render during this task)

**Interfaces (produces):**
- `nav.toml` shape:

```toml
[[pages]]
file = "index.md"        # slug "" → site/docs/index.html
title = "Getting Started"
group = "Using SuperMD"

[[pages]]
file = "editing.md"      # slug "editing" → site/docs/editing/index.html
title = "Editing"
group = "Using SuperMD"
```

- Pure functions in the example (all unit-tested):
  - `parse_nav(toml_src) -> Vec<Page { file, title, group }>` (order preserved; error on duplicate file)
  - `slug_of(file) -> String` (`index.md` → `""`, `editing.md` → `"editing"`)
  - `check_drift(nav, files) -> Result<(), String>` (both directions)
  - `rewrite_links(html, known_slugs) -> String` (`href="editing.md"` → `href="/docs/editing/"`; unknown `.md` targets are an error)
  - `render_page(page, all_pages, markdown) -> String` (full HTML: head with title "<title> — SuperMD Docs", meta description from first paragraph, canonical; header; sidebar with groups + current highlight; content; prev/next from nav order)
  - `patch_sitemap(xml, slugs) -> String` (replaces `<!-- docs -->…<!-- /docs -->` block; idempotent)
  - `internal_links_resolve(all_html, slugs) -> Result<(), String>` (link check over generated output)
- `main()`: read nav + md files → drift check → render all → delete stale files under `site/docs/` → write pages → patch `site/sitemap.xml` → print a summary line per page.

- [ ] **Step 1: Write failing unit tests** in `examples/build_docs.rs` `#[cfg(test)]` covering: nav parse order + duplicate error; slug mapping; drift both directions; link rewrite (valid, unknown-target error, external links untouched, anchors preserved `editing.md#tables` → `/docs/editing/#tables`); sitemap patch idempotence + one `<url>` per slug; render smoke (title tag, meta description from first paragraph, sidebar contains every title, current page marked `class="current"`, prev/next correct at both ends, fenced code renders `<pre><code`); link check catches a dangling internal href.

- [ ] **Step 2: Run `cargo test --example build_docs`** — verify every test fails (missing fns), then implement the pure functions minimally and iterate to green. The HTML shell: copy the palette block from `site/index.html` (lines ~40-55: `--bg/--fg/--strong/--muted/--accent` in light + `prefers-color-scheme: dark`), system font stack, content column `max-width: 44rem`, sidebar `220px` fixed on ≥900px viewports, stacked links above content below 900px (pure CSS media query), `.doc pre` style for code blocks.

- [ ] **Step 3: `main()`** wiring (thin I/O over the pure fns) + `docs/site/index.md` stub (real opening: what SuperMD is + download links; full content lands in Task 2). Run the generator; eyeball `site/docs/index.html` in a browser.

- [ ] **Step 4: Commit** — "feat: docs generator with nav manifest and page shell".

---

### Task 2: "Using SuperMD" content (6 pages)

**Files:**
- Create/complete: `docs/site/{index,editing,shortcuts,diagrams,workspace,themes}.md`
- Modify: `docs/site/nav.toml` (all six entries)

Content sources and page outlines (write full prose, not stubs):

- `index.md` — What SuperMD is (hybrid Markdown editor: clean typography until you touch a thing, then raw markers in place; plain CommonMark on disk, no lock-in). Install: macOS (dmg → drag to Applications; first-launch "move to Applications" prompt; signed + notarized), Linux (.deb / tar.gz), Windows (installer; unsigned note if applicable — check release notes before writing). First launch: welcome tour is a real editable document; Open Folder; recent workspaces. End with "next: Editing".
- `editing.md` — the hybrid model narrated like WELCOME.md's tour (adapt its examples); headings/bold/links/quotes reveal-on-touch; checkboxes click to toggle; tables dissolve on click; code fences with highlighting (78+ languages); ⌘E preview; focus mode; autosave + conflict-safe backups (never clobbers external edits).
- `shortcuts.md` — the full table from `src/workspace.rs:91` (General / Editor / Sidebar groups) as markdown tables with macOS and Windows/Linux columns (⌘→Ctrl, ⌥→Alt, ⌃⌘F→Ctrl-Alt-F per `src/platform.rs` translation rules — verify each against `platform::keybinding`).
- `diagrams.md` — mermaid + graphviz/dot fences render as themed diagrams; click to edit source, click away to re-render; diagrams follow the active theme; failures show the error inline, never crash.
- `workspace.md` — Open Folder; go-to-file (⌘P); project-wide search (⌘⇧F, smart-case); git awareness: modified-file dots, Show Changes diff vs HEAD (⌘⇧D); recents + reopen-last; file associations.
- `themes.md` — theme picker (⌘T), light/dark follows the system, built-in themes, custom themes as TOML files in the config dir (shape documented from `src/theme.rs` theme-file keys — verify key names before writing).

- [ ] **Step 1:** Write all six pages; run the generator; fix drift/link errors it reports.
- [ ] **Step 2:** Proofread pass: every keyboard shortcut cross-checked against the SHORTCUTS table; every feature claim checked against the repo (no vaporware).
- [ ] **Step 3:** Commit — "docs: user guide pages".

---

### Task 3: "Extending SuperMD" content (4 pages)

**Files:**
- Create: `docs/site/{plugins,writing-plugins,capabilities,grammars}.md`
- Modify: `docs/site/nav.toml`

Content sources: `plugins/template/README.md`, phase specs, `src/extensions.rs` manifest shapes.

- `plugins.md` (user-facing) — what installed plugins can add (rendered blocks, palette commands, inline replacements, formatters, paste transforms, exporters, file viewers, status widgets, templates, save hooks, syntax highlighting for new languages); install = drop a folder in `~/.supermd/plugins/` + Reload Plugins; the safety model in plain words: plugins run sandboxed, pure by default; reading your workspace or talking to a website each needs your one-time consent (per website domain); plugins can never run programs, touch files outside what you granted, draw their own UI, or run in the background; a misbehaving plugin is cut off after 2 seconds and the app carries on; load failures appear at the bottom of the command palette.
- `writing-plugins.md` (author-facing) — copy the template; `plugin.toml` fields with a complete example; pick your surfaces (table mapping manifest keys → WIT exports); build (`rustup target add wasm32-wasip2`, `cargo build --release --target wasm32-wasip2`); install + Reload Plugins; troubleshooting (from template README). Link to the template dir on GitHub.
- `capabilities.md` (author-facing contract) — pure by default (no fs/env/net/clock); `workspace-read`: read-only mount at `/workspace` after consent; `net`: `host-api.fetch`, https-only, per-domain consent prompted on first use, 5 s / 2 MB / 4 fetches per call, redirects only within granted domains; exporters/templates return content — the host owns every path; deadlines (2 s compute; net calls get a network budget) and crash isolation (a panicking plugin returns an error and gets a fresh instance).
- `grammars.md` — layout (plugin.toml + grammar.wasm + highlights.scm, no component wasm needed), `[[grammars]]` manifest with extensions, `scripts/build_grammar_wasm.sh` (tree-sitter CLI 0.23 + emscripten, one-time), Helix-style capture names in highlights.scm, built-ins win name collisions, multi-grammar `files` stems.

- [ ] **Step 1:** Write all four pages; regenerate; fix reported errors.
- [ ] **Step 2:** Accuracy pass against `src/extensions.rs` (manifest field names, limits: MAX_FETCHES_PER_CALL=4, MAX_RESPONSE_BYTES=2MB, deadlines) — copy exact values.
- [ ] **Step 3:** Commit — "docs: plugin author guide pages".

---

### Task 4: Cross-links, generation, review, PR

**Files:**
- Modify: `site/index.html` (header/footer "Docs" link → `/docs/`)
- Modify: `WELCOME.md` (one line pointing to supermd.app/docs)
- Modify: `site/sitemap.xml` (add the `<!-- docs -->` marker block the generator patches)
- Generated: `site/docs/**` committed

- [ ] **Step 1:** Add the sitemap marker block; add the "Docs" link to the landing page nav and footer (match existing link styling); add the WELCOME.md pointer under its closing section.
- [ ] **Step 2:** `cargo run --example build_docs` → commit generated output. `cargo test --example build_docs` green; `cargo test` green (app untouched, but proves the example compiles).
- [ ] **Step 3:** Visual review: open `site/docs/index.html` and one page per group locally in a browser (light + dark via OS toggle); screenshot for the record.
- [ ] **Step 4:** WELCOME.md changed → app CI will run on the PR (expected: it's compiled in). Push branch, open PR "Docs site: user guide + plugin author guide", body summarizing structure + generator.

## Self-Review Notes

- Spec coverage: content pages ✔ (T2/T3 match the spec's ten pages), generator behaviors ✔ (T1 — clean URLs, link rewrite, drift, sitemap, stale-file deletion), shell ✔ (T1 Step 2 mirrors spec), cross-links ✔ (T4), testing ✔ (T1 unit tests + T4 link check runs inside generation via `internal_links_resolve`).
- No placeholders: every page has an outline with named sources; exact repo locations cited for facts that must be verified (shortcuts, theme keys, limits).
- Consistency: slugs/URLs identical across `slug_of`, `rewrite_links`, `patch_sitemap`, canonical tags.
