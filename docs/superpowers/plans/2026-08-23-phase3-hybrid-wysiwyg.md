# Phase 3: Hybrid WYSIWYG Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hide Markdown syntax markers when the cursor is elsewhere; reveal the span under the cursor — via a pure per-line display transform with a source↔display segment map.

**Architecture:** New pure module `src/editor/display.rs` (transform + mapping, fully TDD). `spans.rs` gains `StyleKind::FenceDelimiter`. `editor/mod.rs` routes its four geometry paths (caret, selection quads, mouse, IME) through the map. Buffer/core/movement/autosave unchanged.

**Tech Stack:** Rust, existing Phase 2 modules, no new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-23-phase3-hybrid-wysiwyg-design.md`

## Global Constraints

- TDD iron law for `display.rs` and `spans.rs` changes: failing test first, watch it fail, minimal code, watch it pass.
- The buffer is never touched by display code — transforms are render-only.
- Reveal rule everywhere: `span.start <= sel.end && sel.start <= span.end` (inclusive touch), `sel` = normalized selection.
- Replacement strings: bullet `"• "`, quote bar `"▍"` (multi-byte UTF-8 — all mapping code is byte-based).
- Directives sorted by start; overlap resolution is first-wins; empty/out-of-line directives dropped.
- `cargo test` green before every commit; `cargo build` green for shell tasks. Commit per task with this repo's trailer format.

---

### Task 1: `StyleKind::FenceDelimiter` spans

**Files:** Modify `src/editor/spans.rs`

**Interfaces:**
- Consumes: existing `fence_infos`.
- Produces: `StyleKind::FenceDelimiter` variant; `markdown_spans` emits delimiter spans (trailing newline trimmed) around each fenced body. Indented code blocks emit none.

- [ ] **Step 1: Failing tests** (append to spans tests)

```rust
    #[test]
    fn fence_delimiters_spanned() {
        let src = "```rust\nlet x = 1;\n```\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::FenceDelimiter),
            vec![0..7, 19..22]
        );
    }

    #[test]
    fn tilde_fence_delimiters() {
        let src = "~~~\ncode\n~~~\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::FenceDelimiter),
            vec![0..3, 9..12]
        );
    }

    #[test]
    fn indented_code_has_no_delimiters() {
        let src = "para\n\n    indented code\n";
        assert!(spans_of_kind(src, |k| *k == StyleKind::FenceDelimiter).is_empty());
    }
