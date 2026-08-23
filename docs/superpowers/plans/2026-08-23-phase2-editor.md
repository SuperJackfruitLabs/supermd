# Phase 2: Styled-Source Editing Engine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Markdown and code files editable in place with rich typography (all syntax visible), autosaving to disk with session backups.

**Architecture:** A pure, fully-tested editing core (`EditorCore`: rope buffer + selection + undo) and pure styling/persistence modules (`spans.rs`, `autosave.rs`), wrapped by a thin GPUI shell that renders one logical line per virtualized list item and feeds input through `EntityInputHandler`. Invariant: buffer byte offset == rendered text offset.

**Tech Stack:** Rust, gpui 0.2.2 (pinned), ropey, pulldown-cmark 0.13 (`into_offset_iter`), existing tree-sitter `Languages`, unicode-segmentation, tempfile (dev).

**Spec:** `docs/superpowers/specs/2026-08-23-phase2-editor-design.md`

## Global Constraints

- gpui is pinned at `0.2.2` with feature `runtime_shaders`; its full source is unpacked for API reference at `/private/tmp/claude-502/-Users-rakesh-Projects-supermd/76d0d10c-39ce-4e12-939f-dd8c3965f492/scratchpad/gpui-0.2.2/` (check exact signatures there before writing GPUI code; `examples/input.rs` is the IME pattern).
- TDD iron law for `src/editor/buffer.rs`, `src/editor/spans.rs`, `src/editor/autosave.rs`: write the failing test, run it, watch it fail, then implement. GPUI shell tasks (9–12) are manual-verify.
- Byte offsets everywhere. Never char indices in public APIs.
- All colors/sizes come from `theme::Theme` — no literals in GPUI code.
- Run `cargo test` (all tests) before every commit. Run `cargo build` before every commit that touches GPUI code.
- Commit after every task with the trailer lines used in this repo's history (`git log` shows the format).
- New deps allowed: `ropey = "1"`, `[dev-dependencies] tempfile = "3"`. No others.

---

### Task 1: Buffer — rope wrapper with replace

**Files:**
- Create: `src/editor/mod.rs` (module declarations only for now)
- Create: `src/editor/buffer.rs`
- Modify: `src/main.rs` (add `mod editor;`)
- Modify: `Cargo.toml` (add `ropey = "1"`)

**Interfaces:**
- Consumes: nothing
- Produces (used by every later task):
  - `Buffer::from_text(&str) -> Buffer`
  - `Buffer::text(&self) -> String`
  - `Buffer::len_bytes(&self) -> usize`
  - `Buffer::line_count(&self) -> usize`
  - `Buffer::line_text(&self, ix: usize) -> String` (no trailing newline)
  - `Buffer::line_range(&self, ix: usize) -> Range<usize>` (bytes, excludes trailing newline)
  - `Buffer::line_of_byte(&self, offset: usize) -> usize`
  - `Buffer::slice(&self, range: Range<usize>) -> String`
  - `Buffer::replace(&mut self, range: Range<usize>, text: &str) -> Edit`
  - `struct Edit { pub range: Range<usize>, pub old: String, pub new: String }` (`range` is the pre-edit range)

- [ ] **Step 1: Write the failing tests**

In `src/editor/buffer.rs` (implementation stub + tests in one file, standard Rust style):

```rust
//! Text buffer: a rope with byte-offset addressing and a single edit
//! primitive. Byte offsets are the universal currency across supermd.

use std::ops::Range;

use ropey::Rope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Pre-edit byte range that was replaced.
    pub range: Range<usize>,
    pub old: String,
    pub new: String,
}

pub struct Buffer {
    rope: Rope,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_text_round_trips() {
        let buf = Buffer::from_text("hello\nworld\n");
        assert_eq!(buf.text(), "hello\nworld\n");
        assert_eq!(buf.len_bytes(), 12);
    }

    #[test]
    fn line_accessors() {
        let buf = Buffer::from_text("alpha\nbëta\n\ngamma");
        assert_eq!(buf.line_count(), 4);
        assert_eq!(buf.line_text(0), "alpha");
        assert_eq!(buf.line_text(1), "bëta");
        assert_eq!(buf.line_text(2), "");
        assert_eq!(buf.line_text(3), "gamma");
        assert_eq!(buf.line_range(0), 0..5);
        assert_eq!(buf.line_range(1), 6..11); // "bëta" is 5 bytes (ë = 2)
        assert_eq!(buf.line_range(2), 12..12);
        assert_eq!(buf.line_range(3), 13..18);
        assert_eq!(buf.line_of_byte(0), 0);
        assert_eq!(buf.line_of_byte(5), 0);  // the newline belongs to line 0
        assert_eq!(buf.line_of_byte(6), 1);
        assert_eq!(buf.line_of_byte(18), 3); // end of buffer
    }

    #[test]
    fn replace_inserts_deletes_and_reports_edit() {
        let mut buf = Buffer::from_text("hello world");
        let edit = buf.replace(5..5, ",");
        assert_eq!(buf.text(), "hello, world");
        assert_eq!(edit, Edit { range: 5..5, old: String::new(), new: ",".into() });

        let edit = buf.replace(0..6, "");
        assert_eq!(buf.text(), " world");
        assert_eq!(edit, Edit { range: 0..6, old: "hello,".into(), new: String::new() });

        let edit = buf.replace(1..6, "moon");
        assert_eq!(buf.text(), " moon");
        assert_eq!(edit.old, "world");
    }

    #[test]
    fn slice_returns_byte_range() {
        let buf = Buffer::from_text("héllo");
        assert_eq!(buf.slice(0..3), "hé");
        assert_eq!(buf.slice(3..6), "llo");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cargo test editor::buffer 2>&1 | tail -20`
Expected: compile errors — methods not defined. (A compile failure of the *test target* counts as RED here; make sure the errors are the missing methods, not typos.)

- [ ] **Step 3: Minimal implementation**

Append to `src/editor/buffer.rs` above the tests:

```rust
impl Buffer {
    pub fn from_text(text: &str) -> Self {
        Self { rope: Rope::from_str(text) }
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_text(&self, ix: usize) -> String {
        let line = self.rope.line(ix).to_string();
        line.strip_suffix('\n').map(str::to_string).unwrap_or(line)
    }

    pub fn line_range(&self, ix: usize) -> Range<usize> {
        let start = self.rope.byte_of_line(ix);
        start..start + self.line_text(ix).len()
    }

    pub fn line_of_byte(&self, offset: usize) -> usize {
        self.rope.byte_to_line(offset.min(self.rope.len_bytes()))
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.slice(start..end).to_string()
    }

    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Edit {
        let old = self.slice(range.clone());
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.remove(start..end);
        self.rope.insert(start, text);
        Edit { range, old, new: text.to_string() }
    }
}
```

`src/editor/mod.rs`:

```rust
pub mod buffer;
```

In `src/main.rs`, add `mod editor;` alongside the other `mod` lines. Add `ropey = "1"` to `[dependencies]` in `Cargo.toml`.

Note: ropey's API is char-indexed; if `byte_of_line` does not exist under that name, check ropey docs (`line_to_byte` is the actual name in ropey 1.x — verify and use it).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test editor::buffer`
Expected: 4 passed. Also run `cargo build` — must stay green.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): Buffer rope wrapper with byte-offset replace"
```
(Include the repo's standard commit trailers.)

---

### Task 2: Movement — grapheme, word, line-edge functions

**Files:**
- Create: `src/editor/movement.rs`
- Modify: `src/editor/mod.rs` (add `pub mod movement;`)

**Interfaces:**
- Consumes: `Buffer` from Task 1.
- Produces (used by Tasks 3, 10):
  - `movement::next_grapheme(&Buffer, usize) -> usize`
  - `movement::prev_grapheme(&Buffer, usize) -> usize`
  - `movement::next_word(&Buffer, usize) -> usize`
  - `movement::prev_word(&Buffer, usize) -> usize`
  - `movement::line_start(&Buffer, usize) -> usize`
  - `movement::line_end(&Buffer, usize) -> usize`

- [ ] **Step 1: Write the failing tests**

`src/editor/movement.rs`:

```rust
//! Cursor movement over a Buffer. All functions take and return byte
//! offsets, clamped to valid positions. Vertical (up/down) movement is
//! the view layer's job — it needs wrapped-line geometry.

use unicode_segmentation::UnicodeSegmentation;

