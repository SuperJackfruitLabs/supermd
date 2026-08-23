# Phase 4: Document Projection — Design Spec

**Date:** 2026-08-23
**Status:** Approved design, pending implementation plan
**Depends on:** Phase 3 (hybrid WYSIWYG, spec `2026-08-23-phase3-hybrid-wysiwyg-design.md`)

## Goal

Cross-line constructs render as real widgets while the cursor is
elsewhere: tables as proper tables, whole-line images as the actual
image, fenced code blocks without their ``` delimiter lines. Touching a
block with the selection dissolves it back into ordinary editable
source lines. This extends Phase 3's span-level reveal rule to block
granularity.

## Decisions already made

- Tables: **whole-table reveal** (widget ↔ raw pipe lines). In-place
  cell editing is explicitly out (future phase; Approach C successor).
- Images: **local + remote**. Local paths resolve against the file's
  parent directory; remote http(s) through gpui's async image loading
  and cache. Only images whose markup is the entire trimmed content of
  their line render as blocks; inline images stay Phase 3 text.
- Fences: delimiter lines are omitted while the block is untouched;
  body lines always remain individually editable `Line` items.
- Approach A (block-projection list). B (overlays) rejected — geometry
  lies. C (full projected document / piece table) deferred; A's
  projection is a strict subset C can absorb.
- Accepted roughness: list reset when blocks form/dissolve may adjust
  scroll (mitigated by cursor-follow); table cells render plain text
  (rich cells stay a ⌘E preview feature); remote image failure UI is
  whatever gpui's element provides, else raw-markup fallback.

## Module architecture

```
src/editor/blocks.rs      — block discovery                    (pure, TDD)
src/editor/projection.rs  — items, reveal, line↔item mapping   (pure, TDD)
src/editor/mod.rs         — item list + table/image widgets     (thin shell)
```

`buffer.rs`, `core.rs`, `movement.rs`, `display.rs`, `autosave.rs`:
no changes. Phase 3's per-line transform continues to run inside every
`Line` item.

## blocks.rs — discovery

```rust
pub enum BlockKind {
    Table,
    Image { alt: String, dest: String },
    Fence { open_line: Range<usize>, close_line: Range<usize> }, // byte ranges of delimiter lines
}
pub struct BlockInfo { pub range: Range<usize>, pub kind: BlockKind }
pub fn blocks(source: &str) -> Vec<BlockInfo>
```

- Tables: pulldown `Tag::Table` offset range.
- Images: `Tag::Image` offset range; alt from inner text events, dest
  from the tag. Qualifies only if the trimmed content of its line
  equals the image markup exactly.
- Fences: from the existing `fence_infos`; only fenced blocks (never
  indented) produce delimiter info. An unclosed fence at EOF produces
  no close-delimiter range and its delimiters are never omitted.
- Sources over `MAX_STYLED_BYTES` return no blocks.

## projection.rs — the item list

```rust
pub enum Item {
    Line(usize),                                  // source line index
    Table { lines: Range<usize> },                // line index range
    Image { line: usize, alt: String, dest: String },
}
pub fn project(lines: &[Range<usize>], blocks: &[BlockInfo], selection: Range<usize>) -> Vec<Item>
pub fn item_of_line(items: &[Item], line: usize) -> usize
```

- **Block reveal rule:** a block dissolves iff
  `block.range.start <= sel.end && sel.start <= block.range.end`
  (inclusive touch — identical shape to Phase 3's span rule).
- Untouched Table → one `Table` item consuming its lines. Untouched
  whole-line Image → one `Image` item. Untouched Fence → its delimiter
  lines are omitted; body lines emit as `Line` items. Touched anything →
  plain `Line` items.
- `item_of_line`: exact item when the line is emitted or consumed by a
  widget; a fence-omitted delimiter line maps to the nearest emitted
  neighbor item; clamped at the ends.
- Overlapping/nested blocks (table inside quote etc.): blocks are
  processed in range order; a block starting inside an already-consumed
  range is skipped (first-wins, same policy as display.rs directives).

## Widgets (shell)

**Table:** pure `parse_row(line) -> Vec<String>` (split on unescaped
`|`, honor `\|`, trim cells, leading/trailing empties from edge pipes
dropped) plus `is_separator_row(line)` (dashes/colons). Rendering:
first row = header (semibold, `panel_bg`), separator skipped, hairline
row borders, equal-width cells — visually consistent with the Reader.
Clicking a row sets the cursor to that row's source line start (block
dissolves next frame).

**Image:** gpui `img` element; `ImageSource` from a resolved local path
(joined to the editing file's parent dir) or remote URI (gpui async
load + cache). Constrained to column width, natural aspect. Missing
local file → raw markup rendered with muted warning tint. Click →
cursor to the image line.

## Editor integration (shell)

- `restyle` computes `blocks` alongside spans.
- `render` recomputes `project(...)` from the current selection;
  compares to the stored projection; only on change: store, 
  `list_state.reset(items.len())`, scroll to the cursor's item.
- List closure dispatches on `Item`; `Line` renders exactly today's
  per-line path (display transform included).
- `reveal_cursor` and outline `scroll_to_line` route through
  `item_of_line`.
- Vertical movement: unchanged. Widgets don't populate the line layout
  cache, so stepping into one uses the logical-line fallback, lands on
  the block's edge line, and dissolves it.

## Behavior guarantees

- Copy/cut yield raw source; the buffer never contains widget state.
- Typing inside a dissolved block keeps it dissolved; a block that no
  longer parses (broken table) simply stops being detected and remains
  ordinary text.
- ⌘A dissolves all blocks (selection touches everything) — intended.
- Files over 1 MB: no blocks, no projection changes — Phase 2 behavior.
- A projection bug can never corrupt the file: render-only, buffer
  untouched.

## Out of scope

In-place cell editing, column alignment/rich text inside table cells,
image resizing/captions, remote image failure UI beyond fallback text,
collapsing anything else cross-line (setext headings, HTML blocks).

## Testing strategy

TDD for the pure modules:
- blocks.rs: table byte ranges; image whole-line rule (positive,
  indented, inline-in-paragraph negative), alt/dest extraction; fence
  delimiter line ranges incl. tilde and unclosed-at-EOF; oversize cap.
- projection.rs: mixed-document item sequences; dissolve at exact
  boundary offsets (start-1, start, end, end+1); fence delimiter
  omission and re-emission; `item_of_line` for emitted, consumed, and
  omitted lines; first-wins nesting.
- parse_row/is_separator_row: escaped pipes, empty cells, edge pipes,
  alignment-colon separators.

Shell (widgets, list reset, scroll follow, image loading) is
manual-verify.
