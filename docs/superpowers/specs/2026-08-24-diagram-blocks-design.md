# Projector Registry & Diagram Blocks Design

**Date:** 2026-08-24
**Status:** Approved for planning
**Origin:** Vision doc ideas #1 (projector registry) and #3 (diagrams as
first-class blocks), `docs/vision/2026-08-23-evolution-wild-to-wildest.md`.

## Purpose

Two things, deliberately coupled: (a) the load-bearing refactor that
turns block widgets into registered *projectors* so future block kinds
(math, calculators, …) plug in instead of forking the projection; and
(b) its first new consumer — ` ```mermaid ` fences rendering as live,
theme-native diagrams with the same touch-to-dissolve contract tables
already honor. Diagrams are pictures until you touch them; the source
of truth stays plain CommonMark on disk.

## Crate research (settled)

- **`merman` 0.8.0-alpha.5** (MIT/Apache-2.0): headless Mermaid in pure
  Rust — parse, layout, themed SVG out; tracks mermaid 11.x, all 35
  diagram families; no JS runtime. Adopted by Zed as their Mermaid
  backend (same pre-1.0-but-production posture as GPUI). Pinned exact
  (`=0.8.0-alpha.5`); alpha crates may break API between releases.
- **`resvg` 0.48** (+ its `usvg`/`tiny-skia`): SVG → RGBA raster at 2×.
  Needed because GPUI's `svg()` is monochrome-tint only; full-color
  diagrams enter the scene as images.
- Evaluated and deferred: `mermaid-rs-renderer` (Zed compared both,
  chose merman for accuracy), `layout` (pure-Rust Graphviz DOT — the
  natural ` ```dot ` follow-up), `svgbob`, `pikchr` bindings.
- Future unlock noted: merman exposes layout JSON / editor facts —
  the path to node-level hit-testing ("click a node, jump to the
  source line") and to `tree-sitter-mermaid` highlighting inside
  revealed fences. Both out of scope for v1.

## Component 1: Projector registry — `src/editor/projector.rs` (new), `src/editor/projection.rs`, `src/editor/blocks.rs`, `src/editor/mod.rs`

### Item generalization

```rust
// projection.rs
pub enum Item {
    Line(usize),
    Widget {
        projector: usize,                       // index into projectors()
        lines: std::ops::Range<usize>,           // source lines replaced
        payload: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    },
}
```

`Item::Table`/`Item::Image` disappear; their payloads become typed
structs (`TablePayload { lines: Range<usize> }`,
`ImagePayload { line: usize, alt: String, dest: String }`) downcast by
their projectors at render time. `PartialEq` for `Item` compares
`(projector, lines)` — payload equality is not required for the
reset-on-change check because payloads are pure functions of the text,
and the text hash participates via the line ranges. Where that is too
coarse (same range, different content — e.g. an image dest edit inside
one line), `project()` runs on every selection/text change exactly as
today, and `layout_cache`/`list_state.reset` behavior is unchanged.

### The trait

```rust
// projector.rs
pub struct Claim {
    pub lines: std::ops::Range<usize>,
    pub payload: std::sync::Arc<dyn std::any::Any + Send + Sync>,
}

pub trait Projector: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// Pure discovery over the parsed document. `blocks` is the
    /// existing blocks::blocks() output; `line_ranges` the byte range
    /// of each line.
    fn discover(&self, text: &str, blocks: &[crate::editor::blocks::BlockInfo],
                line_ranges: &[std::ops::Range<usize>]) -> Vec<Claim>;
    /// Render the widget. Only this method touches GPUI.
    fn render(&self, ctx: &mut WidgetCtx<'_>) -> gpui::AnyElement;
}

pub struct WidgetCtx<'a> {
    pub editor: &'a gpui::Entity<crate::editor::Editor>,
    pub item_ix: usize,
    pub lines: std::ops::Range<usize>,
    pub payload: &'a std::sync::Arc<dyn std::any::Any + Send + Sync>,
    pub theme: &'a crate::theme::Theme,
    pub cx: &'a mut gpui::App,
}

/// Registered in priority order; earlier projectors claim first.
pub fn projectors() -> &'static [&'static dyn Projector]; // [Table, Image, Diagram]
```

### Central invariants (unchanged, now provably single-sourced)

`project()` keeps sole ownership of: (a) the dissolve rule — a claim
whose line range intersects the selection's line span projects raw
`Item::Line`s instead of the widget; (b) overlap resolution — claims
sorted by (start line, registry order), first claim wins, losers are
dropped; (c) fence-delimiter-line omission and everything else it does
today. `item_of_line` gains no new semantics.

### Port with zero behavior change

`TableProjector` and `ImageProjector` wrap today's discovery (from
`blocks()`) and today's `render_table`/`render_image` bodies. Every
existing projection/blocks/editor test must pass unchanged — that is
the port's acceptance gate. The `blocks()` function itself is
untouched (fences/tables/images discovery is shared infrastructure).