use super::buffer::Buffer;

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> Buffer {
        Buffer::from_text(s)
    }

    #[test]
    fn grapheme_steps_within_line() {
        let b = buf("aé👍b");
        // bytes: a=1, é=2, 👍=4, b=1
        assert_eq!(next_grapheme(&b, 0), 1);
        assert_eq!(next_grapheme(&b, 1), 3);
        assert_eq!(next_grapheme(&b, 3), 7);
        assert_eq!(next_grapheme(&b, 7), 8);
        assert_eq!(next_grapheme(&b, 8), 8); // clamp at end
        assert_eq!(prev_grapheme(&b, 8), 7);
        assert_eq!(prev_grapheme(&b, 7), 3);
        assert_eq!(prev_grapheme(&b, 0), 0); // clamp at start
    }

    #[test]
    fn grapheme_steps_across_newline() {
        let b = buf("ab\ncd");
        assert_eq!(next_grapheme(&b, 2), 3); // from end of line 0 onto line 1
        assert_eq!(prev_grapheme(&b, 3), 2);
    }

    #[test]
    fn word_steps() {
        let b = buf("foo bar_baz  qux");
        assert_eq!(next_word(&b, 0), 3);   // end of "foo"
        assert_eq!(next_word(&b, 3), 11);  // end of "bar_baz"
        assert_eq!(next_word(&b, 11), 16); // end of "qux"
        assert_eq!(prev_word(&b, 16), 13); // start of "qux"
        assert_eq!(prev_word(&b, 13), 4);  // start of "bar_baz"
        assert_eq!(prev_word(&b, 2), 0);
    }

    #[test]
    fn word_steps_cross_lines() {
        let b = buf("foo\nbar");
        assert_eq!(next_word(&b, 3), 7);  // from line end into next word
        assert_eq!(prev_word(&b, 4), 0);  // from line start into previous word
    }

    #[test]
    fn line_edges() {
        let b = buf("hello\nworld");
        assert_eq!(line_start(&b, 8), 6);
        assert_eq!(line_end(&b, 8), 11);
        assert_eq!(line_start(&b, 3), 0);
        assert_eq!(line_end(&b, 3), 5);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test editor::movement 2>&1 | tail -15`
Expected: compile errors — functions not defined.

- [ ] **Step 3: Minimal implementation**

Add above the tests in `src/editor/movement.rs`:

```rust
pub fn next_grapheme(buf: &Buffer, offset: usize) -> usize {
    let len = buf.len_bytes();
    if offset >= len {
        return len;
    }
    let line_ix = buf.line_of_byte(offset);
    let range = buf.line_range(line_ix);
    if offset >= range.end {
        // Sitting on the newline: step onto the next line's start.
        return (offset + 1).min(len);
    }
    let line = buf.line_text(line_ix);
    let local = offset - range.start;
    line[local..]
        .grapheme_indices(true)
        .nth(1)
        .map(|(i, _)| range.start + local + i)
        .unwrap_or(range.end)
}

pub fn prev_grapheme(buf: &Buffer, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let line_ix = buf.line_of_byte(offset);
    let range = buf.line_range(line_ix);
    if offset <= range.start {
        // At a line start: step back over the newline.
        return offset - 1;
    }
    let line = buf.line_text(line_ix);
    let local = offset - range.start;
    line[..local]
        .grapheme_indices(true)
        .last()
        .map(|(i, _)| range.start + i)
        .unwrap_or(range.start)
}

fn is_word(s: &str) -> bool {
    s.chars().any(|c| c.is_alphanumeric() || c == '_')
}

pub fn next_word(buf: &Buffer, offset: usize) -> usize {
    let text = buf.text();
    let mut seen_word = false;
    for (start, word) in text[offset..].split_word_bound_indices() {
        let abs = offset + start;
        if is_word(word) {
            return abs + word.len();
        } else if seen_word {
            return abs;
        }
        let _ = seen_word; // suppress unused when first segment is non-word
    }
    text.len()
}

pub fn prev_word(buf: &Buffer, offset: usize) -> usize {
    let text = buf.text();
    let mut result = 0;
    for (start, word) in text[..offset].split_word_bound_indices() {
        if is_word(word) {
            result = start;
        }
    }
    result
}

pub fn line_start(buf: &Buffer, offset: usize) -> usize {
    buf.line_range(buf.line_of_byte(offset)).start
}

pub fn line_end(buf: &Buffer, offset: usize) -> usize {
    buf.line_range(buf.line_of_byte(offset)).end
}
```

Note: the `next_word` sketch above contains a subtle flaw around the unused `seen_word`; drive the real logic from the tests — the tests are the contract (they encode "skip non-word, land after word end" for next and "land at word start" for prev). Implement whatever passes them cleanly.

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test editor::movement`
Expected: 5 passed. If `word_steps` fails, fix the implementation — not the test.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): grapheme/word/line movement over Buffer"
```

---

### Task 3: EditorCore — selection + editing operations

**Files:**
- Create: `src/editor/core.rs`
- Modify: `src/editor/mod.rs` (add `pub mod core;`)

**Interfaces:**
- Consumes: `Buffer`, `Edit`, `movement::*`.
- Produces (used by Tasks 4, 9–12):
  - `struct Selection { pub anchor: usize, pub head: usize }` with `Selection::cursor(usize)`, `.is_cursor() -> bool`, `.range() -> Range<usize>` (min..max)
  - `EditorCore::new(&str) -> EditorCore` with public fields `buffer: Buffer`, `selection: Selection`
  - `EditorCore::insert(&mut self, text: &str, now: Instant)` — replaces selection
  - `EditorCore::backspace(&mut self, now: Instant)`, `delete_forward(&mut self, now: Instant)`
  - `EditorCore::set_cursor(&mut self, offset: usize)`, `select_to(&mut self, offset: usize)`, `select_all(&mut self)`
  - `EditorCore::selected_text(&self) -> String`

- [ ] **Step 1: Write the failing tests**

`src/editor/core.rs`:

```rust
//! EditorCore: the tested facade the GPUI shell drives. Owns the buffer,
//! one selection, and (Task 4) the undo history.

use std::ops::Range;
use std::time::Instant;

use super::buffer::{Buffer, Edit};
use super::movement;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn insert_at_cursor_advances_cursor() {
        let mut ed = EditorCore::new("world");
        ed.insert("hello ", now());
        assert_eq!(ed.buffer.text(), "hello world");
        assert_eq!(ed.selection, Selection::cursor(6));
    }

    #[test]
    fn insert_replaces_selection() {
        let mut ed = EditorCore::new("hello world");
        ed.set_cursor(0);
        ed.select_to(5);
        ed.insert("goodbye", now());
        assert_eq!(ed.buffer.text(), "goodbye world");
        assert_eq!(ed.selection, Selection::cursor(7));
    }

    #[test]
    fn backspace_deletes_grapheme_or_selection() {
        let mut ed = EditorCore::new("ab👍");
        ed.set_cursor(6);
        ed.backspace(now());
        assert_eq!(ed.buffer.text(), "ab");
        assert_eq!(ed.selection, Selection::cursor(2));

        let mut ed = EditorCore::new("hello world");
        ed.set_cursor(5);
        ed.select_to(11);
        ed.backspace(now());
        assert_eq!(ed.buffer.text(), "hello");
    }

    #[test]
    fn delete_forward_deletes_next_grapheme() {
        let mut ed = EditorCore::new("a👍b");
        ed.set_cursor(1);
        ed.delete_forward(now());
        assert_eq!(ed.buffer.text(), "ab");
        assert_eq!(ed.selection, Selection::cursor(1));
    }

    #[test]
    fn select_all_and_selected_text() {
        let mut ed = EditorCore::new("héllo");
        ed.select_all();
        assert_eq!(ed.selected_text(), "héllo");
        assert_eq!(ed.selection.range(), 0..6);
    }

    #[test]
    fn selection_range_normalizes_reversed() {
        let sel = Selection { anchor: 9, head: 2 };
        assert_eq!(sel.range(), 2..9);
        assert!(!sel.is_cursor());
        assert!(Selection::cursor(4).is_cursor());
    }
}
```

- [ ] **Step 2: Run tests, verify failure**

Run: `cargo test editor::core 2>&1 | tail -15`
Expected: compile errors — `EditorCore` and `Selection` methods missing.

- [ ] **Step 3: Minimal implementation**

Add above tests:

```rust
impl Selection {
    pub fn cursor(offset: usize) -> Self {
        Self { anchor: offset, head: offset }
    }

    pub fn is_cursor(&self) -> bool {
        self.anchor == self.head
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

pub struct EditorCore {
    pub buffer: Buffer,
    pub selection: Selection,
}

impl EditorCore {
    pub fn new(text: &str) -> Self {
        Self {
            buffer: Buffer::from_text(text),
            selection: Selection::cursor(0),
        }
    }

    pub fn set_cursor(&mut self, offset: usize) {
        let offset = offset.min(self.buffer.len_bytes());
        self.selection = Selection::cursor(offset);
    }

    pub fn select_to(&mut self, offset: usize) {
        self.selection.head = offset.min(self.buffer.len_bytes());
    }

    pub fn select_all(&mut self) {
        self.selection = Selection { anchor: 0, head: self.buffer.len_bytes() };
    }

    pub fn selected_text(&self) -> String {
        self.buffer.slice(self.selection.range())
    }

    fn apply(&mut self, range: Range<usize>, text: &str, _now: Instant) -> Edit {
        let edit = self.buffer.replace(range.clone(), text);
        self.selection = Selection::cursor(range.start + text.len());
        edit
    }

    pub fn insert(&mut self, text: &str, now: Instant) {
        self.apply(self.selection.range(), text, now);
    }

    pub fn backspace(&mut self, now: Instant) {
        let range = if self.selection.is_cursor() {
            movement::prev_grapheme(&self.buffer, self.selection.head)..self.selection.head
        } else {
            self.selection.range()
        };
        if !range.is_empty() {
            self.apply(range, "", now);
        }
    }

    pub fn delete_forward(&mut self, now: Instant) {
        let range = if self.selection.is_cursor() {
            self.selection.head..movement::next_grapheme(&self.buffer, self.selection.head)
        } else {
            self.selection.range()
        };
        if !range.is_empty() {
            self.apply(range, "", now);
        }
    }
}
```

(The `_now` parameter is deliberately threaded through `apply` now; Task 4 uses it for undo grouping.)

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test editor::core`
Expected: 6 passed. Full `cargo test` also green.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): EditorCore with selection and edit operations"
```

---

### Task 4: Undo/redo with time-grouped coalescing

**Files:**
- Modify: `src/editor/core.rs`

**Interfaces:**
- Consumes: everything from Task 3.
- Produces (used by Tasks 10):
  - `EditorCore::undo(&mut self) -> bool`, `EditorCore::redo(&mut self) -> bool` (return whether anything changed)
  - `EditorCore::break_undo_group(&mut self)` (called by the shell on cursor moves)
  - `pub const GROUP_WINDOW: Duration = Duration::from_millis(700);`

- [ ] **Step 1: Write the failing tests**

Append to the tests module in `src/editor/core.rs`:

```rust
    use std::time::Duration;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn undo_reverts_and_redo_reapplies() {
        let mut ed = EditorCore::new("abc");
        ed.set_cursor(3);
        ed.insert("d", t0());
        assert!(ed.undo());
        assert_eq!(ed.buffer.text(), "abc");
        assert_eq!(ed.selection, Selection::cursor(3));
        assert!(ed.redo());
        assert_eq!(ed.buffer.text(), "abcd");
        assert_eq!(ed.selection, Selection::cursor(4));
        assert!(!ed.redo());
    }

