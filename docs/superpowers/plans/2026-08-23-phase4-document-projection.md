# Phase 4: Document Projection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tables render as real tables, whole-line images as images, fences without their ``` lines — each dissolving to raw source when the selection touches its block range.

**Architecture:** Pure `blocks.rs` (discovery) + `projection.rs` (item list, block reveal, line↔item mapping), consumed by an item-based list in the shell. Widgets enabled kind-by-kind: fences (Task 4), tables (Task 5), images (Task 6).

**Tech Stack:** Existing crates only. gpui `img` for images (API from vendored `gpui-0.2.2/examples/image_loading.rs` + `image/` example assets).

**Spec:** `docs/superpowers/specs/2026-08-23-phase4-document-projection-design.md`

## Global Constraints

- TDD iron law for `blocks.rs` and `projection.rs`.
- Block reveal rule everywhere: `block.range.start <= sel.end && sel.start <= block.range.end`.
- Buffer untouched by all of this — render-only.
- `fence_infos`/`FenceInfo` in spans.rs become `pub(crate)` rather than duplicating the parse.
- `cargo test` green before every commit; repo trailer format on commits.

---

### Task 1: blocks.rs — discovery (TDD)

**Files:** Create `src/editor/blocks.rs`; modify `src/editor/mod.rs` (`pub mod blocks;`), `src/editor/spans.rs` (pub(crate) fence access)

**Interfaces produced:**
```rust
pub enum BlockKind { Table, Image { alt: String, dest: String }, Fence { open_line: Range<usize>, close_line: Option<Range<usize>> } }
pub struct BlockInfo { pub range: Range<usize>, pub kind: BlockKind }
pub fn blocks(source: &str) -> Vec<BlockInfo>   // sorted by range.start; empty over MAX_STYLED_BYTES
```

- [ ] **Step 1: Failing tests**

