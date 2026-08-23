# Phase 2: Styled-Source Editing Engine — Design Spec

**Date:** 2026-08-23
**Status:** Approved design, pending implementation plan
**Depends on:** Phase 0 (block renderer), Phase 1 (workspace shell, tree-sitter highlighting)

## Goal

Make supermd's documents editable, in place, with rich typography while
every source byte remains visible (Bear 1.x model): headings render large
with their `#` marks shown, `**bold**` shows its asterisks, fences render
in a tinted mono panel. Applies to Markdown files and code files alike.
Changes autosave to disk with session backups.

Phase 3 (hybrid WYSIWYG, hiding markers) builds directly on this engine;
Phase 2 deliberately preserves the invariant **buffer byte offset ==
rendered text offset** everywhere.

## Decisions already made

- Editing model: continuous styled-source surface (Approach A). Not
  per-block widgets; not a separate plain source pane.
- Save model: autosave everywhere (no dirty-dot ceremony) + first-write
  session backups. Not git-conditional.
- Edit targets: Markdown and code files. Non-UTF-8 files stay read-only.
- Development process: strict TDD (red-green-refactor) for all pure-logic
  modules; GPUI layer stays a thin untested shell.

## Module architecture

```
src/editor/
  buffer.rs    — rope, edits, selection, undo/redo      (pure, TDD)
  spans.rs     — source text → style spans              (pure, TDD)
  autosave.rs  — save policy + session backups          (pure + fs, TDD)
  mod.rs       — Editor entity + GPUI line element      (thin shell)
```

Existing modules are consumed, not modified: `markdown.rs` (Reader
preview), `highlight.rs` (`Languages::highlight` reused by spans.rs),
`reader.rs` (preview mode), `theme.rs` (all colors/typography).

## buffer.rs — text core

- `ropey::Rope` holds the text. **Byte offsets are the universal
  currency** (matches pulldown-cmark, tree-sitter, and GPUI shaping).
- One edit primitive: `replace(range, text) -> Edit`. Typing, deletion,
  paste, and enter are all replaces.
- `Selection { anchor: usize, head: usize }` in bytes; cursor =
  collapsed selection. Multi-cursor is out of scope.
- Movement (all pure functions over the rope):
  - left/right: by grapheme cluster (unicode-segmentation)
  - alt+left/right: by word boundary
  - cmd+left/right: logical line start/end
  - cmd+up/down: document start/end
  - up/down: **view-layer responsibility** (needs wrapped-line geometry);
    core exposes logical-line info the view uses.
- Undo/redo:
  - Edits coalesce into undo groups: same kind (insert vs delete),
    contiguous position, within 700 ms. Group breaks on cursor jump or
    kind change.
  - Redo stack clears on any new edit.
  - Clock is injected (`fn now() -> Instant` style) so grouping is
    unit-testable without sleeping.
- Every mutation returns enough information for the view to invalidate
  affected lines.

## spans.rs — styling

One shape, two providers:

```rust
fn style_spans(source: &str, ...) -> Vec<(Range<usize>, StyleKind)>
```

- `StyleKind`: `Heading(u8)`, `Strong`, `Emphasis`, `Strikethrough`,
  `InlineCode`, `Link`, `ListMarker`, `QuoteMarker`, `FenceDelimiter`,
  `FenceContent`, `Rule`, `Syntax(u8)` (tree-sitter capture index into
  `highlight::CAPTURE_NAMES`).
- **Markdown provider:** pulldown-cmark `Parser::into_offset_iter()`
  supplies byte ranges for block/inline structures. Fence bodies are
  additionally highlighted with the existing `Languages::highlight`,
  emitted as `Syntax(_)` spans shifted by the fence body's start offset.
- **Code provider:** entire file through `Languages::highlight` →
  `Syntax(_)` spans. Language chosen by extension via
  `reader::language_for_path`.
- Spans are recomputed synchronously on each edit (full re-parse).
  Budget: files ≤ 1 MB. Larger files render unstyled (still editable).
  Incremental re-parse is explicitly deferred.
