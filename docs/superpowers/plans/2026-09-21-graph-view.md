# Workspace Graph Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a 2,500-node graph hold 60fps with every node still on screen, let you learn what a node is without leaving the view, and stop clicking a node from destroying the graph.

**Architecture:** Decisions move out of `render_graph` into pure units in `graph.rs` (`Lod`, `orphan_dim`, `Picker`, `Hover`), tested without a window. Nodes stop being GPUI elements and become quads painted into the canvas the edges already use, with `Picker` resolving the mouse to a node. Hover reveals names and then a card; clicking peeks into a panel beside a graph that keeps its layout, pan, zoom and filter.

**Tech Stack:** Rust, GPUI 0.2.2 (vendored at `vendor/gpui`), the existing Barnes-Hut layout in `src/graph.rs`.

**Spec:** `docs/superpowers/specs/2026-09-21-graph-view-design.md`

## Global Constraints

- **Editing logic is pure Rust under test; the GPUI shell stays thin.** Pure here means no element construction — value types like `Hsla` are fine, as `src/elevation.rs` already establishes.
- **Byte offsets into the ropey rope** are the universal currency for every buffer position. Graph coordinates are a separate space: node `x`/`y` are in the unit square (0..1), and board pixels are `pan + n.x * 900.0 * zoom + 60.0`.
- **Both suites must pass:** `cargo test --bin supermd` and `cargo test --bin supermd --no-default-features --features mas`.
- **`cargo build --bin supermd` must succeed.** It is NOT covered by the suites — a test-only GPUI API reached production code in 0.0.16, compiled under `cargo test`, and broke `cargo build`. Run it explicitly and read its warnings. The documented baseline is 48 warnings; none should be in code you touch.
- **CI enforces a 90% line coverage floor.** New production lines need covering tests.
- **Tests live inline** as `#[cfg(test)]` modules next to the code they cover.
- **Tests must compile on macOS, Linux and Windows.** Anything using `std::os::unix` needs `#[cfg(unix)]`.
- **Any test touching `HOME` must go through `workspace::tests::temp_home()`**, which holds `HOME_LOCK`.
- **A new `KeyBinding` is declared in `commands.rs`**, never by hand in `main.rs`. Then `UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table` and `cargo run --example build_docs`.
- **The user docs are generated:** `docs/site/*.md` are sources; `cargo run --example build_docs` renders them into `site/docs/` (committed). Editing a source without committing the regenerated output is a defect.
- **Do not split `graph.rs`.** It is 1,716 lines and this adds ~300 more. Ruled out deliberately in the spec; Task 10 files it in BACKLOG instead.
- **Do not move the simulation off the UI thread.** Out of scope; Task 10 records the measurement that would justify it later.
- **Known flake, do not chase:** `editor::tests::scrollbar_drag_scrubs_through_the_document`.
- **Builds go to an external SSD.** `.cargo/config.toml` may set `target-dir = "/Volumes/supermd-build/target"`. Verify it is mounted (`df -h /Volumes/supermd-build`) before building; never `cargo clean`; if space runs low, `rm -rf /Volumes/supermd-build/target/debug/incremental` with no build running. Long silent cargo runs get agents killed by a 600s watchdog — **run builds in the background and poll**.

## File Structure

**Modified:**
- `src/graph.rs` — gains `node_radius`, `Lod`/`lod`, `ORPHAN_ALPHA`/`orphan_dim`, `Picker`, `Hover`/`Hovering`. All pure, all tested inline. No element construction.
- `src/workspace.rs` — `render_graph` rewritten to paint into one canvas; `GraphViewState` gains cached `group_keys`, edge geometry, `hover`, `peek`; `open_graph_node` stops destroying the graph; Esc ordering; the hover card and peek panel.
- `docs/site/graph.md` + regenerated `site/docs/` — the interaction model, documented.
- `docs/BACKLOG.md` — the `graph.rs` size note, and the physics-on-UI-thread note.

**No new files.** Everything belongs to an existing responsibility: policy in `graph.rs`, presentation in `workspace.rs`.

---

### Task 1: One radius rule, and a level-of-detail rule

**Files:**
- Modify: `src/graph.rs` (add near `label_opacity`, `graph.rs:904`)
- Test: inline in `src/graph.rs`

**Interfaces:**
- Produces: `pub fn node_radius(degree: usize, zoom: f32) -> f32`, `pub struct Lod { pub arrowheads: bool, pub labels: bool }`, `pub fn lod(zoom: f32) -> Lod`, `pub const ARROWHEAD_MIN_ZOOM: f32`.
- Consumed by Tasks 3, 5, 6.

The radius expression `(5.0 + (degree as f32).sqrt() * 3.0) * zoom.sqrt()` is currently written **twice** in `render_graph` (`workspace.rs:5205-5207` as `node_r`, and `workspace.rs:5257` inline). Task 3's `Picker` will need the same number: if the painter and the hit-tester ever disagree, clicks land on the wrong note. One function, three callers.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The painter and the hit-tester must agree to the float. This
    /// expression used to be written twice in `render_graph`, which is
    /// exactly the arrangement that drifts.
    #[test]
    fn radius_grows_with_degree_and_zoom() {
        let base = node_radius(0, 1.0);
        assert!((base - 5.0).abs() < 1e-4, "an orphan is the bare dot: {base}");
        assert!(node_radius(9, 1.0) > node_radius(1, 1.0), "degree widens it");
        // Zoom scales by its square root, so a graph zoomed 4x has dots
        // twice the size rather than four times -- the board gets
        // denser as you zoom out without the dots vanishing.
        let (near, far) = (node_radius(4, 4.0), node_radius(4, 1.0));
        assert!((near / far - 2.0).abs() < 1e-3, "{near} vs {far}");
    }

    /// Arrowheads are one or two extra tessellated paths per edge and
    /// are sub-pixel below half zoom -- which is the whole-vault view,
    /// where nothing can be culled because everything is on screen.
    #[test]
    fn arrowheads_and_labels_switch_on_with_zoom() {
        assert!(!lod(0.3).arrowheads, "whole vault: lines only");
        assert!(!lod(0.3).labels);
        assert!(lod(0.9).arrowheads, "close in: direction is readable");
        assert!(lod(0.9).labels);
        // The boundary is the same one labels already used, so the two
        // appear together rather than at two unexplained zooms.
        assert_eq!(ARROWHEAD_MIN_ZOOM, LABEL_FADE_START);
        assert!(lod(ARROWHEAD_MIN_ZOOM).arrowheads, "inclusive at the edge");
    }
