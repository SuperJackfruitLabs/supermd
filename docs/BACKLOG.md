# Backlog

Everything consciously deferred, cut from a spec's scope, or discussed and
parked — with why, so future planning starts from decisions instead of
archaeology. Living document: prune what ships, add what gets cut.

_Last groomed: 2026-08-25, after v0.0.12 (knowledge release)._

## Knowledge features (deferred from M1–M4)

| Item | Notes |
| ---- | ----- |
| Drag-and-drop file moves | M1 shipped keyboard move (⌘⇧M picker); drag in the sidebar needs new interaction machinery |
| Multi-select file operations | M1 non-goal; single-row ops only today |
| Unresolved-link styling | `[[Ghost]]` renders like any wiki link; a distinct color/underline would telegraph "will create" before you click |
| Tag completion | Typing `#` could complete against known tags the way `[[` does for notes |
| Unlinked mentions | Backlinks panel shows explicit links only; Obsidian-style "this note's name appears un-linked in 4 files" is a separate index pass |
| Embeds / transclusion | `![[note]]` rendering a note inline — needs a block-projection surface decision |
| Note aliases / frontmatter | YAML frontmatter is currently plain text; aliases would feed resolution and completion |
| Graph: tag nodes & ghosts | Tags and unresolved targets as first-class graph nodes; color clusters by folder/tag |
| Graph: live physics | Layout is computed once at open (150 iterations); a running simulation with drag-a-node would feel alive |
| Index scaling | Full synchronous scan at workspace-open and per-event re-read; fine to ~thousands of notes, wants a background/incremental pass for huge vaults |

## Writing ergonomics (deferred from the v0.0.11 batch)

| Item | Notes |
| ---- | ----- |
| Table row/column commands | Insert/delete row & column via palette; batch shipped Tab-nav + auto-align only |
| Ordered-list renumbering | Reordering/deleting numbered items doesn't renumber siblings |
| Drag-drop image files | Image *paste* shipped; dropping a file onto the editor should do the same |
| Auto-pair markers at cursor | Typing `*` wraps a selection; with a bare cursor there's no `**` pairing |

## Plugin ecosystem

| Item | Notes |
| ---- | ----- |
| Uninstall UI | Deleting a plugin still means deleting its folder |
| Update nudge | Compare installed plugin versions against the catalog; offer one-click update |
| Catalog hashes in CI | `scripts/update_catalog_hashes.sh` is a manual post-release step; automating it into the release workflow kills the "pending sha256" window |
| Panels surface (E1) | Plugins contributing sidebar/panel UI — prerequisite for several plugin ideas; also the dogfooding path for the knowledge panel |
| On-open hooks | Symmetric to save hooks |
| Inline math | KaTeX-style rendering wants a richer inline surface (sized inline images) |
| Computed tables | Spreadsheet-style formulas in markdown tables |
| Third-party registry | Catalog is org-pinned by design; a community registry plus a plugin starter-repo extraction of `plugins/template/` |

## Themes / flux

| Item | Notes |
| ---- | ----- |
| Wake-time preference | f.lux's "keep night mode until my morning" — stay warm past midnight |
| System location (opt-in) | Manual coordinates only today, by design; CoreLocation could be an explicit opt-in later |
| Per-theme flux pairing | One global light/dark pair today; themes could declare their own day/night partners |

## Distribution & platform

| Item | Notes |
| ---- | ----- |
| Windows code signing | SmartScreen still shows one "unrecognized app" prompt |
| In-app auto-update | Launch check shows an "update available" pill linking out; downloading and swapping in place (Sparkle-style) is the retention feature |
| Homebrew cask / winget | Cheap once naming is stable |
| README screenshot | Predates the toolbar, knowledge panel, and graph — retake on v0.0.12 |

## Shortcuts, menus & chrome — reorganization candidate

A 2026-08-25 audit ahead of a possible UX pass. Current state: **120
keybindings**, menus that haven't grown since ~v0.0.5, and almost no
clickable chrome. Findings:

**Menu gaps (biggest issue):**
- **No Edit menu at all.** No Undo/Redo/Cut/Copy/Paste/Select All in the
  menu bar — this also breaks macOS conventions (Edit is where the
  system hangs dictation, emoji, and substitutions).
- **None of the knowledge features are in menus**: knowledge panel,
  Graph View, follow link — invisible to menu browsers.
- Missing from menus: Save Now, Find in File, Install Plugins…,
  formatting actions (bold/italic/…), flux toggle, sidebar file ops.
- **View is a grab-bag**: navigation ("Go to File…", "Search in
  Workspace…"), plugin management ("Open Plugins Folder", "Reload
  Plugins"), and view toggles all in one menu. Wants a split into
  View / Go / Plugins (or Tools).

**Shortcut observations:**
- ⌘⇧-letter space is nearly saturated (D F G K M N O P + `[` `]`);
  future features will collide soon.
- No shortcut for Graph View (palette-only today).
- ⌘⇧M (move file) vs ⌘⇧K (knowledge) vs ⌘⇧N (new folder) are
  memorable individually but have no mnemonic system.
- The ⌘/ dialog and docs/site/shortcuts.md are hand-synced with
  main.rs bindings; a generation step (or change-detector test tying
  them together) would prevent drift.

**Chrome / quick-action candidates (deliberately minimal today):**
- Panel toggle buttons (sidebar / outline / knowledge) in the titlebar —
  the standard three-pane affordance; currently keyboard-only.
- A `+` new-file button in the sidebar header.
- Status-corner toggles: flux on/off (sun icon), graph view.
- A discoverable home for "Show Changes" beyond ⌘⇧D.

Suggested shape when picked up: one bounded pass — restructure menus
(add Edit, split View, add knowledge/format items), rationalize the
shortcut map (documenting a mnemonic scheme before the space runs out),
and add the 3–5 highest-value chrome buttons behind a design review.

## Explicit non-goals (decided, not deferred)

- **No LSP** — code files get viewer-plus depth, deliberately.
- **No proprietary format** — plain CommonMark on disk, always; the
  knowledge index stays a rebuildable cache.
- **SuperMD never writes to the user's git repository.**
- **No plugin-drawn arbitrary UI** — plugins render content, not chrome
  (the panels surface, when it comes, is host-controlled).
