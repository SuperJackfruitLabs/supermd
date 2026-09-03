# Getting Started

SuperMD is a Markdown editor that reads like a finished page and edits like plain text. Formatting markers stay hidden until your cursor touches them, then reveal in place — headings, lists, tables, diagrams, all live. Open a folder and your notes [know each other](knowledge.md): wiki links with completion, backlinks, tags, and a graph — all computed from your files. Which stay plain CommonMark on disk: no database, no proprietary format, no lock-in.

## Install

**macOS — Mac App Store** — [get SuperMD Editor](https://apps.apple.com/app/supermd-editor/id6807117461?mt=12). Installs and updates itself, and runs in Apple's sandbox. Needs Apple silicon and macOS 12 or later.

**macOS — direct download** — [get the DMG](https://github.com/SuperJackfruitLabs/supermd/releases/latest), open it, and drag SuperMD into Applications. Signed and notarized, so it opens without warnings, and if you launch it straight from the DMG, SuperMD offers to move itself into Applications for you. Also needs Apple silicon.

The two builds are the same editor. The sandboxed one asks permission the first time it opens a folder, keeps its settings inside the app container rather than `~/.supermd`, and cannot load [grammar plugins](grammars.md) — so GraphQL fences render as plain text there. Everything else is identical.

**Linux** — grab the `.deb` or the `.tar.gz` from the [latest release](https://github.com/SuperJackfruitLabs/supermd/releases/latest).

**Windows** — run the installer from the [latest release](https://github.com/SuperJackfruitLabs/supermd/releases/latest).

## First launch

SuperMD opens with a welcome tour — and the tour itself is a real, editable document. Click things: checkboxes toggle, tables dissolve into editable text, bold markers appear under your cursor and fold away when you leave.

When you're ready to work with your own notes, press **⌘ O** (Ctrl+O on Windows/Linux) or click **Open Folder** in the sidebar. SuperMD remembers your recent workspaces and can reopen the last one on launch.
