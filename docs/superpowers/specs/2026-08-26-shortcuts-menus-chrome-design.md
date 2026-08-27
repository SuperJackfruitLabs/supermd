# Shortcuts, Menus & Chrome — Design

Picks up the reorganization candidate groomed into `docs/BACKLOG.md` on
2026-08-25. That audit found 120 keybindings, menus that had not grown
since ~v0.0.5, and almost no clickable chrome. Re-surveying the code
ahead of this design turned up a defect the audit missed, which changes
the shape of the work: **the menu structure exists twice, by hand, and
the two copies already disagree.**

## The drift problem

Five surfaces describe the same command set, each maintained
independently:

| # | Surface | Where |
| - | ------- | ----- |
| 1 | 126 keybindings | `main.rs::app_keybindings()` |
| 2 | macOS menu bar | `main.rs::app_menus()` — ~17 actions |
| 3 | Linux/Windows ☰ popover | `workspace.rs:3728` — ~8 actions |
| 4 | ⌘/ dialog | `workspace.rs::SHORTCUTS` |
| 5 | Docs | `docs/site/shortcuts.md` → generated `site/docs/` |
| 6 | Palette built-ins | `workspace.rs:1155-1200`, dispatched by string id at `1342-1452` |

Surface 6 is the one most easily missed: the command palette carries
built-in entries keyed by *string id* (`__graph`, `__flux`, `__install`,
`__format`, `__export:*`, `__template:*`) dispatched through a chain of
`if id == "…"` comparisons. **Graph View, Flux, and Install Plugins…
therefore have no action types at all** — which is precisely why they
appear in no menu and (for the first two) have no shortcut.

Surfaces 2 and 3 are separate hand-written lists, and the popover
carries roughly half what the menu bar does — despite the code comment
at `workspace.rs:2930` asserting it "carries the same actions." Linux
and Windows users see a materially smaller menu than macOS users.

