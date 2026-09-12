# SuperMD 0.0.16 — "things the app should already have"

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the gaps between what SuperMD does and what a Mac editor is
expected to do — durable settings, right-click menus, find and replace, table
and list editing, a default-app offer, visible gitignored files, and multiple
windows.

**Architecture:** Every item follows the house rule: the decision is a pure
function in a tested module, and the GPUI shell only drives it. Three tasks add
new pure modules (`replace.rs`, `menus.rs`, `table_ops.rs`); the rest extend
existing ones. The multi-window work is different in kind — it moves
per-workspace state off process globals onto the `Workspace` entity — so it is
sequenced last, where a long-running refactor cannot hold up the smaller items.

**Tech Stack:** Rust, GPUI 0.2.2 (vendored at `vendor/gpui`), ropey, ignore,
wasmtime 48 (Pulley under `mas`).

**Spec:** No separate spec. The scope is GitHub issues #23, #24, #25, #28, #29,
#30, plus two requests made directly: show gitignored files in the sidebar, and
open a workspace in a new window. Decisions taken before writing, recorded here
because the tasks argue from them:

- Gitignored files appear **in the sidebar only**, dimmed. The knowledge index,
  graph, backlinks and ⌘P keep excluding them — a gitignored draft is not part
  of the note graph, and indexing `node_modules` would flood it.
- New Window offers **both** an empty window (⌘⇧N) and Open Folder in New
  Window.
- Multi-window ships in this release despite the refactor it needs.

## Global Constraints

- **Editing logic is pure Rust under test; the GPUI shell stays thin.** New
  logic goes in a pure module with the shell driving it.
- **Byte offsets into the ropey rope** are the universal currency for every
  position, span and selection.
- **`src/editor/display.rs` is the only place** the "buffer offset == rendered
  offset" invariant may break.
- **CI enforces a 90% line coverage floor.** Run
  `bash scripts/build_plugins.sh --fixtures` before `cargo llvm-cov`, or
  `extensions.rs` reads ~47% and the total looks like a failure.
- **Both suites must pass:** `cargo test` and
  `cargo test --no-default-features --features mas`.
- **Tests must compile on macOS, Linux and Windows.** Anything using
  `std::os::unix` needs `#[cfg(unix)]` — this broke CI in 0.0.15.
- **A new `KeyBinding` is declared in `commands.rs`**, never by hand in
  `main.rs`. Then `UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table` and
  `cargo run --example build_docs`.
- **SuperMD never writes to the user's git repository.**
- **The plain-text Markdown file is the source of truth.**

---

### Task 1: Settings survive an interrupted write (#28)

**Files:**
- Modify: `src/settings.rs:98-110`
- Test: `src/settings.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `crate::editor::autosave::atomic_write(path: &Path, contents: &str) -> io::Result<()>`
- Produces: `settings::load` unchanged in signature; a corrupt file is preserved
  at `settings.toml.corrupt` rather than silently discarded.

`settings::save` is a plain `fs::write`, so an interrupted write truncates the
file. `load` then falls back to `Settings::default()` and says nothing — losing
the theme, recent folders, workspace bookmarks, `format_on_save`, flux settings
and **every plugin permission grant**, including the per-site preview grants
0.0.15 added. That release added a new writer to this file, which is why it is
first.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/settings.rs`:

```rust
    /// A torn write must not cost the user their settings. `save` was a
    /// plain `fs::write`, so an interrupted one truncated the file and
    /// `load` silently returned defaults — discarding themes, recents,
    /// bookmarks and every plugin permission grant with no message.
    #[test]
    fn a_corrupt_settings_file_is_preserved_not_discarded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.toml"), "this is not = valid toml [[[").unwrap();

        let loaded = load(dir.path());
        assert_eq!(loaded, Settings::default(), "unreadable settings fall back");
        assert!(
            dir.path().join("settings.toml.corrupt").exists(),
            "and the unreadable file is kept, not thrown away"
        );
        assert!(
            std::fs::read_to_string(dir.path().join("settings.toml.corrupt"))
                .unwrap()
                .contains("not = valid"),
            "the preserved copy is the original bytes"
        );
    }

    /// A missing file is not a corrupt one: first run must not leave a
    /// `.corrupt` file lying beside the settings.
    #[test]
    fn a_missing_settings_file_leaves_no_corrupt_copy() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Settings::default());
        assert!(!dir.path().join("settings.toml.corrupt").exists());
    }

    /// Writes go through a temp file and a rename, so a reader never
    /// observes a half-written file.
    #[test]
    fn save_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.format_on_save = true;
        save(dir.path(), &s).unwrap();

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["settings.toml".to_string()], "one file, no scratch: {names:?}");
        assert!(load(dir.path()).format_on_save, "and it round-trips");
    }
```

- [ ] **Step 2: Run them and watch them fail**

```sh
cargo test settings::tests::a_corrupt_settings_file_is_preserved_not_discarded
cargo test settings::tests::save_leaves_no_temp_file_behind
```

Expected: the first fails (no `.corrupt` file is written); the third may pass by
accident today — that is fine, it guards the change.

- [ ] **Step 3: Implement**

Replace `load` and `save` in `src/settings.rs`:

```rust
pub fn load(dir: &Path) -> Settings {
    let path = dir.join("settings.toml");
    let Ok(body) = std::fs::read_to_string(&path) else {
        // No file is the ordinary first-run case, not a problem.
        return Settings::default();
    };
    match toml::from_str(&body) {
        Ok(settings) => settings,
        Err(err) => {
            // Defaulting is right for a missing file and wrong for a
            // corrupt one: the user's themes, recents, bookmarks and
            // every plugin permission grant live here. Keep the bytes
            // so they can be recovered, and say so.
            let kept = dir.join("settings.toml.corrupt");
            let _ = std::fs::write(&kept, &body);
            eprintln!(
                "supermd: {} could not be read ({err}); the previous file was kept at {}",
                path.display(),
                kept.display()
            );
            Settings::default()
        }
    }
}

pub fn save(dir: &Path, settings: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let body = toml::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    // Temp file plus rename, as documents already do: an interrupted
    // write leaves the old file intact rather than a truncated one.
    crate::editor::autosave::atomic_write(&dir.join("settings.toml"), &body)
}
```

- [ ] **Step 4: Run the tests**

```sh
cargo test settings::
```
Expected: all pass.

- [ ] **Step 5: Full suites**

```sh
cargo test
cargo test --no-default-features --features mas
```

- [ ] **Step 6: Commit**

