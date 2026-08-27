# Notes that Know Each Other

Open a folder and SuperMD quietly indexes every markdown file in it — links, backlinks, and tags, all computed locally and kept fresh as you type. Your files stay plain CommonMark on disk; the index is just a cache that rebuilds itself.

## Wiki links

Type `[[` and a completion popup lists every note in the workspace, filtered as you type. Confirm with ⏎ and you get a `[[Note]]` link. Standard `[text](note.md)` links work everywhere wiki links do — both styles resolve, complete the graph, and count as backlinks.

- **Follow a link** with **⌘ click** or **⌘ ⏎** with the cursor on it.
- **Link to a note that doesn't exist yet** and following it creates the note beside the current file — write first, organize later.
- `[[Note|label]]` shows the label but links to the note.
- Wiki targets match by file name anywhere in the workspace (case-insensitive); use `folder/Note` to disambiguate duplicates.

## Renames never break the graph

Rename or move a note — or a whole folder — in the sidebar, and every link pointing at it is rewritten across the workspace: wiki links get the new name, standard links get the recomputed relative path, labels survive. Open tabs follow along too.

## Backlinks and tags

**⌘ 3** opens the knowledge panel:

- **Backlinks** — every note linking to the one you're reading, each with the line of context it links from. Click to jump there.
- **Tags** — write `#tag` (or nested `#area/subtopic`) anywhere in a note. The panel shows every workspace tag with its count; click one to search for it.

## The graph

The knowledge panel starts with a **local graph**: the current note and its immediate neighbors. Click any node to navigate.

**Graph View** in the command palette (⌘ ⇧ P) opens the full workspace graph — every note, every link, laid out by a force simulation and rendered natively. Drag to pan, scroll to zoom, click a note to open it, esc to close. Connected clusters pull together; orphan notes drift to the edges, which is exactly how you find them.

## Where's the database?

There isn't one. Everything above is computed from your markdown files at open time and kept current by the file watcher. Delete `.md` files, edit them in another app, sync them however you like — the knowledge features follow the files.
