# Workspace Hygiene, Project Search & Preview Tabs Design

**Date:** 2026-08-24
**Status:** Approved for planning

## Purpose

Make SuperMD behave properly inside real repositories and scale its
navigation: ignore-aware file listing, industry-grade fuzzy matching,
project-wide text search, a pure-Rust git backend ready for a richer
git client later, and VS Code-style preview tabs for keyboard-driven
browsing.

## Decisions (settled during brainstorming)

- Adopt now: `ignore` (walking), `nucleo-matcher` (fuzzy), ripgrep's
  `grep-searcher`/`grep-regex` (project search), `gix` (replacing
  `git2`).
- Project search UI: finder-style overlay dialog, not a sidebar panel.
- ⌘⇧F becomes project search; Focus Mode moves to ⌃⌘F.
- Search engine: background streaming (approach A) — no index, no
  synchronous scans.
- Preview tabs: sidebar arrow-navigation and single click open into
  one reusable preview tab; Enter, double-click, or editing promotes
  it to permanent.

## Component 1: Ignore-aware walking — `src/files.rs`, `src/finder.rs`

Both the sidebar FileTree and the finder's candidate walk switch to
the `ignore` crate:

- `WalkBuilder` semantics: respect `.gitignore` (repo, global, and
  nested), skip hidden entries (dotfiles) and `.git`, keep untracked
  files visible. Outside a repo this reduces to "skip hidden".
- The FileTree keeps its lazy per-directory expansion model; `ignore`
  supplies the per-entry filter. Implementation detail: a directory
  listing helper `files::list_dir(dir, root) -> Vec<FsEntry>` builds a
  single-level listing using an `ignore::gitignore::Gitignore` matcher
  chain rooted at the workspace (the `ignore` crate's `WalkBuilder`
  with `max_depth(1)` is acceptable if simpler).
- The finder's full-workspace candidate walk uses `WalkBuilder`
  directly (parallel walker not required; the sequential walker is
  fast enough for finder candidate collection).
- The file watcher continues to watch the root recursively; events for
  ignored paths are dropped before triggering refresh (path checked
  against the same matcher) so `target/` churn doesn't redraw the app.

## Component 2: Nucleo fuzzy matching — `src/finder.rs`

Replace the homegrown matcher with `nucleo-matcher` (library layer —
we own candidate collection and the UI):

- One `nucleo_matcher::Matcher` per finder session; pattern parsed
  with `Pattern::parse(query, CaseMatching::Smart, Normalization::Smart)`.
- Candidates are workspace-relative path strings; score via
  `pattern.score(Utf32Str, &mut matcher)`; keep the existing
  sort-by-score-desc + stable-name tiebreak; drop zero-score.
- Match indices (via `pattern.indices`) drive per-character highlight
  styling in result rows (accent color on matched chars) — new visual
  affordance the old matcher never provided.

## Component 3: Project search — new `src/search.rs` + overlay

**Engine (`src/search.rs`, pure logic + background task):**

```rust
pub struct SearchMatch {
    pub path: PathBuf,          // workspace-relative
    pub line_number: u64,       // 1-based
    pub line_text: String,      // trimmed to ~240 bytes around the hit
    pub ranges: Vec<Range<usize>>, // hit byte ranges within line_text
}

/// Blocking search, called on the background executor. Streams
/// batches through `tx`; returns early once `cap` matches are sent or
/// `cancelled` flips.
pub fn search_workspace(
    root: &Path,
    query: &str,
    cap: usize,
    cancelled: &AtomicBool,
    tx: Sender<Vec<SearchMatch>>,
);
```

- Matcher: `grep_regex::RegexMatcherBuilder` with
  `fixed_strings(true)` (literal query) and smart case: if the query
  has no uppercase letter, `case_insensitive(true)`.
- Walk: `ignore::WalkBuilder` with the Component 1 rules; binary files
  skipped by `grep-searcher`'s default binary detection.
- Cap: 500 matches, then stop the walk; the final batch is flagged so
  the UI can show "500+ matches (capped)".
- Cancellation: an `AtomicBool` checked per file; the UI flips it when
  the query changes or the dialog closes.

**Overlay (`src/workspace.rs` + search state):**

- ⌘⇧F opens a dialog in the finder family: same fixed 680×440
  two-pane layout. Query input (existing `TextInput`) on top of the
  left pane; below it the match list grouped by file — a file header
  row (Seti icon + relative path), match rows underneath (line number
  in muted, line text with hit ranges washed in `find_match_bg`).
- Right pane: preview of the selected match's file (existing
  finder preview machinery), scrolled so the match line is visible,
  the line highlighted.