```bash
git add src/settings.rs
git commit -m "fix: settings survive an interrupted write

A plain fs::write left a truncated file when interrupted, and load
silently returned defaults -- discarding the theme, recent folders,
workspace bookmarks and every plugin permission grant, including the
per-site preview grants 0.0.15 added, with no message.

Writes now go through the same temp-file-and-rename autosave already
uses for documents. A file that cannot be parsed is preserved beside
the new one rather than thrown away, and the reason is printed: a
missing file is an ordinary first run, a corrupt one is not.

Closes #28"
```

---

### Task 2: Gitignored files appear in the sidebar

**Files:**
- Modify: `src/files.rs:44-58` (`walk_builder`), and `FsEntry`
- Modify: `src/workspace.rs` (dim ignored rows)
- Test: `src/files.rs` (inline `mod tests`)

**Interfaces:**
- Produces: `files::FsEntry` gains `pub ignored: bool`;
  `files::walk_builder` keeps skipping build directories but no longer hides
  gitignored paths from the tree.

The sidebar walks with `git_ignore(true)`, so a `.gitignore`d file is invisible
in the file tree. That is right for the knowledge index — a gitignored draft is
not part of the note graph — and wrong for a file tree, which should show what
is on disk. It also hid a real bug during 0.0.15: a `[[drafts/secret]]` link
into a gitignored folder resolved to nothing precisely because the index could
not see it.

The index keeps excluding them. Only the tree changes.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/files.rs`:

```rust
    /// A file tree should show what is on disk. Gitignored files were
    /// invisible in the sidebar, which is right for the note index and
    /// wrong for a file browser — a draft you deliberately kept out of
    /// git simply vanished.
    #[test]
    fn the_tree_shows_gitignored_files_and_marks_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "drafts/\n").unwrap();
        std::fs::create_dir(dir.path().join("drafts")).unwrap();
        std::fs::write(dir.path().join("drafts/secret.md"), "x").unwrap();
        std::fs::write(dir.path().join("visible.md"), "x").unwrap();

        let entries = children(dir.path(), dir.path());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"drafts"), "the ignored folder is listed: {names:?}");
        assert!(names.contains(&"visible.md"));

        let drafts = entries.iter().find(|e| e.name == "drafts").unwrap();
        assert!(drafts.ignored, "and is marked so the sidebar can dim it");
        let visible = entries.iter().find(|e| e.name == "visible.md").unwrap();
        assert!(!visible.ignored);
    }

    /// Build output stays collapsed. Showing `target/` or
    /// `node_modules/` would bury the actual notes.
    #[test]
    fn build_directories_are_still_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/x.md"), "x").unwrap();
        std::fs::create_dir(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("node_modules/y.md"), "x").unwrap();
        std::fs::write(dir.path().join("real.md"), "x").unwrap();

        let names: Vec<String> =
            children(dir.path(), dir.path()).into_iter().map(|e| e.name).collect();
        assert!(names.contains(&"real.md".to_string()));
        assert!(!names.contains(&"target".to_string()), "{names:?}");
        assert!(!names.contains(&"node_modules".to_string()), "{names:?}");
    }

    /// The knowledge index is unchanged: a gitignored note is still not
    /// part of the note graph.
    #[test]
    fn the_index_still_skips_gitignored_notes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "drafts/\n").unwrap();
        std::fs::create_dir(dir.path().join("drafts")).unwrap();
        std::fs::write(dir.path().join("drafts/secret.md"), "# Secret\n").unwrap();
        std::fs::write(dir.path().join("open.md"), "# Open\n").unwrap();

        let index = crate::knowledge::Index::scan(dir.path());
        let names: Vec<String> =
            index.note_names().iter().map(|(n, _)| n.clone()).collect();
        assert!(names.iter().any(|n| n.eq_ignore_ascii_case("open")));
        assert!(
            !names.iter().any(|n| n.eq_ignore_ascii_case("secret")),
            "the index keeps excluding ignored notes: {names:?}"
        );
    }
```

- [ ] **Step 2: Run and watch the first two fail**

```sh
cargo test files::tests::the_tree_shows_gitignored_files_and_marks_them
cargo test files::tests::the_index_still_skips_gitignored_notes
```
Expected: the first fails (`drafts` is not listed); the third passes already and
is the guard that the index is untouched.

- [ ] **Step 3: Give `FsEntry` the flag**

In `src/files.rs`, add to `pub struct FsEntry`:

```rust
    /// Excluded by a `.gitignore`. Listed in the tree, dimmed, and kept
    /// out of the knowledge index.
    pub ignored: bool,
```

- [ ] **Step 4: Split the walker in two**

In `src/files.rs`, rename the existing `walk_builder` to `index_walk_builder`
(the index keeps today's behaviour) and add a tree walker beside it:

```rust
/// The walker the sidebar uses. Shows gitignored files — a file tree
/// should show what is on disk — but still skips hidden files and the
/// build directories that would bury everything else.
fn tree_walk_builder(root: &Path) -> ignore::WalkBuilder {
    let mut b = ignore::WalkBuilder::new(root);
    b.hidden(true)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .filter_entry({
            let root = root.to_path_buf();
            move |e| {
                should_descend(&root, e.path(), e.file_type().is_some_and(|t| t.is_dir()))
            }
        });
    b
}
```

Point `children` at `tree_walk_builder`, and set `ignored` per entry using the
existing root-gitignore check that `is_visible` already performs.

- [ ] **Step 5: Dim ignored rows**

In `src/workspace.rs`, in the sidebar row renderer, where `label_color` is
chosen, fold in the flag:

```rust
            // Ignored files are listed but recede: present when you
            // need them, never competing with the notes.
            let row_color = if entry.ignored { t.fg_muted } else { t.fg };
```

- [ ] **Step 6: Run the tests**

```sh
cargo test files::
cargo test knowledge::
```

- [ ] **Step 7: Full suites and commit**

```sh
cargo test && cargo test --no-default-features --features mas
```

```bash
git add src/files.rs src/workspace.rs
git commit -m "feat: the sidebar shows gitignored files, dimmed

A file tree should show what is on disk. Walking with git_ignore(true)
meant a deliberately-uncommitted draft simply vanished from the
sidebar, with nothing to say it existed.

The knowledge index keeps excluding them -- a gitignored note is not
part of the note graph, and indexing node_modules would flood it -- so
the walkers are now separate: index_walk_builder is unchanged, and the
tree uses its own. Build directories stay collapsed either way."
```

---

### Task 3: Right-click menus, driven by the command table (#23)

**Files:**
- Create: `src/menus.rs`
- Modify: `src/main.rs` (`mod menus;`)
- Modify: `src/workspace.rs` (sidebar, tab and graph right-click handlers)
- Modify: `src/commands.rs` (two new commands)
- Test: `src/menus.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `commands::COMMANDS` entries carrying `id`, `label`, `keys`, `ctx`.
- Produces:
  - `menus::Surface` — `SidebarFile`, `SidebarFolder`, `Tab`, `GraphNode`,
    `GraphGhost`, `Editor`
  - `menus::items_for(surface: Surface) -> Vec<MenuItem>`
  - `menus::MenuItem { pub id: &'static str, pub label: &'static str, pub keys: &'static str }`

