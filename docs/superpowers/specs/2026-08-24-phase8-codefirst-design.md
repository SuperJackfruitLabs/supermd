# Phase 8: Code-First Editing + inkjet Migration — Design Spec

**Date:** 2026-08-24
**Status:** Approved design, pending implementation plan

## Goal

Make code files first-class in the editor (full-width layout, line-number
gutter, auto-indent), migrate highlighting to inkjet (78 languages, one
dependency), and fix two go-to-file dialog defects (selection background
width; list not scrolling with arrow keys).

## Decisions already made

- inkjet replaces the 30 per-grammar crates AND our tree-sitter-highlight
  0.26 runtime (inkjet pins 0.23; two tree-sitter C runtimes cannot
  coexist). `Languages::highlight`'s public shape stays: byte-range spans
  with capture indices — now into inkjet's Helix-style HIGHLIGHT_NAMES,
  which our prefix-matching color map already handles. All existing
  grammar smoke tests must keep passing; the vendored D query and
  assets/queries go away if inkjet covers D (it does).
- Code mode applies to Provider::Code and Provider::Plain files:
  full-width layout (16px padding, no 760px column), line-number gutter
  (muted, right-aligned, width = digit count of line total, mono), and
  Enter copies the previous line's leading whitespace (auto-indent).
  Markdown keeps the prose column exactly as today.
- Deferred: no-wrap + horizontal scroll, bracket pairing, language-aware
  indent depth, gutter click to select line.
- Finder fixes: result rows span the full pane width (selection bg too);
  the list tracks a UniformListScrollHandle and scrolls the selected row
  into view on arrow navigation and refilter.

## Design notes

- highlight.rs: `Languages::new()` builds nothing (inkjet lazy-loads
  per-language configs internally or we hold a map of
  inkjet::Language values); `highlight(lang, src)` runs inkjet's raw
  highlight events into `(Range, u8)` spans; `CAPTURE_NAMES` becomes
  inkjet's name table (re-exported or copied constant). Language name
  canonicalization moves to inkjet::Language::from_token which accepts
  aliases/extensions natively — keep our alias match as fallback.
- editor/mod.rs: `is_code_mode()` from provider; the list closure and
  LineElement wrapper choose layout per mode; gutter is a flex_none
  sibling of LineElement inside the row (not part of the shaped text, so
  all byte-offset math is untouched). Mouse hit-testing already works on
  the LineElement bounds; the gutter area is inert.
- core.rs: `insert_newline_auto_indent(&mut self, now)` — inserts "\n" +
  leading whitespace of the current line (up to the cursor's line);
  pure, TDD. The editor's Newline action uses it only in code mode
  (markdown keeps plain newline — list continuation is a future feature).

## Testing

- Grammar smoke tests unchanged and green post-migration (+ smoke cases
  for newly gained languages: dockerfile, julia, perl).
- auto-indent: TDD in core (tabs, spaces, empty previous indent, cursor
  mid-line).
- Gutter width helper `gutter_cols(line_count)` TDD (1→1, 9→1, 10→2,
  9999→4).
- Finder fixes + layout: manual.