    #[test]
    fn rapid_typing_coalesces_into_one_group() {
        let mut ed = EditorCore::new("");
        let start = t0();
        ed.insert("h", start);
        ed.insert("e", start + Duration::from_millis(100));
        ed.insert("y", start + Duration::from_millis(200));
        assert!(ed.undo());
        assert_eq!(ed.buffer.text(), "");
        assert!(!ed.undo());
    }

    #[test]
    fn pause_breaks_group() {
        let mut ed = EditorCore::new("");
        let start = t0();
        ed.insert("hi", start);
        ed.insert(" there", start + Duration::from_millis(1500));
        ed.undo();
        assert_eq!(ed.buffer.text(), "hi");
        ed.undo();
        assert_eq!(ed.buffer.text(), "");
    }

    #[test]
    fn kind_change_breaks_group() {
        let mut ed = EditorCore::new("");
        let start = t0();
        ed.insert("hey", start);
        ed.backspace(start + Duration::from_millis(50));
        ed.insert("y", start + Duration::from_millis(100));
        assert_eq!(ed.buffer.text(), "hey");
        ed.undo(); // undoes the trailing "y" insert
        assert_eq!(ed.buffer.text(), "he");
        ed.undo(); // undoes the backspace
        assert_eq!(ed.buffer.text(), "hey");
        ed.undo(); // undoes the initial insert
        assert_eq!(ed.buffer.text(), "");
    }

    #[test]
    fn cursor_jump_breaks_group() {
        let mut ed = EditorCore::new("xx");
        let start = t0();
        ed.set_cursor(2);
        ed.insert("a", start);
        ed.set_cursor(0);
        ed.break_undo_group();
        ed.insert("b", start + Duration::from_millis(50));
        ed.undo();
        assert_eq!(ed.buffer.text(), "xxa");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut ed = EditorCore::new("");
        ed.insert("a", t0());
        ed.undo();
        ed.insert("b", t0());
        assert!(!ed.redo());
        assert_eq!(ed.buffer.text(), "b");
    }
```

- [ ] **Step 2: Run tests, verify failure**

Run: `cargo test editor::core 2>&1 | tail -15`
Expected: compile errors — `undo`/`redo`/`break_undo_group` missing.

- [ ] **Step 3: Implementation**

Add to `src/editor/core.rs`:

```rust
use std::time::Duration;

pub const GROUP_WINDOW: Duration = Duration::from_millis(700);

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
}

struct UndoGroup {
    /// Edits in application order.
    edits: Vec<Edit>,
    selection_before: Selection,
    selection_after: Selection,
}

#[derive(Default)]
struct History {
    undo: Vec<UndoGroup>,
    redo: Vec<UndoGroup>,
    last_kind: Option<EditKind>,
    last_at: Option<Instant>,
    broken: bool,
}
```

Change `EditorCore`:
- add field `history: History` (init in `new`).
- in `apply`, before mutating, capture `selection_before = self.selection` and classify `kind` (`Insert` if `!text.is_empty()` else `Delete`).
- Coalesce: if `!history.broken`, `history.last_kind == Some(kind)`, `history.last_at` within `GROUP_WINDOW` of `now`, and the new edit is contiguous with the last group (for Insert: `edit.range.start == last group's last edit range.start + last new.len()` adjusted; simplest correct rule that satisfies the tests: same kind + within window + not broken), push the edit into the last undo group and update `selection_after`; else push a fresh `UndoGroup`.
- Always: `history.redo.clear(); history.last_kind = Some(kind); history.last_at = Some(now); history.broken = false;`
- `break_undo_group()` sets `history.broken = true`.
- `undo()`: pop group; apply its edits **in reverse order**, each inverted (`replace(new_range, &old)` where `new_range = edit.range.start..edit.range.start + edit.new.len()`); restore `selection_before`; push group onto redo; return true. Careful: when reversing multiple coalesced edits, offsets of earlier edits are unaffected by later ones only if later edits are at increasing offsets — for coalesced typing they are contiguous appends, so reverse-order inversion is correct.
- `redo()`: pop from redo, re-apply edits in forward order (`replace(edit.range, &edit.new)`), restore `selection_after`, push back onto undo.
- Undo/redo must NOT go through `apply` (they must not create new history); mutate `self.buffer` directly.

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test editor::core`
Expected: 12 passed (6 from Task 3 + 6 new). Full `cargo test` green.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): undo/redo with time-window edit coalescing"
```

---

### Task 5: spans.rs — Markdown structural spans

**Files:**
- Create: `src/editor/spans.rs`
- Modify: `src/editor/mod.rs` (add `pub mod spans;`)

**Interfaces:**
- Consumes: pulldown-cmark (already a dependency).
- Produces (used by Tasks 6, 9):
  - `enum StyleKind { Heading(u8), Strong, Emphasis, Strikethrough, InlineCode, Link, ListMarker, QuoteMarker, FenceContent, Rule, Syntax(u8) }`
  - `struct StyleSpan { pub range: Range<usize>, pub kind: StyleKind }`
  - `fn markdown_spans(source: &str) -> Vec<StyleSpan>` (sorted by range start; may overlap/nest)

- [ ] **Step 1: Write the failing tests**

`src/editor/spans.rs`:

```rust
//! Source text → style spans for the styled-source editor. Byte ranges
//! over the raw source; markers are included in their spans (they stay
//! visible in Phase 2).

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleKind {
    Heading(u8),
    Strong,
    Emphasis,
    Strikethrough,
    InlineCode,
    Link,
    ListMarker,
    QuoteMarker,
    FenceContent,
    Rule,
    /// Tree-sitter capture index into `highlight::CAPTURE_NAMES`.
    Syntax(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub kind: StyleKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans_of_kind(source: &str, kind_match: fn(&StyleKind) -> bool) -> Vec<Range<usize>> {
        markdown_spans(source)
            .into_iter()
            .filter(|s| kind_match(&s.kind))
            .map(|s| s.range)
            .collect()
    }

    #[test]
    fn heading_span_covers_marks_and_text() {
        let src = "# Title\n\nbody\n";
        let spans = markdown_spans(src);
        assert!(spans.contains(&StyleSpan { range: 0..7, kind: StyleKind::Heading(1) }));
    }

    #[test]
    fn heading_levels() {
        let src = "## Two\n### Three\n";
        let spans = markdown_spans(src);
        assert!(spans.contains(&StyleSpan { range: 0..6, kind: StyleKind::Heading(2) }));
        assert!(spans.contains(&StyleSpan { range: 7..16, kind: StyleKind::Heading(3) }));
    }

    #[test]
    fn strong_and_emphasis_include_markers() {
        let src = "a **bold** and *it*\n";
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::Strong), vec![2..10]);
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::Emphasis), vec![15..19]);
    }

    #[test]
    fn inline_code_includes_backticks() {
        let src = "use `foo()` here\n";
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::InlineCode), vec![4..11]);
    }

    #[test]
    fn fence_content_covers_body_not_delimiters() {
        let src = "```rust\nlet x = 1;\n```\n";
        // body is "let x = 1;\n" at bytes 8..19
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::FenceContent), vec![8..19]);
    }

    #[test]
    fn list_markers_are_spanned() {
        let src = "- one\n- two\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::ListMarker),
            vec![0..2, 6..8]
        );
    }

    #[test]
    fn ordered_list_markers() {
        let src = "1. one\n2. two\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::ListMarker),
            vec![0..3, 7..10]
        );
    }

    #[test]
    fn quote_markers_per_line() {
        let src = "> a\n> b\n";
        assert_eq!(
            spans_of_kind(src, |k| *k == StyleKind::QuoteMarker),
            vec![0..1, 4..5]
        );
    }

    #[test]
    fn link_span() {
        let src = "see [zed](https://zed.dev) now\n";
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::Link), vec![4..26]);
    }

    #[test]
    fn rule_span() {
        let src = "a\n\n---\n\nb\n";
        assert_eq!(spans_of_kind(src, |k| *k == StyleKind::Rule), vec![3..7]);
    }
}
```

Note on expected byte ranges: verify each range by hand-counting bytes in the test source before trusting a failure — off-by-one in the *test* is as bad as in the code. pulldown-cmark's `Rule` event range may include the trailing newline (the `3..7` above assumes it does); if the first RED run shows a boundary disagreement from pulldown (e.g. `3..6`), inspect with a scratch `println!` of all events+ranges, then correct **the expected constant** to pulldown's actual, documented behavior — that is fixing a wrong test oracle, not weakening the test. Do this only for exact-boundary constants, never for "span exists / kind" assertions.

- [ ] **Step 2: Run tests, verify failure**

Run: `cargo test editor::spans 2>&1 | tail -15`
Expected: compile error — `markdown_spans` not defined.

- [ ] **Step 3: Implementation**

Add to `src/editor/spans.rs`:

```rust
pub fn markdown_spans(source: &str) -> Vec<StyleSpan> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut spans = Vec::new();
    let mut fence_body: Option<Range<usize>> = None;

    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let mut r = range.clone();
                trim_trailing_newline(source, &mut r);
                spans.push(StyleSpan { range: r, kind: StyleKind::Heading(level as u8) });
            }
            Event::Start(Tag::Strong) => spans.push(StyleSpan { range, kind: StyleKind::Strong }),
            Event::Start(Tag::Emphasis) => {
                spans.push(StyleSpan { range, kind: StyleKind::Emphasis })
            }
            Event::Start(Tag::Strikethrough) => {
                spans.push(StyleSpan { range, kind: StyleKind::Strikethrough })
            }
            Event::Code(_) => spans.push(StyleSpan { range, kind: StyleKind::InlineCode }),
            Event::Start(Tag::Link { .. }) => {
                spans.push(StyleSpan { range, kind: StyleKind::Link })
            }
            Event::Start(Tag::Item) => {
                if let Some(len) = list_marker_len(&source[range.clone()]) {
                    spans.push(StyleSpan {
                        range: range.start..range.start + len,
                        kind: StyleKind::ListMarker,
                    });
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                for (line_start, line) in line_offsets(&source[range.clone()]) {
                    let abs = range.start + line_start;
                    let marks = line.bytes().take_while(|b| *b == b'>').count();
                    if marks > 0 {
                        spans.push(StyleSpan {
                            range: abs..abs + marks,
                            kind: StyleKind::QuoteMarker,
                        });
                    }
                }
            }
            Event::Start(Tag::CodeBlock(_)) => fence_body = Some(range.start..range.start),
            Event::Text(_) if fence_body.is_some() => {
                let body = fence_body.as_mut().unwrap();
                if body.is_empty() {
                    *body = range;
                } else {
                    body.end = range.end;
                }
            }
            Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                if let Some(body) = fence_body.take() {
                    if !body.is_empty() {
                        spans.push(StyleSpan { range: body, kind: StyleKind::FenceContent });
                    }
                }
            }
            Event::Rule => spans.push(StyleSpan { range, kind: StyleKind::Rule }),
            _ => {}
        }
    }

    spans.sort_by_key(|s| (s.range.start, s.range.end));
    spans
}

fn trim_trailing_newline(source: &str, range: &mut Range<usize>) {
    while range.end > range.start && source.as_bytes()[range.end - 1] == b'\n' {
        range.end -= 1;
    }
}

/// Byte length of a list marker ("- ", "* ", "12. ", "3) ") at the start
/// of an item, including one trailing space.
fn list_marker_len(item: &str) -> Option<usize> {
    let bytes = item.as_bytes();
    let mut i = 0;
    if bytes.first().copied() == Some(b'-') || bytes.first().copied() == Some(b'*')
        || bytes.first().copied() == Some(b'+')
    {
        i = 1;
    } else {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == 0 || i >= bytes.len() || !(bytes[i] == b'.' || bytes[i] == b')') {
            return None;
        }
        i += 1;
    }
    if bytes.get(i).copied() == Some(b' ') {
        i += 1;
    }
    Some(i)
}

/// (byte offset, line) pairs over a str, offsets relative to the input.
fn line_offsets(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for line in s.split_inclusive('\n') {
        out.push((start, line.trim_end_matches('\n')));
        start += line.len();
    }
    out
}
```

Heading ranges from pulldown include the trailing newline — hence `trim_trailing_newline` (the Task test `0..7` for `"# Title\n"` encodes this). Task-list markers (`- [ ] `) will produce a `TaskListMarker` event; ignore for now — the `- ` is already spanned.

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test editor::spans`
Expected: 10 passed. If exact boundaries differ, debug with a scratch print of `(event, range)` pairs per the note in Step 1.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): markdown structural style spans"
```

---

### Task 6: spans.rs — code highlighting, line typography, size cap

**Files:**
- Modify: `src/editor/spans.rs`

**Interfaces:**
- Consumes: `highlight::Languages` (existing), `markdown_spans` (Task 5).
- Produces (used by Task 9):
  - `fn code_spans(source: &str, lang: &str, langs: &Languages) -> Vec<StyleSpan>`
  - `fn markdown_spans_highlighted(source: &str, langs: &Languages) -> Vec<StyleSpan>` (markdown_spans + `Syntax` spans inside fence bodies)
  - `enum LineKind { Body, Heading(u8), Code }`
  - `fn line_kinds(source: &str, spans: &[StyleSpan]) -> Vec<LineKind>` (one entry per line, `source.split('\n')` count)
  - `pub const MAX_STYLED_BYTES: usize = 1_000_000;`

- [ ] **Step 1: Write the failing tests**

Append to tests in `src/editor/spans.rs`:

```rust
    use crate::highlight::Languages;

    #[test]
    fn code_spans_highlight_rust() {
        let langs = Languages::new();
        let spans = code_spans("fn main() {}\n", "rust", &langs);
        // "fn" must be captured as something (keyword) => a Syntax span at 0..2
        assert!(spans.iter().any(|s| s.range == (0..2) && matches!(s.kind, StyleKind::Syntax(_))));
    }

    #[test]
    fn code_spans_unknown_language_is_empty() {
        let langs = Languages::new();
        assert!(code_spans("hello\n", "klingon", &langs).is_empty());
    }

    #[test]
    fn fence_bodies_get_syntax_spans_at_absolute_offsets() {
        let langs = Languages::new();
        let src = "pre\n\n```rust\nfn main() {}\n```\n";
        // fence body "fn main() {}\n" starts at byte 13
        let spans = markdown_spans_highlighted(src, &langs);
        assert!(spans
            .iter()
            .any(|s| s.range == (13..15) && matches!(s.kind, StyleKind::Syntax(_))));
        // structural spans still present
        assert!(spans.iter().any(|s| s.kind == StyleKind::FenceContent));
    }

    #[test]
    fn oversized_source_returns_no_spans() {
        let langs = Languages::new();
        let big = "x".repeat(MAX_STYLED_BYTES + 1);
        assert!(markdown_spans_highlighted(&big, &langs).is_empty());
        assert!(code_spans(&big, "rust", &langs).is_empty());
    }

    #[test]
    fn line_kinds_classify_heading_code_body() {
        let src = "# Title\nbody\n```rust\nlet x = 1;\n```\ntail";
        let langs = Languages::new();
        let spans = markdown_spans_highlighted(src, &langs);
        let kinds = line_kinds(src, &spans);
        assert_eq!(kinds.len(), 6);
        assert_eq!(kinds[0], LineKind::Heading(1));
        assert_eq!(kinds[1], LineKind::Body);
        assert_eq!(kinds[2], LineKind::Code); // ``` delimiter line renders mono
        assert_eq!(kinds[3], LineKind::Code);
        assert_eq!(kinds[4], LineKind::Code);
        assert_eq!(kinds[5], LineKind::Body);
    }
```

- [ ] **Step 2: Run tests, verify failure**

Run: `cargo test editor::spans 2>&1 | tail -15`
Expected: compile errors — new items missing.

- [ ] **Step 3: Implementation**

Add to `src/editor/spans.rs`:

```rust
use crate::highlight::Languages;

pub const MAX_STYLED_BYTES: usize = 1_000_000;

pub fn code_spans(source: &str, lang: &str, langs: &Languages) -> Vec<StyleSpan> {
    if source.len() > MAX_STYLED_BYTES {
        return Vec::new();
    }
    langs
        .highlight(lang, source)
        .into_iter()
        .map(|(range, capture)| StyleSpan { range, kind: StyleKind::Syntax(capture) })
        .collect()
}

pub fn markdown_spans_highlighted(source: &str, langs: &Languages) -> Vec<StyleSpan> {
    if source.len() > MAX_STYLED_BYTES {
        return Vec::new();
    }
    let mut spans = markdown_spans(source);
    let fences = fence_infos(source);
    for (body, lang) in fences {
        if let Some(lang) = lang {
            for (range, capture) in langs.highlight(&lang, &source[body.clone()]) {
                spans.push(StyleSpan {
                    range: body.start + range.start..body.start + range.end,
                    kind: StyleKind::Syntax(capture),
                });
            }
        }
    }
    spans.sort_by_key(|s| (s.range.start, s.range.end));
    spans
}