There is no context-menu handling anywhere: `MouseButton::Right` appears nowhere
in `src/`. On macOS that is where people look first, so a set of features we
already ship — Rename, Delete to Trash, New File Here, Move to Folder… — are
reachable only by people who know the shortcut.

Build it from `COMMANDS`. The ⌘/ dialog and the palette already render from
that table; a hand-written third list is exactly how `SHORTCUTS` in
`workspace.rs` went stale enough that CLAUDE.md documented a table which no
longer existed.

- [ ] **Step 1: Write the failing tests**

Create `src/menus.rs` with the tests first:

```rust
//! Which commands a right-click offers, per surface.

#[cfg(test)]
mod tests {
    use super::*;

    /// The sidebar's actions all exist already and are keyboard-only.
    /// A right-click is where a Mac user looks for them first.
    #[test]
    fn a_sidebar_file_offers_the_file_actions() {
        let ids: Vec<&str> = items_for(Surface::SidebarFile).iter().map(|i| i.id).collect();
        for expected in ["sidebar_rename", "sidebar_trash", "sidebar_move", "reveal_in_finder", "copy_path"] {
            assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
        }
    }

    /// A folder cannot be renamed into a file's actions: New File Here
    /// and New Folder Here belong to it, and Move does not.
    #[test]
    fn a_sidebar_folder_offers_creation_not_file_actions() {
        let ids: Vec<&str> = items_for(Surface::SidebarFolder).iter().map(|i| i.id).collect();
        assert!(ids.contains(&"sidebar_new_file"), "{ids:?}");
        assert!(ids.contains(&"sidebar_new_folder"), "{ids:?}");
        assert!(ids.contains(&"reveal_in_finder"), "{ids:?}");
    }

    /// Every item must name a real command, or the menu offers
    /// something that cannot be dispatched.
    #[test]
    fn every_menu_item_names_a_real_command() {
        for surface in Surface::ALL {
            for item in items_for(*surface) {
                assert!(
                    crate::commands::COMMANDS.iter().any(|c| c.id == item.id),
                    "{surface:?} offers {:?}, which is not in COMMANDS",
                    item.id
                );
            }
        }
    }

    /// Labels and shortcuts come from the table, so a renamed command
    /// or a changed key updates the menu for free.
    #[test]
    fn labels_and_keys_come_from_the_command_table() {
        let item = items_for(Surface::SidebarFile)
            .into_iter()
            .find(|i| i.id == "sidebar_rename")
            .expect("rename is offered");
        let cmd = crate::commands::COMMANDS.iter().find(|c| c.id == "sidebar_rename").unwrap();
        assert_eq!(item.label, cmd.label);
        assert!(!item.keys.is_empty(), "a bound command shows its shortcut");
    }

    /// No surface offers nothing: an empty menu is worse than none.
    #[test]
    fn no_surface_is_empty() {
        for surface in Surface::ALL {
            assert!(!items_for(*surface).is_empty(), "{surface:?} has no items");
        }
    }
}
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test menus::
```
Expected: compile error — `Surface` and `items_for` do not exist.

- [ ] **Step 3: Add the two missing commands**

In `src/commands.rs`, beside the other sidebar entries:

```rust
    ws::RevealInFinder => { id: "reveal_in_finder", label: "Reveal in Finder",
        keys: [], ctx: Some("Sidebar"), menu: None, help: None },
    ws::CopyPath => { id: "copy_path", label: "Copy Path",
        keys: [], ctx: Some("Sidebar"), menu: None, help: None },
```

Declare both actions in `src/workspace.rs`'s action list, and implement them:

```rust
    /// Show the file in Finder. `NSWorkspace` rather than a process
    /// spawn: the App Store build cannot spawn processes.
    fn reveal_in_finder(&mut self, _: &RevealInFinder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.sidebar_selected_path() else { return };
        // `reveal_dir` already exists and uses
        // `activateFileViewerSelectingURLs`, which selects the item —
        // so it reveals a file, not only a directory. NSWorkspace
        // rather than spawning `open`: the App Store build cannot
        // spawn processes.
        crate::platform::reveal_dir(&path);
        let _ = cx;
    }

    fn copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.sidebar_selected_path() else { return };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            path.display().to_string(),
        ));
        self.show_command_error("Path copied".into(), cx);
    }
```

`platform::reveal_dir` already exists (`src/platform.rs:175`) and is the
function to call — `platform.rs` is the one home for per-OS decisions. Do not
add a second one.

- [ ] **Step 4: Implement the menu model**

Add above the tests in `src/menus.rs`:

```rust
use crate::commands::COMMANDS;

/// Where a right-click happened. The menu is a filter over the command
/// table rather than its own list, so renaming a command or changing
/// its shortcut updates every menu for free — a hand-written list is
/// how `SHORTCUTS` in workspace.rs went stale enough that CLAUDE.md
/// documented a table which no longer existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    SidebarFile,
    SidebarFolder,
    Tab,
    GraphNode,
    GraphGhost,
    Editor,
}

impl Surface {
    pub const ALL: &'static [Surface] = &[
        Surface::SidebarFile,
        Surface::SidebarFolder,
        Surface::Tab,
        Surface::GraphNode,
        Surface::GraphGhost,
        Surface::Editor,
    ];

    /// Command ids this surface offers, in the order they appear.
    fn command_ids(self) -> &'static [&'static str] {
        match self {
            Surface::SidebarFile => &[
                "sidebar_rename",
                "sidebar_move",
                "sidebar_trash",
                "reveal_in_finder",
                "copy_path",
            ],
            Surface::SidebarFolder => &[
                "sidebar_new_file",
                "sidebar_new_folder",
                "sidebar_rename",
                "sidebar_trash",
                "reveal_in_finder",
                "copy_path",
            ],
            Surface::Tab => &["close_tab", "preview", "changes"],
            Surface::GraphNode => &["graph_local", "graph_fit"],
            Surface::GraphGhost => &["graph_local"],
            Surface::Editor => &["follow_link", "bold", "italic"],
        }
    }
}

/// One row of a context menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: &'static str,
    pub label: &'static str,
    /// Shortcut as written in the command table, or "" when unbound.
    pub keys: &'static str,
}

/// The menu for a surface, resolved against the command table. An id
/// with no matching command is dropped rather than shown as a dead row.
pub fn items_for(surface: Surface) -> Vec<MenuItem> {
    surface
        .command_ids()
        .iter()
        .filter_map(|id| {
            COMMANDS.iter().find(|c| &c.id == id).map(|c| MenuItem {
                id: c.id,
                label: c.label,
                keys: c.keys.first().copied().unwrap_or(""),
            })
        })
        .collect()
}
```