```rust
    #[test]
    fn table_discovered_with_full_range() {
        let src = "a\n\n| h | i |\n| - | - |\n| 1 | 2 |\n\nb\n";
        let all = blocks(src);
        let table = all.iter().find(|b| matches!(b.kind, BlockKind::Table)).unwrap();
        assert_eq!(table.range.start, 3);
        assert!(src[table.range.clone()].contains("| 1 | 2 |"));
    }

    #[test]
    fn whole_line_image_discovered() {
        let src = "para\n\n![alt text](img.png)\n\nafter\n";
        let all = blocks(src);
        let img = all
            .iter()
            .find_map(|b| match &b.kind {
                BlockKind::Image { alt, dest } => Some((b.range.clone(), alt.clone(), dest.clone())),
                _ => None,
            })
            .unwrap();
        assert_eq!(img.1, "alt text");
        assert_eq!(img.2, "img.png");
        assert_eq!(&src[img.0], "![alt text](img.png)");
    }

    #[test]
    fn inline_image_is_not_a_block() {
        let src = "see ![a](b.png) here\n";
        assert!(!blocks(src).iter().any(|b| matches!(b.kind, BlockKind::Image { .. })));
    }

    #[test]
    fn fence_block_with_delimiter_lines() {
        let src = "```rust\nlet x = 1;\n```\n";
        let all = blocks(src);
        let fence = all
            .iter()
            .find_map(|b| match &b.kind {
                BlockKind::Fence { open_line, close_line } => {
                    Some((open_line.clone(), close_line.clone()))
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(fence.0, 0..7);
        assert_eq!(fence.1, Some(19..22));
    }

    #[test]
    fn unclosed_fence_has_no_close_line() {
        let src = "```rust\nlet x = 1;\n";
        let all = blocks(src);
        let fence = all
            .iter()
            .find_map(|b| match &b.kind {
                BlockKind::Fence { close_line, .. } => Some(close_line.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(fence, None);
    }

    #[test]
    fn oversize_source_has_no_blocks() {
        let big = "x".repeat(crate::editor::spans::MAX_STYLED_BYTES + 1);
        assert!(blocks(&big).is_empty());
    }
```

- [ ] **Step 2: RED** (`cargo test editor::blocks` — module contents missing)
- [ ] **Step 3: Implement.** Tables from `Tag::Table` start-event ranges. Images: track inside-image state, collect alt from `Event::Text`, dest from the tag; whole-line check compares the trimmed containing line to the markup slice. Fences: mark `spans::FenceInfo`/`fence_infos` `pub(crate)`; open = `block.start..body.start`, close = `Some(body.end..block.end)` only when non-empty after `trim_trailing_newline` (else `None`). Sort by start.
- [ ] **Step 4: GREEN**, full suite green. (Table exact end boundary: if pulldown's range disagrees with the hand-count, print the events once, correct the constant, note in commit.)
- [ ] **Step 5: Commit** — `feat(editor): block discovery for tables, images, fences`

---

### Task 2: projection.rs — items + mapping (TDD)

**Files:** Create `src/editor/projection.rs`; modify `src/editor/mod.rs`

**Interfaces produced:**
```rust
pub enum Item { Line(usize), Table { lines: Range<usize> }, Image { line: usize, alt: String, dest: String } }
pub fn project(lines: &[Range<usize>], blocks: &[BlockInfo], selection: Range<usize>) -> Vec<Item>
pub fn item_of_line(items: &[Item], line: usize) -> usize
```

- [ ] **Step 1: Failing tests** (pure — synthetic lines/blocks, no markdown)

```rust
    fn lines_of(src: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut start = 0;
        for line in src.split('\n') {
            out.push(start..start + line.len());
            start += line.len() + 1;
        }
        out
    }

    #[test]
    fn untouched_table_becomes_one_item() {
        // lines: 0 "a", 1 "", 2..5 table rows, 5 "", 6 "b"
        let src = "a\n\n|h|\n|-|\n|1|\n\nb";
        let lines = lines_of(src);
        let blocks = [BlockInfo { range: 3..15, kind: BlockKind::Table }];
        let items = project(&lines, &blocks, 0..0);
        assert_eq!(items.len(), 5); // a, blank, Table, blank, b
        assert!(matches!(items[2], Item::Table { ref lines } if *lines == (2..5)));
        assert!(matches!(items[3], Item::Line(5)));
    }

    #[test]
    fn touched_table_dissolves() {
        let src = "a\n\n|h|\n|-|\n|1|\n\nb";
        let lines = lines_of(src);
        let blocks = [BlockInfo { range: 3..15, kind: BlockKind::Table }];
        for sel in [3..3, 10..10, 15..15, 1..4] {
            let items = project(&lines, &blocks, sel);
            assert_eq!(items.len(), 7, "all lines emitted");
            assert!(items.iter().all(|i| matches!(i, Item::Line(_))));
        }
        // One-past the range does NOT touch.
        let items = project(&lines, &blocks, 16..16);
        assert_eq!(items.len(), 5);
    }

    #[test]
    fn untouched_image_becomes_item() {
        let src = "x\n![a](p.png)\ny";
        let lines = lines_of(src);
        let blocks = [BlockInfo {
            range: 2..13,
            kind: BlockKind::Image { alt: "a".into(), dest: "p.png".into() },
        }];
        let items = project(&lines, &blocks, 0..0);
        assert!(matches!(items[1], Item::Image { line: 1, .. }));
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn untouched_fence_omits_delimiter_lines() {
        let src = "```rust\nlet x = 1;\n```\ntail";
        let lines = lines_of(src);
        let blocks = [BlockInfo {
            range: 0..22,
            kind: BlockKind::Fence { open_line: 0..7, close_line: Some(19..22) },
        }];
        let items = project(&lines, &blocks, 100..100);
        // body line 1 and tail line 3 remain
        assert_eq!(items, vec![Item::Line(1), Item::Line(3)]);
        // touched -> all four lines
        let items = project(&lines, &blocks, 10..10);
        assert_eq!(items.len(), 4);
    }

    #[test]
    fn unclosed_fence_never_omits() {
        let src = "```rust\nbody";
        let lines = lines_of(src);
        let blocks = [BlockInfo {
            range: 0..12,
            kind: BlockKind::Fence { open_line: 0..7, close_line: None },
        }];
        let items = project(&lines, &blocks, 100..100);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn item_of_line_maps_emitted_consumed_and_omitted() {
        let src = "a\n\n|h|\n|-|\n|1|\n\nb";
        let lines = lines_of(src);
        let blocks = [BlockInfo { range: 3..15, kind: BlockKind::Table }];
        let items = project(&lines, &blocks, 0..0);
        assert_eq!(item_of_line(&items, 0), 0);
        assert_eq!(item_of_line(&items, 3), 2); // inside table -> table item
        assert_eq!(item_of_line(&items, 6), 4);
        // omitted fence delimiter maps to nearest emitted neighbor
        let src2 = "```rust\nbody\n```";
        let lines2 = lines_of(src2);
        let blocks2 = [BlockInfo {
            range: 0..16,
            kind: BlockKind::Fence { open_line: 0..7, close_line: Some(13..16) },
        }];
        let items2 = project(&lines2, &blocks2, 100..100);
        assert_eq!(items2, vec![Item::Line(1)]);
        assert_eq!(item_of_line(&items2, 0), 0);
        assert_eq!(item_of_line(&items2, 2), 0);
    }
```

- [ ] **Step 2: RED.**
- [ ] **Step 3: Implement.** Walk lines with a cursor; for each untouched block (first-wins on overlap) map its byte range to line indices (`line_of_byte` by scanning `lines`); Table consumes its line range into one item; Image consumes one line; Fence marks its delimiter *lines* as skipped (only when `close_line` is `Some` — an unclosed fence never omits). `item_of_line`: last item whose first line ≤ query, clamped.
- [ ] **Step 4: GREEN**, full suite.
- [ ] **Step 5: Commit** — `feat(editor): projection — display items with block-level reveal`

---

### Task 3: table row parsing (TDD)

**Files:** Modify `src/editor/blocks.rs`

**Interfaces produced:** `pub fn parse_row(line: &str) -> Vec<String>`, `pub fn is_separator_row(line: &str) -> bool`

- [ ] **Step 1: Failing tests**

```rust
    #[test]
    fn parse_row_basic_and_edge_pipes() {
        assert_eq!(parse_row("| a | b |"), vec!["a", "b"]);
        assert_eq!(parse_row("a | b"), vec!["a", "b"]);
        assert_eq!(parse_row("| a |  | c |"), vec!["a", "", "c"]);
    }

    #[test]
    fn parse_row_escaped_pipe() {
        assert_eq!(parse_row(r"| a \| x | b |"), vec![r"a \| x", "b"]);
    }

    #[test]
    fn separator_rows() {
        assert!(is_separator_row("| --- | --- |"));
        assert!(is_separator_row("|:-:|----:|"));
        assert!(!is_separator_row("| a | b |"));
        assert!(!is_separator_row(""));
    }
```

- [ ] **Step 2: RED.** **Step 3:** byte walk with backslash-escape state; strip leading/trailing empty cells produced by edge pipes; separator = non-empty cells all matching `:?-+:?` after trim. **Step 4: GREEN.** **Step 5: Commit** — `feat(editor): table row parsing`

---

### Task 4: Shell — item-based list + fence collapse live

**Files:** Modify `src/editor/mod.rs` (manual verify)

- [ ] Editor gains `blocks: Vec<BlockInfo>` (computed in `restyle`) and `projection: Vec<projection::Item>`; `reproject(&mut self)` builds line-range table, filters blocks to **Fence kind only for this task**, calls `project`, and on change stores + `list_state.reset(len)`.
- [ ] `render` calls `reproject` before building the list; the list closure dispatches on `Item`: `Line(l)` is today's path (all inner code keyed by source line `l` instead of raw `ix`); `Table`/`Image` render a temporary muted placeholder row (replaced in Tasks 5/6, unreachable while filtered).
- [ ] `reveal_cursor` and `scroll_to_line` go through `projection::item_of_line`.
- [ ] Manual verify: ``` lines vanish when the cursor is outside a fence; enter the block (click in code / arrow through) → delimiters reappear; leave → collapse; editing inside works; undo fine. `cargo test` + build green, zero warnings.
- [ ] Commit — `feat(editor): projection-driven list, fence delimiter collapse`

---

### Task 5: Table widget

**Files:** Modify `src/editor/mod.rs`

- [ ] Enable Table kind in `reproject`. Render `Item::Table { lines }`: header row from `parse_row(line 0)` (semibold, `panel_bg`, rounded top), skip `is_separator_row` lines, body rows with `border_t_1`; cells `flex_1 px_3 py_2`, `t.body_size - 1.`; container `rounded_lg border_1 border t.border` within the reading column. Each row `.id(("trow", item_ix, row_ix))` with `on_click` → `editor.update`: `set_cursor(line_range(row's source line).start)`, `break_undo_group`, notify (dissolves next frame).
- [ ] Manual verify: WELCOME.md roadmap table renders as a real table while the cursor is elsewhere; click a row → raw pipes with the cursor on that row; leave → re-forms; edit a cell's text → still dissolved while inside; break the table (delete a `|` line) → stays plain text after leaving. Suite + build green.
- [ ] Commit — `feat(editor): table widget with whole-table reveal`

---

### Task 6: Image widget

**Files:** Modify `src/editor/mod.rs`

- [ ] Read `gpui-0.2.2/examples/image_loading.rs` + `img` element source for the exact `ImageSource` constructors (file path vs remote URI) in 0.2.2.
- [ ] Enable Image kind. Render `Item::Image { line, alt, dest }`: dest starting `http://`/`https://` → remote source; otherwise join to `self.path.parent()`; if the local file doesn't exist, render the raw markup line muted with a warning tint instead. Element: `img(source)` constrained `max_w` full column, `max_h(px(480.))`, `rounded_md`, natural aspect; `.id(("img", item_ix))`, `on_click` → cursor to `line_range(line).start`. Alt text is the hover tooltip if the API offers one cheaply; otherwise skip.
- [ ] Manual verify: drop a PNG next to a note, reference it `![shot](shot.png)` alone on a line → renders; cursor on the line → raw syntax; remote URL renders after load; bad path → muted raw markup. Suite + build green, zero warnings.
- [ ] Commit — `feat(editor): inline image widget (local + remote)`

---

### Task 7: Roadmap + finish

- [ ] WELCOME.md: Phase 4 → Done (leave Phase 5 undeclared — next phase gets chosen later); adjust the near-term checklist; add an image + keep the table exercising the new widgets.
- [ ] Full suite + build, zero project warnings. Commit — `docs: Phase 4 complete in roadmap`
- [ ] superpowers:finishing-a-development-branch (fresh test run, 3-option menu).

## Self-review

- **Spec coverage:** discovery incl. whole-line rule/unclosed fence/oversize (T1); reveal boundaries, fence omission + unclosed never-omit, item_of_line for all three line classes, first-wins (T2); row parsing incl. escapes/separators (T3); item list, cursor-follow scroll, outline mapping (T4); widgets + dissolve interactions incl. broken-table graceful degradation (T5/T6 manual). Guarantees section exercised across T4–T6 manual checks. ✓
- **Placeholders:** none; T4–T6 name exact functions/fields.
- **Type consistency:** `BlockInfo`/`BlockKind` T1↔T2↔T4; `Item` T2↔T4–T6; `parse_row`/`is_separator_row` T3↔T5. ✓
