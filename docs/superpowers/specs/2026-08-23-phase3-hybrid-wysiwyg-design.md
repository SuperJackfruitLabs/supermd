# Phase 3: Hybrid WYSIWYG — Design Spec

**Date:** 2026-08-23
**Status:** Approved design, pending implementation plan
**Depends on:** Phase 2 (styled-source editing engine, spec `2026-08-23-phase2-editor-design.md`)

## Goal

Hide Markdown syntax markers when the cursor is elsewhere — the product's
signature behavior. `**bold**` renders as **bold** until the cursor
touches that span, at which point the raw markers reveal in place.
Editing state remains entirely in source space; the file on disk is
always the plain CommonMark the buffer holds.

## Decisions already made

- Reveal model: **span under cursor** (Typora/Obsidian behavior), not
  whole-line reveal.
- Hide scope (all four): inline markers (`**`, `*`/`_`, `~~`, backticks,
  link syntax), heading `#` marks, list/quote dress-up (`- ` → `•`,
  `>` → `▍`; ordered markers stay but styled), fence delimiters fade
  (styling only — they never hide).
- Approach A: per-line display transform with an explicit
  source↔display segment map. Approach B (zero-width styling) rejected —
  GPUI has no zero-width run mechanism. Approach C (document-level
  projection) deferred to the tables/images phase.
- Layout reflow on reveal/hide is accepted (industry standard).
- Line count is always preserved: every transform is intra-line.

## Module architecture

```
src/editor/display.rs   — line transform + src↔disp mapping   (pure, TDD)
src/editor/spans.rs     — add StyleKind::FenceDelimiter        (pure, TDD)
src/editor/mod.rs       — geometry routed through the map      (thin shell)
```

`buffer.rs`, `core.rs`, `movement.rs`, `autosave.rs`: **no changes**.
The byte-offset identity invariant from Phase 2 breaks only inside
`display.rs`; every other module still speaks source offsets.

## display.rs — the transform

```rust
pub struct Seg {
    pub src: Range<usize>,   // absolute source byte range
    pub disp: Range<usize>,  // range in DisplayLine::text (line-local)
    pub kind: SegKind,       // Verbatim | Hidden | Replacement
}

pub struct DisplayLine {
    pub text: String,
    pub segs: Vec<Seg>,
}

pub fn display_line(
    line: &str,
    line_start: usize,
    spans: &[StyleSpan],
    selection: Range<usize>,
) -> DisplayLine;

pub fn src_to_disp(dl: &DisplayLine, src: usize) -> usize;
pub fn disp_to_src(dl: &DisplayLine, disp: usize) -> usize;
```

### Transform rules (applied only when the span is NOT revealed)

| Construct | Transform |
| --- | --- |
| Strong | hide the actual delimiters read from source (`**` or `__`), both ends |
| Emphasis | hide `*` or `_`, both ends |
| Strikethrough | hide `~~`, both ends |
| Inline code | hide the leading and trailing backtick runs (length counted from source) |
| Link | bracket scanner finds `[text](dest)`; hide `[` and `](dest)`, keep inner text. If the scan fails (malformed/autolink), the span stays fully visible |
| Heading | hide leading `#{1..6}` plus one following space |
| Bullet list marker | replace `- ` / `* ` / `+ ` with `• ` (accent-styled). Ordered markers (`1. `) are never hidden — styled only |
| Quote marker | replace the `>` run with `▍` |
| Fence delimiter | never hidden — new `StyleKind::FenceDelimiter` span renders the ``` line faded and small (pure styling) |

Directives are collected as (source range, action), sorted by start;
an overlapping later directive is dropped (first-wins). Deterministic
and covered by tests.

### Reveal rule

A span is revealed iff its range **touches** the selection, boundaries
inclusive: `span.start <= sel.end && sel.start <= span.end` where `sel`
is the normalized selection (cursor = empty range). Consequences:
- Cursor inside or at either edge of `**bold**` → markers visible.
- Heading spans cover the whole line → `#` visible whenever the cursor
  is anywhere on that line.