Adjust the ids in `command_ids` to the real ones in `commands.rs` — read the
table rather than trusting this list; `every_menu_item_names_a_real_command` is
the test that catches a wrong guess.

- [ ] **Step 5: Run the tests**

```sh
cargo test menus::
```
Expected: all five pass. If `every_menu_item_names_a_real_command` fails, an id
above is wrong — fix the id, not the test.

- [ ] **Step 6: Render the menu**

In `src/workspace.rs`, add the element. Follow the popover pattern from
`editor/mod.rs` (`deferred(anchored()…)`), which already handles positioning and
window snapping:

```rust
    /// The open context menu: where it was raised, and for what.
    context_menu: Option<(gpui::Point<Pixels>, crate::menus::Surface, PathBuf)>,
```

Raise it from a right mouse-down on a sidebar row:

```rust
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            let surface = if is_dir {
                                crate::menus::Surface::SidebarFolder
                            } else {
                                crate::menus::Surface::SidebarFile
                            };
                            this.sidebar_selected = row_ix;
                            this.context_menu = Some((event.position, surface, path.clone()));
                            cx.notify();
                        }),
                    )
```

Render it with `deferred(anchored().position(at).snap_to_window_with_margin(px(8.)))`,
one row per `MenuItem`, each dispatching its command id by name and clearing
`context_menu`. A click anywhere else clears it.

- [ ] **Step 7: Regenerate the shortcut docs**

```sh
UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table
cargo run --example build_docs
```

- [ ] **Step 8: Full suites and commit**

```sh
cargo test && cargo test --no-default-features --features mas
```

```bash
git add src/menus.rs src/main.rs src/workspace.rs src/commands.rs src/platform.rs docs site
git commit -m "feat: right-click menus, built from the command table

There was no context-menu handling anywhere -- MouseButton::Right
appeared nowhere in src/. On macOS that is where people look first, so
Rename, Delete to Trash, New File Here and Move to Folder were
reachable only by people who already knew the shortcut.

The menu is a per-surface filter over COMMANDS rather than its own
list, so a renamed command or a changed key updates every menu for
free. A test asserts every id resolves to a real command, because a
hand-written third list is how SHORTCUTS went stale enough that
CLAUDE.md documented a table that no longer existed.

Adds Reveal in Finder and Copy Path, which did not exist and are the
first things expected from a file-tree right-click.

Closes #23"
```

---

### Task 4: Find and replace (#25)

**Files:**
- Create: `src/editor/replace.rs`
- Modify: `src/editor/mod.rs` (`FindState`, the find bar, two actions)
- Modify: `src/commands.rs` (two new commands)
- Test: `src/editor/replace.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `find::find_matches(text: &str, query: &str) -> Vec<Range<usize>>`
- Produces:
  - `replace::replace_all(text: &str, query: &str, with: &str) -> Option<ReplaceEdit>`
  - `replace::replace_one(text: &str, at: Range<usize>, with: &str) -> ReplaceEdit`
  - `replace::ReplaceEdit { pub range: Range<usize>, pub replacement: String, pub select: Range<usize>, pub count: usize }`

Find exists; replace does not. Named in the 0.0.15 plan as the first candidate
for this release.

`replace_all` returns **one** contiguous edit spanning the whole affected
region, not one per match — so ⌘Z takes back the entire Replace All, which is
the only sane undo for it. That mirrors `formatting.rs`, where every toggle is
one replacement plus a post-edit selection.

- [ ] **Step 1: Write the failing tests**

Create `src/editor/replace.rs`:

```rust
//! Replace, as one contiguous edit. Pure; the shell applies the result.

use std::ops::Range;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_all_rewrites_every_match_in_one_edit() {
        let text = "one cat, two cat, red cat";
        let e = replace_all(text, "cat", "dog").expect("matches");
        assert_eq!(e.count, 3);
        let mut out = text.to_string();
        out.replace_range(e.range.clone(), &e.replacement);
        assert_eq!(out, "one dog, two dog, red dog");
    }

    /// One edit, not three: undoing a Replace All one match at a time
    /// is not undo.
    #[test]
    fn replace_all_spans_only_the_affected_region() {
        let text = "keep this: cat and cat :keep that";
        let e = replace_all(text, "cat", "x").expect("matches");
        assert_eq!(&text[..e.range.start], "keep this: ", "untouched head is outside the edit");
        assert!(text[e.range.end..].starts_with(" :keep"), "untouched tail too");
    }

    #[test]
    fn replacing_with_a_longer_or_shorter_string_keeps_the_rest_intact() {
        for (with, expect) in [("elephant", "a elephant b elephant c"), ("", "a  b  c")] {
            let text = "a cat b cat c";
            let e = replace_all(text, "cat", with).expect("matches");
            let mut out = text.to_string();
            out.replace_range(e.range.clone(), &e.replacement);
            assert_eq!(out, expect);
        }
    }

    /// The replacement must not be re-scanned, or replacing "a" with
    /// "aa" never terminates.
    #[test]
    fn a_replacement_containing_the_query_terminates() {
        let e = replace_all("a a a", "a", "aa").expect("matches");
        assert_eq!(e.count, 3);
        let mut out = "a a a".to_string();
        out.replace_range(e.range.clone(), &e.replacement);
        assert_eq!(out, "aa aa aa");
    }

    #[test]
    fn no_match_is_no_edit() {
        assert!(replace_all("hello", "xyz", "1").is_none());
        assert!(replace_all("hello", "", "1").is_none(), "an empty query matches nothing");
    }

    /// Multi-byte text: ranges must stay on char boundaries.
    #[test]
    fn replace_all_handles_multibyte_text() {
        let text = "café cat café";
        let e = replace_all(text, "cat", "chien").expect("matches");
        let mut out = text.to_string();
        out.replace_range(e.range.clone(), &e.replacement);
        assert_eq!(out, "café chien café");
    }

    #[test]
    fn replace_one_touches_a_single_match_and_selects_it() {
        let text = "a cat b cat";
        let at = 2..5;
        let e = replace_one(text, at, "dog");
        assert_eq!(e.count, 1);
        let mut out = text.to_string();
        out.replace_range(e.range.clone(), &e.replacement);
        assert_eq!(out, "a dog b cat");
        assert_eq!(&out[e.select.clone()], "dog", "the new text is selected");
    }
}
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test replace::
```
Expected: compile error — the functions do not exist.

- [ ] **Step 3: Implement**

Above the tests in `src/editor/replace.rs`:

```rust
/// One contiguous replacement, plus what to select afterwards.
#[derive(Debug, PartialEq, Eq)]
pub struct ReplaceEdit {
    /// Bytes of the document to replace.
    pub range: Range<usize>,
    pub replacement: String,
    /// Selection after the edit, in post-edit coordinates.
    pub select: Range<usize>,
    /// How many matches were rewritten.
    pub count: usize,
}

