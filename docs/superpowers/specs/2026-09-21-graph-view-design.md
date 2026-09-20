# Workspace Graph: a view you can read, and stay in

**Status:** approved design, ready for an implementation plan
**Date:** 2026-09-21

## The problem

Two complaints, one view.

**It is not smooth.** A ~2,500-node vault stutters. The layout is not the
cause: `graph.rs` already carries a Barnes-Hut quadtree (`graph.rs:167`,
`THETA = 0.9`, `BARNES_HUT_THRESHOLD = 64`), repulsion is O(n log n), and
the simulation provably settles. The cause is `render_graph`
(`workspace.rs:5159`), which rebuilds the entire scene every frame:

- A label element is built for **every** node whether or not it is
  visible (`workspace.rs:5361-5369`). `label_opacity` returns `0.0` below
  zoom 0.55 (`graph.rs:904`), and the whole-vault view sits well below
  that — so ~2,500 text elements are shaped and laid out per frame at
  zero opacity.
- A `String` is allocated per node per frame for that invisible label
  (`workspace.rs:5258-5262`).
- `group_keys` clones every node's folder/tag, sorts and dedups, every
  frame (`workspace.rs:5189-5201`), for data that changes only when the
  index or the colour mode changes.
- Four boxed listener closures are allocated per node per frame
  (`workspace.rs:5312-5378`).
- `edge_px` is rebuilt every frame (`workspace.rs:5209-5221`), and each
  edge paints two or three tessellated paths (`workspace.rs:5226-5248`).
- Nothing is culled to the viewport.
- `sim.step()` runs inside `this.update` (`workspace.rs:4951`), so
  physics and painting share one 16ms budget.

**It switches context.** Clicking a node calls `open_graph_node`, whose
second statement is `self.graph = None` (`workspace.rs:5128`). The graph
is destroyed: layout, pan, zoom and filter all go, and reopening
restarts the simulation from `alpha = 1.0`. Hovering does nothing beyond
lighting the neighbourhood, and at whole-vault zoom nothing has a name,
so there is no way to learn what a dot is without committing to opening
it.

## Goals

1. A 2,500-node graph holds 60fps while the layout settles, with every
   node still on screen.
2. You can tell what a node is without leaving the graph.
3. Opening a note does not destroy where you were.

## Non-goals

Decided against, for this change:

- **Hiding or collapsing orphans.** They stay visible; they dim.
- **Arrow-key navigation between neighbours**, multi-select, and editing
  inside the peek panel. Each is reasonable later; none is needed here,
  and each widens the surface under test.
- **Splitting `graph.rs`.** It is 1,716 lines and this adds ~300 more.
  The force code, the view-model code and their tests read as one
  pipeline, and splitting mid-change would make the diff much harder to
  review. Note it in `docs/BACKLOG.md` instead.
- **Moving the simulation off the UI thread.** Worth doing if
  measurement still shows contention after the render work; the render
  work is the larger and safer win, and this design does not depend on
  it.

## Architecture

The codebase rule is that editing logic is pure Rust under test and the
GPUI shell stays thin. `render_graph` currently breaks it: ~250 lines of
geometry, colour policy, string building and event wiring inside one
render function, which is why it is hard to make fast. The decisions
move out into `graph.rs`, all testable without a window.

### `Picker` — position to node

A spatial index over node positions. `pick(x, y) -> Option<usize>`
returns the topmost node whose radius contains the point, matching what
GPUI's per-node hit-boxes resolve today (later siblings win, so the
search runs back to front). Rebuilt when positions change, which the
simulation already reports through `settled()`.

This is what allows 2,500 interactive hit-boxes to become one.

### `Lod` — what to draw at this zoom

Derived from zoom alone:

| Zoom | Edges | Arrowheads | Labels |
| --- | --- | --- | --- |
| < 0.35 | lines | no | none |
| 0.35 – 0.55 | lines | no | none |
| 0.55 – 0.85 | lines | yes | fading in |
| > 0.85 | lines | yes | full |

The arrowhead thresholds matter most: an arrowhead is one or two extra
paths per edge and is sub-pixel below ~0.5 zoom, which is exactly the
view where nothing can be culled because everything is genuinely on
screen. Label thresholds keep the existing constants
(`LABEL_FADE_START = 0.55`, `LABEL_FULL = 0.85`, `graph.rs:900-901`) so
the fade behaviour people already have does not change.

### `Dimming` — orphans recede, nothing disappears

`degree == 0` renders at reduced alpha. Pure, so the resulting ink goes
through the same contrast checking as every other colour in the app
rather than being judged by eye.

Rationale: repulsion is scaled by `BARNES_HUT_THRESHOLD / n`
(`graph.rs:527-529`), so at 2,500 nodes it runs at ~2.6% strength. Nodes
with no edges feel only repulsion and centring and settle into a uniform
annulus occupying roughly half the board's radius — the ring visible in
the reported screenshot. Dimming lets the structure read without
removing anything.

### `Hover` — a dwell state machine over injected time

`hover(ix, now) -> Idle | Lit(ix) | Carded(ix)`. Instant transition to
`Lit`; `Carded` at 400ms of dwell on the same node. Time is injected,
following the precedent `autosave.rs` set for policy as a pure state
machine, so the delay is testable without a window or a sleep.

### `Peek` — which node is open in the panel, and whether it is pinned

Small enough to be a struct and a couple of transitions, but it belongs
with the others so the shell holds no policy.

### What the shell keeps

`workspace.rs` keeps painting into the canvas, running timers, and
assembling the peek panel, the hover card and the revealed labels as
elements.

## The renderer

