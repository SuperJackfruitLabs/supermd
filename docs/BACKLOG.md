# Backlog

Everything consciously deferred, cut from a spec's scope, or discussed and
parked — with why, so future planning starts from decisions instead of
archaeology. Living document: prune what ships, add what gets cut.

_Last groomed: 2026-09-21, after the graph-view render pass._

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
| Graph: tag nodes | Unresolved targets are graph nodes now (drawn hollow) and colour clusters by folder or tag; **tags themselves** as nodes you can link through is the part still outstanding |
| Index scaling | Full synchronous scan at workspace-open and per-event re-read; fine to ~thousands of notes, wants a background/incremental pass for huge vaults |

## Graph view (deferred from the 2026-09-21 render pass)

### `graph.rs` carries two subjects and wants splitting at the seam

The file is ~2,000 lines: the force simulation on one side, the
view-model rules on the other — radius, level of detail, dimming, the
picker, the dwell timer, the fit maths — and the tests for both. The
split was deliberately deferred so the render pass stayed reviewable as
one change rather than arriving mixed with a file move.

The seam is already visible in the file: `Simulation` and `layout` know
nothing about zoom, and everything below `node_radius` knows nothing
about forces. A `graph/sim.rs` and a `graph/view.rs` would fall out
along that line with no logic moved, which is exactly why it can wait —
and exactly why it should not wait indefinitely.

### The simulation still steps on the UI thread

`graph_tick` steps the layout inside `this.update`, on the foreground,
once every 16ms while the graph has motion in it. The render pass made
the frame much cheaper — the board is one canvas now instead of an
element per node — so on the vaults measured the step is no longer what
you feel.

If measurement ever shows step time dominating a frame again, the next
move is the background executor with a positions snapshot: step off
thread, hand the render a plain `Vec<(f32, f32)>`, and keep pins and
drags as messages into the simulation rather than mutations of it. Not
done now because it trades a measurable win for real complexity —
pointer interaction has to stay correct against a layout that is one
frame behind — and the measurement does not currently ask for it.

### The arrowhead detector cannot see the painter

`arrowheads_drawn` is derived from the `head_to`/`head_from` flags baked
into `edge_px`, not from what `paint_path` was actually called with. So
it catches the level-of-detail gate disappearing — the thing that was
regressed and fixed — but it would stay green if the paint closure
started ignoring the gate's output: delete the `if *head_to` guards
inside the closure and the test still passes.

Pinning it properly needs a paint-recording harness that GPUI does not
offer today: something that captures the primitives a frame actually
submitted so a test can count them. Left as is because the failure mode
is bounded — a silent performance regression at low zoom, never a wrong
picture — and a fake harness would cost more confidence than it bought.

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

### The emoji plugin reads a table alignment row as a shortcode

Opening `examples/vault/Guide/Tables.md` logs `supermd: inline render
failed (emoji): unknown shortcode :-----:` and raises a red error strip
over the status bar. The delimiter row of an aligned table (`:---:`,
`:-----:`) is valid CommonMark, and the emoji plugin's inline pass reads
the colons as shortcode delimiters.

So the shipped example vault greets a first-time user with an error on
one of the six guide pages. Found by right-clicking a table in the
running app while verifying #35.

Two candidate fixes: have the emoji plugin refuse a shortcode that is
all hyphens, or suppress inline replacement inside a table delimiter row
the way `spans.rs` already suppresses it inside frontmatter and code.

## Themes / flux

| Item | Notes |
| ---- | ----- |
| Wake-time preference | f.lux's "keep night mode until my morning" — stay warm past midnight |
| System location (opt-in) | Manual coordinates only today, by design; CoreLocation could be an explicit opt-in later |
| Per-theme flux pairing | One global light/dark pair today; themes could declare their own day/night partners |

### One row builder, so contrast can be enumerated instead of grepped

Tuning the themes for the page surface turned up the same defect four
times in a row, and each time it was found by a reviewer rather than by
a test: a colour token painted on a background nobody had measured it
against. `code_bg` on the page; `hover_bg` and `panel_bg` on the page;
`fg_muted` on `hover_bg` and `selected_bg`; `fg` on `selected_bg`. The
contrast tests now assert a **hand-maintained list of (ink, surface)
pairs**. Nothing makes that list complete, and nothing fails when a new
render site paints an existing token on a new surface — which is how the
list grew from one pair to sixteen, one review at a time.

