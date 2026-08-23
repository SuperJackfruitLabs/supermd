# Phase 5: Polish — Design Spec

**Date:** 2026-08-23
**Status:** Approved design, pending implementation plan
**Depends on:** Phase 4 (document projection)

## Goal

Four quality workstreams: task checkboxes (✓/○ with click-to-toggle),
find-in-file (⌘F), file watching (sidebar refresh + clean-buffer
reload), and chrome (window title, editor scrollbar, sidebar
auto-expand, ⌘N new file). Fixes the reported gap that `- [x]` items
render with raw brackets in the editor.

## Decisions already made

- Checkbox glyphs match the preview: `✓` (accent) checked, `○` (muted)
  unchecked; raw `[x]`/`[ ]` reveal on cursor touch like all markers.
- Clicking the glyph toggles the source between `[x]` and `[ ]` without
  moving the cursor into the line; full undo/autosave integration.
- Find: case-insensitive, smart-case when the query contains uppercase;
  cycling sets the selection to the match (auto-reveal + scroll reuse).
- Watching: `notify` crate, 200 ms coalescing poll bridge; dirty
  buffers are never clobbered by reloads (mtime-conflict backups
  already protect the save path).
- Skipped deliberately: animated block transitions, sidebar rename,
  sidebar scroll-into-view (parents auto-expand only).
- Build order: checkboxes → find → chrome → watching (new dep last).

## 1. Task checkboxes

- `spans.rs`: `StyleKind::TaskMarker(bool)` from pulldown's
  `TaskListMarker` event offset range (the `[x]`/`[ ]` bytes).
- `display.rs`: replacement directive `[x]` → `✓`, `[ ]` → `○` when
  not revealed. `Seg` gains `pub toggle: Option<bool>` (the checked
  state) set on TaskMarker replacements; all other segs `None`.
- `core.rs`: `pub fn replace_range(&mut self, range, text, now)` —
  applies an arbitrary edit through the normal history path and places
  the cursor at the edit end (tested).
- Shell: `on_line_mouse_down` first hit-tests the clicked display
  offset against segs with `toggle: Some(_)`; on hit, flips the source
  bytes via `replace_range` **preserving the prior selection** (save
  and restore around the edit), then `after_edit`. No cursor move into
  the line.
- Styling: `✓` uses `t.accent`, `○` uses `t.fg_muted` (attr override at
  projection time keyed off the TaskMarker span kind).

## 2. Find in file

- New pure fn (in a new `src/editor/find.rs`):
  `find_matches(text: &str, query: &str) -> Vec<Range<usize>>` —
  empty query → empty; case-insensitive unless the query contains an
  uppercase char (smart case); non-overlapping, sorted; byte ranges on
  char boundaries.
- Editor state: `find: Option<FindState { input: Entity<TextInput>,
  matches: Vec<Range<usize>>, active: usize, _watch: Subscription }>`.
- Actions (`editor` namespace): `OpenFind` (⌘F, routed from the
  workspace to the active editor tab), `FindNext` (enter/⌘G),
  `FindPrev` (shift-enter/⇧⌘G), `CloseFind` (escape) — enter/escape
  bound in a `"FindBar"` key context on the bar.
- Behavior: query changes recompute matches (observe the input);
  next/prev sets the core selection to the match (reveals + scrolls via
  existing paths) and breaks the undo group; edits recompute matches in
  `after_edit`; closing returns focus to the editor.
- Highlighting: `line_attrs` overlays `bg = t.find_match_bg` on all
  match ranges, `t.find_active_bg` on the active one. Two new theme
  colors in both light and dark palettes.
- UI: a bar pinned at the top of the editor pane (input, "n of m"
  count, and it is dismissible with escape); rendered by the Editor
  when `find` is `Some`.

## 3. File watching

- Dependency: `notify = "6"` (FSEvents on macOS), recommended watcher,
  recursive on the workspace root.
- Bridge: watcher events → `std::sync::mpsc`; a detached gpui task
  loops: sleep 200 ms (background timer), drain `try_recv`, and if any
  events arrived, notify the workspace entity.
- Sidebar: `FileTree::refresh()` (restored) + `cx.notify()`.
- Open editors: for each editor tab whose path saw an event, if
  `!save.is_dirty()` and disk mtime differs from `disk_mtime`, reload:
  new `Editor::reload_from_disk(&mut self, cx)` — re-reads the file,
  `EditorCore::new` content swap preserving cursor offset (clamped to
  the new length) — history reset is accepted (documented), restyle,
  update `disk_mtime`. Dirty editors: skip (log once); the save-path
  conflict backup already protects divergence.
- Watcher lifecycle: owned by the workspace; recreated when the root
  changes (⌘O folder).

## 4. Chrome & feel

- **Window title:** on active-tab change and tab open/close, set
  `supermd — <title>` (check `Window::set_window_title` in the
  vendored source; fall back to leaving the static title if absent —
  documented in the commit).
- **Scrollbar:** thin right-edge overlay in the editor built on
  `ListState` scrollbar APIs (`max_offset_for_scrollbar`,
  `scroll_px_offset_for_scrollbar`, `set_offset_from_scrollbar`,
  `scrollbar_drag_started/ended`). Visible on hover/scroll, subtle
  otherwise; drag to scrub, click track to jump.
- **Sidebar auto-expand:** when a file opens, expand all its ancestor
  directories inside the workspace root (`FileTree::expand_to(path)`),
  so the highlighted row is present in the tree. No scroll-into-view.
- **⌘N new file:** `NewFile` action creates `Untitled.md` (then
  `Untitled 2.md`, …) in the workspace root, refreshes the sidebar,
  and opens it in an editor tab. Requires a workspace root; no-op in
  single-file mode.

## Out of scope

Rename/delete in sidebar, sidebar scroll-into-view, animated
dissolve/reveal transitions, find-and-replace (find only), multi-file
search, watching files outside the workspace root.

## Testing strategy

TDD for pure logic:
- spans: TaskMarker ranges/checked flags (checked, unchecked, nested
  lists).
- display: checkbox replacement text + toggle payload on segs; reveal
  behavior; mapping unchanged elsewhere.
- core: `replace_range` history/selection semantics.
- find: smart-case, non-overlap, boundaries, empty query, recompute
  determinism.
- files: `FileTree::expand_to` ancestor expansion; untitled-name
  allocation (pure helper choosing the first free `Untitled N.md`).
- Reload policy: pure decision fn `should_reload(dirty, mtime_changed)`
  (trivial but documents the contract).

Shell (bar UI, scrollbar painting/drag, watcher bridge, title calls):
manual verification.