- Drag selections reveal every span they touch.
- Typing inside a revealed span keeps it revealed.

### Mapping semantics

- `src_to_disp`: verbatim bytes map exactly; bytes inside a hidden
  segment snap to the segment's display edge; a replacement's source
  start maps to the replacement's display start, its source end to the
  replacement's display end, and source bytes strictly inside the
  replaced range snap to the replacement's display end. Offsets past
  the line map to `text.len()`.
- `disp_to_src`: verbatim maps exactly; a display offset at a
  replacement's start maps to the source start, anywhere else in the
  replacement maps to the source end (clicking the bullet puts the
  cursor at content start).
- Hidden segments carry a **bias** discovered during implementation:
  opening markers bias Right (a click on the shared display boundary
  lands after the marker, at content start), closing markers bias Left
  (before the marker, at content end). Either resolution touches the
  span, so it reveals.
- Round trip: `disp_to_src(src_to_disp(x)) == x` for every source byte
  in a verbatim segment, EXCEPT the single byte immediately following a
  hidden/replaced segment — that display boundary is shared with the
  marker and resolves to the marker's content side by bias (one display
  offset cannot invert to multiple source offsets).

## spans.rs — FenceDelimiter

`markdown_spans` gains `StyleKind::FenceDelimiter` spans covering the
fence delimiter lines (the whole-code-block range from pulldown minus
the body range already computed by `fence_infos`). `line_kinds` is
unchanged (delimiter lines are already `Code`).

## editor/mod.rs — geometry threading (thin shell)

- The list closure computes, per visible line:
  source attrs (existing pipeline) → `display_line(...)` → projected
  display attrs (verbatim segments copy their slice; replacements
  inherit the attr at their source start; hidden segments drop) →
  compressed `TextRun`s over the display text.
- `LineElement` receives display text/runs plus the `DisplayLine`;
  `CachedLine` stores the `DisplayLine` for event handling.
- Geometry paths route through the map:
  - caret: source head → `src_to_disp` → `position_for_index`
  - selection quads: endpoints via `src_to_disp` (clamped per line)
  - mouse: `closest_index_for_position` → display index → `disp_to_src`
  - IME `bounds_for_range` / `character_index_for_point`: same
  - vertical movement: operates in display space (unchanged), converts
    the final display index to source via `disp_to_src`
- FenceDelimiter styling: faded (`fg_muted`), small (11px) mono.
- No caching across frames: transforms recompute per visible line per
  render, so reveal state follows the selection with no invalidation
  bookkeeping.

## Behavior guarantees

- Copy/cut always yield **source** text (markers included).
- Undo/redo, autosave, ⌘E preview, outline: unaffected.
- Files over `MAX_STYLED_BYTES` (1 MB) have no spans → no transforms →
  behave exactly as Phase 2 plain text.
- A transform bug can never corrupt the file: `display.rs` output is
  render-only; the buffer is untouched by definition.

## Out of scope for Phase 3

Tables/images as widgets, collapsing fence delimiter lines, checkbox
glyphs for task lists, ⌘-click to open links (stretch, not committed),
any cross-line transform (future document-projection layer).

## Testing strategy

TDD (red-green-refactor) for the pure modules:

- display.rs: byte-exact transform output per construct; reveal-rule
  boundary cases (cursor at span start/end/one-past); mapping round
  trips for verbatim bytes; snap semantics for hidden and replacement
  segments; overlap first-wins; link scanner on well-formed, nested-
  bracket, and malformed input; ordered lists and fence delimiter lines
  pass through untransformed.
- spans.rs: FenceDelimiter ranges for fenced blocks (with and without
  language info, tilde fences).
- editor/mod.rs remains the logic-free shell; manual verification:
  markers fold/unfold while typing across spans, clicks land correctly
  on transformed lines, selection quads match visible text, IME
  composition inside transformed lines.