```

- [ ] **Step 2: Run them and watch them fail**

```sh
cargo test --bin supermd radius_grows_with_degree arrowheads_and_labels_switch
```

Expected: FAIL to compile — `node_radius`, `lod`, `Lod`, `ARROWHEAD_MIN_ZOOM` do not exist.

- [ ] **Step 3: Implement**

```rust
/// A node's drawn radius in board pixels. The painter, the hit-tester
/// and the arrowhead inset all read this one function: a hit-test that
/// disagrees with the paint by a pixel opens the wrong note.
pub fn node_radius(degree: usize, zoom: f32) -> f32 {
    (5.0 + (degree as f32).sqrt() * 3.0) * zoom.sqrt()
}

/// Below this, an arrowhead is sub-pixel and costs a tessellated path
/// per edge for nothing. Deliberately the same threshold labels use, so
/// detail arrives all at once instead of in two unexplained stages.
pub const ARROWHEAD_MIN_ZOOM: f32 = LABEL_FADE_START;

/// What the renderer draws at a given zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lod {
    pub arrowheads: bool,
    pub labels: bool,
}

pub fn lod(zoom: f32) -> Lod {
    Lod {
        arrowheads: zoom >= ARROWHEAD_MIN_ZOOM,
        labels: label_opacity(zoom) > 0.0,
    }
}
```

- [ ] **Step 4: Run the tests**

```sh
cargo test --bin supermd radius_grows_with_degree arrowheads_and_labels_switch
cargo test --bin supermd graph::
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/graph.rs
git commit -m "refactor: one radius rule, and a level-of-detail rule

The radius expression was written twice inside render_graph, and the
hit-tester in the next task needs the same number -- a painter and a
hit-tester that disagree open the wrong note.

lod() answers what a zoom is worth drawing. An arrowhead is one or two
extra paths per edge and sub-pixel below half zoom, which is exactly
the whole-vault view where nothing can be culled."
```

---

### Task 2: Orphans recede

**Files:**
- Modify: `src/graph.rs`
- Test: inline in `src/graph.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub const ORPHAN_ALPHA: f32`, `pub fn orphan_dim(color: Hsla, degree: usize) -> Hsla`.
- Consumed by Task 5.

Repulsion is scaled by `BARNES_HUT_THRESHOLD / n` (`graph.rs:527-529`), so at 2,500 nodes it runs at ~2.6% strength. Nodes with no edges feel only repulsion and centring, so they settle into a uniform ring taking roughly half the board's radius — the structure worth reading is squeezed into the middle. They stay visible; they stop competing.

- [ ] **Step 1: Write the failing test**

```rust
    /// An unlinked note is still a note: it shows, dimmed, rather than
    /// being hidden behind a toggle nobody finds. Hue and lightness are
    /// untouched so a dimmed orphan still reads as its colour group.
    #[test]
    fn an_orphan_is_dimmer_but_still_itself() {
        let c = Hsla { h: 0.5, s: 0.6, l: 0.6, a: 1.0 };
        let linked = orphan_dim(c, 3);
        let orphan = orphan_dim(c, 0);
        assert_eq!(linked, c, "a linked node is untouched");
        assert!(orphan.a < c.a, "the orphan recedes");
        assert_eq!((orphan.h, orphan.s, orphan.l), (c.h, c.s, c.l), "only alpha moves");
        // Dim enough to recede, not so dim it reads as absent.
        assert!(orphan.a > 0.3, "still visible: {}", orphan.a);
    }

    /// Dimming multiplies whatever alpha the caller already chose --
    /// the filter fades non-matching nodes to 0.25, and an orphan that
    /// also fails the filter must not come back brighter than a linked
    /// one that failed it.
    #[test]
    fn dimming_composes_with_an_already_faded_colour() {
        let faded = Hsla { h: 0.5, s: 0.6, l: 0.6, a: 0.25 };
        assert!(orphan_dim(faded, 0).a < faded.a);
    }
```

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd an_orphan_is_dimmer dimming_composes
```

Expected: FAIL to compile — `orphan_dim` does not exist.

- [ ] **Step 3: Implement**

```rust
/// How much of its alpha an unlinked note keeps.
pub const ORPHAN_ALPHA: f32 = 0.45;

/// An unlinked note recedes rather than disappearing. Multiplying
/// rather than assigning keeps this composable with the filter's own
/// fade: an orphan that also fails the filter must not end up brighter
/// than a linked node that failed it.
pub fn orphan_dim(color: Hsla, degree: usize) -> Hsla {
    if degree > 0 {
        return color;
    }
    Hsla { a: color.a * ORPHAN_ALPHA, ..color }
}
```

Add `use gpui::Hsla;` to the imports at the top of `src/graph.rs` if it is not already there — `elevation.rs` sets the precedent that value types are fine in a pure module.

- [ ] **Step 4: Run the tests**

```sh
cargo test --bin supermd an_orphan_is_dimmer dimming_composes
cargo test --bin supermd graph::
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/graph.rs
git commit -m "feat: an unlinked note recedes instead of competing

Repulsion is scaled by BARNES_HUT_THRESHOLD / n, so at a few thousand
nodes orphans settle into a uniform ring that takes half the board and
squeezes the structure worth reading into the middle. They keep their
colour and their place; they give up some alpha.

Multiplying rather than assigning keeps this composable with the
filter's fade."
```

---

### Task 3: `Picker` — the mouse finds a node without 2,500 hit-boxes

**Files:**
- Modify: `src/graph.rs`
- Test: inline in `src/graph.rs`

**Interfaces:**
- Consumes: `node_radius` (Task 1).
- Produces: `pub struct Picker`, `Picker::build(dots: Vec<(f32, f32, f32)>) -> Picker`, `Picker::pick(&self, x: f32, y: f32) -> Option<usize>`, `Picker::len(&self) -> usize`.
- Consumed by Task 5.

This is the load-bearing correctness change in the whole plan: it replaces GPUI's per-node hit-boxes. GPUI paints later siblings on top, so where dots overlap the **last** one wins — the search runs back to front to match.

