# Working with a Folder

Open a folder (**⌘ O**, or drop one on the window) and SuperMD becomes a workspace: a file tree in the sidebar, fast navigation, search, and git awareness.

## Getting around

- **Go to file** — **⌘ P** opens a fuzzy finder over every file in the workspace. Type a few letters, ⏎ opens.
- **Sidebar browsing** — arrow keys move through the tree; files open in a single preview tab as you move, so browsing doesn't leave a trail of tabs. ⏎ or a double-click pins the file in its own tab.
- **Outline** — **⌘ ⇧ O** shows the current document's headings; click one to jump.
- **Recents** — SuperMD remembers your recent workspaces (File → Open Recent), and can reopen the last one automatically on launch.

## Search the whole workspace

**⌘ ⇧ F** searches every file, streaming results as they're found. The search is smart-case: all-lowercase queries match case-insensitively, and any capital letter makes the match exact. Results are grouped by file; ⏎ jumps to the hit. Files ignored by `.gitignore` are skipped.

## Git awareness

If your folder is a git repository:

- Files with uncommitted changes get a dot in the sidebar.
- **⌘ ⇧ D** shows the current file's changes against the last commit — a read-only diff view, additions and deletions colored. Esc returns to editing.

SuperMD never writes to your repository — no commits, no staging, no surprises. It only reads.

## Non-Markdown files

The workspace isn't Markdown-only: source files open with syntax highlighting, images open in a zoomable viewer, and CSV files render as tables (see [Plugins](plugins.md) — the CSV viewer is one, and you can add more).