/// Rewrite every match as a single edit spanning first match to last.
///
/// One edit rather than one per match: ⌘Z has to take back the whole
/// Replace All, and stepping back through a hundred replacements is not
/// undo. The span is trimmed to the affected region so untouched text
/// stays outside the undo entry.
///
/// The replacement is never re-scanned, so replacing `a` with `aa`
/// terminates.
pub fn replace_all(text: &str, query: &str, with: &str) -> Option<ReplaceEdit> {
    let matches = crate::editor::find::find_matches(text, query);
    let first = matches.first()?.start;
    let last = matches.last()?.end;

    let mut out = String::with_capacity(last - first);
    let mut at = first;
    for m in &matches {
        out.push_str(&text[at..m.start]);
        out.push_str(with);
        at = m.end;
    }
    out.push_str(&text[at..last]);

    Some(ReplaceEdit {
        range: first..last,
        select: first..first + out.len(),
        replacement: out,
        count: matches.len(),
    })
}

/// Rewrite one already-located match.
pub fn replace_one(_text: &str, at: Range<usize>, with: &str) -> ReplaceEdit {
    ReplaceEdit {
        range: at.clone(),
        select: at.start..at.start + with.len(),
        replacement: with.to_string(),
        count: 1,
    }
}
```

Add `mod replace;` to `src/editor/mod.rs`.

- [ ] **Step 4: Run the tests**

```sh
cargo test replace::
```
Expected: all seven pass.

- [ ] **Step 5: Wire the UI**

In `src/editor/mod.rs`, add a second input to `FindState`:

```rust
struct FindState {
    input: Entity<crate::input::TextInput>,
    /// The replacement field, shown only once the user asks for it.
    replace_input: Entity<crate::input::TextInput>,
    replacing: bool,
    matches: Vec<Range<usize>>,
    active: usize,
    _watch: gpui::Subscription,
}
```

Add the two commands in `src/commands.rs`:

```rust
    ed::ReplaceNext => { id: "replace_next", label: "Replace",
        keys: ["cmd-alt-e"], ctx: Some("Editor"), menu: Some((Edit, 6)), help: Some(Editor) },
    ed::ReplaceAll => { id: "replace_all", label: "Replace All",
        keys: ["cmd-alt-shift-e"], ctx: Some("Editor"), menu: Some((Edit, 7)), help: Some(Editor) },
```

Both handlers apply their `ReplaceEdit` through the same path `formatting.rs`
edits already use — one `core.replace_range(range, text, Instant::now())`, one undo group, then
`core.selection = Selection::cursor(edit.select.end)` and a recount of matches.

- [ ] **Step 6: Editor-level test**

Add to `mod tests` in `src/editor/mod.rs`:

```rust
    /// Replace All is one undo entry. Stepping back through a hundred
    /// replacements one at a time is not undo.
    #[gpui::test]
    fn replace_all_is_a_single_undo_entry(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "a cat b cat c cat\n");
        editor.update(cx, |ed, cx| {
            let text = ed.core.buffer.text();
            let e = crate::editor::replace::replace_all(&text, "cat", "dog").expect("matches");
            ed.core.replace_range(e.range.clone(), &e.replacement, std::time::Instant::now());
            cx.notify();
        });
        editor.update(cx, |ed, _| {
            assert_eq!(ed.core.buffer.text(), "a dog b dog c dog\n");
            ed.core.undo();
            assert_eq!(
                ed.core.buffer.text(),
                "a cat b cat c cat\n",
                "one undo takes back the whole Replace All"
            );
        });
    }
```

- [ ] **Step 7: Docs, suites, commit**

```sh
UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table
cargo run --example build_docs
cargo test && cargo test --no-default-features --features mas
```

```bash
git add src/editor/replace.rs src/editor/mod.rs src/commands.rs docs site
git commit -m "feat: find and replace

Find existed; replace did not. Replace All is one contiguous edit
spanning first match to last, not one edit per match, so a single undo
takes the whole thing back -- stepping through a hundred replacements
one at a time is not undo. The span is trimmed to the affected region
so untouched text stays out of the undo entry.

The replacement is never re-scanned, so replacing 'a' with 'aa'
terminates rather than running away.

