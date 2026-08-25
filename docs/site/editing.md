# Editing

SuperMD's editor is *hybrid*: your document always looks typeset, and the raw Markdown appears exactly where — and only where — you're working. There is no separate source mode to switch into and no preview pane to keep in sync. The page is the editor.

## Markers reveal where you touch

Click inside a **bold phrase** and the `**` markers appear around it; click away and they fold up again. The same applies everywhere:

- Headings show their `#` marks while your cursor is on the line.
- Links display as their text; touch one and the `[text](url)` form opens for editing. Links between notes — `[[wiki links]]` included — get completion, backlinks, and a graph: see [Links, Tags & Graph](knowledge.md).
- Blockquotes, lists, and inline code all reveal their markers on contact.

You always edit real Markdown — SuperMD never rewrites your file behind your back. What lands on disk is exactly the plain CommonMark you typed.

## Checkboxes, tables, and code

- **Task lists** — click a checkbox to toggle it. `- [ ]` becomes `- [x]`, one undo step.
- **Tables** — rendered as proper tables. Click a row and the whole table dissolves into editable pipe syntax; click away and it re-renders.
- **Code fences** — syntax highlighting for 78+ languages, live as you type. The ``` fence delimiters hide while you're outside the block.
- **Images** — display inline; click one to edit its `![alt](path)` source.

Diagrams work the same way — see [Diagrams](diagrams.md).

## Lists that keep up

Press **⏎** inside a list and the next item is ready: bullets carry over, numbers increment, task items continue unchecked. Press ⏎ on an empty item to end the list. **Tab** and **⇧ Tab** indent and outdent the current item.

## Tables you can Tab through

Inside a table, **Tab** jumps to the next cell with its content selected — from the last cell it adds a fresh row. **⇧ Tab** goes back. Whenever you hop cells or leave the table, the pipes re-align so the source stays tidy without you ever counting spaces.

## Paste images straight in

Copy an image anywhere and paste it into a document: SuperMD saves it as `assets/pasted-<timestamp>.png` beside the file and inserts the `![](…)` link for you. The document folder stays portable — move it and its images move too.

## Formatting toolbar

Select text with the mouse and a small toolbar appears above the selection: **bold**, *italic*, `code`, ~~strikethrough~~, link, heading level, and blockquote — each a single click, each one undo step. Toggles work both ways: bolding a bold selection unbolds it.

**⌘ B** (Ctrl+B) and **⌘ I** (Ctrl+I) do the same from the keyboard while text is selected. Without a selection, ⌘ B keeps its usual job of toggling the sidebar.

Typing a marker key over a selection wraps instead of replacing: select a word, press `*`, and it's *italic* — press `*` again for **bold**. The same works for `` ` ``, `_`, and `~`.

## Preview and focus

- **⌘ E** (Ctrl+E) toggles a fully rendered read-only preview of the current document — useful for a final read-through.
- **⌃ ⌘ F** (Ctrl+Alt+F) enters focus mode: chrome fades away, just you and the text.

## Saving is not your job

SuperMD autosaves shortly after you stop typing, and **⌘ S** (Ctrl+S) saves immediately if you want the reassurance. If a file changed on disk while you had unsaved edits — say, from a git checkout — SuperMD never silently overwrites either side: the disk version is backed up first, every time.