```

- [ ] **Step 2: Run, verify RED** — `cargo test editor::spans` fails: no `FenceDelimiter` variant.
- [ ] **Step 3: Implement** — add the variant; change `fence_infos` to also return the whole block range and a `fenced: bool` (from `CodeBlockKind`); in `markdown_spans` push `FenceDelimiter` for `block.start..body.start` and `body.end..block.end`, both `trim_trailing_newline`'d, only when fenced and non-empty.
- [ ] **Step 4: Run, verify GREEN** — all spans tests pass; full suite green.
- [ ] **Step 5: Commit** — `feat(editor): FenceDelimiter spans for fenced code blocks`

---

### Task 2: display.rs — hide engine core + mapping

**Files:** Create `src/editor/display.rs`; modify `src/editor/mod.rs` (add `pub mod display;`)

**Interfaces (produced, used by Tasks 3–5):**
```rust
pub enum SegKind { Verbatim, Hidden, Replacement }
pub struct Seg { pub src: Range<usize>, pub disp: Range<usize>, pub kind: SegKind }
pub struct DisplayLine { pub text: String, pub segs: Vec<Seg> }
pub fn display_line(line: &str, line_start: usize, spans: &[StyleSpan], selection: Range<usize>) -> DisplayLine
pub fn src_to_disp(dl: &DisplayLine, src: usize) -> usize
pub fn disp_to_src(dl: &DisplayLine, disp: usize) -> usize
```
This task covers Strong/Emphasis/Strikethrough/InlineCode hiding (delimiters read from source, cross-line-safe: leading directive only if span start is on this line, trailing only if span end is), the reveal rule, and both mapping functions with spec snap semantics.

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::spans::{StyleKind, StyleSpan};

    fn span(range: Range<usize>, kind: StyleKind) -> StyleSpan {
        StyleSpan { range, kind }
    }

    #[test]
    fn strong_markers_hide_when_cursor_elsewhere() {
        let dl = display_line("a **bold** b", 0, &[span(2..10, StyleKind::Strong)], 0..0);
        assert_eq!(dl.text, "a bold b");
    }

    #[test]
    fn strong_reveals_when_selection_touches() {
        for sel in [2..2, 5..5, 10..10, 1..3] {
            let dl = display_line("a **bold** b", 0, &[span(2..10, StyleKind::Strong)], sel);
            assert_eq!(dl.text, "a **bold** b");
        }
    }

    #[test]
    fn cursor_one_past_end_hides() {
        let dl = display_line("a **bold** b", 0, &[span(2..10, StyleKind::Strong)], 11..11);
        assert_eq!(dl.text, "a bold b");
    }

    #[test]
    fn underscore_emphasis_and_strike_and_code() {
        let dl = display_line(
            "_it_ ~~no~~ ``x``",
            0,
            &[
                span(0..4, StyleKind::Emphasis),
                span(5..11, StyleKind::Strikethrough),
                span(12..17, StyleKind::InlineCode),
            ],
            100..100,
        );
        assert_eq!(dl.text, "it no x");
    }

    #[test]
    fn mapping_around_hidden_segments() {
        // "a **bold** b" -> "a bold b"
        let dl = display_line("a **bold** b", 0, &[span(2..10, StyleKind::Strong)], 0..0);
        assert_eq!(src_to_disp(&dl, 0), 0);
        assert_eq!(src_to_disp(&dl, 2), 2);  // hidden ** start snaps
        assert_eq!(src_to_disp(&dl, 3), 2);  // inside hidden
        assert_eq!(src_to_disp(&dl, 4), 2);  // 'b' of bold
        assert_eq!(src_to_disp(&dl, 8), 6);  // hidden closing
        assert_eq!(src_to_disp(&dl, 11), 7); // ' ' after
        assert_eq!(disp_to_src(&dl, 2), 4);  // clicking 'b' -> after markers? no: exact verbatim
        assert_eq!(disp_to_src(&dl, 6), 8);
        assert_eq!(disp_to_src(&dl, 7), 11);
    }

    #[test]
    fn round_trip_for_visible_bytes() {
        let line = "x **b** `c` y";
        let spans = [
            span(2..7, StyleKind::Strong),
            span(8..11, StyleKind::InlineCode),
        ];
        let dl = display_line(line, 0, &spans, 100..100);
        for seg in dl.segs.iter().filter(|s| s.kind == SegKind::Verbatim) {
            for src in seg.src.clone() {
                assert_eq!(disp_to_src(&dl, src_to_disp(&dl, src)), src, "byte {src}");
            }
        }
    }

    #[test]
    fn cross_line_span_hides_only_local_delimiter() {
        // Simulates line 2 of "**bold\ntext**": span extends before this line.
        let dl = display_line("text**", 100, &[span(90..106, StyleKind::Strong)], 0..0);
        assert_eq!(dl.text, "text");
        // And line 1: only the leading delimiter is local.
        let dl = display_line("**bold", 84, &[span(84..100, StyleKind::Strong)], 0..0);
        assert_eq!(dl.text, "bold");
    }

    #[test]
    fn no_spans_passthrough_identity() {
        let dl = display_line("plain text", 50, &[], 0..0);
        assert_eq!(dl.text, "plain text");
        assert_eq!(src_to_disp(&dl, 53), 3);
        assert_eq!(disp_to_src(&dl, 3), 53);
    }
}
```

Note on `mapping_around_hidden_segments`: verify each constant by hand against "a bold b" before trusting a RED failure; the `disp_to_src(&dl, 2) == 4` line asserts exact verbatim mapping ('b' of bold sits at display 2, source 4).