Closes #25"
```

---

### Task 5: Table row and column commands, list renumbering (#29)

**Files:**
- Create: `src/editor/table_ops.rs`
- Modify: `src/editor/lists.rs`
- Modify: `src/editor/mod.rs` (four actions)
- Modify: `src/commands.rs`
- Test: inline in both modules

**Interfaces:**
- Consumes: `table_edit::{table_block, rows, cell_at, align, Row, CellPos}`
- Produces:
  - `table_ops::insert_row(block: &str, after: usize) -> String`
  - `table_ops::delete_row(block: &str, row: usize) -> Option<String>`
  - `table_ops::insert_column(block: &str, after: usize) -> String`
  - `table_ops::delete_column(block: &str, col: usize) -> Option<String>`
  - `lists::renumber(text: &str, block: Range<usize>) -> Option<String>`

Tab navigation and pipe alignment exist; structural edits do not, so adding a
row means hand-editing pipes. Ordered lists never renumber, so a list edited in
the middle reads `1. 2. 3. 3. 4.`

Both are pure text transforms over a block — the shape `table_edit.rs` and
`lists.rs` already are.

- [ ] **Step 1: Write the failing table tests**

Create `src/editor/table_ops.rs`:

```rust
//! Structural table edits: whole rows and columns. Pure.

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";

    #[test]
    fn insert_row_adds_an_empty_row_after_the_given_one() {
        // Row 2 is "| 1 | 2 |" (row 1 is the separator).
        let out = insert_row(T, 2);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 5, "one more row: {out}");
        assert!(lines[3].starts_with('|') && lines[3].contains("  "), "blank cells: {:?}", lines[3]);
        assert_eq!(lines[4], "| 3 | 4 |", "the row below survives");
    }

    /// The separator row is structure, not data.
    #[test]
    fn the_separator_row_cannot_be_deleted() {
        assert!(delete_row(T, 1).is_none(), "row 1 is the separator");
    }

    #[test]
    fn delete_row_removes_only_that_row() {
        let out = delete_row(T, 2).expect("a body row");
        assert!(!out.contains("| 1 | 2 |"), "{out}");
        assert!(out.contains("| 3 | 4 |"), "{out}");
        assert!(out.contains("| --- |"), "the separator stays: {out}");
    }

    #[test]
    fn insert_column_widens_every_row_including_the_separator() {
        let out = insert_column(T, 0);
        for line in out.lines() {
            assert_eq!(line.matches('|').count(), 4, "three cells now: {line:?}");
        }
    }

    #[test]
    fn delete_column_narrows_every_row() {
        let out = delete_column(T, 0).expect("two columns exist");
        for line in out.lines() {
            assert_eq!(line.matches('|').count(), 2, "one cell left: {line:?}");
        }
        assert!(out.contains('b'), "the other column survives: {out}");
    }

    /// A one-column table cannot lose its last column.
    #[test]
    fn the_last_column_cannot_be_deleted() {
        let one = "| a |\n| --- |\n| 1 |";
        assert!(delete_column(one, 0).is_none());
    }
}
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test table_ops::
```

- [ ] **Step 3: Implement the table operations**

Write the four functions above the tests, operating on `table_edit::rows` and
returning an `align`ed block so the pipes line up after the edit. Each returns
`None` when the operation would destroy the table's structure (the separator
row, the last column).

- [ ] **Step 4: Write the failing renumber test**

Add to `mod tests` in `src/editor/lists.rs`:

```rust
    /// A list edited in the middle reads "1. 2. 3. 3. 4." Renumbering
    /// rewrites the run, keeping the first number the author chose.
    #[test]
    fn renumber_fixes_a_run_after_an_insertion() {
        let text = "1. one\n2. two\n2. inserted\n3. three\n";
        let out = renumber(text, 0..text.len()).expect("an ordered list");
        assert_eq!(out, "1. one\n2. two\n3. inserted\n4. three\n");
    }

    /// A list that starts at 5 keeps starting at 5.
    #[test]
    fn renumber_keeps_the_starting_number() {
        let text = "5. five\n5. six\n5. seven\n";
        let out = renumber(text, 0..text.len()).expect("an ordered list");
        assert_eq!(out, "5. five\n6. six\n7. seven\n");
    }

    /// Nested items renumber within their own level.
    #[test]
    fn renumber_treats_each_indent_level_separately() {
        let text = "1. a\n   1. x\n   1. y\n2. b\n";
        let out = renumber(text, 0..text.len()).expect("an ordered list");
        assert_eq!(out, "1. a\n   1. x\n   2. y\n2. b\n");
    }

    /// Bullets are left alone.
    #[test]
    fn renumber_ignores_an_unordered_list() {
        assert!(renumber("- a\n- b\n", 0..6).is_none());
    }
```

- [ ] **Step 5: Implement `renumber`**

In `src/editor/lists.rs`, using the existing `list_item` parser to find each
line's indent and marker, rewriting the number per indent level and leaving
non-ordered lines untouched. Return `None` when the block holds no ordered item.

- [ ] **Step 6: Wire four actions**

In `src/commands.rs`, all `ctx: Some("Editor")`:

```rust
    ed::TableInsertRow => { id: "table_insert_row", label: "Insert Row Below",
        keys: [], ctx: Some("Editor"), menu: Some((Format, 8)), help: Some(Editor) },
    ed::TableDeleteRow => { id: "table_delete_row", label: "Delete Row",
        keys: [], ctx: Some("Editor"), menu: Some((Format, 9)), help: Some(Editor) },
    ed::TableInsertColumn => { id: "table_insert_column", label: "Insert Column Right",
        keys: [], ctx: Some("Editor"), menu: Some((Format, 10)), help: Some(Editor) },
    ed::TableDeleteColumn => { id: "table_delete_column", label: "Delete Column",
        keys: [], ctx: Some("Editor"), menu: Some((Format, 11)), help: Some(Editor) },
```

Each handler finds the block with `table_block`, locates the cursor's cell with
`cell_at`, applies the operation, and replaces the block as one edit. Renumbering
runs on Enter-continuation in an ordered list, and as `Format → Renumber List`.

- [ ] **Step 7: Docs, suites, commit**

```sh
UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table
cargo run --example build_docs
cargo test && cargo test --no-default-features --features mas
```

```bash
git add src/editor/table_ops.rs src/editor/lists.rs src/editor/mod.rs src/commands.rs docs site
git commit -m "feat: table row and column commands, ordered-list renumbering

Tab navigation and pipe alignment existed; structural edits did not, so
adding a row meant hand-editing pipes. Ordered lists never renumbered,
so a list edited in the middle read '1. 2. 3. 3. 4.'

Both are pure transforms over a block, which is what table_edit.rs and
lists.rs already are. The separator row and the last column are
refused rather than silently destroying the table.

Closes #29"
```

---

### Task 6: Offer to become the default Markdown app (#24)

**Files:**
- Modify: `src/platform.rs`
- Modify: `src/settings.rs` (one field)
- Modify: `src/workspace.rs` (the offer, and a command)
- Modify: `src/commands.rs`
- Test: `src/platform.rs`, `src/settings.rs`

**Interfaces:**
- Produces:
  - `platform::is_default_markdown_handler() -> bool`
  - `platform::request_default_markdown_handler() -> Result<(), String>`
  - `settings::Settings.default_handler_asked: bool`

The bundle already declares `net.daringfireball.markdown` with
`LSHandlerRank: Owner`, so SuperMD appears under "Open With". What is missing is
any way to say so from inside the app: today it takes five steps in Finder that
most people will not find.

**Spike first — this task's shape depends on the answer.** Whether
`LSSetDefaultRoleHandlerForContentType` works under the App Sandbox is not
known. It mutates only the user's own LaunchServices database, so it is
plausibly allowed, but that must be verified against the **sandboxed** build,
not the DMG. macOS 14+ also has `NSWorkspace.setDefaultApplication(at:toOpen:)`,
which prompts the user itself and may be the better path; it needs a fallback,
since `LSMinimumSystemVersion` is 12.0.

- [ ] **Step 1: Spike — does it work under the sandbox?**

Build and run the `mas` configuration, call the API, and observe whether the
default actually changes:

```sh
cargo build --release --no-default-features --features mas
```

Record the answer in the task's report. If it is refused under the sandbox, the
task becomes "open System Settings at the right pane and tell the user what to
choose" — still better than nothing, and the plan is unchanged below except for
the body of `request_default_markdown_handler`.

- [ ] **Step 2: Write the failing tests**

In `src/settings.rs`:

```rust
    /// A refusal is remembered forever. A prompt that comes back is
    /// worse than no prompt.
    #[test]
    fn the_default_handler_answer_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        assert!(!s.default_handler_asked, "not asked on a fresh install");
        s.default_handler_asked = true;
        save(dir.path(), &s).unwrap();
        assert!(load(dir.path()).default_handler_asked);
    }