The reader-keyboard-scroll fix (v0.0.12+, PR #10) demonstrated the cost
directly: six new bindings required four hand-edits across `main.rs`,
`SHORTCUTS`, `docs/site/shortcuts.md`, and a count assertion.

So this pass is not only "fix the menus." It is **derive every surface
from one table**, then fix the content.

## Goals

- One declaration per command; every surface is a projection of it.
- The macOS/non-macOS menu divergence becomes unrepresentable.
- An Edit menu exists, restoring macOS system integration (Emoji &
  Symbols, dictation, substitutions) that is currently absent.
- The shortcut map has a written scheme, so future bindings have a
  principled home before the ⌘⇧-letter space runs out.
- The three panels, file creation, and the diff view gain clickable
  affordances; they are keyboard-only today.
- An About dialog reports the running version and can check for a
  newer one.

## Non-goals

- **In-app self-update.** The About dialog checks and links out. Download
  -and-swap is deferred to its own spec: it is three per-platform
  implementations, and on a `.deb` install the binary is root-owned in
  `/usr/bin`, where the correct answer is an apt repository rather than
  self-replacement. `BACKLOG.md` is updated to record this.
- **Aggressive rebinding.** See "Shortcut scheme" — the map is mostly
  cross-app convention and is left alone deliberately.
- **Context menus.** No right-click machinery exists in the app; none is
  introduced (consistent with the file-ops design).
- **User-editable keymaps.** The table is compile-time. A JSON keymap is
  a separate feature.

## Architecture

### `src/commands.rs` — the table

A new pure module. One entry per user-facing command:

```rust
pub struct Command {
    pub id:      &'static str,            // stable; used by tests and docs
    pub label:   &'static str,            // menu and ⌘/ text
    pub keys:    &'static [&'static str], // empty = menu/palette only;
                                          // first is shown in menus and docs,
                                          // all are bound (⌘1 + ⌘B alias)
    pub context: Option<&'static str>,    // "Workspace" | "Editor" | "Reader" | …
    pub menu:    Option<(MenuId, u8)>,    // menu + group; group change emits a separator
    pub help:    Option<HelpSection>,     // ⌘/ dialog section
    pub action:  fn() -> Box<dyn Action>, // the only type-carrying field
}
```

`action` is the sole field carrying a concrete type. This works because
both GPUI entry points accept boxed actions:

- `KeyBinding::load(keystrokes, Box<dyn Action>, predicate, …)` is public
  (`gpui/src/keymap/binding.rs:48`), and `gpui::DummyKeyboardMapper` is
  reachable (`gpui.rs:89` → `platform.rs:72`).
- `MenuItem::Action { name, action: Box<dyn Action>, os_action }` is a
  public variant, so the `impl Action` helper can be bypassed.

Entries are authored through a `commands!` macro so each line reads as
one fact:

```rust
commands! {
    ToggleSidebar { label: "Toggle Sidebar", keys: ["cmd-1", "cmd-b"], ctx: None,
                    menu: (View, 1), help: General },
}
```

### The five derivations

Each is a pure function over `&[Command]`:

| Function | Feeds |
| -------- | ----- |
| `bindings()` | `app_keybindings()` in `main.rs` |
| `menus()` | `app_menus()` — grouped by `MenuId`, separator on group change |
| `popover_items()` | the ☰ list in `workspace.rs` |
| `help_sections()` | the ⌘/ dialog, replacing `SHORTCUTS` |
| `help_sections()` | `docs/site/shortcuts.md`, via the `build_docs` example |

The ☰ popover currently invokes `Workspace` methods through
`fn(&mut Self, &mut Window, &mut Context<Self>)` pointers. It moves to
dispatching `(cmd.action)()` via `window.dispatch_action`, so popover,
menu bar, and keystroke all traverse one path — a test of one covers all
three.

### Static commands vs dynamic palette entries

The table covers **fixed** commands. `__graph`, `__flux`, and `__install`
become real actions and move into it, replacing three arms of the
string-id dispatch chain.

The parameterised entries — `__export:<id>`, `__template:<id>`, and
`__format` — are generated at runtime from installed plugins and cannot
live in a static table. The palette keeps its dynamic path for those and
merges the two sources. This boundary is the design's one genuine
subtlety: **static commands are declared, plugin commands are
discovered**, and the palette is the only surface showing both.

### Platform-resolved placement

"About SuperMD" belongs in the app menu on macOS (first item, above
Services) and in Help elsewhere. `MenuId::About` resolves per-platform
inside `platform.rs`, which CLAUDE.md designates the single home for
per-OS decisions.

## Menu structure

| Menu | Contents |
| ---- | -------- |
| **SuperMD** (macOS) | About SuperMD · — · Services · — · Quit |
| **File** | New File ⌘N · Open… ⌘O · Open Recent ▸ · — · Save Now ⌘S · — · Close Tab ⌘W |
| **Edit** | Undo ⌘Z · Redo ⌘⇧Z · — · Cut ⌘X · Copy ⌘C · Paste ⌘V · Select All ⌘A · — · Find in File ⌘F · Find Next ⌘G · Find Previous ⌘⇧G · — · *(macOS: Emoji & Symbols, Start Dictation)* |
| **Format** | Bold ⌘B · Italic ⌘I · Code · Strikethrough · Link · — · Heading · Quote |
| **View** | Toggle Edit/Preview ⌘E · Show Changes ⌘⇧D · — · Sidebar ⌘1 · Outline ⌘2 · Knowledge ⌘3 · — · Focus Mode ⌃⌘F · Flux ⌃⌘N · — · Zoom In/Out/Reset · — · Theme… ⌘T |
| **Go** | Go to File… ⌘P · Search in Workspace… ⌘⇧F · Graph View ⌘⇧G · — · Next Tab · Previous Tab · — · Follow Link ⌘⏎ |
| **Tools** | Command Palette… ⌘⇧P · Install Plugins… · Open Plugins Folder · Reload Plugins |
| **Help** | About SuperMD *(non-macOS)* · Keyboard Shortcuts ⌘/ |

Notes:

- **Edit is the biggest gap closed.** Beyond discoverability, macOS hangs
  Emoji & Symbols, dictation, and substitutions off a conventional Edit
  menu; without one those are unavailable. `MenuItem::os_submenu` is
  already used for Services and is the mechanism.
- **Format requires ~5 new actions.** Bold and Italic exist. Code,
  Strikethrough, Link, Heading, and Quote are implemented in
  `formatting.rs` and reachable only from the selection toolbar; they
  need action types, not new logic.
- **Graph View, Flux, and Install Plugins… gain actions.** All three are
  string-keyed palette entries today (`__graph`, `__flux`, `__install`),
  which is why they appear in no menu and the first two have no shortcut.
- **Tools is split out** rather than folded into View, because the audit
  identified View as a grab-bag mixing navigation, plugin management, and
  view toggles.
- Zoom is image-tab zoom, not app zoom. View is the least-bad home.

## Shortcut scheme

The audit's complaint was that no scheme was written down, not that the
bindings were wrong. Most of the map is cross-app convention users arrive
with — ⌘B sidebar (VS Code, Zed, Obsidian), ⌘P go-to-file, ⌘F, ⌘N, ⌘O,
⌘S, ⌘W, ⌘Z, ⌘⇧F. Rebinding those would make SuperMD harder for the people
most likely to try it. **The scheme is therefore documented, and only what
it actively indicts is changed.**

| Tier | Scope | Examples |
| ---- | ----- | -------- |
| ⌘ + letter | the file and the text | ⌘N ⌘O ⌘S ⌘W ⌘F ⌘Z ⌘B ⌘I ⌘P ⌘E ⌘T |
| ⌘⇧ + letter | the workspace | ⌘⇧F search · ⌘⇧P palette · ⌘⇧D changes · ⌘⇧G graph |
| ⌃⌘ + letter | modes / environment | ⌃⌘F focus · ⌃⌘N flux (night) |
| ⌘ + digit | panels | ⌘1 sidebar · ⌘2 outline · ⌘3 knowledge |
| context-scoped | live only in their surface | Sidebar: F2 · ⌘⌫ · ⌘⇧M |

**Changes:**

1. **Panels move to ⌘1/⌘2/⌘3 as toggles.** This is the change that pays:
   it frees **⌘⇧O and ⌘⇧K** from the saturated ⌘⇧-letter space and gives
   the three panels one rule instead of three unrelated chords. ⌘1
   currently *focuses* the sidebar; toggle is the more useful verb, and
   focus-sidebar is retained on the palette.
2. **⌘B retained as a sidebar alias** — the convention is too strong to
   drop.
3. **⌘⇧G for Graph View** — its first binding.
4. **⌃⌘N for flux**, completing the modes tier.

⌘⇧M (Move) stays: it is mnemonic and Sidebar-scoped, so it consumes no
global space. The audit was wrong to cite it as disorder.

### The ⌘B overload is load-bearing

⌘B binds `ToggleSidebar` globally *and* `editor::ToggleBold` in the
`Editor` context; the editor handler propagates a cursor-only press so the
sidebar toggle still fires (`main.rs:269`). The duplicate-detection test
must reject collisions **within** a context while permitting deliberate
ones **across** contexts.

## About dialog

Reuses `update.rs` wholesale — `is_newer()`, `parse_tag()`,
`fetch_latest_tag()` all exist and are tested.

- Running version from `env!("CARGO_PKG_VERSION")`.
- "Check for Updates" runs `fetch_latest_tag()` on the background
  executor; failure is silent, matching the launch check.
- A "Download <version>…" button appears only when `is_newer()` is true,
  opening `update::RELEASES_URL`.
- Rendered as an overlay in the existing family (`ThemePicker`,
  `Shortcuts`), with key context `About` and Escape to dismiss.

This is a real gain over today, where a newer version is discoverable
only at launch, via a pill that disappears on dismissal.

## Chrome

### UI icon set (new infrastructure)

No UI icons exist. `assets/icons/` holds 169 Seti **file-type** icons,
served only under the `icons/seti/` prefix from `seti.rs`, which CLAUDE.md
marks GENERATED — do not edit. Existing chrome uses text glyphs (`☰`, `×`).

Add ~6 SVGs in `assets/icons/ui/`, a **hand-written** `src/ui_icons.rs`
with an `include_bytes!` table, and a second prefix in
`AssetSource::load`. This mirrors the `seti_tests.rs` convention: hand
-written code beside generated code, so regeneration cannot destroy it.

### The four affordances

1. **Panel toggles** — three icon buttons in the 34px titlebar, in a
   right-hand cluster after the `flex_1` drag spacer and before the update
   pill. Active at `fg_strong`, inactive at `fg_muted`, matching tabs. The
   drag region must survive: buttons sit inside the cluster, never
   replacing the spacer.
2. **Sidebar `+`** — in the sidebar header, dispatching `SidebarNewFile`;
   a small popover offers New Folder. ⌘N/⌘⇧N are keyboard-only today.
3. **Show Changes button** — titlebar, shown only when the active file
   differs from HEAD, reusing the sidebar's git-status dot data
   (`refresh_git_status`, `workspace.rs:5134`) rather than a second git
   query.
4. **Status bar** — *replaces* the transient pill at `workspace.rs:4008`
   with a persistent 22px bottom strip: plugin widget text left, flux ☀
   and graph toggles right. Hidden in focus mode.

**Risk on (4):** a persistent status bar is a visual-density change to
every screen, not merely a button. It is sequenced last and is cleanly
droppable — the other three stand without it.

## Testing

`commands.rs` is pure, with no GPUI dependency, matching the core rule
that editing/policy logic is tested Rust and the shell stays thin.

- Every command resolves to a unique `(key, context)` pair; collisions
  within a context fail, across contexts are permitted (see ⌘B).
- Every command reaches at least one discoverable surface — menu, ⌘/, or
  palette. **This is the assertion that would have caught the current
  gaps**, and it replaces the `assert_eq!(bindings.len(), 126)` change
  -detector with something that tests a property rather than a number.
- Every keystroke parses and binds (retained from
  `every_keybinding_parses_and_binds`), including alias keys.
- Every static palette built-in resolves to a table entry, and the
  dynamic plugin entries still merge in alongside them.
- `menus()` and `popover_items()` derive from the same grouping —
  asserted directly, so the macOS/Linux divergence cannot recur.
- Menu groups emit separators only between groups, never leading or
  trailing.
- About: version string renders; `is_newer()` gates the download button.
- Chrome: panel toggles reflect and mutate panel state; the `+` button
  dispatches; Show Changes appears only for a modified file; the status
  bar hides in focus mode.

Coverage must hold the 90% floor (currently 94.4%).

## Migration

- `app_keybindings()` and `app_menus()` in `main.rs` shrink to calls into
  `commands`.
- `SHORTCUTS` in `workspace.rs` is deleted.
- The ☰ popover's hand-written item list is replaced by iteration.
- `docs/site/shortcuts.md` becomes generated; `site/docs/` regenerated via
  `cargo run --example build_docs`.

Net effect: `main.rs` and `workspace.rs` both shrink, and the two largest
files in the repo lose responsibility rather than gain it.

## Sequencing

Each stage is independently shippable:

1. `commands.rs` + derivations; surfaces reproduce today's behaviour
   exactly. Pure refactor, no user-visible change — the safest possible
   first step, and it makes every later stage a one-line table edit.
2. Menu restructure (Edit, Format, View/Go/Tools split), the ~5 new
   formatting actions, and the conversion of `__graph`, `__flux`, and
   `__install` from string ids into actions.
3. Shortcut scheme: the four rebinds, plus the written scheme in
   `docs/site/shortcuts.md`.
4. About dialog.
5. Chrome: UI icon set, panel toggles, sidebar `+`, Show Changes.
6. Status bar (droppable).