/// (body byte range, language) for every fenced code block.
fn fence_infos(source: &str) -> Vec<(Range<usize>, Option<String>)> {
    use pulldown_cmark::{CodeBlockKind, TagEnd};
    let mut out = Vec::new();
    let mut current: Option<(Range<usize>, Option<String>)> = None;
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) if !info.is_empty() => {
                        Some(info.split_whitespace().next().unwrap_or("").to_string())
                    }
                    _ => None,
                };
                current = Some((range.start..range.start, lang));
            }
            Event::Text(_) => {
                if let Some((body, _)) = current.as_mut() {
                    if body.is_empty() {
                        *body = range;
                    } else {
                        body.end = range.end;
                    }
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(entry) = current.take() {
                    if !entry.0.is_empty() {
                        out.push(entry);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Body,
    Heading(u8),
    Code,
}

pub fn line_kinds(source: &str, spans: &[StyleSpan]) -> Vec<LineKind> {
    // Whole-fence ranges (delimiters included) come from re-scanning
    // fence delimiter lines: a line is Code if it's inside or adjacent
    // to a FenceContent span, or starts with ``` while a fence span
    // exists on the following/preceding content.
    let mut kinds = Vec::new();
    let mut offset = 0;
    for line in source.split('\n') {
        let range = offset..offset + line.len();
        let mut kind = LineKind::Body;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            kind = LineKind::Code;
        }
        for span in spans {
            if span.range.start >= range.end.max(range.start + 1) {
                break;
            }
            if span.range.end <= range.start {
                continue;
            }
            match span.kind {
                StyleKind::Heading(n) => kind = LineKind::Heading(n),
                StyleKind::FenceContent => kind = LineKind::Code,
                _ => {}
            }
        }
        kinds.push(kind);
        offset = range.end + 1;
    }
    kinds
}
```

Refactor note (REFACTOR step of TDD): Task 5's inline fence-tracking inside `markdown_spans` duplicates `fence_infos`; after green, simplify `markdown_spans` to call `fence_infos` for its `FenceContent` spans and delete the inline tracking. Keep all tests green.

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test editor::spans`
Expected: 15 passed.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): fence/code syntax spans, line typography, 1MB cap"
```

---

### Task 7: autosave.rs — save policy state machine

**Files:**
- Create: `src/editor/autosave.rs`
- Modify: `src/editor/mod.rs` (add `pub mod autosave;`)

**Interfaces:**
- Consumes: nothing (std only).
- Produces (used by Tasks 8, 11):
  - `pub const DEBOUNCE: Duration = Duration::from_secs(1);`
  - `SavePolicy::default() -> SavePolicy`
  - `SavePolicy::record_edit(&mut self, now: Instant)`
  - `SavePolicy::is_dirty(&self) -> bool`
  - `SavePolicy::should_flush(&self, now: Instant) -> bool` (dirty && debounce elapsed)
  - `SavePolicy::take_flush_now(&mut self) -> bool` (for ⌘S/tab-switch/close/quit: returns dirty state; caller writes then calls `mark_saved`)
  - `SavePolicy::mark_saved(&mut self)`

- [ ] **Step 1: Write the failing tests**

`src/editor/autosave.rs`:

```rust
//! Autosave policy and session backups. The policy is a pure state
//! machine driven by injected time; the fs helpers do real IO and are
//! tested against temp dirs.

use std::time::{Duration, Instant};

pub const DEBOUNCE: Duration = Duration::from_secs(1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_policy_never_flushes() {
        let policy = SavePolicy::default();
        assert!(!policy.is_dirty());
        assert!(!policy.should_flush(Instant::now()));
    }

    #[test]
    fn flushes_only_after_debounce_idle() {
        let mut policy = SavePolicy::default();
        let start = Instant::now();
        policy.record_edit(start);
        assert!(policy.is_dirty());
        assert!(!policy.should_flush(start + Duration::from_millis(500)));
        assert!(policy.should_flush(start + Duration::from_millis(1001)));
    }

    #[test]
    fn new_edit_restarts_debounce() {
        let mut policy = SavePolicy::default();
        let start = Instant::now();
        policy.record_edit(start);
        policy.record_edit(start + Duration::from_millis(900));
        assert!(!policy.should_flush(start + Duration::from_millis(1500)));
        assert!(policy.should_flush(start + Duration::from_millis(1901)));
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let mut policy = SavePolicy::default();
        let start = Instant::now();
        policy.record_edit(start);
        policy.mark_saved();
        assert!(!policy.is_dirty());
        assert!(!policy.should_flush(start + Duration::from_secs(10)));
    }

    #[test]
    fn take_flush_now_reports_dirty_once_meaningfully() {
        let mut policy = SavePolicy::default();
        assert!(!policy.take_flush_now());
        policy.record_edit(Instant::now());
        assert!(policy.take_flush_now());
        // still dirty until mark_saved — the caller saves, then marks
        assert!(policy.is_dirty());
        policy.mark_saved();
        assert!(!policy.take_flush_now());
    }
}
```

- [ ] **Step 2: Run tests, verify failure**

Run: `cargo test editor::autosave 2>&1 | tail -10`
Expected: compile error — `SavePolicy` missing.

- [ ] **Step 3: Minimal implementation**

```rust
#[derive(Default)]
pub struct SavePolicy {
    dirty: bool,
    last_edit: Option<Instant>,
}

impl SavePolicy {
    pub fn record_edit(&mut self, now: Instant) {
        self.dirty = true;
        self.last_edit = Some(now);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn should_flush(&self, now: Instant) -> bool {
        match (self.dirty, self.last_edit) {
            (true, Some(at)) => now.duration_since(at) >= DEBOUNCE,
            _ => false,
        }
    }

    pub fn take_flush_now(&mut self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
        self.last_edit = None;
    }
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test editor::autosave`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): autosave debounce policy state machine"
```

---

### Task 8: autosave.rs — backups, atomic write, conflict detection

**Files:**
- Modify: `src/editor/autosave.rs`
- Modify: `Cargo.toml` (add `[dev-dependencies] tempfile = "3"`)

**Interfaces:**
- Consumes: Task 7 items.
- Produces (used by Task 11):
  - `BackupRegistry::new(dir: PathBuf) -> BackupRegistry`
  - `BackupRegistry::backup_if_needed(&mut self, source: &Path) -> io::Result<Option<PathBuf>>` — copies once per session per path; `Ok(None)` if already backed up this session or source doesn't exist
  - `BackupRegistry::force_backup(&mut self, source: &Path) -> io::Result<Option<PathBuf>>` — copies regardless of session state (used for external-change conflicts); `Ok(None)` only if source doesn't exist
  - `fn atomic_write(path: &Path, contents: &str) -> io::Result<()>` — temp file + rename
  - `fn disk_mtime(path: &Path) -> Option<SystemTime>`
  - `fn has_conflict(expected: Option<SystemTime>, path: &Path) -> bool` — true when the file exists on disk with an mtime different from `expected`

- [ ] **Step 1: Write the failing tests**

Append to tests in `src/editor/autosave.rs`:

```rust
    use std::fs;

    #[test]
    fn backup_copies_original_once_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.md");
        fs::write(&file, "original").unwrap();

        let mut reg = BackupRegistry::new(backups.path().to_path_buf());
        let first = reg.backup_if_needed(&file).unwrap();
        let backup_path = first.expect("first write must back up");
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "original");

        fs::write(&file, "changed").unwrap();
        assert!(reg.backup_if_needed(&file).unwrap().is_none());
        // the original backup is untouched
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "original");
    }

    #[test]
    fn backup_of_missing_file_is_none() {
        let backups = tempfile::tempdir().unwrap();
        let mut reg = BackupRegistry::new(backups.path().to_path_buf());
        assert!(reg.backup_if_needed(Path::new("/nonexistent/x.md")).unwrap().is_none());
    }

    #[test]
    fn force_backup_copies_again() {
        let dir = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.md");
        fs::write(&file, "v1").unwrap();

        let mut reg = BackupRegistry::new(backups.path().to_path_buf());
        let p1 = reg.backup_if_needed(&file).unwrap().unwrap();
        fs::write(&file, "v2-external").unwrap();
        let p2 = reg.force_backup(&file).unwrap().unwrap();
        assert_ne!(p1, p2);
        assert_eq!(fs::read_to_string(&p2).unwrap(), "v2-external");
    }

    #[test]
    fn atomic_write_replaces_contents() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.md");
        atomic_write(&file, "hello").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
        atomic_write(&file, "goodbye").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "goodbye");
        // no stray temp files left behind
        let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn conflict_detection() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.md");
        fs::write(&file, "a").unwrap();
        let mtime = disk_mtime(&file);
        assert!(mtime.is_some());
        assert!(!has_conflict(mtime, &file));

        // externally modified => conflict (set mtime forward explicitly
        // to avoid flaky sub-second granularity)
        let later = std::time::SystemTime::now() + Duration::from_secs(5);
        let f = fs::File::options().write(true).open(&file).unwrap();
        f.set_modified(later).unwrap();
        assert!(has_conflict(mtime, &file));

        // missing file, expected mtime => no conflict (nothing to clobber)
        assert!(!has_conflict(mtime, Path::new("/nonexistent/y.md")));
    }
```

Add `use std::path::Path;` to the test imports as needed, and `tempfile = "3"` under `[dev-dependencies]` in `Cargo.toml`.

- [ ] **Step 2: Run tests, verify failure**

Run: `cargo test editor::autosave 2>&1 | tail -15`
Expected: compile errors — `BackupRegistry`, `atomic_write`, `disk_mtime`, `has_conflict` missing.

- [ ] **Step 3: Implementation**

Add to `src/editor/autosave.rs`:

```rust
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct BackupRegistry {
    dir: PathBuf,
    seen: HashSet<PathBuf>,
    counter: u64,
}

impl BackupRegistry {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, seen: HashSet::new(), counter: 0 }
    }

    /// Default location: ~/.supermd/backups
    pub fn default_dir() -> PathBuf {
        std::env::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".supermd")
            .join("backups")
    }

    pub fn backup_if_needed(&mut self, source: &Path) -> io::Result<Option<PathBuf>> {
        if self.seen.contains(source) {
            return Ok(None);
        }
        let result = self.copy_backup(source)?;
        self.seen.insert(source.to_path_buf());
        Ok(result)
    }

    pub fn force_backup(&mut self, source: &Path) -> io::Result<Option<PathBuf>> {
        self.seen.insert(source.to_path_buf());
        self.copy_backup(source)
    }

    fn copy_backup(&mut self, source: &Path) -> io::Result<Option<PathBuf>> {
        if !source.exists() {
            return Ok(None);
        }
        std::fs::create_dir_all(&self.dir)?;
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.counter += 1;
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into());
        let dest = self.dir.join(format!("{stamp}-{:03}-{name}", self.counter));
        std::fs::copy(source, &dest)?;
        Ok(Some(dest))
    }
}

pub fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let tmp = path.with_extension("supermd-tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

pub fn disk_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn has_conflict(expected: Option<SystemTime>, path: &Path) -> bool {
    match (expected, disk_mtime(path)) {
        (Some(expected), Some(actual)) => actual != expected,
        (None, Some(_)) => true, // we never saw a file, but one exists now
        (_, None) => false,      // nothing on disk => nothing to clobber
    }
}
```

Note: `std::env::home_dir` is un-deprecated in recent Rust; if the toolchain still warns, read `$HOME` directly. The `counter` in backup names prevents same-second collisions.

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test editor::autosave`
Expected: 10 passed. Full `cargo test` green.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): session backups, atomic write, mtime conflict check"
```

---

### Task 9: Editor entity — read-only styled rendering (GPUI shell)

**Files:**
- Modify: `src/editor/mod.rs` (Editor entity + line rendering)
- Modify: `src/theme.rs` (add `pub fn style_color(&self, kind: &StyleKind) -> ...` helper is NOT added here; color mapping lives in editor/mod.rs to keep theme dependency-free)

**Interfaces:**
- Consumes: `EditorCore`, `spans`, `theme`, `highlight::languages`.
- Produces (used by Tasks 10–12):
  - `Editor::open(path: &Path, langs: &Languages) -> io::Result<Editor>` (reads file, picks provider by extension via `reader::language_for_path` / markdown detection)
  - `Editor::text(&self) -> String`, `Editor::path(&self) -> &Path`, `Editor::title(&self) -> SharedString`
  - `Editor::heading_lines(&self) -> Vec<(u8, String, usize)>` (level, text, line ix — for the outline)
  - `Editor::scroll_to_line(&mut self, ix: usize)`
  - `impl Render for Editor`, `impl Focusable for Editor`

**No unit tests (GPUI shell).** Manual verification at the end.

- [ ] **Step 1: Study the exact shaping APIs**

Read in the unpacked gpui source (path in Global Constraints):
- `src/text_system.rs`: `WindowTextSystem::shape_text(text: SharedString, font_size, runs: &[TextRun], wrap_width: Option<Pixels>, ...) -> Result<SmallVec<[WrappedLine; 1]>>` — confirm exact signature; `WrappedLine` provides `size(line_height)`, `position_for_index`, `index_for_position`, `paint`.
- `examples/text_wrapper.rs` for a working wrap example.

- [ ] **Step 2: Implement the Editor entity and line element**

`src/editor/mod.rs` becomes:

```rust
pub mod autosave;
pub mod buffer;
pub mod core;
pub mod movement;
pub mod spans;

use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui::prelude::*;
use gpui::{
    div, list, px, relative, App, Context, FocusHandle, Focusable, Font, FontFeatures, FontStyle,
    FontWeight, Hsla, IntoElement, ListAlignment, ListOffset, ListState, Render, SharedString,
    StyledText, TextRun, UnderlineStyle, Window,
};

use crate::highlight::Languages;
use crate::reader::language_for_path;
use crate::theme::{theme, Theme};
use autosave::SavePolicy;
use core::EditorCore;
use spans::{LineKind, StyleKind, StyleSpan};

enum Provider {
    Markdown,
    Code(&'static str),
    Plain,
}

pub struct Editor {
    core: EditorCore,
    provider: Provider,
    spans: Vec<StyleSpan>,
    line_kinds: Vec<LineKind>,
    path: PathBuf,
    pub save: SavePolicy,
    pub disk_mtime: Option<SystemTime>,
    list_state: ListState,
    focus_handle: FocusHandle,
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "markdown" | "mdown" | "mdx")
    )
}

impl Editor {
    pub fn open(path: &Path, langs: &Languages, cx: &mut Context<Self>) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let provider = if is_markdown(path) {
            Provider::Markdown
        } else if let Some(lang) = language_for_path(path) {
            Provider::Code(lang)
        } else {
            Provider::Plain
        };
        let core = EditorCore::new(&text);
        let line_count = core.buffer.line_count();
        let mut editor = Self {
            core,
            provider,
            spans: Vec::new(),
            line_kinds: Vec::new(),
            path: path.to_path_buf(),
            save: SavePolicy::default(),
            disk_mtime: autosave::disk_mtime(path),
            list_state: ListState::new(line_count, ListAlignment::Top, px(512.)),
            focus_handle: cx.focus_handle(),
        };
        editor.restyle(langs);
        Ok(editor)
    }

    pub fn text(&self) -> String {
        self.core.buffer.text()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn title(&self) -> SharedString {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
            .into()
    }

    fn restyle(&mut self, langs: &Languages) {
        let text = self.core.buffer.text();
        self.spans = match self.provider {
            Provider::Markdown => spans::markdown_spans_highlighted(&text, langs),
            Provider::Code(lang) => spans::code_spans(&text, lang, langs),
            Provider::Plain => Vec::new(),
        };
        self.line_kinds = spans::line_kinds(&text, &self.spans);
        self.list_state.reset(self.core.buffer.line_count());
    }

    pub fn heading_lines(&self) -> Vec<(u8, String, usize)> {
        self.spans
            .iter()
            .filter_map(|s| match s.kind {
                StyleKind::Heading(level) => {
                    let line = self.core.buffer.line_of_byte(s.range.start);
                    let text = self
                        .core
                        .buffer
                        .slice(s.range.clone())
                        .trim_start_matches('#')
                        .trim()
                        .to_string();
                    Some((level, text, line))
                }
                _ => None,
            })
            .collect()
    }

    pub fn scroll_to_line(&mut self, ix: usize) {
        self.list_state
            .scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
    }

    // ── styling helpers ────────────────────────────────────────────────

    fn line_typography(&self, ix: usize, t: &Theme) -> (f32, FontWeight, SharedString, f32) {
        // (font size, base weight, family, line height multiple)
        match self.line_kinds.get(ix) {
            Some(LineKind::Heading(n)) => {
                let weight = if *n <= 2 { FontWeight::BOLD } else { FontWeight::SEMIBOLD };
                (t.heading_size(*n), weight, t.body_family.clone(), 1.35)
            }
            Some(LineKind::Code) => (t.code_size, FontWeight::NORMAL, t.mono_family.clone(), 1.55),
            _ => (t.body_size, FontWeight::NORMAL, t.body_family.clone(), 1.65),
        }
    }

    fn syntax_color(capture: u8, t: &Theme) -> Option<Hsla> {
        let name = crate::highlight::CAPTURE_NAMES.get(capture as usize)?;
        let root = name.split('.').next().unwrap_or(name);
        let s = &t.syntax;
        Some(match root {
            "attribute" => s.attribute,
            "comment" => s.comment,
            "constant" | "number" => s.constant,
            "constructor" | "type" => s.kind,
            "function" => s.function,
            "keyword" => s.keyword,
            "operator" | "punctuation" => s.operator,
            "property" => s.property,
            "string" => s.string,
            "tag" => s.tag,
            _ => return None,
        })
    }

    /// Build the styled TextRuns for one line (must cover the whole line).
    fn line_runs(&self, ix: usize, t: &Theme) -> (SharedString, Vec<TextRun>) {
        let range = self.core.buffer.line_range(ix);
        let text = self.core.buffer.line_text(ix);
        let (_, base_weight, family, _) = self.line_typography(ix, t);

        let base_font = |weight: FontWeight, italic: bool, fam: &SharedString| Font {
            family: fam.clone(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight,
            style: if italic { FontStyle::Italic } else { FontStyle::Normal },
        };

        // Collect per-byte attribute deltas from spans intersecting this line.
        #[derive(Clone)]
        struct Attr {
            color: Hsla,
            weight: FontWeight,
            italic: bool,
            family: SharedString,
            bg: Option<Hsla>,
            underline: bool,
            strike: bool,
        }
        let default_attr = Attr {
            color: t.fg,
            weight: base_weight,
            italic: false,
            family: family.clone(),
            bg: None,
            underline: false,
            strike: false,
        };
        let mut attrs: Vec<Attr> = vec![default_attr.clone(); text.len()];
        for span in &self.spans {
            let start = span.range.start.max(range.start);
            let end = span.range.end.min(range.end);
            if start >= end {
                continue;
            }
            for a in &mut attrs[start - range.start..end - range.start] {
                match &span.kind {
                    StyleKind::Heading(_) => a.color = t.fg_strong,
                    StyleKind::Strong => a.weight = FontWeight::BOLD,
                    StyleKind::Emphasis => a.italic = true,
                    StyleKind::Strikethrough => a.strike = true,
                    StyleKind::InlineCode => {
                        a.family = t.mono_family.clone();
                        a.bg = Some(t.code_bg);
                        a.color = t.code_fg;
                    }
                    StyleKind::Link => {
                        a.color = t.link;
                        a.underline = true;
                    }
                    StyleKind::ListMarker | StyleKind::QuoteMarker => a.color = t.accent,
                    StyleKind::Rule => a.color = t.fg_muted,
                    StyleKind::FenceContent => a.color = t.code_fg,
                    StyleKind::Syntax(capture) => {
                        if let Some(c) = Self::syntax_color(*capture, t) {
                            a.color = c;
                        }
                        if crate::highlight::CAPTURE_NAMES
                            .get(*capture as usize)
                            .is_some_and(|n| n.starts_with("comment"))
                        {
                            a.italic = true;
                        }
                    }
                }
            }
        }

        // Compress equal-attr byte runs into TextRuns (must respect char
        // boundaries; attrs only change at span edges which are char
        // boundaries by construction).
        let mut runs: Vec<TextRun> = Vec::new();
        let mut i = 0;
        while i < attrs.len() {
            let mut j = i + 1;
            let eq = |a: &Attr, b: &Attr| {
                a.color == b.color
                    && a.weight == b.weight
                    && a.italic == b.italic
                    && a.family == b.family
                    && a.bg == b.bg
                    && a.underline == b.underline
                    && a.strike == b.strike
            };
            while j < attrs.len() && eq(&attrs[i], &attrs[j]) {
                j += 1;
            }
            let a = &attrs[i];
            runs.push(TextRun {
                len: j - i,
                font: base_font(a.weight, a.italic, &a.family),
                color: a.color,
                background_color: a.bg,
                underline: a.underline.then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(a.color),
                    wavy: false,
                }),
                strikethrough: a.strike.then_some(gpui::StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(t.fg_muted),
                }),
            });
            i = j;
        }
        if runs.is_empty() {
            // Empty line still needs one zero-len-safe run for shaping.
            runs.push(TextRun {
                len: 0,
                font: base_font(base_weight, false, &family),
                color: t.fg,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        (SharedString::from(text), runs)
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.weak_entity();
        let t = theme(cx);
        div()
            .size_full()
            .bg(t.bg)
            .key_context("Editor")
            .track_focus(&self.focus_handle)
            .child(
                list(self.list_state.clone(), move |ix, _window, cx| {
                    let Some(editor) = entity.upgrade() else {
                        return div().into_any_element();
                    };
                    let t = theme(cx);
                    let editor = editor.read(cx);
                    let (size, _, family, line_height) = editor.line_typography(ix, &t);
                    let (text, runs) = editor.line_runs(ix, &t);
                    let is_code = matches!(editor.line_kinds.get(ix), Some(LineKind::Code));
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .child(
                            div()
                                .w_full()
                                .max_w(px(760.))
                                .px(px(48.))
                                .when(ix == 0, |d| d.pt(px(40.)))
                                .when(ix + 1 == editor.core.buffer.line_count(), |d| {
                                    d.pb(px(96.))
                                })
                                .font_family(family)
                                .text_size(px(size))
                                .line_height(relative(line_height))
                                .when(is_code, |d| d.bg(t.code_bg))
                                .child(if text.is_empty() {
                                    // preserve empty-line height
                                    div().h(px(size * line_height)).into_any_element()
                                } else {
                                    StyledText::new(text).with_runs(runs).into_any_element()
                                })
                        )
                        .into_any_element()
                })
                .size_full(),
            )
    }
}
```

- [ ] **Step 3: Temporary wiring for manual verification**

In `src/workspace.rs`, TEMPORARILY make `open_path` route `.md` files to the editor so it can be seen (Task 12 does this properly with the tab enum — for now, add a `debug_editor: Option<Entity<crate::editor::Editor>>` field rendered instead of the reader when set, or simply test via a one-off: change `open_path` to open an Editor and render it as the content pane when present). Keep the change minimal and clearly commented `// TEMP: Task 9 verification, replaced in Task 12`.

- [ ] **Step 4: Build and manually verify**

Run: `cargo build && (pkill -f target/debug/supermd; ./target/debug/supermd &)`
Verify by opening `WELCOME.md` and `src/main.rs` from the sidebar:
- Markdown: headings render large WITH `#` visible; `**bold**` bold with asterisks; inline code tinted mono; list dashes accent-colored; fence bodies syntax-colored on `code_bg` background lines.
- Rust file: whole file mono with tree-sitter colors.
- Scrolling is smooth; light/dark theme both look right.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): Editor entity with styled-source line rendering"
```

---

### Task 10: Editing input — IME, keyboard actions, mouse

**Files:**
- Modify: `src/editor/mod.rs`
- Modify: `src/main.rs` (bindings)

**Interfaces:**
- Consumes: Tasks 3, 4, 9.
- Produces:
  - `impl EntityInputHandler for Editor` (typing/IME path)
  - Actions in `editor` namespace: `MoveLeft, MoveRight, MoveUp, MoveDown, SelectLeft, SelectRight, SelectUp, SelectDown, MoveWordLeft, MoveWordRight, SelectWordLeft, SelectWordRight, LineStart, LineEnd, SelectLineStart, SelectLineEnd, DocStart, DocEnd, Backspace, Delete, DeleteWordLeft, Newline, InsertTab, Undo, Redo, SelectAll, Copy, Cut, Paste, SaveNow`
  - Editor caret/selection painting and mouse click/drag positioning

**No unit tests (GPUI shell) — all editing LOGIC is already tested in core. Manual verification.**

- [ ] **Step 1: Line layout cache + custom line element**

Replace the `StyledText` child in the line item with a custom `LineElement` (same pattern as `input.rs`'s `TextElement`, one per line):

```rust
struct LineElement {
    editor: gpui::Entity<Editor>,
    line_ix: usize,
    runs: Vec<TextRun>,
    text: SharedString,
    font_size: gpui::Pixels,
    line_height: gpui::Pixels,
}
```

- `request_layout`: `window.request_measured_layout` with a measure closure that shapes `text` via `window.text_system().shape_text(text, font_size, &runs, Some(known_width), None)` and returns `size(width, total_height_of_wrapped_lines)`. (Check `shape_text`'s exact signature in the gpui source first.)
- `prepaint`: shape again (or reuse via `RefCell` cache keyed by `(line_ix, width)`), compute:
  - selection quads: intersect `editor.core.selection.range()` with this line's byte range; for each wrapped row, `position_for_index` start/end → `fill(bounds, selection_color)`.
  - caret quad if `selection.is_cursor()` and the head is on this line: `position_for_index(head - line_start)` → 2px wide `fill` with `t.accent`.
- `paint`:
  - `window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.editor.clone()), cx)` **only for the line containing the cursor** (keeps IME candidate window positioned at the caret).
  - paint selection quads, then `WrappedLine::paint`, then caret if focused.
  - store `(line_ix, WrappedLine, bounds)` into the editor's `RefCell<HashMap<usize, (WrappedLine, Bounds<Pixels>)>>` layout cache for mouse hit-testing and `bounds_for_range`.
  - `on_mouse_down` for the line: convert position → byte offset via `index_for_position`, then `editor.update(cx, |e, cx| { e.core.set_cursor(line_start + ix); e.core.break_undo_group(); cx.notify(); })`; shift-click extends (`select_to`). Register drag via mouse-move while a `dragging` flag is set on the editor (same approach as `input.rs`'s `is_selecting`).

- [ ] **Step 2: EntityInputHandler on Editor**

Implement `EntityInputHandler for Editor` exactly as `input.rs` does for `TextInput`, with these differences:
- offsets span the whole buffer: `text_for_range`/`replace_text_in_range` etc. use `self.core.buffer` text; UTF-16↔UTF-8 helpers copy from `input.rs` (operate on `self.core.buffer.text()` — O(n), acceptable this phase).
- `replace_text_in_range(range, text)`: map range → bytes; `self.core.set_cursor/select` to that range then `self.core.insert(text, Instant::now())`; then `self.after_edit(cx)`.
- `after_edit(&mut self, cx)`: `self.restyle(langs)`, `self.save.record_edit(Instant::now())`, schedule debounce flush (Task 11), `self.list_state.reset(line_count)`, scroll cursor line into view (`scroll_to_reveal_item(cursor_line)`), `cx.notify()`.
- `bounds_for_range`: look up the cursor line in the layout cache; return caret rect from `position_for_index`.
- `replace_and_mark_text_in_range` / `marked_text_range`: keep a `marked_range: Option<Range<usize>>` field like `input.rs`; marked text renders underlined (add to `line_runs` via a transient span — acceptable to skip visual underline for marked text in this task if it complicates; correctness of composition commits matters more).

- [ ] **Step 3: Actions + handlers + bindings**

In `src/editor/mod.rs`:

```rust
gpui::actions!(
    editor,
    [
        MoveLeft, MoveRight, MoveUp, MoveDown, SelectLeft, SelectRight, SelectUp, SelectDown,
        MoveWordLeft, MoveWordRight, SelectWordLeft, SelectWordRight, LineStart, LineEnd,
        SelectLineStart, SelectLineEnd, DocStart, DocEnd, Backspace, Delete, DeleteWordLeft,
        Newline, InsertTab, Undo, Redo, SelectAll, Copy, Cut, Paste, SaveNow
    ]
);
```

Handlers as `impl Editor` methods with signature `fn(&mut self, &Action, &mut Window, &mut Context<Self>)`, registered with `.on_action(cx.listener(...))` on the root div in `render`. Movement handlers: compute target via `movement::*` (horizontal) — each also calls `break_undo_group`; `Move*` collapse the selection to the target (`set_cursor`), `Select*` extend (`select_to`). Up/Down: find cursor's wrapped row via layout cache; move to prev/next row at `preferred_x` (a `preferred_x: Option<Pixels>` field set on first vertical move, cleared on horizontal moves); if at first/last row of a line, cross to adjacent line. Fall back to logical-line movement when the layout cache misses (line off-screen). `Copy`/`Cut`/`Paste` use `cx.write_to_clipboard`/`read_from_clipboard` like `input.rs`; `Newline` inserts `"\n"`; `InsertTab` inserts `"\t"`; `DeleteWordLeft` selects to `prev_word` then deletes. `Undo`/`Redo` call core then `after_edit` (minus `record_edit`? No: undo/redo change the buffer → they must also `record_edit` for autosave; do call the full `after_edit`).

In `src/main.rs`, add `"Editor"`-context bindings:

```rust
KeyBinding::new("left", editor::MoveLeft, Some("Editor")),
KeyBinding::new("right", editor::MoveRight, Some("Editor")),
KeyBinding::new("up", editor::MoveUp, Some("Editor")),
KeyBinding::new("down", editor::MoveDown, Some("Editor")),
KeyBinding::new("shift-left", editor::SelectLeft, Some("Editor")),
KeyBinding::new("shift-right", editor::SelectRight, Some("Editor")),
KeyBinding::new("shift-up", editor::SelectUp, Some("Editor")),
KeyBinding::new("shift-down", editor::SelectDown, Some("Editor")),
KeyBinding::new("alt-left", editor::MoveWordLeft, Some("Editor")),
KeyBinding::new("alt-right", editor::MoveWordRight, Some("Editor")),
KeyBinding::new("alt-shift-left", editor::SelectWordLeft, Some("Editor")),
KeyBinding::new("alt-shift-right", editor::SelectWordRight, Some("Editor")),
KeyBinding::new("cmd-left", editor::LineStart, Some("Editor")),
KeyBinding::new("cmd-right", editor::LineEnd, Some("Editor")),
KeyBinding::new("cmd-shift-left", editor::SelectLineStart, Some("Editor")),
KeyBinding::new("cmd-shift-right", editor::SelectLineEnd, Some("Editor")),
KeyBinding::new("home", editor::LineStart, Some("Editor")),
KeyBinding::new("end", editor::LineEnd, Some("Editor")),
KeyBinding::new("cmd-up", editor::DocStart, Some("Editor")),
KeyBinding::new("cmd-down", editor::DocEnd, Some("Editor")),
KeyBinding::new("backspace", editor::Backspace, Some("Editor")),
KeyBinding::new("delete", editor::Delete, Some("Editor")),
KeyBinding::new("alt-backspace", editor::DeleteWordLeft, Some("Editor")),
KeyBinding::new("enter", editor::Newline, Some("Editor")),
KeyBinding::new("tab", editor::InsertTab, Some("Editor")),
KeyBinding::new("cmd-z", editor::Undo, Some("Editor")),
KeyBinding::new("cmd-shift-z", editor::Redo, Some("Editor")),
KeyBinding::new("cmd-a", editor::SelectAll, Some("Editor")),
KeyBinding::new("cmd-c", editor::Copy, Some("Editor")),
KeyBinding::new("cmd-x", editor::Cut, Some("Editor")),
KeyBinding::new("cmd-v", editor::Paste, Some("Editor")),
KeyBinding::new("cmd-s", editor::SaveNow, Some("Editor")),
```

(`SaveNow`'s handler is a no-op stub until Task 11 — `eprintln!("save requested")`.)

- [ ] **Step 4: Build and manually verify**

Run: `cargo build && (pkill -f target/debug/supermd; ./target/debug/supermd &)`
Verify in an open .md file: click positions caret; typing inserts styled text live (type `**hi**` → turns bold as the closing `*` lands); enter/backspace across lines; arrows + shift-selection; alt/cmd arrows; ⌘A/C/V; ⌘Z undoes a typing burst as one group; styles update as you type; heading line grows when you prefix `# `. Screenshot-level check in both themes.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(editor): full editing input — IME, keyboard actions, mouse"
```

---

### Task 11: Autosave wiring — debounce task, backups, conflicts, flush points

**Files:**
- Modify: `src/editor/mod.rs`
- Modify: `src/main.rs` (quit flush)
- Modify: `src/workspace.rs` (tab switch/close flush — finalized in Task 12; here just the Editor-side API)

**Interfaces:**
- Consumes: Tasks 7, 8, 10.
- Produces:
  - `Editor::flush(&mut self, cx: &mut Context<Self>)` — the one save path: conflict check → backup(s) → atomic write → mtime update → `mark_saved`
  - Global `SessionBackups(pub Arc<Mutex<BackupRegistry>>)` in `main.rs` (one registry per app session)

- [ ] **Step 1: Implement flush + debounce**

In `src/editor/mod.rs`:

```rust
pub struct SessionBackups(pub std::sync::Arc<std::sync::Mutex<autosave::BackupRegistry>>);
impl gpui::Global for SessionBackups {}