A linear scan is deliberate. At 2,500 nodes it is 2,500 float comparisons per mouse-move, which is microseconds; a spatial index would be more code, more state to invalidate, and no measurable gain at this size. Revisit only if the node count grows by an order of magnitude.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The dot you click is the dot you get. Board pixels in, node
    /// index out.
    #[test]
    fn pick_finds_the_dot_under_the_point() {
        let p = Picker::build(vec![(10.0, 10.0, 5.0), (100.0, 100.0, 8.0)]);
        assert_eq!(p.pick(10.0, 10.0), Some(0), "dead centre");
        assert_eq!(p.pick(13.0, 13.0), Some(0), "inside the radius");
        assert_eq!(p.pick(100.0, 104.0), Some(1), "the bigger dot");
        assert_eq!(p.pick(50.0, 50.0), None, "empty board is not a node");
    }

    /// gpui paints later siblings over earlier ones, so where dots
    /// overlap the last drawn is the one the pointer was hitting. This
    /// replaces gpui's own hit-testing and has to agree with it, or
    /// clicking a crowded cluster opens the wrong note.
    #[test]
    fn overlapping_dots_resolve_to_the_topmost() {
        let p = Picker::build(vec![(10.0, 10.0, 6.0), (12.0, 10.0, 6.0)]);
        assert_eq!(p.pick(11.0, 10.0), Some(1), "the later dot is on top");
    }

    /// A miss just outside the edge must not round into a hit: at the
    /// whole-vault zoom dots are a few pixels across and sit close
    /// together, so a generous radius would open a neighbour.
    #[test]
    fn a_near_miss_is_a_miss() {
        let p = Picker::build(vec![(0.0, 0.0, 5.0)]);
        assert_eq!(p.pick(4.9, 0.0), Some(0));
        assert_eq!(p.pick(5.1, 0.0), None);
    }

    /// Built from real nodes, the picker's radii are the painter's --
    /// one rule, so a click cannot drift from the dot it looks at.
    #[test]
    fn picker_radii_match_the_painter() {
        let mut nodes = chain(3);
        nodes.0[0].x = 0.0;
        nodes.0[0].y = 0.0;
        let zoom = 2.0;
        let dots: Vec<(f32, f32, f32)> = nodes
            .0
            .iter()
            .map(|n| (n.x, n.y, node_radius(n.degree, zoom)))
            .collect();
        let r = dots[0].2;
        let p = Picker::build(dots);
        assert_eq!(p.pick(0.0, r * 0.9), Some(0), "just inside the painted edge");
        assert_eq!(p.pick(0.0, r * 1.1), None, "just outside it");
    }
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd pick_finds overlapping_dots a_near_miss picker_radii
```

Expected: FAIL to compile — `Picker` does not exist.

- [ ] **Step 3: Implement**

```rust
/// Resolves a board position to a node, replacing the per-node gpui
/// hit-boxes the graph used to build. One element now covers the whole
/// board, so the pointer has to be matched against the dots by hand.
///
/// A linear scan is deliberate: 2,500 float comparisons per mouse-move
/// is microseconds, while a spatial index is more code and more state
/// to invalidate for no measurable gain at this size.
pub struct Picker {
    /// (x, y, radius) in board pixels, in paint order.
    dots: Vec<(f32, f32, f32)>,
}

impl Picker {
    pub fn build(dots: Vec<(f32, f32, f32)>) -> Self {
        Self { dots }
    }

    pub fn len(&self) -> usize {
        self.dots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dots.is_empty()
    }

    /// The topmost dot containing the point, or `None`. Back to front,
    /// because gpui paints later siblings over earlier ones and this
    /// has to resolve a crowded cluster the same way gpui did.
    pub fn pick(&self, x: f32, y: f32) -> Option<usize> {
        self.dots.iter().enumerate().rev().find_map(|(ix, &(dx, dy, r))| {
            let (ox, oy) = (x - dx, y - dy);
            (ox * ox + oy * oy <= r * r).then_some(ix)
        })
    }
}
```

- [ ] **Step 4: Prove the guard is real**

Change `.rev()` to nothing and confirm `overlapping_dots_resolve_to_the_topmost` fails; change `<=` to `<= r * r * 1.5` and confirm `a_near_miss_is_a_miss` fails. Restore both. Report both results.

- [ ] **Step 5: Run and commit**

```sh
cargo test --bin supermd graph::
```

```bash
git add src/graph.rs
git commit -m "feat: the mouse finds a node without 2,500 hit-boxes

Painting the board as one element means gpui no longer hit-tests each
node, so the pointer has to be matched against the dots directly.

Back to front, because gpui painted later siblings on top: a crowded
cluster has to resolve to the same note it did before. Linear, because
2,500 float comparisons per mouse-move is microseconds and an index
would be state to invalidate for no gain."
```

---

### Task 4: `Hover` — names now, card after a dwell

**Files:**
- Modify: `src/graph.rs`
- Test: inline in `src/graph.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub enum Hovering { Idle, Lit(usize), Carded(usize) }`, `pub struct Hover`, `Hover::at(&mut self, node: Option<usize>, now: Instant) -> Hovering`, `pub const CARD_DWELL: Duration`.
- Consumed by Tasks 5 and 7.

Time is injected, exactly as `autosave.rs` does for its policy, so the delay is tested without a window and without sleeping.

- [ ] **Step 1: Write the failing tests**

```rust
    /// Names appear the instant you point at something; the card waits
    /// until you have actually stopped. Sweeping the pointer across a
    /// dense cluster should not fire a dozen cards and a dozen file
    /// reads behind them.
    #[test]
    fn the_card_waits_for_a_dwell_but_the_name_does_not() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        assert_eq!(h.at(Some(4), t0), Hovering::Lit(4), "lit immediately");
        assert_eq!(h.at(Some(4), t0 + Duration::from_millis(399)), Hovering::Lit(4));
        assert_eq!(h.at(Some(4), t0 + CARD_DWELL), Hovering::Carded(4), "settled");
    }

    /// Moving to another node restarts the wait: the card belongs to
    /// the node you are on, and inheriting the previous node's elapsed
    /// time would flash a card the moment the pointer crossed one.
    #[test]
    fn moving_to_another_node_restarts_the_dwell() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.at(Some(1), t0);
        assert_eq!(h.at(Some(2), t0 + Duration::from_millis(390)), Hovering::Lit(2));
        assert_eq!(h.at(Some(2), t0 + Duration::from_millis(390) + CARD_DWELL), Hovering::Carded(2));
    }

    /// Leaving the board clears everything, and the next hover starts
    /// its own clock rather than resuming the old one.
    #[test]
    fn leaving_resets_the_clock() {
        let t0 = Instant::now();
        let mut h = Hover::default();
        h.at(Some(1), t0);
        assert_eq!(h.at(None, t0 + Duration::from_millis(100)), Hovering::Idle);
        assert_eq!(h.at(Some(1), t0 + Duration::from_millis(200)), Hovering::Lit(1));
        assert_eq!(
            h.at(Some(1), t0 + Duration::from_millis(200) + CARD_DWELL),
            Hovering::Carded(1),
            "the clock restarted on re-entry"
        );
    }
```

Add `use std::time::{Duration, Instant};` to the test module if not already present.

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd the_card_waits moving_to_another_node leaving_resets
```

Expected: FAIL to compile — `Hover` does not exist.