- [ ] **Step 2: RED** — `cargo test editor::display` fails to compile (module contents missing).
- [ ] **Step 3: Implement** — directive collection per construct (delimiters detected by reading the line text at the span's local start/end: `**`/`__` 2 bytes, `*`/`_` 1, `~~` 2, backtick runs counted per side); reveal check first; sort + first-wins overlap drop + bounds clamp; `build()` walks directives emitting Verbatim/Hidden/Replacement segs; mapping functions per spec (verbatim exact; hidden snap to display edge; replacement start→start, end→end, interior→end; past-line → `text.len()`).
- [ ] **Step 4: GREEN** — all display tests pass; full suite green.
- [ ] **Step 5: Commit** — `feat(editor): display transform core — inline marker hiding + mapping`

---

### Task 3: Replacements (bullet, quote) + heading hashes

**Files:** Modify `src/editor/display.rs`

- [ ] **Step 1: Failing tests**

```rust
    #[test]
    fn bullet_replacement_and_mapping() {
        let dl = display_line("- item", 0, &[span(0..2, StyleKind::ListMarker)], 100..100);
        assert_eq!(dl.text, "• item"); // "• " is 4 bytes
        assert_eq!(src_to_disp(&dl, 0), 0);
        assert_eq!(src_to_disp(&dl, 1), 4); // interior of replacement -> disp end
        assert_eq!(src_to_disp(&dl, 2), 4);
        assert_eq!(disp_to_src(&dl, 0), 0);
        assert_eq!(disp_to_src(&dl, 2), 2); // inside bullet glyph -> content start
        assert_eq!(disp_to_src(&dl, 4), 2);
    }

    #[test]
    fn bullet_reveals_with_cursor_in_marker() {
        let dl = display_line("- item", 0, &[span(0..2, StyleKind::ListMarker)], 1..1);
        assert_eq!(dl.text, "- item");
    }

    #[test]
    fn ordered_marker_never_transforms() {
        let dl = display_line("1. item", 0, &[span(0..3, StyleKind::ListMarker)], 100..100);
        assert_eq!(dl.text, "1. item");
    }

    #[test]
    fn quote_marker_becomes_bar() {
        let dl = display_line("> quoted", 0, &[span(0..1, StyleKind::QuoteMarker)], 100..100);
        assert_eq!(dl.text, "▍ quoted");
    }

    #[test]
    fn heading_hashes_hide_when_cursor_off_line() {
        let dl = display_line("## Title", 0, &[span(0..8, StyleKind::Heading(2))], 100..100);
        assert_eq!(dl.text, "Title");
        // Cursor anywhere on the line (span covers it) reveals.
        let dl = display_line("## Title", 0, &[span(0..8, StyleKind::Heading(2))], 5..5);
        assert_eq!(dl.text, "## Title");
    }
```

- [ ] **Step 2: RED.** Failures: transforms not implemented for these kinds.
- [ ] **Step 3: Implement** — `ListMarker` starting `-`/`*`/`+` → `Replace("• ")` (digits: no directive); `QuoteMarker` → `Replace("▍")`; `Heading(_)` → hide leading `#` run + one space at span start (only if the run is on this line).
- [ ] **Step 4: GREEN**; full suite green.
- [ ] **Step 5: Commit** — `feat(editor): bullet/quote replacements and heading hash hiding`

---

### Task 4: Link scanner + overlap resolution

**Files:** Modify `src/editor/display.rs`

- [ ] **Step 1: Failing tests**

```rust
    #[test]
    fn link_collapses_to_inner_text() {
        // "see [zed](https://zed.dev) now" — Link span 4..26
        let dl = display_line(
            "see [zed](https://zed.dev) now",
            0,
            &[span(4..26, StyleKind::Link)],
            100..100,
        );
        assert_eq!(dl.text, "see zed now");
    }

    #[test]
    fn link_reveals_on_touch() {
        let dl = display_line(
            "see [zed](https://zed.dev) now",
            0,
            &[span(4..26, StyleKind::Link)],
            8..8,
        );
        assert_eq!(dl.text, "see [zed](https://zed.dev) now");
    }

    #[test]
    fn nested_brackets_in_link_text() {
        // "[a[b]c](u)" span 0..10 -> inner "a[b]c"
        let dl = display_line("[a[b]c](u)", 0, &[span(0..10, StyleKind::Link)], 100..100);
        assert_eq!(dl.text, "a[b]c");
    }

    #[test]
    fn unscannable_link_stays_visible() {
        // Span deliberately not matching [text](dest) shape.
        let dl = display_line("<https://x.y>", 0, &[span(0..13, StyleKind::Link)], 100..100);
        assert_eq!(dl.text, "<https://x.y>");
    }

    #[test]
    fn overlapping_directives_first_wins() {
        // Two spans claiming overlapping delimiter bytes: the second's
        // conflicting directive is dropped, output stays consistent.
        let spans = [
            span(0..7, StyleKind::Strong),
            span(1..6, StyleKind::Emphasis), // "*inner*" inside "**...**" markers overlap
        ];
        let dl = display_line("***it***", 0, &spans, 100..100);
        // Strong hides 0..2 and 5..7 first; emphasis directives at 1 and 5 overlap and drop.
        assert_eq!(dl.text, "*it*");
        // Round trip still holds for verbatim bytes.
        for seg in dl.segs.iter().filter(|s| s.kind == SegKind::Verbatim) {
            for src in seg.src.clone() {
                assert_eq!(disp_to_src(&dl, src_to_disp(&dl, src)), src);
            }
        }
    }
```

Note: `overlapping_directives_first_wins` encodes the *mechanism* (sorted, first-wins, no panic, mapping stays coherent) on synthetic spans; real pulldown spans for `***it***` nest without delimiter overlap.

- [ ] **Step 2: RED.**
- [ ] **Step 3: Implement** — scanner over the span's line-local slice: expect `[`, walk with bracket depth to matching `]`, require `(` immediately after, walk to matching `)`, require that to be the span end; on success emit `Hide` for `[` and for `](…)`. Any failure → no directives (span stays visible). Confirm the overlap dropper covers the synthetic test.
- [ ] **Step 4: GREEN**; full suite green.
- [ ] **Step 5: Commit** — `feat(editor): link collapse scanner and overlap resolution`

---

### Task 5: Shell integration — geometry through the map

**Files:** Modify `src/editor/mod.rs`

**No unit tests (GPUI shell). Manual verification.**

- [ ] **Step 1: Refactor styling into a projection pipeline**
  - Split `line_runs` into `line_attrs(ix, t) -> (String, Vec<Attr>)` (existing source-space logic, plus a `StyleKind::FenceDelimiter` arm: `color = t.fg_muted`, and in `line_typography` delimiter lines keep Code metrics) and a standalone `runs_from_attrs(attrs, t) -> Vec<TextRun>` compressor.
  - New `display_for_line(ix, t) -> (SharedString, Vec<TextRun>, DisplayLine)`: source attrs → `display::display_line(line_text, line_start, &self.spans, self.core.selection.range())` → project attrs (Verbatim: copy slice; Replacement: repeat the attr at the segment's source start across the replacement bytes; Hidden: drop) → compress.
- [ ] **Step 2: Thread the map**
  - `LineElement` gains `display: DisplayLine` (built in the list closure via `display_for_line`); `CachedLine` stores it.
  - Caret + selection quads in `prepaint`: display offsets via `display::src_to_disp` before `position_for_index`.
  - `offset_at_point`: display index from `closest_index_for_position` → `display::disp_to_src`.
  - `vertical_move`: local display index for the head via `src_to_disp`; the landed display index → `disp_to_src` (both current and neighbor entries).
  - `bounds_for_range` (IME): via `src_to_disp`.
- [ ] **Step 3: Build + full suite** — `cargo build` and `cargo test` green, zero new warnings.
- [ ] **Step 4: Manual verification** (running app)
  - Type `**bold**` in a paragraph: markers visible while inside, fold away when the cursor leaves; the line reflows.
  - Heading line: `#`s hidden until the cursor enters the line.
  - Bullets show `•` (accent), quotes show `▍`; both reveal raw on cursor entry. Ordered lists unchanged.
  - Fence ``` lines render faded; code inside stays highlighted.
  - Links show only their text; URL appears on entry.
  - Click accuracy on transformed lines (click a word after a hidden marker — caret lands on that word); selection drag across folded spans reveals them and the quads track the visible text; ⌘A/copy yields raw source; undo/autosave unaffected.
- [ ] **Step 5: Commit** — `feat(editor): hybrid WYSIWYG — geometry through display map`

---

### Task 6: Roadmap + docs touch-up

**Files:** Modify `WELCOME.md`

- [ ] **Step 1:** Mark Phase 3 done in the roadmap table; set Phase 4 (tables/images/document projection) as Next; tick the "Hide syntax markers" task checkbox; keep the file exercising every feature (it now demonstrates hiding when opened).
- [ ] **Step 2:** `cargo test` green; commit — `docs: Phase 3 complete in roadmap`

---

## Self-review (performed at plan time)

- **Spec coverage:** FenceDelimiter (Task 1); hide engine, reveal rule, mapping + snap semantics, cross-line safety (Task 2); replacements/heading (Task 3); link scanner incl. malformed fallback, overlap first-wins (Task 4); all four geometry paths, styling projection, fence fade, no-cache recomputation (Task 5); behavior guarantees exercised in Task 5 manual checks (copy = source, >1 MB unaffected — no spans means identity transform, covered by `no_spans_passthrough_identity`). Out-of-scope untouched. ✓
- **Placeholder scan:** all test code concrete; shell steps name exact functions being changed. ✓
- **Type consistency:** `Seg`/`SegKind`/`DisplayLine`/`display_line`/`src_to_disp`/`disp_to_src` uniform across Tasks 2–5; `StyleKind::FenceDelimiter` matches Task 1; `line_attrs`/`runs_from_attrs`/`display_for_line` only in Task 5. ✓
