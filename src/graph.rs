//! Note graph: nodes are notes, edges are resolved links. A small
//! deterministic force layout — no randomness, so frames and tests
//! reproduce exactly.

use crate::knowledge::Index;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct GraphNode {
    pub path: PathBuf,
    /// Layout position in the unit square (0..1, 0..1).
    pub x: f32,
    pub y: f32,
    /// Velocity, carried between ticks. A settled layout has none; a
    /// nudged one coasts to rest, which is what makes the graph feel
    /// alive rather than redrawn.
    pub vx: f32,
    pub vy: f32,
    /// Held in place by the pointer. A pinned node still pushes and
    /// pulls its neighbours but is not itself moved by them.
    pub pinned: bool,
    /// Link count (in + out) — drives node size.
    pub degree: usize,
}

/// Indexes into the node list, a < b, deduplicated.
pub type GraphEdge = (usize, usize);

/// Every note and every resolved link in the workspace.
pub fn build(index: &Index) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let names = index.note_names();
    let mut nodes: Vec<GraphNode> = names
        .iter()
        .enumerate()
        .map(|(ix, (_, path))| {
            // Deterministic seed positions on a circle, by index.
            let angle = ix as f32 / names.len().max(1) as f32 * std::f32::consts::TAU;
            GraphNode {
                path: path.clone(),
                x: 0.5 + 0.35 * angle.cos(),
                y: 0.5 + 0.35 * angle.sin(),
                vx: 0.0,
                vy: 0.0,
                pinned: false,
                degree: 0,
            }
        })
        .collect();
    let index_of: std::collections::BTreeMap<PathBuf, usize> = nodes
        .iter()
        .enumerate()
        .map(|(ix, n)| (n.path.clone(), ix))
        .collect();
    let mut edges: Vec<GraphEdge> = Vec::new();
    for (from, to) in index.edges() {
        let (Some(&a), Some(&b)) = (index_of.get(&from), index_of.get(&to)) else {
            continue;
        };
        nodes[a].degree += 1;
        nodes[b].degree += 1;
        let edge = (a.min(b), a.max(b));
        if !edges.contains(&edge) {
            edges.push(edge);
        }
    }
    (nodes, edges)
}

/// The one-hop neighborhood of `center`: outgoing links + backlinks.
pub fn local(index: &Index, center: &Path) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let mut neighbors: Vec<PathBuf> = Vec::new();
    for (from, to) in index.edges() {
        let other = if from == center {
            Some(to)
        } else if to == center {
            Some(from)
        } else {
            None
        };
        if let Some(other) = other {
            if !neighbors.contains(&other) && other != center {
                neighbors.push(other);
            }
        }
    }
    let mut nodes = vec![GraphNode {
        path: center.to_path_buf(),
        x: 0.5,
        y: 0.5,
        vx: 0.0,
        vy: 0.0,
        pinned: false,
        degree: neighbors.len(),
    }];
    let n = neighbors.len().max(1) as f32;
    for (ix, path) in neighbors.into_iter().enumerate() {
        let angle = ix as f32 / n * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        nodes.push(GraphNode {
            path,
            x: 0.5 + 0.32 * angle.cos(),
            y: 0.5 + 0.32 * angle.sin(),
            vx: 0.0,
            vy: 0.0,
            pinned: false,
            degree: 1,
        });
    }
    let edges = (1..nodes.len()).map(|ix| (0, ix)).collect();
    (nodes, edges)
}

/// Deterministic force layout over the unit square: springs along
/// edges, repulsion between all pairs, `iterations` rounds.
/// Tunable force strengths. Named after what a reader would call
/// them, because these are meant to be exposed as settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forces {
    /// How hard nodes push each other apart.
    pub repel: f32,
    /// Rest length of a link.
    pub link_distance: f32,
    /// How strongly a link pulls to that length.
    pub link_strength: f32,
    /// Pull toward the middle, so disconnected notes stay on screen.
    pub center: f32,
    /// Fraction of velocity kept each tick. Lower settles sooner.
    pub velocity_decay: f32,
}

impl Default for Forces {
    fn default() -> Self {
        Self {
            repel: 0.004,
            link_distance: 0.18,
            link_strength: 0.9,
            center: 0.05,
            velocity_decay: 0.6,
        }
    }
}

/// A running force layout.
///
/// The previous layout ran a fixed 150 iterations once and drew the
/// result — a still picture. This carries velocity between ticks and
/// cools toward rest, so the shell can step it per frame: nodes settle
/// visibly, a drag pushes its neighbours around, and releasing lets
/// everything coast back. That motion is the whole difference between
/// a diagram and something that feels alive.
///
/// The cooling model is d3-force's: `alpha` falls geometrically toward
/// `alpha_target`, every force is scaled by it, and an interaction
/// "reheats" by raising it again.
#[derive(Debug, Clone)]
pub struct Simulation {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub forces: Forces,
    alpha: f32,
    alpha_target: f32,
}