- [ ] **Step 3: Implement**

```rust
/// How long the pointer must rest on a node before its card appears.
/// Long enough that sweeping across a cluster fires nothing, short
/// enough that stopping feels answered.
pub const CARD_DWELL: Duration = Duration::from_millis(400);

/// What the pointer is currently doing to a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hovering {
    /// Nothing under the pointer.
    Idle,
    /// Pointed at: this node and its neighbours show their names.
    Lit(usize),
    /// Rested on: the card is up as well.
    Carded(usize),
}

/// The dwell timer behind the hover card. Time is injected rather than
/// read, so the delay is testable without a window and without
/// sleeping -- the arrangement `autosave.rs` uses for its policy.
#[derive(Debug, Default)]
pub struct Hover {
    /// The node under the pointer and when it arrived there.
    since: Option<(usize, Instant)>,
}

impl Hover {
    pub fn at(&mut self, node: Option<usize>, now: Instant) -> Hovering {
        let Some(ix) = node else {
            self.since = None;
            return Hovering::Idle;
        };
        match self.since {
            // Still on the same node: the card is owed once the dwell
            // has elapsed.
            Some((prev, since)) if prev == ix => {
                if now.duration_since(since) >= CARD_DWELL {
                    Hovering::Carded(ix)
                } else {
                    Hovering::Lit(ix)
                }
            }
            // A different node, or the first one: start its own clock.
            _ => {
                self.since = Some((ix, now));
                Hovering::Lit(ix)
            }
        }
    }
}
```

Add `use std::time::{Duration, Instant};` to the top of `src/graph.rs`.

- [ ] **Step 4: Run the tests and both suites**

```sh
cargo test --bin supermd graph::
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

Expected: PASS, build clean at the documented 48 warnings.

- [ ] **Step 5: Commit**

```bash
git add src/graph.rs
git commit -m "feat: a dwell timer for the hover card

Names appear the instant you point at something. The card waits until
you have stopped, so sweeping across a cluster fires neither a dozen
cards nor the dozen file reads behind them.

Time is injected rather than read, so the delay is tested without a
window and without sleeping -- the arrangement autosave.rs already
uses for its policy."
```

---

### Task 5: The board becomes one canvas

**Files:**
- Modify: `src/workspace.rs` (`render_graph`, `workspace.rs:5159` — the node loop at `5255-5379`, and the container the board sits in)
- Test: inline in `src/workspace.rs`

**Interfaces:**
- Consumes: `node_radius`, `lod` (Task 1), `orphan_dim` (Task 2), `Picker` (Task 3), `Hovering`/`Hover` (Task 4).
- Produces: `GraphViewState::picker: Option<crate::graph::Picker>`, `GraphViewState::hover: crate::graph::Hover`, `GraphViewState::hover_state: crate::graph::Hovering`.
- Consumed by Tasks 6, 7, 8.

The heart of the plan. Today every node is a `div` with four boxed listeners plus an always-built label element (`workspace.rs:5297-5379`), rebuilt every frame. After this task the board is one canvas painting edges then dots, with labels as elements only for the revealed set.

**Paint dots as quads, not paths.** `window.paint_quad(gpui::quad(bounds, corner_radii, background, border_widths, border_color, border_style))` (`vendor/gpui/src/window.rs:2839`, constructor at `5059`) with `corner_radii` equal to half the box is a filled circle that never reaches the path tessellator.

**Z-order is the canvas's paint order:** edges first, then dots. A mistake here shows as edges drawn over dots.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The regression that started this work: a label element was
    /// built for every node whether or not it could be seen, and at
    /// the whole-vault zoom label_opacity is 0 -- so a 2,500-note vault
    /// shaped 2,500 invisible text elements every frame.
    #[gpui::test]
    fn no_label_is_built_when_no_label_is_visible(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        for i in 0..40 {
            std::fs::write(root.path().join(format!("n{i}.md")), "# n\n").unwrap();
        }
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.open_graph(&OpenGraph, window, cx);
            if let Some(g) = ws.graph.as_mut() {
                g.zoom = 0.3;
            }
        });
        cx.run_until_parked();
        let labels = ws.update_in(cx, |ws, _, _| ws.graph_label_count());
        assert_eq!(labels, 0, "zoomed out, nothing is named");
    }

    /// Hovering names the node and its neighbours at any zoom -- the
    /// whole-vault view otherwise has no way to tell you what a dot is
    /// short of opening it.
    #[gpui::test]
    fn hovering_names_the_neighbourhood_even_zoomed_out(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.md"), "# a\n\n[[b]]\n").unwrap();
        std::fs::write(root.path().join("b.md"), "# b\n").unwrap();
        std::fs::write(root.path().join("c.md"), "# c\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.open_graph(&OpenGraph, window, cx);
            if let Some(g) = ws.graph.as_mut() {
                g.zoom = 0.3;
                g.hovered = Some(0);
                g.hover_state = crate::graph::Hovering::Lit(0);
            }
        });
        cx.run_until_parked();
        let labels = ws.update_in(cx, |ws, _, _| ws.graph_label_count());
        assert_eq!(labels, 2, "the hovered note and the one it links to");
    }

    /// The picker replaces gpui's per-node hit-boxes, so it has to be
    /// built from the same positions and radii the canvas paints.
    #[gpui::test]
    fn the_picker_covers_every_node(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        for i in 0..12 {
            std::fs::write(root.path().join(format!("n{i}.md")), "# n\n").unwrap();
        }
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_graph(&OpenGraph, window, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, _| {
            let g = ws.graph.as_ref().expect("graph open");
            let picker = g.picker.as_ref().expect("built during render");
            assert_eq!(picker.len(), g.nodes().len(), "one dot per node");
        });
    }
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd no_label_is_built hovering_names_the the_picker_covers
```

Expected: FAIL to compile — `graph_label_count`, `hover_state`, `picker` do not exist.

- [ ] **Step 3: Add the state**

In `GraphViewState` (`workspace.rs:761`):

```rust
    /// Board-pixel dots, rebuilt each render, so the pointer can be
    /// matched against what was actually painted.
    picker: Option<crate::graph::Picker>,
    /// The dwell timer behind the card.
    hover: crate::graph::Hover,
    /// What the pointer is doing to `hovered`, as of the last event.
    hover_state: crate::graph::Hovering,
```

Initialise `picker: None`, `hover: Default::default()`, `hover_state: crate::graph::Hovering::Idle` where the state is constructed (`workspace.rs:4910`).

- [ ] **Step 4: Paint the dots into the canvas**

Replace the `for (ix, node) in state.nodes().iter().enumerate()` loop (`workspace.rs:5255-5379`) with a second canvas layered over the edges canvas. Build the paint list before the closure, since the closure must own its data:

