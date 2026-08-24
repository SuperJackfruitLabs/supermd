# Git Diff Viewer ("Show Changes") Design

**Date:** 2026-08-24
**Status:** Approved for planning

## Purpose

Answer "what changed in this document?" beautifully. A read-only,
document-centric diff of the current file against git HEAD — word-level
marks woven into the editor's own typography for Markdown, classic line
diffs in code-mode for code files. Not a git client: no staging, no
commits, no history, no blame.

## Decisions (settled during brainstorming)

- **Baseline:** git HEAD only. No backup diffing, no arbitrary commits
  in v1.
- **Surface:** in-place toggle on the editor tab (like ⌘E preview), not
  a separate tab or split.
- **Prose rendering:** styled prose with word-level marks — deleted
  words struck through on a muted red wash, added words on a soft green
  wash, inline in the rendered flow.
- **Scope of view:** whole document with changes marked in place; no
  hunk fragmentation.
- **Sidebar:** subtle accent dot on files with uncommitted changes
  (modified or untracked), in v1.
- **Architecture:** Approach A — merged-document projection through the
  existing rendering pipeline (see below).

## Architecture

The diff view is "the preview renderer plus one overlay." A pure engine
merges old (HEAD) and new (buffer) text into a single synthetic
document plus a change map; that merged document flows through the
existing styling and layout machinery (`markdown_spans`, line layout,
tables, outline), and the renderer paints one extra span layer for the
change marks. No second rendering pipeline.

```
HEAD blob ──┐
            ├── diff_doc() ──> DiffDoc { text, changes } ──> markdown_spans(text)
buffer ─────┘                                          └──> change overlay (washes)
```

### Component 1: Diff engine — `src/diff.rs` (new, pure)

No GPUI, no git2 dependency. Fully unit-testable.

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangeKind { Added, Deleted }

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// Byte range into `DiffDoc::text` (the merged document).
    pub range: std::ops::Range<usize>,
    pub kind: ChangeKind,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiffDoc {
    /// The new text with deleted runs spliced back in at their
    /// original positions.
    pub text: String,
    /// Non-overlapping, sorted by start. Empty == no changes.
    pub changes: Vec<Change>,
}

pub fn diff_doc(old: &str, new: &str) -> DiffDoc;
```

Algorithm (built on the `similar` crate):

1. Line-level diff of `old` vs `new` (`TextDiff::from_lines`).
2. Equal runs are emitted once, unmarked.
3. Pure insertions are emitted from `new`, marked `Added`.
4. Pure deletions are spliced in from `old`, marked `Deleted`.
5. Replace runs (delete adjacent to insert) are refined at word level:
   a word diff of the deleted run against the inserted run interleaves
   the result — unchanged words once, changed words as
   deleted-then-added pairs — so a one-word edit marks one word, not a
   paragraph. Word segmentation is `similar`'s Unicode word splitter.
6. All `Change` ranges are byte ranges into the merged text, lie on
   `char` boundaries, are non-overlapping, and are sorted by start —
   the same discipline `spans.rs` guarantees.

Size cap: if `old.len() + new.len() > MAX_STYLED_BYTES` (the existing
1 MB constant from `spans.rs`), skip step 5 (no word refinement; whole
replaced lines are marked delete + add). The diff never silently
degrades to nothing.

Invariants (test-enforced):

- Removing all `Deleted` ranges from `text` reproduces `new` exactly.
- Removing all `Added` ranges from `text` reproduces `old` exactly.
- `diff_doc(x, x)` returns `text == x` with empty `changes`.

### Component 2: Git baseline — `src/git.rs` (new)

Thin read-only wrapper over the `git2` crate. Uses the
`vendored-libgit2` feature so CI needs no system library. Never opens
the index for writing, never mutates the repo.

```rust
pub enum Baseline {
    Text(String),   // blob content at HEAD for this path
    NotInRepo,      // no repository above this file
    Untracked,      // repo exists, path absent from HEAD tree
    Binary,         // blob exists but is not valid UTF-8
}

/// HEAD content for `path`, discovered from its parent directory.
pub fn head_text(path: &Path) -> Baseline;

