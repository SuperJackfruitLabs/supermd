# Backlog

Everything consciously deferred, cut from a spec's scope, or discussed and
parked — with why, so future planning starts from decisions instead of
archaeology. Living document: prune what ships, add what gets cut.

_Last groomed: 2026-08-26, after the shortcuts/menus/chrome pass._

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
| In-app auto-update | About (⌘/ → About) checks on demand and links out. Self-replacement is **three** implementations, not one, and on a `.deb` install the binary is root-owned in `/usr/bin` — the correct Linux answer is an apt repository, not self-update. Wants its own spec |
| Homebrew cask / winget | Cheap once naming is stable |

## Shortcuts, menus & chrome

The 2026-08-25 audit is **done** — see
`docs/superpowers/specs/2026-08-26-shortcuts-menus-chrome-design.md` and
the plan beside it. Commands are declared once in `src/commands.rs`; the
keybindings, macOS menu bar, ☰ popover, ⌘/ dialog and generated docs are
all projections of that table, and tests assert the popover and menu bar
cannot diverge again.

Shipped: Edit / Format / Go / Tools menus, panels on ⌘1/⌘2/⌘3 (freeing
⌘⇧O and ⌘⇧K), ⌘⇧G for the graph, ⌃⌘N for flux, a written modifier
scheme, the About dialog, titlebar panel toggles, a sidebar `+`, a Show
Changes button, and the status bar.

What the pass left open:

| Item | Notes |
| ---- | ----- |
| Format toggles have no chords | Code / strike / link / heading / quote are menu- and toolbar-reachable only. Deliberate — the ⌘⇧-letter space was just relieved and should not be re-filled without a reason |
| Edit-menu items in non-editor tabs | Undo/Cut/Copy/Paste/Select All are bound in the `Editor` context but appear in the menu always; on a Reader or Image tab they dispatch into nothing |
| User-editable keymaps | The table is compile-time. A JSON keymap is a separate feature |

## Explicit non-goals (decided, not deferred)

- **No LSP** — code files get viewer-plus depth, deliberately.
- **No proprietary format** — plain CommonMark on disk, always; the
  knowledge index stays a rebuildable cache.
- **SuperMD never writes to the user's git repository.**
- **No plugin-drawn arbitrary UI** — plugins render content, not chrome
  (the panels surface, when it comes, is host-controlled).