impl Editor {
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        if !self.save.take_flush_now() {
            return;
        }
        let text = self.core.buffer.text();
        let backups = cx.global::<SessionBackups>().0.clone();
        let mut backups = backups.lock().unwrap();
        if autosave::has_conflict(self.disk_mtime, &self.path) {
            // Never silently clobber external edits: preserve the disk copy.
            if let Err(err) = backups.force_backup(&self.path) {
                eprintln!("supermd: conflict backup failed for {}: {err}", self.path.display());
            } else {
                eprintln!(
                    "supermd: {} changed on disk; disk version backed up before overwrite",
                    self.path.display()
                );
            }
        } else if let Err(err) = backups.backup_if_needed(&self.path) {
            eprintln!("supermd: backup failed for {}: {err}", self.path.display());
        }
        match autosave::atomic_write(&self.path, &text) {
            Ok(()) => {
                self.disk_mtime = autosave::disk_mtime(&self.path);
                self.save.mark_saved();
            }
            Err(err) => {
                // Stay dirty; next edit or flush point retries.
                eprintln!("supermd: save failed for {}: {err}", self.path.display());
            }
        }
    }
}
```

Debounce scheduling in `after_edit` (add field `save_task: Option<gpui::Task<()>>`):

```rust
self.save.record_edit(std::time::Instant::now());
self.save_task = Some(cx.spawn(async move |this, cx| {
    cx.background_executor().timer(autosave::DEBOUNCE).await;
    this.update(cx, |editor, cx| {
        if editor.save.should_flush(std::time::Instant::now()) {
            editor.flush(cx);
        }
    })
    .ok();
}));
```

(Replacing `save_task` drops the previous task, cancelling the stale timer — that plus the `should_flush` re-check makes the debounce correct. Check `cx.spawn`/`background_executor().timer` exact signatures in the gpui source.)

`SaveNow` handler: `self.flush(cx)`.

In `main.rs`: `cx.set_global(editor::SessionBackups(Arc::new(Mutex::new(autosave::BackupRegistry::new(autosave::BackupRegistry::default_dir())))));` and register quit flush — check gpui source for `cx.on_app_quit`; in each editor's release or via workspace iteration on quit, call `flush`. (`cx.on_app_quit(move |cx| ...)` receives the app context; iterate the workspace's editors and flush each. If `on_app_quit` is awkward with entity access, an acceptable fallback: flush in `Workspace`'s `Drop`-adjacent hook or on window close action — document what you did in the commit message.)

- [ ] **Step 2: Build and manually verify**

Run: `cargo build && (pkill -f target/debug/supermd; ./target/debug/supermd &)`
- Edit a scratch file, stop typing 1s → `stat -f %m <file>` shows fresh mtime; content on disk matches.
- First save of the session created `~/.supermd/backups/<stamp>-...-<name>` with the ORIGINAL content.
- `⌘S` flushes immediately.
- External-change drill: open file in supermd, `echo external >> file` in terminal, type in supermd, wait for autosave → disk version appears as a second backup, supermd's content wins on disk, warning on stderr.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(editor): autosave wiring — debounce, session backups, conflict guard"
```