```

In `src/platform.rs`:

```rust
    /// Never true off macOS, and never panics anywhere.
    #[test]
    fn the_default_handler_query_is_safe_on_every_platform() {
        let answer = is_default_markdown_handler();
        if !MACOS {
            assert!(!answer, "only macOS has a Markdown handler to be");
        }
    }
```

- [ ] **Step 3: Implement**

Add `pub default_handler_asked: bool` to `Settings` (serde defaults to `false`,
so existing files load unchanged).

In `src/platform.rs`, both functions behind `#[cfg(target_os = "macos")]` with
non-macOS stubs returning `false` / `Err("not supported".into())`.

- [ ] **Step 4: The offer**

In `src/workspace.rs`, show it **after the user has opened Markdown**, never on
first launch — an app demanding to be your default before you have used it is
the behaviour people resent. A reasonable trigger is the third Markdown file
opened in a session, gated on `!settings.default_handler_asked`.

Both answers set `default_handler_asked = true` and save. Add a command so it
can be reached later:

```rust
    ws::MakeDefaultMarkdownApp => { id: "make_default_app",
        label: "Use SuperMD for Markdown Files", keys: [],
        ctx: None, menu: Some((Tools, 4)), help: None },
```

- [ ] **Step 5: Docs, suites, commit**

```sh
UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table
cargo run --example build_docs
cargo test && cargo test --no-default-features --features mas
```

```bash
git add src/platform.rs src/settings.rs src/workspace.rs src/commands.rs docs site
git commit -m "feat: offer to open Markdown files with SuperMD

The bundle already declared itself Owner for Markdown, so it appeared
under Open With -- but saying so meant five steps in Finder that most
people never find.

The offer waits until the user has actually opened Markdown rather than
appearing on first launch, and a refusal is remembered permanently: a
prompt that returns is worse than no prompt. Also available from the
Tools menu for anyone who said no and changed their mind.

Closes #24"
```

---

### Task 7: The parked findings from the 0.0.15 review (#30)

**Files:**
- Modify: `src/editor/mod.rs` (pending-link and hover lifetime)
- Modify: `src/knowledge.rs` (hardlinks)
- Modify: `src/workspace.rs` (`on_fs_events` visibility)
- Modify: `src/preview.rs` (settings read off the UI thread)
- Modify: `docs/site/knowledge.md`
- Test: inline beside each

**Interfaces:** none new — each is a correction inside an existing function.

Six small correctness items the 0.0.15 review found and judged non-blocking.
Grouped because each is a handful of lines; they share a review gate.

- [ ] **Step 1: A pending link cannot outlive its press**

Clear `pending_link` and `hover_link` in `reproject` and on focus loss. Today,
pressing on a link, switching tabs, returning and releasing over blank space
navigates — the background editor never received a mouse-up because it was no
longer rendered.

```rust
    /// A press belongs to the document that was on screen when it
    /// happened. Neither survives the buffer being swapped or the
    /// editor leaving the screen.
    #[gpui::test]
    fn a_reload_clears_a_pending_press(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "see [[Target]] here\n");
        editor.update(cx, |ed, _| {
            ed.pending_link = ed.link_at_offset(6).cloned().map(|link| PendingLink { offset: 6, link });
            assert!(ed.pending_link.is_some(), "precondition");
        });
        editor.update(cx, |ed, cx| ed.reload_from_disk(cx));
        editor.update(cx, |ed, _| {
            assert!(ed.pending_link.is_none(), "a reload drops the press");
            assert!(ed.hover_link.is_none(), "and the hover it belonged to");
        });
    }
```

- [ ] **Step 2: A drag that leaves the editor cancels the click**

`on_mouse_up_out` currently follows a pending link. Browsers treat a release
off-target as a cancelled click; match that by clearing rather than navigating.

- [ ] **Step 3: Hardlinks are not indexed**

`Index::scan` filters symlinks only, so `ln ~/.ssh/id_rsa vault/leak.md` is
indexed and read. Not a vault-borne vector — git cannot carry a hardlink — but
the property claimed is "nothing outside the workspace is read". Check
`st_nlink > 1` and skip, with a `#[cfg(unix)]` test (Windows compiles, per the
global constraints).

- [ ] **Step 4: The watcher and the scanner agree**

`on_fs_events` admits every `.md` in a batch when *any* path in it is visible,
so files under `.git/` or `target/` can enter the index that `Index::scan`
excludes. Check each path, not the batch.

- [ ] **Step 5: `preview_for` stops reading settings on the UI thread**

It calls `settings::load` — a file read and a TOML parse — every time a popover
opens. Read the grants once into `PreviewState` and refresh them when a grant is
written.

- [ ] **Step 6: Correct the docs**

`docs/site/knowledge.md:3` says SuperMD "indexes every markdown file", which
stopped being true when symlinked notes were filtered out. Fix the source, then:

```sh
cargo run --example build_docs
```

- [ ] **Step 7: Suites and commit**

```sh
cargo test && cargo test --no-default-features --features mas
```

```bash
git add src docs site
git commit -m "fix: the parked findings from the 0.0.15 review

A pending press no longer outlives the document it was made on: a
press, a tab switch, and a release over blank space used to navigate,
because the background editor never saw the mouse-up. A drag that
leaves the editor now cancels the click, as browsers do.

Hardlinks are no longer indexed -- the scanner filtered symlinks only,
so a hardlink into a file outside the workspace was read. Not carried
by a cloned vault, but the property claimed is that nothing outside the
workspace is read.

The watcher checks each path rather than admitting a whole batch when
one path is visible, so it and Index::scan finally agree on what a note
is. And the hover popover no longer reads settings off disk on the UI
thread every time it opens.

Closes #30"
```

---

### Task 8: Multiple windows

**Files:**
- Modify: `src/knowledge.rs` (`KnowledgeState` becomes per-workspace)
- Modify: `src/extensions.rs` (workspace root per host, not per process)
- Modify: `src/workspace.rs` (33 call sites; window creation)
- Modify: `src/main.rs` (window options, `NewWindow`)
- Modify: `src/commands.rs`
- Test: `src/workspace.rs`

**Interfaces:**
- Produces:
  - `Workspace.knowledge: Arc<Mutex<knowledge::Index>>` — an entity field, not
    a global
  - `workspace::open_in_new_window(path: Option<PathBuf>, cx: &mut App)`

**This is the largest task in the release, and its risk is not the windows.**
`KnowledgeState` is a process-wide global holding the index for *one* workspace;
two windows on different folders would share it, so window B's backlinks and
graph would show window A's notes. `ExtensionState::set_workspace_root` is
worse: it is the plugin sandbox's read boundary, and it rebuilds preopens when
it changes — two windows would fight over which folder plugins may read.