```rust
        // (centre, radius, colour) in board pixels, in paint order.
        let dots: Vec<((f32, f32), f32, Hsla)> = state
            .nodes()
            .iter()
            .enumerate()
            .map(|(ix, node)| {
                let on = lit.as_ref().is_none_or(|l| l.contains(&ix))
                    && state.filter.matches_at(ix, node);
                let base = node_base_color(node, ix, &t, &group_keys, &palette, &open_path);
                let faded = if on { base } else { Hsla { a: 0.25, ..base } };
                (
                    at(node),
                    crate::graph::node_radius(node.degree, state.zoom),
                    crate::graph::orphan_dim(faded, node.degree),
                )
            })
            .collect();
        // The pointer is matched against exactly what was painted.
        let picker = crate::graph::Picker::build(
            dots.iter().map(|&((x, y), r, _)| (x, y, r)).collect(),
        );
        let nodes_canvas = gpui::canvas(
            move |bounds, _, _| bounds,
            move |bounds, _, window, _| {
                for ((x, y), r, color) in &dots {
                    // A quad with corner radii of half its size is a
                    // filled circle, and a quad never reaches the path
                    // tessellator -- which is the whole point: 2,500
                    // tessellated circles a frame is what made this
                    // stutter.
                    let d = px(r * 2.0);
                    let origin = point(bounds.origin.x + px(x - r), bounds.origin.y + px(y - r));
                    window.paint_quad(gpui::quad(
                        Bounds { origin, size: gpui::size(d, d) },
                        gpui::Corners::all(px(*r)),
                        *color,
                        gpui::Edges::default(),
                        gpui::transparent_black(),
                        gpui::BorderStyle::default(),
                    ));
                }
            },
        );
```

Extract the colour decision (`workspace.rs:5270-5290`) into a free function `node_base_color` in `workspace.rs` so both the paint list and the tests can call it without a render.

Store the picker on the state at the end of `render_graph`: `if let Some(g) = self.graph.as_mut() { g.picker = Some(picker); }` — note this needs `&mut self`, which `render_graph` already has.

- [ ] **Step 5: Labels only for the revealed set**

After the canvases, add label elements for the revealed set only:

```rust
        // Names are elements, not canvas text, so they can use the
        // theme's font stack and fade with the same opacity rule as
        // before. The set is small by construction: everything at
        // readable zoom that is on screen, or the hovered
        // neighbourhood at any zoom.
        let named: Vec<usize> = if lod.labels {
            (0..state.nodes().len()).filter(|&ix| on_screen(ix)).collect()
        } else {
            lit.as_ref().map(|l| l.iter().copied().collect()).unwrap_or_default()
        };
        for ix in named {
            board = board.child(label_element(ix, &state, &t, label_alpha));
        }
```

Add the counter the tests read:

```rust
    /// How many node labels the last render produced. The graph used to
    /// build one per node whether or not it was visible; this is what
    /// holds that shut.
    #[cfg(test)]
    fn graph_label_count(&self) -> usize {
        self.graph.as_ref().map_or(0, |g| g.label_count)
    }
```

storing `label_count` on `GraphViewState` as the render sets it.

- [ ] **Step 6: Move interaction to the container**

Delete the per-node `on_hover`, both `on_mouse_down` handlers and the `on_click` from the node loop. On the board container, add one set:

```rust
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                let Some(graph) = this.graph.as_mut() else { return };
                let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
                let hit = graph.picker.as_ref().and_then(|p| p.pick(x, y));
                let state = graph.hover.at(hit, std::time::Instant::now());
                if graph.hovered != hit || graph.hover_state != state {
                    graph.hovered = hit;
                    graph.hover_state = state;
                    cx.notify();
                }
            }))
```

and route left/right mouse-down through `picker.pick` to the same bodies the per-node handlers had (node drag on left, context menu on right). Keep `cx.stop_propagation()` only when a node was actually hit, so a press on empty board still pans.

- [ ] **Step 7: Run everything, then look at it**

```sh
cargo test --bin supermd workspace::tests::no_label_is_built workspace::tests::hovering_names workspace::tests::the_picker_covers
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

Then open the app on a real vault, press the graph shortcut, and confirm: dots are round, edges sit under them, hovering lights the neighbourhood and names it, dragging a node still works, right-click still opens the menu, and clicking empty board still pans. **Report what you saw.** If dots are square, `corner_radii` is not half the size; if edges sit over dots, the canvases are in the wrong order.

- [ ] **Step 8: Commit**

```bash
git add src/workspace.rs
git commit -m "perf: the board is one canvas, not five thousand elements

Every node was a div with four boxed listeners and an always-built
label element, rebuilt every frame -- and at the zoom that fits a whole
vault, label_opacity is 0, so those labels were shaped and laid out
invisibly.

Dots are now quads painted into the canvas the edges already used: a
quad with corner radii of half its size is a circle that never reaches
the path tessellator. Labels are elements only where a name is
actually shown. The pointer is matched against the painted dots by
Picker, which resolves overlaps the way gpui's own hit-testing did."
```

---

### Task 6: Stop rebuilding what has not changed

**Files:**
- Modify: `src/workspace.rs` (`render_graph`, and `GraphViewState`)
- Test: inline in `src/workspace.rs`

**Interfaces:**
- Consumes: `lod` (Task 1), the canvases (Task 5).
- Produces: `GraphViewState::group_keys: Vec<String>`, `GraphViewState::edges_world: Vec<...>`.

Three allocations per frame remain after Task 5: `group_keys` clones every node's folder/tag, sorts and dedups (`workspace.rs:5189-5201`); `edge_px` is rebuilt (`5209-5221`); and arrowheads are tessellated at every zoom.

- [ ] **Step 1: Write the failing tests**

```rust
    /// group_keys cloned every node's folder or tag, sorted and
    /// deduplicated them -- 60 times a second, for data that changes
    /// only when the index or the colour mode does.
    #[gpui::test]
    fn group_keys_are_computed_once_not_per_frame(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("notes")).unwrap();
        std::fs::write(root.path().join("notes/a.md"), "# a\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.open_graph(&OpenGraph, window, cx);
            ws.graph_color_by(&GraphColorBy, window, cx);
        });
        cx.run_until_parked();
        let before = ws.update_in(cx, |ws, _, _| ws.graph.as_ref().unwrap().group_keys.clone());
        // A render changes nothing: only a colour-mode or index change
        // may rebuild this.
        ws.update_in(cx, |_, _, cx| cx.notify());
        cx.run_until_parked();
        let after = ws.update_in(cx, |ws, _, _| ws.graph.as_ref().unwrap().group_keys.clone());
        assert_eq!(before, after, "stable across renders");
        assert!(!before.is_empty(), "the folder is a group");
    }

    /// Below half zoom an arrowhead is sub-pixel, and it costs one or
    /// two tessellated paths per edge -- at exactly the zoom where
    /// nothing can be culled because the whole vault is on screen.
    #[gpui::test]
    fn arrowheads_are_skipped_at_whole_vault_zoom(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.md"), "# a\n\n[[b]]\n").unwrap();
        std::fs::write(root.path().join("b.md"), "# b\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.open_graph(&OpenGraph, window, cx);
            if let Some(g) = ws.graph.as_mut() {
                g.zoom = 0.3;
            }
        });
        cx.run_until_parked();
        assert!(!crate::graph::lod(0.3).arrowheads, "and the renderer asks lod()");
    }
