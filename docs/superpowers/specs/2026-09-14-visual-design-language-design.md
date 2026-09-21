# SuperMD Visual Design Language — Design

**Status:** approved in brainstorming, 2026-09-14
**Target release:** 0.0.17 — "a proper desk"

## Goal

Give SuperMD a visual identity of its own, carried unchanged across macOS,
Windows and Linux. The app today is one flat plane: sidebar, document and
outline sit at near-identical values separated by hairlines, nothing casts a
shadow, tables are drawn as full grids, and the theme has no concept of
elevation. It reads as unfinished rather than minimal.

The organising idea: **the document is a page, resting on a continuous
ground.**

## Non-goals

- **Not a macOS-native look.** macOS 27 "Golden Gate" (GA 2026-09-14) refines
  Liquid Glass, but adopting it would mean three different-looking apps, since
  Windows and Linux have no equivalent. Decided against explicitly.
- **No translucency.** Not in 0.0.17. The capability exists — GPUI exposes
  `WindowBackgroundAppearance::Blurred` — and Zed ships that capability without
  ever enabling it. It stays available for a later, optional, per-platform
  garnish.
- **No new subsystems.** This release changes how things look and clears filed
  bugs. Where a fix restores parity — #39 makes reading-view checkboxes
  clickable, matching the editor, which already toggles them — that counts as
  a bug, not a feature. Direct manipulation (#50, #51, #52) is 0.0.18.

## Evidence behind the decisions

- **Apple HIG (fetched 2026-09):** *"Don't use Liquid Glass in the content
  layer"*; *"limit these effects to the most important functional elements."*
  Clear glass is for media-rich backgrounds, never text-heavy UI. Apple
  publishes almost no numbers — the only hard values are a 35% dimming layer
  and the concentric-radius formula (inner = outer − padding). The rule is the
  spec.
- **Zed's `ElevationIndex`** (same framework as ours): five named levels
  collapsing to three treatments. Persistent surfaces — editor, panels,
  sidebar — get **zero** shadow. Popovers get a two-layer shadow at 2–3px blur,
  3–12% opacity. Modals get four layers up to 12px.
- **Material Design 3:** tonal difference is the default separation mechanism;
  shadow is secondary, for busy backgrounds only. *"The fewer levels in your
  UI, the more power they have."*
- **Value stepping convention:** one adjacent step in whatever scale is in use
  (Radix, Obsidian's two-token system, Linear's opacity-derived elevation).
  Our current stepping is not wrong in magnitude — it is that no other cue is
  layered on top of it.
- **Warm palettes** read as intentional when the ink shares the background's
  temperature, and dated when warmth sits only in the background while text and
  icons stay cool.
- **Tables:** Notion ships "show vertical lines" off by default; Obsidian gives
  outer, header and interior borders separate tokens.

## 1. The depth model

Four surface roles. Every pixel belongs to exactly one.

| Role | What lives there | Treatment |
|---|---|---|
| **Ground** | the window — behind sidebar, tab strip, outline, status bar | darkest in dark mode, warmest in light; carries the palette's warmth |
| **Page** | the document, and only the document | brightest surface, rounded, one soft shadow — the single lifted thing at rest |
| **Floating** | hover previews, ⌘P finder, palette, context menus | two-layer shadow, ~3px blur, 3–12% opacity |
| **Modal** | dialogs, install flow, confirmations | four-layer shadow, up to ~12px blur |

Three rules:

1. **Only the page and things that float cast shadows.** A shadow on a static
   panel implies it can be picked up. The sidebar, outline and status bar are
   the ground; they never lift.
2. **Borders are the secondary cue.** Value difference separates. Borders
   survive only where surfaces of similar value genuinely meet — the page's
   edge, a table's outer boundary — and they get lighter than today.
3. **The ground is continuous.** Sidebar, tab strip and outline share one
   background with no dividers between them. This is what makes the page read
   as a sheet resting on something.

**Consequence:** the editor's background stops being the window background.
The document pane gains an inset, which changes the available measure — the
editor virtualizes one list item per logical line and projectors compute widget
widths against it. This is not a paint-only change.

The reading view and diff view are pages, sharing the document's surface and
inset, so switching between them does not change the geometry underneath the
reader. The graph is ground with controls floating above it — it is a canvas,
not a document.

## 2. Token taxonomy

Five families; only two add colours. Existing user themes must keep working
untouched.

**Surfaces.** `bg` keeps its name and becomes the ground. New: `page_bg`.
Themes that omit it derive it from `bg` — brighter in light, one step lighter
in dark. The exact lightness deltas are pinned in the implementation plan, not
here; the convention they must satisfy is *one adjacent step*, and the contrast
test in §6 is what holds them honest across the eight shipped themes.

**Borders.** `border` stays. New: `border_subtle`, derived from `border` at
reduced alpha.

**Shadows.** One new token, `shadow` (near-black; warm-shifted for light
themes). Geometry — layer count, blur, offset, opacity per tier — lives in
code, not TOML. Exposing twelve numbers to theme authors invites broken themes,
and shadow geometry is design, not palette.

**Radius.** A base radius in code, with Apple's concentric rule applied
(inner = outer − padding). Not themeable.

**Material.** Empty. Reserved for a future optional translucency.

Net: theme files grow by **two optional keys**. The eight shipped themes and
any user theme keep loading unchanged.

### The macro

`Theme::map_colors` currently enumerates all 42 fields by hand, and CLAUDE.md
warns that a colour missing from it is skipped by flux warming — the same
hand-maintained-second-list defect that `Surface::ALL` had before the
`surfaces!` macro removed it.

Declare the colour fields once via a macro that generates both the struct and
`map_colors`. A forgotten colour becomes a compile error instead of something a
reviewer must notice.

## 3. Palette and temperature

**Warm-shift the ink.** `fg`, `fg_strong`, `fg_muted` move off pure
black/neutral-grey to a low-saturation warm hue. Highest-leverage change in the
release: it affects every screen and costs nothing.

**Chrome icons go near-monochrome.** Sidebar and tab icons render in
`fg_muted`; the active file's icon takes the accent.

Note the mechanism: icons are *already* themed. `seti_tint`
(`workspace.rs:488`) maps Seti's palette onto the theme, and blue resolves to
`syntax.function`. The clash is that **chrome is being coloured with a
code-syntax palette** — saturated and cool by design, because it is tuned for
code legibility. `seti_tint` stays and gains a muted variant; the syntax
palette is not touched.

Full-colour Seti tints remain in the ⌘P finder and search results, where
telling file types apart quickly is the actual task.

**Accent gets one job.** Selection, focus ring, active tab, active sidebar row,
links, caret. Never decoration, never a large background fill. This is *more*
accent than today — currently it appears essentially once, on the caret — but
spent deliberately.

**Dark mode keeps the warmth.** Ground goes warm-neutral dark rather than
blue-grey, so light and dark read as the same app. `nord` and `gruvbox-dark`
will fight this and are allowed to: they are themes with their own temperature.
The defaults carry the identity.

**Flux interaction.** `flux.rs` already warms colours by time of day through
`map_colors`. Warm-shifted ink composes with it, but shipped themes need
checking at flux's extremes so late-evening warming does not push everything
to amber.

## 4. Component treatments

- **The page** — margin from the window edge, base radius, resting shadow,
  `page_bg` background.
- **Tab strip** — tabs on the ground; the active tab takes `page_bg` and meets
  the page with no seam, so it reads as attached to the document. Inactive tabs
  are ground with muted labels.
- **Sidebar rows** — active: accent-tinted background plus a 2px accent bar on
  the leading edge. Hover: a value step, no border. Gitignored dimming
  continues to work through `sidebar_row_color`, which is already a tested pure
  function.
- **Toolbar** — glyphs grouped (Apple's guidance: at most three logical
  groups), consistent hit targets, on the ground rather than in a container —
  a container would imply elevation.
- **Tables** — outer border and header rule keep weight; rows separate with a
  hairline; **interior vertical rules removed**.
- **Overlays** — the finder, palette, install dialog and hover preview
  currently each call `shadow_lg()`. They get re-pointed at the Floating and
  Modal tiers so there is one shadow vocabulary.
- **Code fences** — `code_bg` currently sits on the window background and will
  now sit on `page_bg`; contrast needs recomputing in all eight themes.

## 5. Scope

### In 0.0.17

**Visual redesign** — sections 1–4 above.

**Rendering correctness** (same files the redesign touches):
- #38 frontmatter misparsed as a Setext H2 — stop rendering YAML as a giant
  bold heading, keep it out of the outline. *Not* full frontmatter support,
  which is #52.
- #40 thematic breaks never drawn as a rule in the editor.
- #37 HTML blocks silently vanishing from the reading view — render as literal
  text so nothing is lost.

**User-visible bugs:** #33 delete-row caret in the delimiter · #34 silent
command refusals · #35 widget tables raise no context menu · #36 `.ignore`
files absent from the sidebar · #39 reading-view checkboxes inert.

**Watcher predicate split:** #31 — the last piece of 0.0.16's sidebar work.

**Window reopening:** #53 — added after Apple rejected 0.0.16 under Guideline 4.
Closing every window leaves the app running with no way back: New Window is
scoped to a window, so with none open it has nothing to dispatch to. Our own
0.0.16 whole-branch review found this, ranked it Medium, and parked it without
filing — which is how it shipped. It runs first in 0.0.17, ahead of the
remaining visual work, because it is independent of the redesign and the next
App Store submission must clear the guideline regardless of how that lands.

**Chores, batched into two dispatches:** #42 · #43 docs · #44 · #45 perf ·
#46 test gap · #47 CI App Store job · #48 CHANGELOG.

### Deferred to 0.0.18+

- Direct manipulation spine: #50 per-projector reveal policy → #51 editable
  tables → #52 frontmatter properly.
- Graph work: #26, #27.
- #49 footnotes, math, definition lists, admonitions.
- #41 MDX — **decision: support it properly, later.** The extension claim
  stays. #37's fix means block-level JSX renders as literal text instead of
  vanishing, so the worst symptom is mitigated in 0.0.17 while the real epic
  waits.

### Decisions recorded

- **#35 is fixed in 0.0.17**, not left for #51 to dissolve. Right-click on a
  widget table places the caret inside it and opens the menu. If #51 later
  keeps tables rendered while editing, this code is replaced, not wasted —
  caret placement is still the right behaviour for an unfocused table.
- **Tables are restyled now**, so #51 inherits the finished look rather than
  being styled twice.

## 6. Testing

Most of a visual change cannot be asserted. These parts can, and should be:

- **Token derivation** — `page_bg` from `bg` in light and dark;
  `border_subtle` from `border`. Pure functions.
- **The macro** — a forgotten colour is a compile error, not a review catch.
- **A contrast floor across all eight shipped themes** — load each and assert
  minimum contrast for body text on the page, muted text on the ground, and
  that page and ground are distinguishable. This catches "the derived value
  looks wrong in nord" mechanically.
- **Elevation assignment** — each surface role maps to its intended tier; a
  shadow on a static panel fails.

What no test covers is whether it looks right. That is a dev build and the
user's eyes — the route by which the table-discoverability problem surfaced in
0.0.16.

Both suites must pass (`cargo test` and
`cargo test --no-default-features --features mas`), `cargo build` must succeed —
it is not covered by the suites, and a test-only GPUI API reached production
code in 0.0.16 precisely because nobody ran it — and the 90% line coverage
floor holds.

## 7. Risks

- **The shipped themes were designed for a flat world.** Derived defaults keep
  *user* themes working; they do not spare us from hand-tuning `gruvbox-dark`,
  `nord` and `solarized-*`. Budget craft time.
- **The page inset changes editor measure**, touching virtualized line layout
  and projector widget widths. The heaviest task in the release.
- **Scale.** Roughly eighteen tasks — larger than 0.0.16's nine, though
  batching keeps the dispatch count near twelve. If it needs cutting, the
  chores group goes first: none of it is user-visible.