/// Workspace-relative paths with uncommitted changes (modified or
/// untracked, ignoring gitignored files), for sidebar dots.
/// Empty set when `root` is not in a repository.
pub fn modified_paths(root: &Path) -> std::collections::HashSet<PathBuf>;
```

Implementation notes:

- `Repository::discover` from the file's directory; resolve the path
  relative to the workdir; look up the entry in `HEAD^{tree}`.
- A missing or unborn HEAD (fresh repo, no commits) reports
  `Untracked`.
- `modified_paths` is one `statuses()` call with untracked files
  included and ignored files excluded. Repository discovery for the
  workspace root is cached in the workspace (an `Option<PathBuf>` of
  the repo workdir, resolved once at open).

### Component 3: View integration — `src/workspace.rs`, `src/editor/mod.rs`

**Tab refactor.** `Tab::Editor { editor, preview: bool }` becomes:

```rust
pub enum EditorView { Edit, Preview, Diff }
Tab::Editor { editor: Entity<Editor>, view: EditorView }
```

⌘E toggles `Edit ↔ Preview` exactly as today; ⌘⇧D toggles
`Edit ↔ Diff` (and from `Preview`, ⌘⇧D goes straight to `Diff`).
Escape in `Diff` returns to `Edit`.

**Diff state.** Entering `Diff` calls `head_text()` then `diff_doc()`
and stores the result on the editor:

```rust
struct DiffState {
    doc: DiffDoc,
    // Styling spans computed over doc.text (markdown or code path).
    spans: Vec<(Range<usize>, StyleKind)>,
    baseline_missing: Option<Baseline>, // NotInRepo / Untracked / Binary
    adds: usize,    // count of Added changes, for the header strip
    dels: usize,
}
```

The buffer is untouched — the merged text lives only in `DiffState`.
The view is read-only: editing keystrokes are inert in `Diff` (same
mechanism as `Preview`). The diff recomputes when the buffer is saved
or the file watcher reports a disk change while `Diff` is active; the
recompute runs `diff_doc` again from current buffer text.

**Rendering.** The `Diff` view reuses the preview rendering path over
`DiffState.doc.text` with `DiffState.spans`, passing the change list
down to the line renderer as an overlay. Per visible line, the overlay
intersects `changes` with the line's byte range (the same slicing
`display_line` does for style spans) and paints:

- `Added`: background wash `diff_added_bg`, text color
  `diff_added_fg`.
- `Deleted`: background wash `diff_deleted_bg`, text color
  `diff_deleted_fg`, plus a strikethrough run.

Washes and strikethrough are painted per shaped sub-range using the
existing `WrappedLine` geometry, so marks wrap correctly with the
text. Tables and images in the merged document render as raw source
lines in diff view (block widgets stay out of v1 diff rendering —
projecting a half-deleted table into the table widget is undefined;
the raw lines still carry word-level marks). Fence content is
highlighted as usual over the merged text.

**Code files.** `is_code_mode()` files take the same `DiffDoc` through
code-mode rendering: monospace, full width, gutter. The gutter numbers
merged-document lines sequentially, except lines that lie entirely
inside a `Deleted` change show `-` instead of a number. Changed lines
get line washes; word-level marks apply within replaced lines.

**Header strip.** A slim bar at the top of the diff view:
`Changes vs HEAD · +{adds} −{dels}` left, `esc to close` right, themed
like the find bar.

**Empty/edge states** render as a calm centered message in the content
area (no toast, no error styling):

- No changes: "No uncommitted changes."
- `NotInRepo`: "Not in a git repository."
- `Untracked`: "Not tracked in git yet."
- `Binary`: "No text baseline at HEAD."

### Component 4: Sidebar dots — `src/workspace.rs`

A 5 px accent-colored dot, vertically centered, right-aligned in the
file row, shown for files whose workspace-relative path is in the
current `modified_paths` set. Refresh points:

- workspace open,
- each file-watcher drain tick that delivered events (the existing
  200 ms loop),
- after each editor save/flush.

The scan is skipped entirely when the workspace is not in a repo
(cached discovery). The set lives on the workspace; rows read it at
render time. Dot color: `accent`.

### Component 5: Theme keys — `src/theme.rs`

Four new optional `[colors]` keys, defaulted from the palette so every
existing builtin and custom theme works unchanged:

| Key | Light default | Dark default |
|-----|--------------|--------------|
| `diff_added_bg` | `0xe6f0dc` (soft green wash) | `0x2c3a26` |
| `diff_added_fg` | `0x3d6b2f` | `0xa8c897` |
| `diff_deleted_bg` | `0xf7e3e0` (soft red wash) | `0x3d2723` |
| `diff_deleted_fg` | `0xa04b3d` | `0xd18b7f` |

These are hardcoded as defaults in `Theme::light()`/`Theme::dark()`
(tuned to sit on the Jackfruit paper/ink backgrounds); `ThemeFile`
gains the four optional keys so any theme may override them.

## Keybindings & menus

- `cmd-shift-d` → `ShowChanges` (toggle), bound in the Editor context.
- `escape` in `Diff` view → back to `Edit` (extends the existing
  escape handling).
- View menu: "Show Changes" item with the shortcut.
- Shortcuts dialog (`SHORTCUTS` table): new row under the View group.

## Dependencies

- `similar` — line + word diffing.
- `git2` with `vendored-libgit2` — read-only HEAD/status access.

## Error handling

- All git2 errors (corrupt repo, permission, odb failures) collapse to
  `NotInRepo` semantics for `head_text` and the empty set for
  `modified_paths` — the feature degrades to "no baseline", never to a
  crash or dialog.
- Oversized inputs degrade to line-level marks (engine cap above).
- The diff view never writes: not to the buffer, not to disk, not to
  the repo.

## Testing strategy

TDD throughout, matching the crate's pure-core discipline:

- **`diff.rs`** (pure): equal docs → no changes; pure insert; pure
  delete; replace with word refinement (one changed word marks one
  word); multi-hunk documents; changes sorted/non-overlapping/char-
  aligned; the two reconstruction invariants (strip `Deleted` → new,
  strip `Added` → old) as property-style assertions over the case
  matrix; >1 MB fallback keeps line-level marks.
- **`git.rs`**: tempdir repos built with git2 in tests — init + commit
  + modify → `Baseline::Text`; untracked file → `Untracked`; no repo →
  `NotInRepo`; fresh repo without commits → `Untracked`; binary blob →
  `Binary`; `modified_paths` reports modified + untracked, excludes
  ignored and clean files.
- **`theme.rs`**: new keys parse from TOML; defaults present in both
  builtin appearances.
- **Workspace/editor**: `EditorView` transitions (⌘E and ⌘⇧D
  interplay, escape), read-only enforcement in `Diff`, recompute on
  save, empty-state selection per `Baseline` variant — at the entity
  level like existing workspace tests.

## Out of scope

Commit history browsing, diffing arbitrary revisions, staging/commit/
push, blame, backup-file diffing (future), editing inside the diff
view, rendering table/image widgets inside the diff, diff for image
files.