- Line typography derives from spans: a line intersecting `Heading(n)`
  uses the heading type scale; lines inside fences use mono at code
  size; all else body size. This derivation lives in spans.rs (pure,
  tested), not in the view.

## editor/mod.rs — GPUI shell

- `Editor` entity: rope buffer, span cache, per-line layout cache, path,
  language, `ListState` (one item per logical line), focus handle,
  autosave state, preferred-x for vertical movement.
- Rendering: GPUI `list` virtualizes logical lines (variable height
  already supported — heading lines are taller). Each line shapes its
  text with style runs at the current column width; paints its slice of
  selection and the cursor caret.
- Input: `EntityInputHandler` (same IME-correct pattern as
  `input.rs`, multi-line) — IME composition, dead keys, dictation work.
  Keyboard actions bound in `"Editor"` key context.
- Mouse: click positions cursor; shift-click extends; drag selects.
  Double-click word selection is a stretch goal, not required.
- Cursor visibility: `scroll_to_reveal_item(line_ix)` after movement or
  edit.
- Vertical movement (up/down) uses the wrapped-line geometry of the
  shaped lines with a sticky preferred-x column.

## autosave.rs — persistence

- Debounced flush: 1 s after last edit. Immediate flush on: tab switch,
  tab close, app quit, `⌘S`.
- Writes are temp-file-then-rename in the target directory.
- **Session backups:** before the first write to each path per app
  session, copy the on-disk original to
  `~/.supermd/backups/<unix-timestamp>-<filename>`. One backup per file
  per session.
- **External-change safety:** mtime recorded at open and after each of
  our writes. If disk mtime differs from expectation at save time, the
  disk version is backed up (even if this file was already backed up
  this session) before we overwrite, and a warning is logged. Nothing is
  ever silently destroyed.
- The scheduling/backup decision logic is a pure state machine with
  injected clock and fs effects, fully unit-tested; the thin fs wrapper
  does real IO.

## Workspace integration

- A tab holds an `Entity<Editor>` plus an optional preview
  `Entity<Reader>`; `⌘E` toggles edit/preview per tab (preview re-parses
  from the current buffer, so it is always fresh). Real tables/images
  live in preview.
- Sidebar and finder open files into edit mode by default. The Welcome
  tab remains a read-only Reader.
- Outline panel in edit mode is driven by heading spans; clicking
  scrolls the editor to that line. In preview mode it uses the existing
  Reader TOC.
- New `"Editor"`-context bindings: arrows (+shift), alt/cmd arrow
  variants, home/end, pageup/pagedown, backspace/delete (+alt/cmd
  variants), enter, tab (inserts literal tab), `⌘A`, `⌘C/X/V`, `⌘Z`,
  `⇧⌘Z`, `⌘S`, `⌘E`.
- Files that fail UTF-8 decoding open read-only via the existing path.

## Error handling

- Read failure: tab does not open; error logged (status UI later).
- Save failure: buffer stays dirty, error logged, retry on next
  autosave trigger; app never discards buffer contents.
- Span provider failure (grammar missing, parse panic-guard): render
  unstyled; editing unaffected.

## Testing strategy

TDD (red-green-refactor, failing test first) for all pure modules via
`cargo test`:

- buffer: replace semantics, selection normalization, grapheme/word/line
  movement (incl. multi-byte and emoji), undo grouping windows, redo
  invalidation.
- spans: markdown byte ranges (headings, emphasis nesting, inline code,
  lists, quotes), fence body offset shifting, code-file provider,
  oversize-file bypass, line-typography derivation.
- autosave: debounce schedule, flush-now events, backup-once-per-session,
  external-mtime conflict policy — all against injected clock/fs.

The GPUI layer (element shaping, painting, event routing) is the
deliberately thin untested shell; anything in it that grows logic gets
extracted into a tested pure module.

## Out of scope for Phase 2

Hiding syntax markers (Phase 3), incremental parsing, multi-cursor,
find/replace, file watching/live reload, soft-wrap column preferences,
spell check, drag-and-drop text.