**One canvas replaces ~5,000 elements.** It paints in z-order: edges,
then dots. Everything that must be an element — hover card, revealed
labels, peek panel — sits above it. The graph view goes from roughly
5,000 elements to roughly a dozen.

**Dots become quads.** `window.paint_quad` (`vendor/gpui/src/window.rs:2839`)
with `corner_radii` = half the box (`quad(...)`, `window.rs:5059`) is a
filled circle that never reaches the path tessellator. This is the
largest single change: 2,500 tessellated circular paths per frame become
2,500 quad instances, the case a GPU renderer is built for.

**Caching and hoisting:**

- `group_keys` moves into the graph state, recomputed when the index or
  the colour mode changes.
- Node name strings are built only for labels actually drawn.
- Edge geometry is cached in world space and transformed on paint,
  rather than `edge_px` being rebuilt per frame.

**Culling** trims to the visible world rect. At the fit-to-window view
this buys nothing — everything is genuinely on screen, which is the
accepted cost of keeping every node visible — but it bounds label work
when zoomed in, which is when labels switch on.

**Interaction moves to the container.** One set of listeners on the board
div — move, down, up, scroll — with `Picker` resolving position to a
node. Node dragging keeps working: mouse-down picks, and the existing
`hold_warm` path is untouched.

## Interaction

### Hover

Two stages. Instantly, the hovered node and its neighbours reveal their
labels — `state.neighbourhood(ix)` already computes that set and is
already used to light edges; it is simply not used for labels today. At
any zoom, hovering gives you names.

At 400ms dwell, a card appears near the cursor with title, a ~3-line
excerpt and out/in counts. It never takes the mouse, and flips side when
it would leave the window.

**Loading the excerpt** follows the editor's existing link preview, which
reads the file synchronously on hover (`editor/mod.rs:2868`), using the
existing `preview::excerpt` and `preview::title_of` (`preview.rs:26`,
`preview.rs:38`). A second mechanism is not worth inventing: the 400ms
dwell gate bounds this to a couple of small reads per second, served
from page cache. A cache of the last ~32 previews, invalidated by the
existing fs-event path, is cheap insurance. If measurement shows this
hurting, moving it to the background executor is a contained follow-up.

### Click: the peek panel

Clicking a node opens a ~360px panel on the right. The board's viewport
**shrinks** to the remaining width rather than the panel covering the
board, so nothing clickable hides behind it; `fit_to` accounts for the
narrower viewport.

The panel shows title, a longer excerpt, out/in counts, and the backlink
list from the knowledge index. Clicking a backlink re-peeks that note and
centres the graph on it, so the link structure can be walked without
leaving the view.

### Keys

- `Esc` closes the peek panel; the graph stays.
- `Esc` again closes the graph.
- `Enter`, or the panel's Open button, opens the note as a tab and closes
  the graph — an explicit choice to go and read it.

### The state fix underneath

`open_graph_node` stops setting `self.graph = None`
(`workspace.rs:5128`). Nodes, edges, layout, pan, zoom and filter are
cached on the workspace, so committing to a note and pressing ⌘G returns
the exact view, with no re-simulation and no alpha reheat.

Ghost nodes keep their current behaviour: clicking one still offers to
create the file, through `creatable_note_path` and the same containment
check the editor applies (`workspace.rs:5143`).

## Error handling

- **A previewed file that cannot be read** (deleted between index and
  hover) shows the node's name and "could not be read" in card and panel.
  No error strip: hovering is not a command the user issued.
- **A ghost node** has no file. Card and panel say so and offer the
  create action rather than an excerpt.
- **An empty graph** (no notes, or a filter matching nothing) keeps the
  existing status strip behaviour; `Picker` returns `None` and every
  interaction is a no-op.

## Testing

Pure units, tested inline as the codebase does:

- `Picker`: returns the same node a hit-box would for points inside,
  near-miss points outside every radius return `None`, overlapping nodes
  resolve to the topmost, and radius scales with zoom the way
  `node_r` does.
- `Lod`: each threshold, and both sides of each boundary.
- `Dimming`: an orphan is dimmer than a linked node; the result clears
  the contrast floor against the graph background.
- `Hover`: no card before 400ms, card at 400ms, moving to another node
  restarts the dwell, leaving resets to `Idle`.
- `Peek`: open, swap by clicking another node, pin, close; `Esc`
  ordering.

Shell-level, with the GPUI test context:

- Clicking a node no longer clears `self.graph`.
- ⌘G after opening a note restores pan, zoom and filter, and does not
  reheat alpha.
- No label element is constructed when `label_opacity == 0` — the
  regression that motivated this work.
- `group_keys` is computed once per change, not per render.

**Both suites must pass** (`cargo test --bin supermd` and
`--no-default-features --features mas`), and `cargo build --bin supermd`
must be run explicitly — it is not covered by the suites.

## Measurement

Structural tests cannot prove "smooth". Before and after, on the real
~2,500-node vault, with the frame ticker running:

- time for one `render_graph` pass at the fit-to-window view,
- time for one `sim.step()` at the same size,
- element count in the graph view.

The numbers go in the implementation report. This step needs a release
build, and therefore the external SSD mounted (see
`ssd-build-location`), or roughly 8GB free on the internal disk.

## Risks

- **Hit-testing must match what GPUI did**, or clicks land on the wrong
  note — the one change here a user would feel immediately as a bug.
  `Picker`'s tests carry that weight, including the topmost-wins rule.
- **Synchronous preview reads on the UI thread** are inherited from the
  editor, not introduced, but the graph can hover far more often than a
  document can. The dwell gate and the cache bound it; measurement
  decides whether it needs to move.
- **The canvas owns z-order**, so a painting-order mistake shows as
  edges over dots. Cheap to see, cheap to fix, worth one explicit test of
  paint order.