---

### Task 12: Workspace integration — tab enum, ⌘E preview, outline, cleanup

**Files:**
- Modify: `src/workspace.rs`
- Modify: `src/reader.rs` (add `Reader::from_source(title, text, langs)` for preview-from-buffer)
- Modify: `src/main.rs` (⌘E binding + View menu item)

**Interfaces:**
- Consumes: everything.
- Produces:
  - `enum Tab { Reader(Entity<Reader>), Editor { editor: Entity<Editor>, preview: Option<Entity<Reader>> } }`
  - `TogglePreview` action (⌘E) in `workspace` namespace
  - Flush-on-tab-switch and flush-on-close behavior

- [ ] **Step 1: Replace `readers: Vec<Entity<Reader>>` with `tabs: Vec<Tab>`**

- `Tab::title(cx)`, `Tab::path(cx) -> Option<PathBuf>` helpers.
- `open_path`: UTF-8-readable file → `Tab::Editor { editor: cx.new(|cx| Editor::open(...)), preview: None }`; unreadable → existing read-only Reader fallback; Welcome stays `Tab::Reader`.
- Tab activation change (`self.active = ix` in click handler, `next_tab`, `prev_tab`): flush the outgoing editor first: `if let Tab::Editor { editor, .. } = &self.tabs[old] { editor.update(cx, |e, cx| e.flush(cx)) }`. Same in `close_tab_at` before removal. On activation of an editor tab, focus it: `window.focus(&editor.focus_handle(cx))` (requires the click/action handlers to have `window`; they do).
- Content pane: `Tab::Editor { preview: Some(reader), .. }` renders the reader; `preview: None` renders the editor; `Tab::Reader` renders the reader. Remove the Task 9 TEMP wiring.
- `TogglePreview` handler: on the active editor tab — if `preview.is_none()`, flush editor, build `Reader::from_source(title, editor.text(), &languages(cx))`, set `preview = Some(...)`; else set `preview = None` and refocus the editor.
- Outline panel: for `Tab::Editor` in edit mode use `editor.heading_lines()` → click calls `editor.update(cx, |e, cx| { e.scroll_to_line(line); cx.notify(); })`; preview/reader tabs keep the existing Reader TOC path.