- Keys: ↑/↓ move through match rows (skipping file headers), Enter
  opens the file as a permanent tab and jumps to the line
  (`Editor::scroll_to_line`), Escape closes. Typing restarts the
  search after a 120 ms debounce.
- Empty states: "Type to search the workspace" and "No matches".
- Key context "Search"; bindings mirror the Finder context.
- Focus Mode rebinds to ⌃⌘F (`ctrl-cmd-f`); SHORTCUTS table, menus,
  README, WELCOME updated.

## Component 4: git2 → gix — `src/git.rs`

Public API unchanged: `Baseline{Text,NotInRepo,Untracked,Binary}`,
`head_text(&Path) -> Baseline`, `modified_paths(&Path) -> HashSet<PathBuf>`.
Internals swap to `gix`:

- `gix::discover(parent)`, `repo.head_commit()` → tree →
  `lookup_entry` by relative path components → blob data; UTF-8 check
  unchanged. Unborn HEAD / missing entry → `Untracked`; discovery
  failure → `NotInRepo`.
- `modified_paths`: `repo.status(gix::progress::Discard)` iterator
  with untracked files included (per-file granularity), ignored files
  excluded; entries re-relativized from the repo workdir to the
  workspace root exactly as today.
- Dependency: `gix` with default features minus network transports
  (`default-features = false`, features = `["status", "worktree-mutation"]`
  adjusted at implementation time to the minimal set that compiles the
  above — the plan records the exact list).
- Tests: identical assertions to today, but fixture repos are authored
  by shelling out to the system `git` CLI in a helper
  (`git init/add/commit` with `-c user.name -c user.email`), removing
  the git2 dev-dependency entirely. git2 and libgit2 leave the tree.

## Component 5: Preview tabs — `src/workspace.rs`

VS Code-style transient tab for browsing:

- `Workspace.preview_tab: Option<usize>` — index of the current
  preview tab, at most one.
- **Opening as preview** (sidebar ↑/↓ keyboard navigation lands on a
  file; single click on a file): if a permanent tab for the path
  exists, activate it. Else if a preview tab exists, replace its
  contents (new Reader/Editor/Image entity, same tab slot). Else push
  a new tab and mark it preview.
- **Promotion to permanent**: Enter on the sidebar selection,
  double-click on the file row, double-click on the tab itself, or any
  buffer edit in a previewed editor (workspace observes the editor's
  dirty transition) clears `preview_tab`.
- **Replacement discards** the previewed entity (editors flush first —
  autosave discipline unchanged; flush of an unmodified buffer is a
  no-op).
- Visual: preview tab titles render italic; everything else about the
  tab is normal.
- Sidebar keyboard navigation currently only moves the selection;
  with this change, moving onto a file row opens it as preview
  (directories unchanged: → expands, ← collapses). This makes ↑/↓ a
  full file browser.
- ⌘P finder "Enter" continues to open permanent tabs (finder is a
  deliberate jump, not browsing).

## Keybindings summary

| Key | Action |
| --- | --- |
| ⌘⇧F | Project search overlay (new) |
| ⌃⌘F | Focus Mode (moved from ⌘⇧F) |
| Sidebar ↑/↓ | Move selection and open preview tab |
| Sidebar ⏎ / double-click | Open permanent tab |

## Error handling

- Search: unreadable files are skipped silently (grep-searcher error
  sink ignores them); the walk never aborts the app.
- gix errors collapse to `NotInRepo`/`Untracked`/empty set exactly as
  git2 errors do today.
- Preview-tab promotion is idempotent; closing a preview tab clears
  `preview_tab`.

## Testing strategy

- `files.rs`: tempdir with `.gitignore`, hidden files, nested ignores —
  listing excludes ignored/hidden, includes untracked.
- `finder.rs` matching: ranking sanity (exact > prefix > scattered),
  smart-case, match indices returned for highlighting.
- `search.rs`: fixture workspace — literal matching, smart-case, cap
  behavior, ignored files excluded, multi-file grouping order,
  cancellation flag stops the stream.
- `git.rs`: existing 7 tests unchanged in intent, fixtures via git CLI.
- Preview tabs: pure helper tests where extractable (promotion/replace
  state transitions modeled as plain functions over a small state
  struct if feasible); UI wiring verified by compile + manual smoke,
  per the repo's thin-shell convention.

## Out of scope

Regex/glob query syntax, search-and-replace, searching unsaved buffer
contents, persistent search index, git client UI beyond what exists
(the gix migration only re-plumbs today's features), preview-tab
behavior for the ⌘P finder.
