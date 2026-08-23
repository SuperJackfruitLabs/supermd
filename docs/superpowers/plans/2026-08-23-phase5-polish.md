# Phase 5: Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Task checkboxes with click-to-toggle, find-in-file, window/sidebar/scrollbar chrome, and file watching.

**Architecture:** Each workstream follows the established split: pure tested logic (task-marker spans, checkbox display transform, `replace_range`, `find_matches`, `expand_to`/untitled naming, reload policy) under `cargo test`; thin GPUI shell for bars, scrollbar, watcher bridge, and title.

**Tech Stack:** Existing crates + `notify = "6"` (Task 9 only).

**Spec:** `docs/superpowers/specs/2026-08-23-phase5-polish-design.md`

## Global Constraints

- TDD iron law for all pure changes. Byte offsets, char-boundary safety in find.
- Case folding in find is ASCII-only (documented); smart case = query contains an uppercase char.
- Reveal rule for TaskMarker identical to every other span.
- `cargo test` green before each commit; repo trailers.

---

### Task 1: TaskMarker spans (TDD)

**Files:** `src/editor/spans.rs`

- [ ] RED — append tests:

```rust
    #[test]
    fn task_markers_spanned_with_checked_state() {
        let src = "- [x] done\n- [ ] todo\n";
        let spans = markdown_spans(src);
        assert!(spans.contains(&StyleSpan { range: 2..5, kind: StyleKind::TaskMarker(true) }));
        assert!(spans.contains(&StyleSpan { range: 13..16, kind: StyleKind::TaskMarker(false) }));
    }
```