```

- [ ] **Step 2: Run and watch the first fail**

```sh
cargo test --bin supermd group_keys_are_computed_once arrowheads_are_skipped
```

Expected: the group-keys test FAILS to compile (`group_keys` is not a field).

- [ ] **Step 3: Hoist the group keys**

Add `group_keys: Vec<String>` to `GraphViewState`, compute it where the simulation is built (`workspace.rs:4910`) and again in `graph_color_by` and wherever the index is rebuilt, and have `render_graph` read `state.group_keys` instead of building its own.

- [ ] **Step 4: Gate the arrowheads on `lod`**

In the edges canvas (`workspace.rs:5226-5248`), wrap both `arrow_path` paints:

```rust
                    if lod.arrowheads {
                        window.paint_path(crate::graph::arrow_path(pa, pb, rb + 2.0, 7.0), color);
                        if *both {
                            window.paint_path(crate::graph::arrow_path(pb, pa, ra + 2.0, 7.0), color);
                        }
                    }
```

- [ ] **Step 5: Run and commit**

```sh
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

```bash
git add src/workspace.rs
git commit -m "perf: stop rebuilding what has not changed

group_keys cloned, sorted and deduplicated every node's folder or tag
on every frame, for data that changes only when the index or the
colour mode does.

And an arrowhead is one or two tessellated paths per edge that is
sub-pixel below half zoom -- the zoom where the whole vault is on
screen and nothing can be culled."
```

---

### Task 7: The hover card

**Files:**
- Modify: `src/workspace.rs`
- Test: inline in `src/workspace.rs`

**Interfaces:**
- Consumes: `Hovering` (Task 4), `preview::excerpt` (`preview.rs:26`), `preview::title_of` (`preview.rs:38`).
- Produces: `GraphViewState::preview_cache: std::collections::HashMap<PathBuf, (String, String)>`.

A card at `Hovering::Carded(ix)`: title, ~3-line excerpt, out/in counts. It never takes the mouse (`.occlude()` is **not** used; no listeners) and flips side rather than leaving the window.

The excerpt is read synchronously, following the editor's link preview (`editor/mod.rs:2868`). The dwell gate bounds this to a couple of small reads per second. The cache is invalidated wholesale on fs events, where `on_fs_events` already runs.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The card is what makes a dot legible without opening it. A
    /// ghost -- a link to a note that does not exist yet -- has no file
    /// to read, and must say so rather than showing an empty card.
    #[gpui::test]
    fn the_card_describes_a_note_and_admits_a_ghost(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.md"), "# Alpha\n\nFirst line.\n\n[[missing]]\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_graph(&OpenGraph, window, cx));
        cx.run_until_parked();
        let (real, ghost) = ws.update_in(cx, |ws, _, _| {
            let g = ws.graph.as_ref().unwrap();
            let real = g.nodes().iter().position(|n| !n.ghost).unwrap();
            let ghost = g.nodes().iter().position(|n| n.ghost).unwrap();
            (real, ghost)
        });
        let card = ws.update_in(cx, |ws, _, cx| ws.graph_card_text(real, cx));
        assert!(card.contains("Alpha"), "the title: {card}");
        assert!(card.contains("First line"), "the excerpt: {card}");
        let card = ws.update_in(cx, |ws, _, cx| ws.graph_card_text(ghost, cx));
        assert!(card.contains("does not exist"), "a ghost says so: {card}");
    }

    /// A file deleted between indexing and hovering must not raise an
    /// error strip -- hovering is not a command the user issued.
    #[gpui::test]
    fn a_card_for_a_vanished_file_says_so_quietly(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.md"), "# Alpha\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_graph(&OpenGraph, window, cx));
        cx.run_until_parked();
        std::fs::remove_file(root.path().join("a.md")).unwrap();
        let card = ws.update_in(cx, |ws, _, cx| ws.graph_card_text(0, cx));
        assert!(card.contains("could not be read"), "{card}");
        ws.update_in(cx, |ws, _, _| assert!(ws.command_error.is_none(), "no error strip"));
    }
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd the_card_describes a_card_for_a_vanished
```

Expected: FAIL to compile — `graph_card_text` does not exist.

- [ ] **Step 3: Implement the text, then the element**

```rust
    /// Title, excerpt and counts for a node's card. Pure enough to
    /// test: the element around it is layout.
    fn graph_card_text(&mut self, ix: usize, cx: &mut Context<Self>) -> String {
        let Some(node) = self.graph.as_ref().and_then(|g| g.nodes().get(ix)).cloned() else {
            return String::new();
        };
        let name = node.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if node.ghost {
            return format!("{name}\nThis note does not exist yet");
        }
        // The editor's link preview reads on hover too; the dwell gate
        // is what keeps this to a couple of small reads a second.
        let cached = self.graph.as_ref().and_then(|g| g.preview_cache.get(&node.path).cloned());
        let (title, body) = match cached {
            Some(hit) => hit,
            None => {
                let Ok(text) = std::fs::read_to_string(&node.path) else {
                    return format!("{name}\nThis note could not be read");
                };
                let pair = (
                    crate::preview::title_of(&text, &node.path),
                    crate::preview::excerpt(&text, 3),
                );
                if let Some(g) = self.graph.as_mut() {
                    g.preview_cache.insert(node.path.clone(), pair.clone());
                }
                pair
            }
        };
        let (out, inn) = self.graph_link_counts(ix);
        format!("{title}\n{body}\n{out} out · {inn} in")
    }
```

Add `graph_link_counts(&self, ix) -> (usize, usize)` counting `e.from == ix` and `e.to == ix` over `state.edges()`. Render the card as an element positioned near `hovered`, using `Surface::Floating` from `elevation.rs` so it joins the existing shadow vocabulary rather than inventing its own. Flip it to the left of the cursor when `x + card_width` exceeds the viewport.

Clear `preview_cache` in `on_fs_events` where the tree already refreshes.

- [ ] **Step 4: Run, look, commit**

```sh
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