`Reader::from_source` in `src/reader.rs`:

```rust
pub fn from_source(title: SharedString, source: &str, langs: &Languages) -> Self {
    let mut document = markdown::parse(source);
    langs.highlight_document(&mut document);
    Self::from_document(None, title, document)
}
```

- [ ] **Step 2: Bindings and menu**

`src/main.rs`: `KeyBinding::new("cmd-e", TogglePreview, None)` (workspace-level, not Editor context — it must work from preview mode too), View menu: `MenuItem::action("Toggle Edit/Preview", TogglePreview)` plus separator.

- [ ] **Step 3: Build, run full test suite, manually verify**

Run: `cargo test && cargo build && (pkill -f target/debug/supermd; ./target/debug/supermd &)`
- Sidebar/finder open .md and .rs files in the editor; Welcome tab still read-only pretty view.
- `⌘E` on an editor tab shows the pretty preview (tables render properly); `⌘E` again returns to editing with cursor state intact.
- Tab switch and close flush pending edits (verify mtime).
- Outline click scrolls in edit mode AND preview mode.
- Typing works immediately after opening a file (focus follows).

- [ ] **Step 4: Update WELCOME.md roadmap table**

Mark Phase 2 "In progress" → done; Phase 3 becomes "Next". Keep the file exercising every feature.

- [ ] **Step 5: Final commit**

```bash
git add -A && git commit -m "feat(editor): workspace integration — editor tabs, cmd-E preview, outline"
```

---

## Self-review (performed at plan time)

- **Spec coverage:** buffer/selection/undo → Tasks 1–4; span providers incl. fence shifting, oversize cap, line typography → Tasks 5–6; autosave policy/backups/atomic/conflicts → Tasks 7–8; GPUI shell (list-per-line, IME, mouse, vertical movement, cursor reveal) → Tasks 9–10; flush points, session backup registry, quit flush → Task 11; tab enum, ⌘E, outline-in-edit, bindings, welcome exception → Task 12. Error handling: read failure (Task 12 fallback), save failure retry (Task 11), unstyled-on-oversize (Task 6). Out-of-scope items untouched. ✓
- **Placeholders:** GPUI-task steps reference exact gpui source files for signature verification rather than inventing signatures — deliberate, since gpui 0.2.2 signatures must be read, not guessed. All logic code is written out. ✓
- **Type consistency:** `EditorCore` field/method names checked across Tasks 3/4/9/10; `StyleSpan`/`StyleKind` across 5/6/9; `SavePolicy`/`BackupRegistry` across 7/8/11; `Tab` enum across 11/12. `Editor::open` takes `cx` (Task 9) — Task 12's `cx.new(|cx| Editor::open(...))` matches. ✓