(If pulldown's `TaskListMarker` range includes the trailing space, correct the constants to its actual behavior — oracle fix, note in commit.)

- [ ] GREEN — add `StyleKind::TaskMarker(bool)`; in the event loop: `Event::TaskListMarker(done) => spans.push(StyleSpan { range, kind: StyleKind::TaskMarker(done) })`. Add exhaustive-match arms the compiler demands (editor attr arm: checked → `t.accent`, unchecked → `t.fg_muted`).
- [ ] Full suite; commit `feat(editor): TaskMarker spans`.

### Task 2: Checkbox display transform + toggle payload (TDD)

**Files:** `src/editor/display.rs`

- [ ] RED — tests:

```rust
    #[test]
    fn checkbox_replacement_and_toggle_payload() {
        let src = "- [x] done";
        let spans = [
            span(0..2, StyleKind::ListMarker),
            span(2..5, StyleKind::TaskMarker(true)),
        ];
        let dl = display_line(src, 0, &spans, 100..100);
        assert_eq!(dl.text, "• ✓ done");
        let seg = dl.segs.iter().find(|s| s.toggle.is_some()).unwrap();
        assert_eq!(seg.toggle, Some(true));
        assert_eq!(seg.src, 2..5);
        let dl = display_line(src, 0, &spans, 3..3);
        assert_eq!(dl.text, "- [x] done"); // revealed
    }

    #[test]
    fn unchecked_checkbox_glyph() {
        let src = "- [ ] todo";
        let spans = [
            span(0..2, StyleKind::ListMarker),
            span(2..5, StyleKind::TaskMarker(false)),
        ];
        let dl = display_line(src, 0, &spans, 100..100);
        assert_eq!(dl.text, "• ○ todo");
        assert_eq!(dl.segs.iter().find_map(|s| s.toggle), Some(false));
    }
```

- [ ] GREEN — `Seg` gains `pub toggle: Option<bool>` (all existing constructions get `None`); `Action::Replace` becomes `Replace { text: &'static str, toggle: Option<bool> }`; TaskMarker arm emits `✓`/`○` replacements carrying `Some(checked)`; bullet/quote replacements carry `None`.
- [ ] Full suite; commit `feat(editor): checkbox glyph transform with toggle payload`.

### Task 3: EditorCore::replace_range (TDD)

**Files:** `src/editor/core.rs`

- [ ] RED:

```rust
    #[test]
    fn replace_range_edits_through_history() {
        let mut ed = EditorCore::new("- [x] done");
        ed.set_cursor(8);
        ed.replace_range(2..5, "[ ]", t0());
        assert_eq!(ed.buffer.text(), "- [ ] done");
        assert_eq!(ed.selection, Selection::cursor(5));
        assert!(ed.undo());
        assert_eq!(ed.buffer.text(), "- [x] done");
    }
```

- [ ] GREEN — `pub fn replace_range(&mut self, range: Range<usize>, text: &str, now: Instant) { self.apply(range, text, now); }`
- [ ] Full suite; commit `feat(editor): EditorCore::replace_range`.

### Task 4: Checkbox shell — click-to-toggle (manual)

**Files:** `src/editor/mod.rs`

- [ ] In `on_line_mouse_down`, before cursor placement: from the clicked line's `CachedLine`, compute the display index (`closest_index_for_position`); if a seg with `toggle: Some(checked)` contains it (`disp.start <= ix < max(disp.end, disp.start+1)`), save the selection, `replace_range(seg.src, "[ ]"/"[x]")`, restore the selection (same byte length — positions stay valid), `after_edit`, return without moving the cursor.
- [ ] Manual verify in WELCOME.md: ✓/○ glyphs render (accent/muted); clicking toggles the checkbox and the file autosaves; ⌘Z undoes the toggle; cursor entering the line reveals raw `[x]`. Suite + build green.
- [ ] Commit `feat(editor): click-to-toggle task checkboxes`.

### Task 5: find.rs — find_matches (TDD)

**Files:** create `src/editor/find.rs`; `mod.rs` registration

- [ ] RED:

```rust
    #[test]
    fn case_insensitive_by_default() {
        assert_eq!(find_matches("Foo foo FOO", "foo"), vec![0..3, 4..7, 8..11]);
    }

    #[test]
    fn smart_case_when_query_has_uppercase() {
        assert_eq!(find_matches("Foo foo FOO", "Foo"), vec![0..3]);
    }

    #[test]
    fn non_overlapping_and_empty_query() {
        assert_eq!(find_matches("aaaa", "aa"), vec![0..2, 2..4]);
        assert!(find_matches("anything", "").is_empty());
    }

    #[test]
    fn unicode_boundaries_are_respected() {
        assert_eq!(find_matches("héllo héllo", "llo"), vec![3..6, 10..13]);
    }
```

- [ ] GREEN — scan char boundaries; sensitive: slice equality; insensitive: `eq_ignore_ascii_case`; skip `query.len()` after a match; guard `is_char_boundary` on both ends.
- [ ] Full suite; commit `feat(editor): find_matches`.

### Task 6: Find bar shell (manual)

**Files:** `src/editor/mod.rs`, `src/theme.rs`, `src/main.rs`

- [ ] Theme: `find_match_bg` / `find_active_bg` (light: `0xffe9a3` / `0xffc94d`-ish; dark: `0x51431a` / `0x7a6220`-ish), both palettes.
- [ ] Editor: `find: Option<FindState { input: Entity<TextInput>, matches: Vec<Range<usize>>, active: usize, _watch: Subscription }>`; actions `OpenFind, FindNext, FindPrev, CloseFind`. Open: build input ("Find…"), observe → recompute matches from `input.content` (reset `active` to the first match at/after the cursor), focus input. Next/Prev: cycle `active`, set core selection to the match, break undo group, reveal, notify. Close: drop state, focus editor. `after_edit` recomputes when open (clamp `active`).
- [ ] Highlight: in `line_attrs`, overlay `bg` for every match intersecting the line (`find_match_bg`), active match `find_active_bg`.
- [ ] UI: bar rendered above the list when open (row: input flex_1, "n/m" count muted, key_context "FindBar" with enter→FindNext, shift-enter→FindPrev, escape→CloseFind bound in that context). Bindings in main.rs: `cmd-f`→OpenFind, `cmd-g`→FindNext, `cmd-shift-g`→FindPrev (all `"Editor"` context — the bar lives inside the editor's context tree).
- [ ] Manual verify: ⌘F, type, all matches highlighted, enter cycles with reveal+scroll, count updates while typing and editing, escape returns to typing in the editor. Suite + build green; commit `feat(editor): find in file`.

### Task 7: Chrome — expand_to, ⌘N, window title (pure parts TDD)

**Files:** `src/files.rs`, `src/workspace.rs`, `src/main.rs`

- [ ] RED (files.rs):

```rust
    #[test]
    fn expand_to_opens_all_ancestors() {
        let mut tree = FileTree::new(PathBuf::from("/root"));
        tree.expand_to(Path::new("/root/a/b/c.md"));
        assert!(tree.is_expanded(Path::new("/root/a")));
        assert!(tree.is_expanded(Path::new("/root/a/b")));
        assert!(!tree.is_expanded(Path::new("/root/a/b/c.md")));
    }

    #[test]
    fn untitled_picks_first_free_name() {
        assert_eq!(pick_untitled(&[]), "Untitled.md");
        assert_eq!(pick_untitled(&["Untitled.md".into()]), "Untitled 2.md");
        assert_eq!(
            pick_untitled(&["Untitled.md".into(), "Untitled 2.md".into()]),
            "Untitled 3.md"
        );
    }
```

- [ ] GREEN — `FileTree::expand_to` walks `path.ancestors()` strictly between file and root inclusive of dirs; `pub fn pick_untitled(existing: &[String]) -> String`. Restore `FileTree::refresh()`.
- [ ] Shell: `NewFile` action (⌘N, menu File→New): workspace root required; `pick_untitled` over `read_dir` names, `fs::write` empty, `tree.refresh()`, `open_path`. `open_path` calls `tree.expand_to(path)` on file open. Window title: check `Window::set_window_title` in vendored source; call `supermd — <title>` from `set_active`/open/close paths (skip gracefully if API differs — document).
- [ ] Manual verify + suite; commit `feat(workspace): cmd-N, sidebar auto-expand, window title`.

### Task 8: Editor scrollbar (manual)

**Files:** `src/editor/mod.rs`

- [ ] Overlay on the editor's right edge using `ListState` scrollbar APIs: thumb height from viewport/(viewport+max_offset), position from `scroll_px_offset_for_scrollbar`; `on_mouse_down` on track → `set_offset_from_scrollbar` jump + `scrollbar_drag_started`; drag via editor-level mouse-move while flagged; `scrollbar_drag_ended` on up. Subtle styling: 6px wide, `fg_muted` at low alpha, stronger while dragging.
- [ ] Manual verify (long file): thumb tracks wheel scrolling, drag scrubs, click jumps. Suite + build; commit `feat(editor): overlay scrollbar`.

### Task 9: File watching

**Files:** `Cargo.toml` (`notify = "6"`), `src/editor/autosave.rs`, `src/editor/mod.rs`, `src/workspace.rs`

- [ ] RED (autosave.rs): `should_reload(dirty, mtime_changed)` truth-table test (only `(!dirty && changed)` reloads).
- [ ] GREEN — trivial fn.
- [ ] Shell: workspace owns `Option<RecommendedWatcher>` + spawned drain loop (200 ms background timer, `try_recv` drain, collect paths, `this.update` → `on_fs_events`). `on_fs_events`: `tree.refresh()` + notify; per editor tab with a matching path: `should_reload(save.is_dirty(), mtime differs)` → `Editor::reload_from_disk` (re-read, rebuild core, clamp cursor to a char boundary ≤ new len, restyle, fresh `SavePolicy`, new `disk_mtime`; history reset accepted per spec). Watcher (re)created in `new` and when a folder opens.
- [ ] Manual verify: `echo >> file` in terminal → open clean editor updates within ~a second; dirty editor keeps edits (stderr note); creating a file in Finder shows up in the sidebar. Suite + build; commit `feat: file watching with clean-buffer reload`.

### Task 10: Roadmap + finish

- [ ] WELCOME.md: Phase 5 row Done (checkboxes now demo click-to-toggle in the near-term list); suite/build/zero-project-warnings; commit `docs: Phase 5 complete`.
- [ ] superpowers:finishing-a-development-branch.

## Self-review

Spec coverage: checkboxes 1–4 (glyphs, reveal, toggle+undo+selection-preserve); find 5–6 (smart case, highlight colors both palettes, cycling, focus flow); chrome 7–8 (expand_to, untitled naming, title fallback note, scrollbar APIs); watching 9 (policy fn, coalescing bridge, dirty protection, watcher lifecycle); out-of-scope untouched. Placeholders: none. Types: `TaskMarker(bool)` 1↔2↔4; `Seg.toggle` 2↔4; `replace_range` 3↔4; `find_matches` 5↔6; `pick_untitled`/`expand_to` 7; `should_reload` 9. ✓