Do the state migration first, verify it with one window still behaving
correctly, and only then add the second window.

- [ ] **Step 1: Write the failing test**

```rust
    /// Two windows, two folders, two indexes. A process-wide
    /// KnowledgeState meant the second workspace's backlinks and graph
    /// showed the first workspace's notes.
    #[gpui::test]
    fn two_workspaces_keep_separate_indexes(cx: &mut TestAppContext) {
        let _home = temp_home();
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("Alpha.md"), "# Alpha\n").unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(b.path().join("Beta.md"), "# Beta\n").unwrap();

        let (ws_a, cx) = open_workspace(cx, a.path());
        let (ws_b, cx) = open_workspace(cx, b.path());
        cx.run_until_parked();

        cx.update(|_, app| {
            let names_a: Vec<String> = ws_a.read(app).knowledge.lock().unwrap()
                .note_names().iter().map(|(n, _)| n.clone()).collect();
            let names_b: Vec<String> = ws_b.read(app).knowledge.lock().unwrap()
                .note_names().iter().map(|(n, _)| n.clone()).collect();
            assert!(names_a.iter().any(|n| n.eq_ignore_ascii_case("alpha")));
            assert!(!names_a.iter().any(|n| n.eq_ignore_ascii_case("beta")),
                "window A does not see window B's notes: {names_a:?}");
            assert!(names_b.iter().any(|n| n.eq_ignore_ascii_case("beta")));
            assert!(!names_b.iter().any(|n| n.eq_ignore_ascii_case("alpha")),
                "and the reverse: {names_b:?}");
        });
    }
```

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test two_workspaces_keep_separate_indexes
```
Expected: fails — both read the same global index.

- [ ] **Step 3: Move the index onto the workspace**

Add `knowledge: Arc<Mutex<knowledge::Index>>` to `Workspace`. Migrate all 33
`cx.try_global::<KnowledgeState>()` call sites to read the field. Keep
`KnowledgeState` as a type but stop installing it as a global.

The editor reads the index through events or a passed reference rather than a
global — the `Editor` entity does not know which workspace owns it, so the
workspace passes what it needs.

- [ ] **Step 4: Make the plugin sandbox root per-window**

`ExtensionHost::set_workspace_root` is a security boundary: it decides which
directory a `workspace-read` plugin may open. With two windows, the host must
know which workspace is asking, or a plugin invoked from window A could read
window B's folder.

The cheapest correct answer is one host per workspace. Verify the preopen
actually follows by extending the existing sandbox test to two roots.

- [ ] **Step 5: Verify one window still works**

```sh
cargo test && cargo test --no-default-features --features mas
```
Everything must pass with a single window before a second exists.

- [ ] **Step 6: Commit the migration on its own**

```bash
git add src
git commit -m "refactor: per-workspace knowledge index and plugin sandbox root

KnowledgeState was a process-wide global holding the index for one
workspace, and ExtensionState carried one plugin-sandbox root for the
process. Both are per-window state, and both had to move before a
second window could exist -- the sandbox root especially, since it
decides which directory a plugin may read.

No behaviour change with one window; the test asserts two workspaces
keep separate indexes."
```

- [ ] **Step 7: Add the windows**

```rust
    ws::NewWindow => { id: "new_window", label: "New Window",
        keys: ["cmd-shift-n"], ctx: None, menu: Some((File, 2)), help: Some(General) },
    ws::OpenFolderInNewWindow => { id: "open_folder_new_window",
        label: "Open Folder in New Window…", keys: [],
        ctx: None, menu: Some((File, 3)), help: Some(General) },
```

`⌘⇧N` currently means New Folder Here in the sidebar context; that binding stays,
since `ctx: Some("Sidebar")` wins over a global while the sidebar has focus.
Verify that with a test rather than assuming — this is the collision class that
made ⌘⇧G unreachable in 0.0.15.

Each new window builds its own `Workspace` entity with its own index and host.

- [ ] **Step 8: Docs, suites, commit**

```sh
UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table
cargo run --example build_docs
cargo test && cargo test --no-default-features --features mas
```

```bash
git add src docs site
git commit -m "feat: multiple windows, each its own workspace

⌘⇧N opens an empty window; Open Folder in New Window opens a chosen
folder beside what you already have. Each window owns its index, tabs,
graph and plugin sandbox root, so two vaults open at once no longer
show each other's backlinks."
```

---

### Task 9: Ship 0.0.16

**Files:** `Cargo.toml`, `Cargo.lock`

- [ ] **Step 1: Full verification**

```sh
bash scripts/build_plugins.sh --fixtures
cargo test
cargo test --no-default-features --features mas
cargo llvm-cov --summary-only --fail-under-lines 90
bash scripts/check_private_apis.sh target/release/supermd
```

The fixtures matter: without them `extensions.rs` measures ~47% and the total
reads below the floor for a reason that has nothing to do with this release.

- [ ] **Step 2: Bump the version**

`Cargo.toml` to `0.0.16`, then `cargo update -p supermd --precise 0.0.16`.
**Commit before tagging** — cargo-deb reads the manifest while the DMG and the
App Store build take the tag, so bumping after ships a mismatch the tag hides.

```bash
git add Cargo.toml Cargo.lock
git commit -m "release: v0.0.16"
```

- [ ] **Step 3: Hand the release to the user**

Tagging, pushing and submitting to App Review are outward-facing and
irreversible. Stop here and report: the branch is verified, the version is
bumped, and these remain:

```sh
git push origin master
git tag -a v0.0.16 -m "v0.0.16 — things the app should already have"
git push origin v0.0.16
```

Then build and submit the App Store package per `docs/mac-app-store.md`.

---

## Deferred from this release

Named so nobody adds them mid-flight:

- **Graph follow-ups** (#26) — hover previews on nodes, tag nodes, clustering,
  persisted pinned layout. The graph had a great deal of attention in 0.0.15 and
  none of what is left is a gap.
- **Graph time-scrubbing** (#27) — the differentiating idea, and the one that
  most deserves its own design rather than a slot in a mixed release.
- **The agent layer** — 0.1.0. Approach A was approved during the 0.0.15
  brainstorm but the spec was never written, and the app has changed
  considerably since: hover previews, a consent model, a live graph, and a
  read-only-by-default single click. Re-examine the design against what SuperMD
  now is before writing the spec.
- **Certifying the performance baseline.** ~147 MB resident is recorded as a
  *floor*, not a measurement: every run was taken with the screen locked, so the
  window was never composited. The agent layer is the feature most likely to
  undermine the lightweight claim, and that number should be certified before it
  lands rather than after.