/// Below this the layout is at rest and the shell can stop stepping.
pub const ALPHA_REST: f32 = 0.005;
/// Fraction of the remaining heat lost per tick.
const ALPHA_DECAY: f32 = 0.0228;

impl Simulation {
    pub fn new(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> Self {
        Self { nodes, edges, forces: Forces::default(), alpha: 1.0, alpha_target: 0.0 }
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// True once motion has died down enough to stop redrawing.
    pub fn settled(&self) -> bool {
        self.alpha < ALPHA_REST && self.alpha_target < ALPHA_REST
    }

    /// Put heat back in: something changed and the layout should move
    /// again. `0.3` is a nudge, `1.0` a fresh start.
    pub fn reheat(&mut self, to: f32) {
        self.alpha = self.alpha.max(to);
    }

    /// Hold the layout warm while a drag is in progress, so it keeps
    /// responding instead of cooling under the pointer.
    pub fn hold_warm(&mut self, warm: bool) {
        self.alpha_target = if warm { 0.3 } else { 0.0 };
        if warm {
            self.reheat(0.3);
        }
    }

    /// Move a node under the pointer and keep it there.
    pub fn pin(&mut self, ix: usize, x: f32, y: f32) {
        if let Some(n) = self.nodes.get_mut(ix) {
            n.x = x;
            n.y = y;
            n.vx = 0.0;
            n.vy = 0.0;
            n.pinned = true;
        }
    }

    pub fn release(&mut self, ix: usize) {
        if let Some(n) = self.nodes.get_mut(ix) {
            n.pinned = false;
        }
    }

    /// One tick. Forces accumulate into velocity, velocity decays, and
    /// position follows — so a node keeps moving after the force that
    /// started it has gone, which is what reads as momentum.
    pub fn step(&mut self) {
        let n = self.nodes.len();
        if n < 2 {
            return;
        }
        self.alpha += (self.alpha_target - self.alpha) * ALPHA_DECAY;
        let a = self.alpha;
        let f = self.forces;

        for i in 0..n {
            for j in i + 1..n {
                let dx = self.nodes[i].x - self.nodes[j].x;
                let dy = self.nodes[i].y - self.nodes[j].y;
                let d2 = (dx * dx + dy * dy).max(1e-4);
                let d = d2.sqrt();
                let rep = f.repel / d2 * a;
                self.nodes[i].vx += rep * dx / d;
                self.nodes[i].vy += rep * dy / d;
                self.nodes[j].vx -= rep * dx / d;
                self.nodes[j].vy -= rep * dy / d;
            }
        }
        for &(p, q) in &self.edges {
            let dx = self.nodes[q].x - self.nodes[p].x;
            let dy = self.nodes[q].y - self.nodes[p].y;
            let d = (dx * dx + dy * dy).sqrt().max(1e-4);
            let pull = (d - f.link_distance) * f.link_strength * a;
            self.nodes[p].vx += pull * dx / d;
            self.nodes[p].vy += pull * dy / d;
            self.nodes[q].vx -= pull * dx / d;
            self.nodes[q].vy -= pull * dy / d;
        }
        for node in &mut self.nodes {
            node.vx += (0.5 - node.x) * f.center * a;
            node.vy += (0.5 - node.y) * f.center * a;
            node.vx *= f.velocity_decay;
            node.vy *= f.velocity_decay;
            if node.pinned {
                // A pinned node still pushed its neighbours above; it
                // just does not move itself.
                node.vx = 0.0;
                node.vy = 0.0;
                continue;
            }
            node.x = (node.x + node.vx).clamp(0.0, 1.0);
            node.y = (node.y + node.vy).clamp(0.0, 1.0);
        }
    }

    /// Step until at rest, or `max` ticks — whichever comes first.
    /// Used to seed a layout before the first frame is drawn.
    pub fn run(&mut self, max: usize) {
        for _ in 0..max {
            if self.settled() {
                break;
            }
            self.step();
        }
    }
}

pub fn layout(nodes: &mut [GraphNode], edges: &[GraphEdge], iterations: usize) {
    let n = nodes.len();
    if n < 2 {
        return;
    }
    let spring = 0.18f32; // rest length of an edge
    for round in 0..iterations {
        // Cooling: big early moves, tiny late ones.
        let step = 0.05 * (1.0 - round as f32 / iterations as f32).max(0.05);
        let mut fx = vec![0f32; n];
        let mut fy = vec![0f32; n];
        for i in 0..n {
            for j in i + 1..n {
                let dx = nodes[i].x - nodes[j].x;
                let dy = nodes[i].y - nodes[j].y;
                let d2 = (dx * dx + dy * dy).max(1e-4);
                let rep = 0.004 / d2;
                let d = d2.sqrt();
                fx[i] += rep * dx / d;
                fy[i] += rep * dy / d;
                fx[j] -= rep * dx / d;
                fy[j] -= rep * dy / d;
            }
        }
        for &(a, b) in edges {
            let dx = nodes[b].x - nodes[a].x;
            let dy = nodes[b].y - nodes[a].y;
            let d = (dx * dx + dy * dy).sqrt().max(1e-4);
            let pull = (d - spring) * 0.9;
            fx[a] += pull * dx / d;
            fy[a] += pull * dy / d;
            fx[b] -= pull * dx / d;
            fy[b] -= pull * dy / d;
        }
        // Gentle gravity toward the center keeps loners on screen.
        for i in 0..n {
            fx[i] += (0.5 - nodes[i].x) * 0.05;
            fy[i] += (0.5 - nodes[i].y) * 0.05;
        }
        for i in 0..n {
            nodes[i].x = (nodes[i].x + fx[i].clamp(-1.0, 1.0) * step).clamp(0.0, 1.0);
            nodes[i].y = (nodes[i].y + fy[i].clamp(-1.0, 1.0) * step).clamp(0.0, 1.0);
        }
    }
}

/// A thin filled quad along a→b — `paint_path` fills, so an edge line
/// is a two-pixel-wide rectangle.
pub fn line_path(
    a: gpui::Point<gpui::Pixels>,
    b: gpui::Point<gpui::Pixels>,
    width: f32,
) -> gpui::Path<gpui::Pixels> {
    let dx = f32::from(b.x - a.x);
    let dy = f32::from(b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let nx = gpui::px(-dy / len * width / 2.0);
    let ny = gpui::px(dx / len * width / 2.0);
    let mut path = gpui::Path::new(gpui::point(a.x + nx, a.y + ny));
    path.line_to(gpui::point(b.x + nx, b.y + ny));
    path.line_to(gpui::point(b.x - nx, b.y - ny));
    path.line_to(gpui::point(a.x - nx, a.y - ny));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Index) {
        let dir = tempfile::tempdir().unwrap();
        let w = |p: &str, t: &str| std::fs::write(dir.path().join(p), t).unwrap();
        w("Hub.md", "links [[SpokeA]] and [[SpokeB]]\n");
        w("SpokeA.md", "back to [[Hub]]\n");
        w("SpokeB.md", "plain\n");
        w("Loner.md", "nothing\n");
        let index = Index::scan(dir.path());
        (dir, index)
    }

    #[test]
    fn build_collects_notes_and_deduped_edges() {
        let (_dir, index) = fixture();
        let (nodes, edges) = build(&index);
        assert_eq!(nodes.len(), 4);
        // Hub↔SpokeA (two links, one edge) + Hub→SpokeB.
        assert_eq!(edges.len(), 2);
        let hub = nodes.iter().position(|n| n.path.ends_with("Hub.md")).unwrap();
        assert_eq!(nodes[hub].degree, 3, "two out + one in");
        let loner = nodes.iter().position(|n| n.path.ends_with("Loner.md")).unwrap();
        assert_eq!(nodes[loner].degree, 0);
    }

    #[test]
    fn local_rings_the_neighborhood_around_the_center() {
        let (dir, index) = fixture();
        let (nodes, edges) = local(&index, &dir.path().join("SpokeA.md"));
        // SpokeA + Hub (both directions collapse to one neighbor).
        assert_eq!(nodes.len(), 2);
        assert_eq!(edges.len(), 1);
        let center = &nodes[0];
        assert!(center.path.ends_with("SpokeA.md"));
        assert!((center.x - 0.5).abs() < 1e-6 && (center.y - 0.5).abs() < 1e-6);
        let neighbor = &nodes[1];
        let d = ((neighbor.x - 0.5).powi(2) + (neighbor.y - 0.5).powi(2)).sqrt();
        assert!((0.1..0.5).contains(&d), "neighbor on the ring: {d}");
    }

    /// The simulation cools to rest on its own, so the shell knows
    /// when to stop redrawing. A layout that never settles would keep
    /// the GPU busy forever on an idle window.
    #[test]
    fn a_simulation_cools_to_rest() {
        let (_d, index) = fixture();
        let (nodes, edges) = build(&index);
        let mut sim = Simulation::new(nodes, edges);
        assert!(!sim.settled(), "starts hot");
        sim.run(2000);
        assert!(sim.settled(), "reaches rest, alpha {}", sim.alpha());
    }

    /// Reheating is what makes an interaction feel alive: the layout
    /// starts moving again rather than snapping to a new still frame.
    #[test]
    fn reheating_restarts_the_motion() {
        let (_d, index) = fixture();
        let (nodes, edges) = build(&index);
        let mut sim = Simulation::new(nodes, edges);
        sim.run(2000);
        assert!(sim.settled());
        sim.reheat(0.5);
        assert!(!sim.settled(), "a nudge puts it back in motion");
    }

    /// Momentum: a node keeps moving for a tick or two after the force
    /// that started it, instead of stopping dead.
    #[test]
    fn nodes_carry_velocity_between_ticks() {
        let (_d, index) = fixture();
        let (nodes, edges) = build(&index);
        let mut sim = Simulation::new(nodes, edges);
        sim.step();
        assert!(
            sim.nodes.iter().any(|n| n.vx.abs() > 0.0 || n.vy.abs() > 0.0),
            "a tick leaves the graph in motion"
        );
    }

    /// A pinned node is where the pointer put it and stays there, while
    /// everything else reacts around it.
    #[test]
    fn a_pinned_node_stays_put_while_its_neighbours_move() {
        let (_d, index) = fixture();
        let (nodes, edges) = build(&index);
        let mut sim = Simulation::new(nodes, edges);
        sim.pin(0, 0.2, 0.8);
        let others: Vec<(f32, f32)> = sim.nodes[1..].iter().map(|n| (n.x, n.y)).collect();
        for _ in 0..50 {
            sim.step();
        }
        assert_eq!((sim.nodes[0].x, sim.nodes[0].y), (0.2, 0.8), "the held node did not drift");
        let moved = sim.nodes[1..]
            .iter()
            .zip(&others)
            .any(|(n, (x, y))| (n.x - x).abs() > 1e-4 || (n.y - y).abs() > 1e-4);
        assert!(moved, "the rest of the graph responded to it");

        sim.release(0);
        sim.reheat(1.0);
        for _ in 0..50 {
            sim.step();
        }
        assert!(
            (sim.nodes[0].x - 0.2).abs() > 1e-4 || (sim.nodes[0].y - 0.8).abs() > 1e-4,
            "released, it rejoins the layout"
        );
    }

    /// Stepping is deterministic, like the old fixed-iteration layout:
    /// the same vault draws the same frames every run.
    #[test]
    fn stepping_is_deterministic() {
        let (_d, index) = fixture();
        let run = || {
            let (nodes, edges) = build(&index);
            let mut sim = Simulation::new(nodes, edges);
            sim.run(200);
            sim.nodes.iter().map(|n| (n.x, n.y)).collect::<Vec<_>>()
        };
        assert_eq!(run(), run(), "two runs, identical frames");
    }

    /// Holding warm keeps the layout responsive under a drag instead of
    /// cooling to a standstill while the pointer is still moving.
    #[test]
    fn holding_warm_prevents_settling() {
        let (_d, index) = fixture();
        let (nodes, edges) = build(&index);
        let mut sim = Simulation::new(nodes, edges);
        sim.hold_warm(true);
        sim.run(5000);
        assert!(!sim.settled(), "stays warm while held");
        sim.hold_warm(false);
        sim.run(5000);
        assert!(sim.settled(), "and cools once released");
    }

    #[test]
    fn layout_is_deterministic_and_pulls_linked_nodes_together() {
        let (_dir, index) = fixture();
        let (mut a, edges) = build(&index);
        let mut b = a.clone();
        layout(&mut a, &edges, 60);
        layout(&mut b, &edges, 60);
        for (na, nb) in a.iter().zip(&b) {
            assert_eq!((na.x, na.y), (nb.x, nb.y), "two runs, same frame");
        }
        let pos = |name: &str| {
            a.iter()
                .find(|n| n.path.ends_with(name))
                .map(|n| (n.x, n.y))
                .unwrap()
        };
        let d = |p: (f32, f32), q: (f32, f32)| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
        let hub = pos("Hub.md");
        assert!(
            d(hub, pos("SpokeA.md")) < d(pos("SpokeA.md"), pos("Loner.md")),
            "linked nodes sit closer than strangers"
        );
        // Everything stays inside the unit square.
        for n in &a {
            assert!((0.0..=1.0).contains(&n.x) && (0.0..=1.0).contains(&n.y));
        }
    }
}
