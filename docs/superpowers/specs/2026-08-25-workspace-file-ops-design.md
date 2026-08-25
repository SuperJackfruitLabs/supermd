# Workspace File Operations — Design

**Milestone 1 of the knowledge-features arc** (file ops → links+backlinks → tags → graph). Standalone value: the sidebar becomes a real file manager. Milestone 2 hooks link-rewrite into the rename/move paths defined here.

## Goals

- Create, rename, move, and delete files and folders without leaving SuperMD.
- Keyboard-first, matching the sidebar's existing interaction model (no context-menu machinery exists in the app; none is introduced).
- Open tabs and dirty buffers survive every operation: nothing is lost, nothing silently clobbered.
- Deletion is recoverable: files go to the OS trash, never `rm`.

## Non-goals (this milestone)

- Drag-and-drop moving (later; keyboard move covers the need).
- Multi-select operations.
- Link rewriting on rename/move — Milestone 2 plugs into the hooks this design leaves.
- Operations on files outside the workspace root.

## UX

All operations act on the **selected sidebar row** and live in the `Sidebar` key context, so they never collide with editor bindings:

| Key | Operation |
| --- | --- |
| F2 | Rename (inline) |
| ⌘ ⌫ | Delete to trash |
| ⌘ N | New file in the selected folder (or beside the selected file) |
| ⌘ ⇧ N | New folder, same targeting |
| ⌘ ⇧ M | Move… (fuzzy folder picker) |

⌘N in the `Sidebar` context shadows the global new-file binding deliberately: sidebar-focused creation targets the selection's directory instead of the workspace root. Everywhere else ⌘N behaves as today.

**Inline rename / create.** The selected row (or a new row inside the target folder) swaps to the existing `TextInput` component. For rename, the filename stem is pre-selected so typing replaces the name but keeps the extension. Enter commits, Escape cancels, clicking elsewhere cancels. Committing an empty name cancels. A name that already exists in the folder shows the error in place and stays in edit mode.

**Move picker.** ⌘⇧M opens the finder-style overlay (same chrome as ⌘P) listing every folder in the workspace (plus the root), fuzzy-filtered. Enter moves the selection's file/folder there. Moving a folder into its own descendant is rejected with a message.

**Delete.** No confirmation dialog — deletion goes to the OS trash and is recoverable there (Finder semantics). The row's tab (or every tab under a deleted folder) closes; dirty buffers are flushed to disk first so the trashed copy is complete.

## Architecture

**New pure-ish module `src/fileops.rs`** — thin, testable wrappers with the policy decisions:

- `validate_name(name) -> Result<(), String>` — rejects empty, `/`, `\`, leading `.` is allowed, `..` rejected.
- `rename(path, new_name) -> Result<PathBuf, String>` — same-dir rename via `std::fs::rename`; refuses to overwrite an existing entry.
- `move_entry(path, dest_dir) -> Result<PathBuf, String>` — refuses overwrite and folder-into-own-descendant.
- `create_file(dir, name)` / `create_dir(dir, name)` — refuse overwrite; create_file writes an empty file.
- `delete(path) -> Result<(), String>` — `trash::delete` (new dependency: `trash` crate, cross-platform, moves to OS trash/recycle bin).
- `retarget(open_path, old, new) -> Option<PathBuf>` — pure: maps a tab's path through a rename/move, handling the directory-prefix case (a renamed folder retargets everything under it).

**Workspace glue** (`src/workspace.rs`):

- Sidebar state gains `editing: Option<SidebarEdit>` where `SidebarEdit { target: EditKind (Rename(path) | NewFile(dir) | NewDir(dir)), input: Entity<TextInput>, error: Option<String> }`. The sidebar row renderer swaps in the input for the editing row.
- On commit: run the fileops call; on error, stay editing with the message; on success, `tree.refresh()`, retarget open tabs (`Editor::retarget` updates its `path` and re-stats `disk_mtime`; tab titles re-derive), select the resulting row, and — for new files — open the file.
- Delete: flush dirty buffers under the path, close their tabs, `fileops::delete`, refresh, keep selection on the neighbor row.
- Move picker reuses the finder overlay component with a folders-only source and a `MoveTarget(path)` completion action.

**Milestone-2 hook.** Rename/move funnel through one workspace method that ends with `after_path_change(old, new)`. Today it retargets tabs; Milestone 2 adds link rewriting there. This is the seam the knowledge engine plugs into.

**File watcher interplay.** Our own operations already refresh the tree explicitly; the notify watcher will also fire for them — harmless double refresh. External renames keep working as today (watcher refresh; open tabs of vanished paths keep their buffer and save recreates the file — unchanged behavior).

## Error handling

Every fileops error is a plain sentence (`"a file named notes.md already exists"`); inline edits show it under the row, picker/delete errors use the existing command strip. No operation partially applies: fileops functions are single `std::fs` calls plus pre-checks.

## Testing

- `fileops` unit tests on tempdirs: rename/move/create happy paths, overwrite refusal, descendant-move refusal, name validation truth table, `retarget` cases (exact file, under renamed dir, unrelated path).
- gpui workspace tests: F2 inline rename flow end-to-end (dispatch, type, commit, tab retargeted, tree refreshed); duplicate-name stays editing with error; new file targets the selected folder and opens; delete closes the tab and file leaves the directory (trash call injected/faked via a test-only override so tests never touch the real OS trash); move picker moves and retargets.
- Keybinding change-detector bump for the five new Sidebar bindings.

## Dependencies

- `trash` crate (runtime). Everything else is std + existing components.