Open a real vault, rest the pointer on a node for half a second, and confirm the card appears with a readable excerpt, does not swallow the pointer, and flips near the right edge. Report what you saw.

```bash
git add src/workspace.rs
git commit -m "feat: resting on a node tells you what it is

A card after 400ms of dwell: title, three lines, and the link counts.
Sweeping across a cluster fires nothing, which is what keeps the file
reads behind it to a couple a second.

A ghost has no file and says so. A note deleted between indexing and
hovering says so quietly -- hovering is not a command anyone issued,
so it does not deserve an error strip."
```

---

### Task 8: The peek panel

**Files:**
- Modify: `src/workspace.rs`
- Test: inline in `src/workspace.rs`

**Interfaces:**
- Consumes: `graph_card_text`, `graph_link_counts` (Task 7), `fit_to` (`graph.rs:936`).
- Produces: `GraphViewState::peek: Option<usize>`.

Clicking a node opens a ~360px panel on the right. The board's viewport **shrinks** to the remaining width, so nothing clickable hides behind the panel.

- [ ] **Step 1: Write the failing tests**

```rust
    /// Clicking a node used to replace the whole window. Now it opens
    /// a panel beside a graph that is still there.
    #[gpui::test]
    fn clicking_a_node_peeks_instead_of_leaving(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.md"), "# Alpha\n\nBody.\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_graph(&OpenGraph, window, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| ws.graph_peek(0, window, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, _| {
            assert!(ws.graph.is_some(), "the graph survives a click");
            assert_eq!(ws.graph.as_ref().unwrap().peek, Some(0));
            assert_eq!(ws.tabs.len(), 0, "and nothing was opened yet");
        });
    }

    /// Esc closes the panel first and the graph second, so escaping a
    /// peek does not also throw away the view you were exploring.
    #[gpui::test]
    fn escape_closes_the_panel_before_the_graph(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.md"), "# Alpha\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| {
            ws.open_graph(&OpenGraph, window, cx);
            ws.graph_peek(0, window, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| ws.cancel(&Cancel, window, cx));
        ws.update_in(cx, |ws, _, _| {
            assert!(ws.graph.is_some(), "graph stays");
            assert_eq!(ws.graph.as_ref().unwrap().peek, None, "panel closed");
        });
        ws.update_in(cx, |ws, window, cx| ws.cancel(&Cancel, window, cx));
        ws.update_in(cx, |ws, _, _| assert!(ws.graph.is_none(), "now the graph"));
    }

    /// The board narrows so the panel covers no node: a dot hidden
    /// behind the panel is a dot you cannot click.
    #[gpui::test]
    fn the_board_narrows_for_the_panel(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.md"), "# Alpha\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_graph(&OpenGraph, window, cx));
        cx.run_until_parked();
        let wide = ws.update_in(cx, |ws, window, _| ws.graph_board_width(window));
        ws.update_in(cx, |ws, window, cx| ws.graph_peek(0, window, cx));
        let narrow = ws.update_in(cx, |ws, window, _| ws.graph_board_width(window));
        assert!(narrow < wide, "{narrow} < {wide}");
        assert!((wide - narrow - PEEK_W).abs() < 1.0, "exactly the panel's width");
    }
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd clicking_a_node_peeks escape_closes_the_panel the_board_narrows
```

Expected: FAIL to compile — `graph_peek`, `peek`, `graph_board_width`, `PEEK_W` do not exist.

- [ ] **Step 3: Implement**

Add `const PEEK_W: f32 = 360.0;`, `peek: Option<usize>` on the state, and:

```rust
    /// Open the panel on a node without leaving the graph. The click
    /// that used to open a tab and destroy the view now does this.
    fn graph_peek(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph.as_mut() {
            graph.peek = Some(ix);
        }
        cx.notify();
    }

    /// Board width, less the panel when it is open.
    fn graph_board_width(&self, window: &Window) -> f32 {
        let full = f32::from(window.viewport_size().width);
        match self.graph.as_ref().and_then(|g| g.peek) {
            Some(_) => full - PEEK_W,
            None => full,
        }
    }
```