## Component 2: Diagram engine — `src/diagram.rs` (new)

```rust
/// Mermaid source → themed SVG. Pure-ish (no GPUI); errors are
/// human-readable one-liners from merman.
pub fn to_svg(source: &str, theme: &DiagramTheme) -> Result<String, String>;

/// SVG → (RGBA bytes, width_px, height_px) at `scale` (2.0 = retina).
pub fn rasterize(svg: &str, scale: f32) -> Result<(Vec<u8>, u32, u32), String>;

/// Palette handed to mermaid themeVariables, derived from Theme:
/// background, text, accent, muted, border, mono/body font families.
pub struct DiagramTheme { /* fields above, plus fingerprint() -> u64 */ }
```

Implementation notes: merman is driven with mermaid's `base` theme +
`themeVariables` built from `DiagramTheme` so output matches the app in
both appearances (exact merman config API resolved against its docs at
implementation time; the wrapper signature above is the frozen
contract). `rasterize` goes usvg parse → tiny-skia pixmap. Both
functions run on the background executor only.

### Cache — global, shared by editor and reader

```rust
pub struct DiagramCache(HashMap<DiagramKey, DiagramState>);
impl gpui::Global for DiagramCache {}

pub struct DiagramKey { source_hash: u64, theme_fingerprint: u64, width_bucket: u32 }
pub enum DiagramState { Pending, Ready(std::sync::Arc<gpui::Image>), Failed(String) }
```

`width_bucket` = target CSS width rounded to 64 px so window resizes
don't thrash renders. Lookup miss → insert `Pending`, spawn background
render, store result, `cx.refresh()`/notify. A theme switch changes the
fingerprint; old entries are dropped lazily (cache capped at 128
entries, LRU by insertion order — a `VecDeque` of keys suffices).

## Component 3: Diagram projector — in `src/editor/projector.rs`

- **Discovery:** fenced blocks with `lang == "mermaid"` (from the
  existing `FenceInfo`), claiming the whole fence including delimiter
  lines. Payload: `DiagramPayload { body: String, body_hash: u64 }`.
- **Render states** (widget fits the reading column, centered):
  - `Ready`: the image at its natural aspect, `max_w_full`, click →
    dissolve to raw fence lines (standard rule — the claim is simply
    not honored while selection is inside).
  - `Pending`: a quiet rounded box at estimated height (min 120 px)
    with a muted "diagram…" label.
  - `Failed(msg)`: the fence renders as the normal highlighted code
    block it would be without this feature, topped by a slim strip in
    `diff_deleted_bg`/`fg` colors with the error text. Broken syntax
    never hides source and never blocks editing.
- Editing inside the fence (revealed) recomputes on the normal restyle
  path; the hash keys the cache so unchanged diagrams cost nothing.

## Component 4: Reader/preview support — `src/markdown.rs`, `src/view.rs`

`Block::Code { lang: Some("mermaid"), .. }` renders through the same
cache in `view.rs`: Ready → image element; Pending/Failed → the same
states as the editor widget (Failed falls back to the highlighted code
block + error strip). No new parsing — the block model already carries
fence source. The welcome document gains a small mermaid flowchart in
the "Tables and code are live too" section, making the tour show it
off for free.

## Error handling

- merman parse/layout errors → `Failed(msg)` state; never a crash, and
  the source remains fully visible/editable as a code fence.
- resvg failures (malformed SVG) are treated identically.
- Renders are debounced naturally by the cache: a fence being typed in
  is *revealed* (no widget), so no render churn while editing.
- Cache poisoning is impossible: keys include the source hash.

## Testing strategy

- **Registry port:** every existing `projection.rs`/`blocks.rs`/editor
  test passes unchanged. New pure tests: claim priority (two projectors
  claiming overlapping ranges → first registered wins), dissolve rule
  via `project()` with a claim + selection inside/outside.
- **Discovery:** mermaid fences claimed (with/without closing fence —
  unclosed fences are NOT claimed, they stay live-edit like today's
  fence behavior); non-mermaid fences untouched.
- **`diagram.rs`:** known-good flowchart source → SVG contains node
  labels; bad source → Err with non-empty message; rasterize of a
  trivial SVG → correct dimensions at scale 2; `DiagramKey` changes
  with source, theme fingerprint, and width bucket.
- **Reader:** `markdown.rs` keeps mermaid fences as `Block::Code` with
  lang preserved (existing behavior — test asserts it so the view can
  rely on it).
- Widget visuals: compile + manual smoke per repo convention.

## Out of scope (recorded follow-ups)

` ```dot ` via the `layout` crate; node-level hit-testing and
click-to-source via merman layout JSON; `tree-sitter-mermaid`
highlighting inside revealed fences; pan/zoom inside the widget
(image tabs already zoom; the widget fits the column); svgbob/pikchr;
exporting diagrams to files.
