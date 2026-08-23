# Phase 9: Theming + Language Coverage — Design Spec

**Date:** 2026-08-24
**Status:** Approved design, pending implementation plan

## Goal

Custom + prebuilt color themes with live picker and persistence; close
the language gaps: full extension/filename mapping for inkjet's 77
languages, and an extras grammar registry adding XML and GraphQL.

## Decisions already made

- Themes are COLORS ONLY this phase (fonts/sizes stay in code).
- Format: TOML, one appearance per file: `name`, `appearance =
  "light"|"dark"`, hex colors for every Theme color field (surfaces,
  accent/link, code, panels, find, 10 syntax colors).
- Built-ins (compiled in via include_str): Paper + Graphite (today's
  palettes, named), Solarized Light, Solarized Dark, Nord (dark),
  Gruvbox Dark. Custom: `~/.supermd/themes/*.toml`, loaded at startup;
  malformed files skipped with a logged warning.
- Selection model: settings hold a light-theme name and a dark-theme
  name; the existing system-appearance follower switches between them.
  Settings live in `~/.supermd/settings.toml` (first settings file;
  only these two keys for now). Unknown names fall back to
  Paper/Graphite.
- Picker: ⌘T + View menu "Theme…". Overlay grouped Light/Dark; arrow
  keys apply the highlighted theme LIVE; Enter persists (theme name
  written into its appearance's slot); Escape reverts everything.
- Mapping expansion: `language_for_file(path)` (moves to highlight.rs)
  checks exact filenames first (Dockerfile, Makefile, meson.build),
  then a full extension table covering every inkjet language (kt, jl,
  clj, fish, vim, tex, bib, ini, env, scss, rkt, scm, proto, gd, hcl,
  tf, cue, awk, f90, pas, el, diff, wgsl, glsl, ll, asm, m→objc, scad,
  bicep, ada, …). `.fs` stays unmapped (F# vs Forth ambiguity).
- Extras registry: `HighlightConfiguration`s built on inkjet's
  re-exported tree-sitter-highlight 0.23, configured with inkjet's
  HIGHLIGHT_NAMES so capture indices stay unified. XML from
  tree-sitter-xml (bundled query); GraphQL from tree-sitter-graphql
  with Helix's highlights.scm vendored into assets/queries (MPL-2.0,
  attribution file). Extras are consulted before inkjet.
- New deps: serde (derive), toml, tree-sitter-xml, tree-sitter-graphql.

## Testing

TDD: mapping table (extensions, filenames, unmapped cases); theme TOML
round-trip + hex parsing + invalid-file skip; every builtin parses and
matches its declared appearance; settings load/save/defaults (dir
injected for tests); xml/graphql smoke tests. Picker + live preview:
manual shell.

## Out of scope

Fonts/sizes in themes, per-language theme overrides, theme hot-reload,
importing VS Code/Helix theme formats, Vue/Perl/PowerShell grammars.