Every violation so far has had one shape: **a selectable list row with a
muted-or-body-ink child.** The sidebar, the tab strip, the finder, the
palette, the search overlay, the install list and the `[[` completion
popup each hand-roll their own `.when(is_selected, |d| d.bg(...))` plus
a `.hover(|s| s.bg(t.hover_bg))`, and each then paints `fg` and
`fg_muted` children onto whichever of those it chose. `RowState`,
`RowStyle` and `sidebar_row_style` in `workspace.rs` are the bones of the
shared abstraction — one of the seven uses them.

The work: put all seven row builders behind one helper that owns the
`resting / hovered / keyboard-selected / active` background choice and
the ink slots that ride on it, then have the contrast test walk *that*
one place to enumerate the pairs rather than carrying a list someone has
to remember to extend. That closes the observed failure shape at a
fraction of the cost of the general fix.

The general fix — surface-typed colour handles, so painting ink on a
surface is one typed operation rather than two independent `.bg()` and
`.text_color()` calls — stays the fallback, and is only worth its cost
if a violation ever shows up somewhere that is *not* a selectable row.

### The theme picker previews without flux, and commits with it

`theme_picker_apply` previews by setting `ActiveTheme` directly, while
`theme_picker_confirm` goes through `ThemeState::resolve()`, which
applies flux's kelvin warming. With flux on at night, the theme you
arrow onto and the theme you get on Enter differ by the warm shift — the
preview is the cold theme, the result is the warmed one.

Invisible with flux off, which is the default, and invisible in daylight
even with it on. It surfaced while narrowing a different picker defect
(confirming a theme used to pin the appearance outright, which disabled
flux's night switch) and was deliberately left outside that fix's scope.

The fix is to preview through the same `resolve()` path the commit uses
rather than reaching for `ActiveTheme` directly — the same "one
function, two callers" shape 0.0.17 applied to `is_whole_line`,
`resolve_image` and the table style.

## Editor performance

### `fence_window` still scans the document prefix, and that is the cheaper trade

`fence_window` (`src/editor/lists.rs`) bounds the fence scan that
Enter-continuation and renumber run before they touch an ordered list.
It finds the first line in the whole document that *could* be a fence
delimiter and starts the window there, so a note whose first fence sits
above the list re-parses from that fence on every Enter — close to the
whole-document cost #45 set out to remove. The ticket's stated case,
prose then a list, is fixed, and the window never regresses past pre-fix
behaviour; this is the remaining tail.

Parked, not deferred for want of time: tightening it means reimplementing
fence parity outside `blocks.rs`, which owns fences in this codebase. The
win is narrower than the cost of two places deciding what a fence is —
the one piece of logic the architecture says must live in one place.
Worth revisiting only if fence parity ever becomes shareable, at which
point this falls out of that work rather than justifying it.

Worth recording that the windowing is strictly better than what it
replaced in one respect nobody was aiming at: on a document past
`MAX_STYLED_BYTES` (>1 MB) `blocks()` bails, so renumber used to rewrite
numbers *inside* fences up there. The window keeps fence detection alive
above that ceiling.

## Distribution & platform

| Item | Notes |
| ---- | ----- |
| Windows code signing | SmartScreen still shows one "unrecognized app" prompt |
| In-app auto-update | About (⌘/ → About) checks on demand and links out. Self-replacement is **three** implementations, not one, and on a `.deb` install the binary is root-owned in `/usr/bin` — the correct Linux answer is an apt repository, not self-update. Wants its own spec |
| Homebrew cask / winget | Cheap once naming is stable |
| Mac App Store rollout past TestFlight | The `mas` build (sandbox, Pulley host, no wasm grammars) is implemented — see `docs/superpowers/plans/2026-08-31-mac-app-store.md` and its spec for what the sandbox cost and why. What remains is release work, not engineering: external TestFlight (needs Beta App Review), full submission, and App Store Connect metadata. If App Review rejects the plugin system on 2.4.5(iv), turn off `install_ui::catalog_browsable()` and the `supermd://` handler under `mas` — Import Plugin… survives both |
| GraphQL highlighting in the App Store build | Grammar plugins need tree-sitter's wasmtime 24, which JITs. Static linking is blocked too: `tree-sitter-graphql` 0.2.1 emits grammar ABI 15 and tree-sitter 0.23 accepts 13-14. Revisit on a tree-sitter upgrade |

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
