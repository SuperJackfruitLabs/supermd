# Website Documentation Design — supermd.app/docs

**Date:** 2026-08-24
**Status:** Approved for planning
**Branch:** `extensions` (docs describe the plugin system this branch
ships; they land with its PR)

## Purpose

User-friendly documentation on supermd.app for two audiences: people
using SuperMD and people writing plugins for it. Task-first voice, no
internal jargon (no "projector registry", no phase numbers). The
substance largely exists in-repo (WELCOME.md, the SHORTCUTS table,
plugins/template/README.md, the specs) and is adapted, not rewritten
from scratch.

## Architecture

- **Content**: markdown files in `docs/site/`, one page each, plain
  CommonMark (what the app itself edits — dogfooding).
- **Generator**: `examples/build_docs.rs`, run manually via
  `cargo run --example build_docs`. Renders with pulldown-cmark
  (existing dependency), wraps pages in a shared HTML shell, writes
  `site/docs/*.html`, and patches `site/sitemap.xml`.
- **Output committed**: `site/docs/` is checked in (same policy as
  og.png and grammar.wasm). Cloudflare Pages stays a dumb file host;
  no CI build step, no framework.
- **Navigation manifest**: `docs/site/nav.toml` — ordered entries
  `(file, title, group)` with two groups: "Using SuperMD" and
  "Extending SuperMD". The sidebar, prev/next links, and sitemap all
  derive from it.

## Pages (v1)

Using SuperMD:
1. `index.md` — what SuperMD is; install per platform (dmg + Gatekeeper
   note, deb/tar, Windows installer); first launch and opening a folder.
2. `editing.md` — the hybrid model: markers hide until the cursor
   touches them; headings, lists, quotes, links, checkboxes, tables;
   preview (⌘E) and focus mode.
3. `shortcuts.md` — the full shortcut table, macOS and Windows/Linux
   columns.
4. `diagrams.md` — mermaid and graphviz fences; themed rendering;
   reveal-to-edit.
5. `workspace.md` — open folder, go-to-file, project search, git
   change view, recents, autosave/backups.
6. `themes.md` — built-in themes, theme picker, custom TOML themes.

Extending SuperMD:
7. `plugins.md` — what plugins can do (blocks, commands, inline,
   formatters, paste, exporters, viewers, widgets, templates, hooks,
   grammars); the install dir; the capability model in user terms:
   pure by default, consent for workspace-read and per-domain net,
   what plugins can never do (spawn processes, touch arbitrary files,
   draw pixels, run in the background); the load-failure report.
8. `writing-plugins.md` — template walkthrough: plugin.toml, the WIT
   world, implementing a surface, building with wasm32-wasip2,
   installing and Reload Plugins, troubleshooting.
9. `capabilities.md` — author-facing contract: workspace-read preopen,
   host-mediated fetch (https-only, 2 MB / 5 s / 4 fetches, redirect
   containment, consent errors), exporters/templates never choose
   paths, deadlines and poison-recovery.
10. `grammars.md` — grammar plugin layout (plugin.toml + grammar.wasm
    + highlights.scm), build_grammar_wasm.sh, extensions and fences,
    built-ins win collisions.

## Page shell

Same hand-made look as the landing page: its palette variables
(light/dark via prefers-color-scheme), system font stack, inline CSS
(no external assets). Layout: fixed header (SuperMD wordmark → /,
Download, GitHub), left sidebar (two groups from nav.toml, current
page highlighted), content column (max-width ~44rem), prev/next
footer links. Sidebar collapses to a top list on narrow screens
(pure CSS). Code blocks styled like the landing page's `.doc pre`.

## Generator behavior

- Reads nav.toml; fails loudly if a nav entry has no file or a
  `docs/site/*.md` file is absent from nav (drift check).
- Renders each page: title from the first H1; `<title>` =
  "<page> — SuperMD Docs"; meta description from the first paragraph;
  canonical URL `https://supermd.app/docs/<slug>/` — output files are
  `site/docs/<slug>/index.html` for clean URLs (index.md →
  `site/docs/index.html`).
- Rewrites internal links: `editing.md` → `/docs/editing/`.
- Patches sitemap.xml: replaces a marked docs block
  (`<!-- docs -->`…`<!-- /docs -->`) with one `<url>` per page.
- Deletes stale files under site/docs/ not produced by this run.

## Cross-linking

- Landing page header/footer gain a "Docs" link.
- WELCOME.md (in-app tour) gains a pointer to supermd.app/docs.
- Docs pages link to the latest-release download and the repo.

## Testing

Generator logic lives in the example file with unit tests run by
`cargo test --example build_docs` (or `#[cfg(test)]` in the example):
- nav.toml parsing (groups, order); nav↔files drift both directions.
- Markdown→HTML smoke: headings, fences, tables, links survive.
- Internal-link rewrite and a link-check: every internal href in the
  generated HTML resolves to a generated page or existing site file.
- Sitemap patch is idempotent and contains every page exactly once.
Voice/content quality is reviewed by hand (screenshots optional,
reusing existing PNGs only).

## Out of scope (v1)

Search, versioned docs, automated screenshots, docs subdomain,
comments/analytics, i18n, a "changelog" page (releases page covers
it).