Route the canvas click (Task 5's container handler) to `graph_peek` rather than `open_graph_node`. In `cancel`, close the peek first and return; only close the graph when no peek is open. The panel renders title, excerpt, counts, the backlink list, and an Open button bound to `open_graph_node`; a backlink row calls `graph_peek` on that node and re-centres via `fit_to` with the narrowed viewport.

- [ ] **Step 4: Run, look, commit**

```sh
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

Open a vault, click a node, walk two backlinks, press Esc twice. Report what you saw.

```bash
git add src/workspace.rs
git commit -m "feat: clicking a node peeks instead of leaving

A panel beside the graph rather than a window that changed underneath
you: title, excerpt, counts and the backlinks, with the graph still
live next to it. Clicking a backlink walks there without leaving.

The board narrows by exactly the panel's width, because a dot behind
the panel is a dot you cannot click. Esc closes the panel first and
the graph second."
```

---

### Task 9: The graph stops being thrown away

**Files:**
- Modify: `src/workspace.rs` (`open_graph_node`, `workspace.rs:5124-5128`; `open_graph`)
- Test: inline in `src/workspace.rs`

**Interfaces:**
- Consumes: the state from Tasks 5-8.
- Produces: `Workspace::graph_cache: Option<GraphViewState>`.

`open_graph_node`'s second statement is `self.graph = None` — the layout, pan, zoom and filter are destroyed, and reopening restarts the simulation from `alpha = 1.0`. This is the cause of the context switch. Opening a note still closes the graph; it no longer forgets it.

- [ ] **Step 1: Write the failing test**

```rust
    /// Opening a note from the graph used to throw the layout away, so
    /// coming back re-simulated from scratch and landed you somewhere
    /// else entirely. The view you left is the view you return to.
    #[gpui::test]
    fn reopening_the_graph_returns_the_view_you_left(cx: &mut TestAppContext) {
        let _home = temp_home();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.md"), "# Alpha\n").unwrap();
        std::fs::write(root.path().join("b.md"), "# Beta\n").unwrap();
        let (ws, cx) = open_workspace(cx, root.path());
        ws.update_in(cx, |ws, window, cx| ws.open_graph(&OpenGraph, window, cx));
        cx.run_until_parked();
        // Settle, then move the view somewhere recognisable.
        ws.update_in(cx, |ws, _, _| {
            let g = ws.graph.as_mut().unwrap();
            g.zoom = 1.7;
            g.pan = (42.0, -17.0);
            g.filter.query = "alpha".into();
        });
        let positions: Vec<(f32, f32)> = ws.update_in(cx, |ws, _, _| {
            ws.graph.as_ref().unwrap().nodes().iter().map(|n| (n.x, n.y)).collect()
        });
        ws.update_in(cx, |ws, window, cx| ws.open_graph_node(0, window, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, _| {
            assert!(ws.graph.is_none(), "the note is open, the graph closed");
            assert_eq!(ws.tabs.len(), 1);
        });
        ws.update_in(cx, |ws, window, cx| ws.open_graph(&OpenGraph, window, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, _| {
            let g = ws.graph.as_ref().unwrap();
            assert_eq!(g.zoom, 1.7, "the zoom you left");
            assert_eq!(g.pan, (42.0, -17.0), "the pan you left");
            assert_eq!(g.filter.query, "alpha", "and what it was narrowed to");
            let now: Vec<(f32, f32)> = g.nodes().iter().map(|n| (n.x, n.y)).collect();
            assert_eq!(now, positions, "no re-simulation");
        });
    }
```

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd reopening_the_graph_returns
```

Expected: FAIL — zoom is back at its default and the positions differ, because the graph was rebuilt.

- [ ] **Step 3: Implement**

Add `graph_cache: Option<GraphViewState>` to `Workspace`. In `open_graph_node`, replace `self.graph = None` with:

```rust
        // The view is put away, not thrown away: coming back with ⌘G
        // lands where you left rather than re-simulating a layout you
        // had already arranged.
        self.graph_cache = self.graph.take();
```

In `open_graph`, prefer the cache when the index has not changed underneath it:

```rust
        if let Some(mut cached) = self.graph_cache.take() {
            if cached.nodes().len() == crate::graph::build(&index).0.len() {
                cached.ticker = None;
                self.graph = Some(cached);
                cx.notify();
                return;
            }
        }
```

Rebuild from scratch when the node count differs — notes added or deleted while you were away mean the cached layout no longer describes the vault. Drop `graph_cache` in `after_path_change` and when the workspace root changes, where the index is already invalidated.

- [ ] **Step 4: Run everything**

```sh
cargo test --bin supermd reopening_the_graph_returns
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

- [ ] **Step 5: Commit**

```bash
git add src/workspace.rs
git commit -m "fix: the graph is put away, not thrown away

open_graph_node's second statement was `self.graph = None`, so opening
a note destroyed the layout, the pan, the zoom and the filter -- and
reopening re-simulated from alpha 1.0 and landed somewhere else. That
is the whole of the 'it switches context' complaint.

The view is cached and restored, unless the note count changed while
you were away, in which case the cached layout no longer describes the
vault and is rebuilt."
```

---

### Task 10: Measure it, document it, file what was deferred

**Files:**
- Create: `docs/site/graph.md`; modify `docs/site/nav.toml`, regenerate `site/docs/`
- Modify: `docs/BACKLOG.md`
- Modify: `src/workspace.rs` (only if measurement finds something)

**Interfaces:** consumes everything above.

Structural tests cannot prove "smooth". This task produces the numbers.

- [ ] **Step 1: Measure, before and after**

**This step needs a release build, and therefore the external SSD mounted** (`df -h /Volumes/supermd-build`), or ~8GB free on the internal disk. Ask before building to the internal disk.

On a real vault of ~2,500 notes, at the fit-to-window zoom, with the ticker running, record for `HEAD` and for the merge-base:

- wall time of one `render_graph` pass,
- wall time of one `sim.step()`,
- the element count of the graph view.

A `#[ignore]`-marked timing test is the cheapest harness: it runs on demand, never in CI, and cannot flake a suite. Put the numbers in the task report.

- [ ] **Step 2: Write the user documentation**

`docs/site/graph.md`: opening the graph, what a dot's size and colour mean, that unlinked notes are dimmed rather than hidden, hover naming and the card, the peek panel and walking backlinks, the Esc ordering, and that the view is remembered. Add it to `nav.toml`, then:

```sh
cargo run --example build_docs
```

Commit `docs/site/graph.md`, `docs/site/nav.toml` and the regenerated `site/docs/` **together** — a source edited without its rendered output is a defect.

- [ ] **Step 3: File what was deferred**

Add to `docs/BACKLOG.md`, in the file's existing prose-entry voice:

- **`graph.rs` is now ~2,000 lines** carrying the force layout, the view-model rules and their tests. Splitting was deliberately deferred so this change stayed reviewable; the seam is between the simulation and the view rules.
- **The simulation still steps on the UI thread** (`workspace.rs:4951`, inside `this.update`). The render work made the frame much cheaper; if measurement ever shows step time dominating again, moving it to the background executor with a positions snapshot is the next move.

- [ ] **Step 4: Full verification**

```sh
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
cargo llvm-cov --summary-only --fail-under-lines 90
```

`grep -c SKIP` on the test output must be 0.

- [ ] **Step 5: Commit**

```bash
git add docs/site/graph.md docs/site/nav.toml site/docs docs/BACKLOG.md
git commit -m "docs: the graph view, and what it deferred

What a dot's size and colour mean, why unlinked notes are dim rather
than absent, how hover names things, what the panel does, and that the
view is remembered.

And the two deferrals, in the backlog rather than in a commit message
nobody greps: graph.rs is now ~2,000 lines and wants splitting at the
simulation/view seam, and the simulation still steps on the UI thread."
```

---

## Self-review

**Spec coverage.** Every section of the design maps to a task: `Picker` → 3, `Lod` → 1, `Dimming` → 2, `Hover` → 4, canvas/quads/culling → 5, caching and hoisting → 6, hover card and preview loading → 7, peek panel, viewport shrink, backlink walking and Esc ordering → 8, the `self.graph = None` fix → 9, measurement, docs and deferrals → 10. Error handling from the spec appears as tests in Task 7 (unreadable file, ghost) and Task 8 (Esc ordering); the empty-graph case is covered by `Picker::pick` returning `None` on an empty dot list, tested in Task 3.

**Type consistency.** `node_radius(degree, zoom)` is defined in Task 1 and consumed by Tasks 3 and 5 with the same signature. `Picker::build` takes `Vec<(f32, f32, f32)>` in Task 3 and is called that way in Task 5. `Hover::at(Option<usize>, Instant) -> Hovering` is defined in Task 4 and used in Tasks 5 and 7. `graph_card_text(ix, cx) -> String` is defined in Task 7 and consumed by Task 8's panel.

**Known deviation from the codebase's usual shape:** Task 5 is larger than the others. It cannot be split — the canvas, the picker and the removal of the per-node listeners have to land together or the graph is uninteractable in between. Its step list is finer-grained to compensate.
