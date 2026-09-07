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
    /// Top-level folder, and first tag, for colour grouping. Resolved
    /// at build time because the renderer has no index to ask.
    pub folder: Option<String>,
    pub tag: Option<String>,
    /// A note that does not exist yet — something links to it. Drawn
    /// hollow, and clicking it creates the note.
    pub ghost: bool,
}

/// Indexes into the node list: from → to, deduplicated per direction.
///
/// Direction used to be discarded (`a.min(b), a.max(b)`), which threw
/// away exactly what backlinks are about — "A links to B" and "B links
/// to A" were indistinguishable. A pair linked both ways is one edge
/// with `both` set, so it draws a single line with two arrowheads.
pub type GraphEdge = (usize, usize);

/// An edge and whether it is reciprocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub both: bool,
}

/// Every note and every resolved link in the workspace.
pub fn build(index: &Index) -> (Vec<GraphNode>, Vec<Edge>) {
    let names = index.note_names();
    let mut nodes: Vec<GraphNode> = names
        .iter()
        .enumerate()
        .map(|(ix, (_, path))| {
            // Deterministic seed positions on a circle, by index.
            let angle = ix as f32 / names.len().max(1) as f32 * std::f32::consts::TAU;
            let tags = index.note_tags(path);
            GraphNode {
                path: path.clone(),
                x: 0.5 + 0.35 * angle.cos(),
                y: 0.5 + 0.35 * angle.sin(),
                vx: 0.0,
                vy: 0.0,
                pinned: false,
                degree: 0,
                folder: group_key(path, &index.root, &tags, ColorBy::Folder),
                tag: group_key(path, &index.root, &tags, ColorBy::Tag),
                ghost: false,
            }
        })
        .collect();
    let index_of: std::collections::BTreeMap<PathBuf, usize> = nodes
        .iter()
        .enumerate()
        .map(|(ix, n)| (n.path.clone(), ix))
        .collect();
    let mut edges: Vec<Edge> = Vec::new();
    for (from, to) in index.edges() {
        let (Some(&a), Some(&b)) = (index_of.get(&from), index_of.get(&to)) else {
            continue;
        };
        if a == b {
            continue; // a note linking to itself is not a relationship
        }
        nodes[a].degree += 1;
        nodes[b].degree += 1;
        // Already have this exact direction? Nothing to add. Have the
        // opposite? Mark it reciprocated rather than drawing twice.
        if edges.iter().any(|e| e.from == a && e.to == b) {
            continue;
        }
        if let Some(back) = edges.iter_mut().find(|e| e.from == b && e.to == a) {
            back.both = true;
            continue;
        }
        edges.push(Edge { from: a, to: b, both: false });
    }
    (nodes, edges)
}

/// The one-hop neighborhood of `center`: outgoing links + backlinks.
pub fn local(index: &Index, center: &Path) -> (Vec<GraphNode>, Vec<Edge>) {
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
        folder: None,
        tag: None,
        ghost: false,
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
            folder: None,
            tag: None,
            ghost: false,
        });
    }
    let edges =
        (1..nodes.len()).map(|ix| Edge { from: 0, to: ix, both: false }).collect();
    (nodes, edges)
}

/// Deterministic force layout over the unit square: springs along
/// edges, repulsion between all pairs, `iterations` rounds.
/// A quadtree over node positions, for approximating repulsion.
///
/// Exact repulsion compares every pair: 2,000 notes is two million
/// comparisons *per tick*, and the simulation now ticks every frame.
/// Barnes-Hut groups distant nodes into their centre of mass and
/// treats each group as one body, which turns that into roughly
/// n log n.
struct QuadTree {
    cells: Vec<Cell>,
}

#[derive(Clone, Copy)]
struct Cell {
    /// Bounds: origin and side length (always square).
    x: f32,
    y: f32,
    size: f32,
    /// Centre of mass and how many bodies are in it.
    cx: f32,
    cy: f32,
    mass: f32,
    /// A leaf holds one body; a branch holds four child cells.
    body: Option<(f32, f32)>,
    children: Option<[usize; 4]>,
}

/// Opening angle. A cell is treated as a single body when its size
/// over its distance falls below this. 0.9 is d3-force's default:
/// higher is faster and cruder, 0 degenerates to the exact sum.
const THETA: f32 = 0.9;

/// Below this, the exact all-pairs sum is cheaper than building a
/// tree — and it keeps small graphs bit-identical to how they laid
/// out before, so existing layouts do not shift.
const BARNES_HUT_THRESHOLD: usize = 64;

impl QuadTree {
    fn build(points: &[(f32, f32)]) -> Self {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max = f32::MIN;
        for &(x, y) in points {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max = max.max(x).max(y);
        }
        let size = (max - min_x.min(min_y)).max(1e-3);
        let mut tree = Self {
            cells: vec![Cell {
                x: min_x,
                y: min_y,
                size,
                cx: 0.0,
                cy: 0.0,
                mass: 0.0,
                body: None,
                children: None,
            }],
        };
        for &p in points {
            tree.insert(0, p, 0);
        }
        tree
    }

    fn insert(&mut self, cell: usize, p: (f32, f32), depth: usize) {
        // Coincident points would subdivide forever; past this depth
        // they simply share a cell, which is harmless — the repulsion
        // that pushes them apart is computed from the centre of mass.
        const MAX_DEPTH: usize = 24;
        self.cells[cell].mass += 1.0;
        self.cells[cell].cx += p.0;
        self.cells[cell].cy += p.1;
        if depth >= MAX_DEPTH {
            return;
        }
        match (self.cells[cell].body, self.cells[cell].children) {
            // Empty leaf: it holds this body now.
            (None, None) if self.cells[cell].mass <= 1.0 => {
                self.cells[cell].body = Some(p);
            }
            // Occupied leaf: split, and push both bodies down.
            (Some(existing), None) => {
                self.cells[cell].body = None;
                self.subdivide(cell);
                self.push_down(cell, existing, depth);
                self.push_down(cell, p, depth);
            }
            // Branch, or a leaf that filled at max depth.
            _ => {
                if self.cells[cell].children.is_none() {
                    self.subdivide(cell);
                }
                self.push_down(cell, p, depth);
            }
        }
    }

    fn subdivide(&mut self, cell: usize) {
        let c = self.cells[cell];
        let h = c.size / 2.0;
        let base = self.cells.len();
        for (dx, dy) in [(0.0, 0.0), (h, 0.0), (0.0, h), (h, h)] {
            self.cells.push(Cell {
                x: c.x + dx,
                y: c.y + dy,
                size: h,
                cx: 0.0,
                cy: 0.0,
                mass: 0.0,
                body: None,
                children: None,
            });
        }
        self.cells[cell].children = Some([base, base + 1, base + 2, base + 3]);
    }

    fn push_down(&mut self, cell: usize, p: (f32, f32), depth: usize) {
        let c = self.cells[cell];
        let Some(children) = c.children else { return };
        let h = c.size / 2.0;
        let ix = usize::from(p.0 >= c.x + h) + 2 * usize::from(p.1 >= c.y + h);
        self.insert(children[ix], p, depth + 1);
    }

    /// Repulsion on a point from every body in the tree.
    fn force_on(&self, cell: usize, p: (f32, f32), repel: f32) -> (f32, f32) {
        let c = self.cells[cell];
        if c.mass == 0.0 {
            return (0.0, 0.0);
        }
        let (mx, my) = (c.cx / c.mass, c.cy / c.mass);
        let dx = p.0 - mx;
        let dy = p.1 - my;
        let d2 = dx * dx + dy * dy;
        // A cell holding `p` is never approximated. With theta at 0.9 a
        // containing cell *can* satisfy the opening criterion — its
        // centre of mass may sit toward the far corner, far enough that
        // size/d falls below theta — and lumping it in would have `p`
        // repel itself. That produced errors larger than the forces.
        let inside = p.0 >= c.x
            && p.0 <= c.x + c.size
            && p.1 >= c.y
            && p.1 <= c.y + c.size;
        // Far enough to treat the whole cell as one body — or a leaf,
        // which already is one.
        if !inside && (c.children.is_none() || c.size * c.size < THETA * THETA * d2) {
            if d2 <= 1e-8 {
                return (0.0, 0.0); // itself, or a coincident node
            }
            let d2 = d2.max(1e-4);
            let d = d2.sqrt();
            let f = repel * c.mass / d2;
            return (f * dx / d, f * dy / d);
        }
        // A leaf containing p: its only body is p itself (or, at max
        // depth, bodies coincident with it), so it exerts nothing.
        let Some(children) = c.children else {
            return (0.0, 0.0);
        };
        let mut fx = 0.0;
        let mut fy = 0.0;
        for &child in children.iter() {
            let (cfx, cfy) = self.force_on(child, p, repel);
            fx += cfx;
            fy += cfy;
        }
        (fx, fy)
    }
}

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
    pub edges: Vec<Edge>,
    pub forces: Forces,
    alpha: f32,
    alpha_target: f32,
}

/// Below this the layout is at rest and the shell can stop stepping.
pub const ALPHA_REST: f32 = 0.005;
/// Fraction of the remaining heat lost per tick.
const ALPHA_DECAY: f32 = 0.0228;

impl Simulation {
    pub fn new(nodes: Vec<GraphNode>, edges: Vec<Edge>) -> Self {
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

        if n >= BARNES_HUT_THRESHOLD {
            // Approximate: group distant nodes by centre of mass. The
            // exact sum below is O(n^2) per tick, and this now ticks
            // every frame — 2,000 notes would not survive it.
            let points: Vec<(f32, f32)> =
                self.nodes.iter().map(|nd| (nd.x, nd.y)).collect();
            let tree = QuadTree::build(&points);
            let forces: Vec<(f32, f32)> = points
                .iter()
                .map(|&p| tree.force_on(0, p, f.repel * a))
                .collect();
            for (node, (fx, fy)) in self.nodes.iter_mut().zip(forces) {
                node.vx += fx;
                node.vy += fy;
            }
        } else {
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
        }
        for &Edge { from: p, to: q, .. } in &self.edges {
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

pub fn layout(nodes: &mut [GraphNode], edges: &[Edge], iterations: usize) {
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
        for &Edge { from: a, to: b, .. } in edges {
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

/// Nodes within `depth` hops of `center`, as a set of indices.
///
/// Depth 1 is the note and what it touches; 2 adds their neighbours,
/// and so on. Used by local mode, which narrows the graph to the
/// neighbourhood of what you are reading instead of the whole vault.
pub fn within_depth(
    edges: &[Edge],
    center: usize,
    depth: usize,
) -> std::collections::BTreeSet<usize> {
    let mut seen = std::collections::BTreeSet::new();
    seen.insert(center);
    let mut frontier = vec![center];
    for _ in 0..depth {
        let mut next = Vec::new();
        for &ix in &frontier {
            for e in edges {
                // Links are followed both ways here: a note you link to
                // and a note that links to you are both neighbours.
                let other = if e.from == ix {
                    Some(e.to)
                } else if e.to == ix {
                    Some(e.from)
                } else {
                    None
                };
                if let Some(o) = other {
                    if seen.insert(o) {
                        next.push(o);
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    seen
}

/// Ghost nodes: link targets that resolve to nothing.
///
/// A vault's unwritten notes are a to-write list, and the graph is
/// where they are most visible — but `build` only ever made nodes for
/// files that exist, so a `[[Ghost]]` referenced ten times was
/// invisible.
pub fn with_ghosts(
    index: &Index,
    nodes: &mut Vec<GraphNode>,
    edges: &mut Vec<Edge>,
) {
    let mut ghost_ix: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    // Sorted, so ghost nodes land in the same order every run.
    let sources: Vec<PathBuf> = index.note_names().iter().map(|(_, p)| p.clone()).collect();
    for source in sources {
        let Some(from) = nodes.iter().position(|n| n.path == source) else {
            continue;
        };
        for link in index.unresolved_links(&source) {
            let ix = *ghost_ix.entry(link.clone()).or_insert_with(|| {
                nodes.push(GraphNode {
                    // Not a real path: nothing opens it, and the name
                    // is what the link asked for.
                    path: PathBuf::from(format!("{link}.md")),
                    x: 0.5,
                    y: 0.5,
                    vx: 0.0,
                    vy: 0.0,
                    pinned: false,
                    degree: 0,
                    folder: None,
                    tag: None,
                    ghost: true,
                });
                nodes.len() - 1
            });
            nodes[from].degree += 1;
            nodes[ix].degree += 1;
            if !edges.iter().any(|e| e.from == from && e.to == ix) {
                edges.push(Edge { from, to: ix, both: false });
            }
        }
    }
}

/// Which nodes the view is showing.
///
/// A filter never removes nodes from the simulation — the layout would
/// jump every time you typed a character. It marks them, and the
/// renderer fades what does not match, so the shape you were reading
/// stays put while the matches light up.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Case-insensitive substring of the note's name.
    pub query: String,
    /// Only notes carrying this tag.
    pub tag: Option<String>,
    /// Only notes under this top-level folder.
    pub folder: Option<String>,
    /// Only notes with no links at all, in or out.
    pub orphans_only: bool,
    /// Only notes within this many hops of a centre. `None` shows the
    /// whole vault.
    pub local: Option<(usize, usize)>,
    /// Resolved from `local` when the filter is applied, so `matches`
    /// stays a cheap lookup rather than a graph walk per node.
    pub in_scope: Option<std::collections::BTreeSet<usize>>,
}

impl Filter {
    /// Nothing is being filtered, so everything is a match.
    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
            && self.tag.is_none()
            && self.folder.is_none()
            && !self.orphans_only
            && self.local.is_none()
    }

    /// Work out which nodes are in local scope. Called when the centre
    /// or depth changes, not per node.
    pub fn resolve_scope(&mut self, edges: &[Edge]) {
        self.in_scope = self
            .local
            .map(|(center, depth)| within_depth(edges, center, depth));
    }

    /// Index-aware match. `matches` alone cannot answer local scope,
    /// which is about position in the graph rather than the node.
    pub fn matches_at(&self, ix: usize, node: &GraphNode) -> bool {
        if let Some(scope) = &self.in_scope {
            if !scope.contains(&ix) {
                return false;
            }
        }
        self.matches(node)
    }

    pub fn matches(&self, node: &GraphNode) -> bool {
        if self.orphans_only && node.degree > 0 {
            return false;
        }
        if let Some(tag) = &self.tag {
            if node.tag.as_deref() != Some(tag.as_str()) {
                return false;
            }
        }
        if let Some(folder) = &self.folder {
            if node.folder.as_deref() != Some(folder.as_str()) {
                return false;
            }
        }
        if !self.query.is_empty() {
            let name = node
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if !name.contains(&self.query.to_ascii_lowercase()) {
                return false;
            }
        }
        true
    }
}

/// How a node is grouped, for colouring.
///
/// Obsidian calls these colour groups. Folder is the useful default:
/// it matches how people already organise, and needs no configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBy {
    /// Every node the same — the original look.
    None,
    /// The note's top-level folder inside the vault.
    Folder,
    /// The note's first tag.
    Tag,
}

/// A stable colour slot for a node, or None to use the default.
///
/// Returns an index into the caller's palette rather than a colour, so
/// this stays pure and the theme keeps deciding what things look like.
/// Slots are assigned by first appearance in `keys`, which is sorted,
/// so the same vault colours the same way every time.
pub fn color_slot(key: Option<&str>, keys: &[String], palette_len: usize) -> Option<usize> {
    if palette_len == 0 {
        return None;
    }
    let key = key?;
    let ix = keys.iter().position(|k| k == key)?;
    Some(ix % palette_len)
}

/// The grouping key for a node: its top-level folder under `root`, or
/// its first tag. None when the note is at the vault root, or untagged.
pub fn group_key(
    path: &Path,
    root: &Path,
    tags: &[String],
    by: ColorBy,
) -> Option<String> {
    match by {
        ColorBy::None => None,
        ColorBy::Folder => path
            .strip_prefix(root)
            .ok()?
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            // A file directly in the vault root has only its own name
            // as a component, which is not a folder.
            .filter(|first| path.strip_prefix(root).map(|r| r.components().count() > 1).unwrap_or(false) && !first.is_empty()),
        ColorBy::Tag => tags.first().cloned(),
    }
}

/// Labels crowd into illegibility when zoomed out. Below this they are
/// hidden; between here and `LABEL_FULL` they fade in.
pub const LABEL_FADE_START: f32 = 0.55;
pub const LABEL_FULL: f32 = 0.85;

/// Label opacity at a given zoom: 0 hidden, 1 fully drawn.
pub fn label_opacity(zoom: f32) -> f32 {
    if zoom <= LABEL_FADE_START {
        return 0.0;
    }
    if zoom >= LABEL_FULL {
        return 1.0;
    }
    (zoom - LABEL_FADE_START) / (LABEL_FULL - LABEL_FADE_START)
}

/// The bounding box of a layout, as (min_x, min_y, max_x, max_y).
/// Empty layouts give the unit square, so callers need no special case.
pub fn bounds(nodes: &[GraphNode]) -> (f32, f32, f32, f32) {
    if nodes.is_empty() {
        return (0.0, 0.0, 1.0, 1.0);
    }
    let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for n in nodes {
        b.0 = b.0.min(n.x);
        b.1 = b.1.min(n.y);
        b.2 = b.2.max(n.x);
        b.3 = b.3.max(n.y);
    }
    b
}

/// Zoom and pan that fit `nodes` into a `viewport` of pixels, with a
/// margin. Returns (zoom, pan_x, pan_y) for the same transform
/// `render_graph` uses: `screen = pan + unit * 900 * zoom + 60`.
///
/// Without this there was no way back once you panned away from the
/// graph — the only recovery was closing and reopening it.
pub fn fit_to(
    nodes: &[GraphNode],
    viewport: (f32, f32),
    margin: f32,
) -> (f32, f32, f32) {
    let (x0, y0, x1, y1) = bounds(nodes);
    let (w, h) = ((x1 - x0).max(1e-3), (y1 - y0).max(1e-3));
    let avail_w = (viewport.0 - margin * 2.0).max(1.0);
    let avail_h = (viewport.1 - margin * 2.0).max(1.0);
    // `base` is 900 * zoom, and the layout spans `w` of unit space.
    let zoom = ((avail_w / (w * 900.0)).min(avail_h / (h * 900.0))).clamp(0.1, 3.0);
    let base = 900.0 * zoom;
    // Centre what is drawn, then undo the fixed 60px board offset.
    let pan_x = (viewport.0 - w * base) / 2.0 - x0 * base - 60.0;
    let pan_y = (viewport.1 - h * base) / 2.0 - y0 * base - 60.0;
    (zoom, pan_x, pan_y)
}

/// A filled triangle pointing along a→b, sitting `back` pixels short
/// of `b` so it lands beside the node rather than under it.
pub fn arrow_path(
    a: gpui::Point<gpui::Pixels>,
    b: gpui::Point<gpui::Pixels>,
    back: f32,
    size: f32,
) -> gpui::Path<gpui::Pixels> {
    let (dx, dy) = (f32::from(b.x - a.x), f32::from(b.y - a.y));
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / len, dy / len);
    // Tip, pulled back from the node it points at.
    let tip = gpui::point(
        gpui::px(f32::from(b.x) - ux * back),
        gpui::px(f32::from(b.y) - uy * back),
    );
    // Two base corners, perpendicular to the direction of travel.
    let (px_, py) = (-uy, ux);
    let base_x = f32::from(tip.x) - ux * size;
    let base_y = f32::from(tip.y) - uy * size;
    let half = size * 0.45;
    let mut path = gpui::Path::new(tip);
    path.line_to(gpui::point(
        gpui::px(base_x + px_ * half),
        gpui::px(base_y + py * half),
    ));
    path.line_to(gpui::point(
        gpui::px(base_x - px_ * half),
        gpui::px(base_y - py * half),
    ));
    path.line_to(tip);
    path
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

    /// A synthetic graph of `n` notes in a chain, for scale tests.
    fn chain(n: usize) -> (Vec<GraphNode>, Vec<Edge>) {
        let nodes = (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                GraphNode {
                    path: PathBuf::from(format!("n{i}.md")),
                    x: 0.5 + 0.35 * a.cos(),
                    y: 0.5 + 0.35 * a.sin(),
                    vx: 0.0,
                    vy: 0.0,
                    pinned: false,
                    degree: 2,
                    folder: None,
                    tag: None,
                    ghost: false,
                }
            })
            .collect();
        let edges = (0..n.saturating_sub(1))
            .map(|i| Edge { from: i, to: i + 1, both: false })
            .collect();
        (nodes, edges)
    }

    /// The approximation has to agree with the exact sum, or large
    /// vaults would lay out differently from small ones for no reason
    /// the reader could see.
    /// Deterministic scatter. A ring will not do: on a symmetric layout
    /// every node's repulsion nearly cancels, so error measured against
    /// the surviving net is meaningless.
    fn scatter(n: usize) -> Vec<(f32, f32)> {
        let mut seed = 12345u32;
        let mut next = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        (0..n).map(|_| (0.05 + 0.9 * next(), 0.05 + 0.9 * next())).collect()
    }

    #[test]
    fn barnes_hut_approximates_the_exact_repulsion() {
        let points = scatter(300);
        let tree = QuadTree::build(&points);
        let repel = 0.004;

        let mut worst: f32 = 0.0;
        let mut typical: f32 = 0.0;
        for (i, &p) in points.iter().enumerate() {
            // Exact: sum over every other body.
            let (mut ex, mut ey) = (0.0f32, 0.0f32);
            for (j, &q) in points.iter().enumerate() {
                if i == j {
                    continue;
                }
                let (dx, dy) = (p.0 - q.0, p.1 - q.1);
                let d2 = (dx * dx + dy * dy).max(1e-4);
                let d = d2.sqrt();
                ex += repel / d2 * dx / d;
                ey += repel / d2 * dy / d;
            }
            let (ax, ay) = tree.force_on(0, p, repel);
            worst = worst.max(((ax - ex).powi(2) + (ay - ey).powi(2)).sqrt());
            typical = typical.max((ex * ex + ey * ey).sqrt());
        }
        // Absolute error against the largest force in the system. A
        // ratio against each node's own net is meaningless on a
        // symmetric layout, where that net nearly cancels to zero.
        assert!(
            worst < typical * 0.15,
            "worst error {worst} against a typical force of {typical}"
        );
    }

    /// The tree must survive the shapes that break naive quadtrees:
    /// every node in one place, and a single node.
    #[test]
    fn the_quadtree_handles_degenerate_layouts() {
        let coincident = vec![(0.5, 0.5); 40];
        let tree = QuadTree::build(&coincident);
        let (fx, fy) = tree.force_on(0, (0.5, 0.5), 0.004);
        assert!(fx.is_finite() && fy.is_finite(), "no NaN from coincident nodes");

        let single = vec![(0.25, 0.75)];
        let tree = QuadTree::build(&single);
        let (fx, fy) = tree.force_on(0, (0.25, 0.75), 0.004);
        assert_eq!((fx, fy), (0.0, 0.0), "a node does not repel itself");
    }

    /// The whole point: a vault far larger than the exact sum could
    /// handle still steps, and still settles.
    #[test]
    fn a_large_graph_still_steps_and_settles() {
        let (nodes, edges) = chain(1200);
        let mut sim = Simulation::new(nodes, edges);
        assert!(sim.nodes.len() >= BARNES_HUT_THRESHOLD, "uses the approximation");
        for _ in 0..120 {
            sim.step();
        }
        assert!(
            sim.nodes.iter().all(|n| n.x.is_finite() && n.y.is_finite()),
            "no node escaped to infinity"
        );
        assert!(sim.alpha() < 1.0, "it is cooling");
    }

    /// Direction is what backlinks are about, and it used to be thrown
    /// away: edges were stored as (min, max), so "A links to B" and "B
    /// links to A" were the same edge.
    #[test]
    fn edges_keep_their_direction() {
        let (_d, index) = fixture();
        let (nodes, edges) = build(&index);
        let ix = |name: &str| {
            nodes.iter().position(|n| n.path.ends_with(name)).expect(name)
        };
        let (hub, a, b) = (ix("Hub.md"), ix("SpokeA.md"), ix("SpokeB.md"));

        // Hub links to SpokeB and SpokeB does not link back.
        let one_way = edges
            .iter()
            .find(|e| (e.from == hub && e.to == b) || (e.from == b && e.to == hub))
            .expect("hub -> SpokeB exists");
        assert_eq!(one_way.from, hub, "the edge runs out of the note that wrote it");
        assert_eq!(one_way.to, b);
        assert!(!one_way.both, "SpokeB never links back");

        // Hub and SpokeA link to each other.
        let mutual = edges
            .iter()
            .find(|e| (e.from == hub && e.to == a) || (e.from == a && e.to == hub))
            .expect("hub <-> SpokeA exists");
        assert!(mutual.both, "a mutual pair is marked reciprocated");
    }

    /// A pair that links both ways is one edge with two arrowheads, not
    /// two lines drawn over each other.
    #[test]
    fn a_mutual_link_is_one_reciprocated_edge() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.md"), "see [[B]]\n").unwrap();
        std::fs::write(dir.path().join("B.md"), "see [[A]]\n").unwrap();
        let index = Index::scan(dir.path());
        let (_nodes, edges) = build(&index);
        assert_eq!(edges.len(), 1, "one edge, not two: {edges:?}");
        assert!(edges[0].both, "marked as going both ways");
    }

    /// A note linking to itself is not a relationship worth drawing.
    #[test]
    fn a_self_link_makes_no_edge() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.md"), "see [[A]] again\n").unwrap();
        let index = Index::scan(dir.path());
        let (_nodes, edges) = build(&index);
        assert!(edges.is_empty(), "no self edge: {edges:?}");
    }

    fn node_named(name: &str, degree: usize, folder: Option<&str>, tag: Option<&str>) -> GraphNode {
        GraphNode {
            path: PathBuf::from(format!("/v/{name}.md")),
            x: 0.5,
            y: 0.5,
            vx: 0.0,
            vy: 0.0,
            pinned: false,
            degree,
            folder: folder.map(str::to_string),
            tag: tag.map(str::to_string),
            ghost: false,
        }
    }

    /// A vault's unwritten notes are a to-write list, and the graph is
    /// where they show. `build` only made nodes for files that exist,
    /// so a `[[Ghost]]` referenced anywhere was simply invisible.
    #[test]
    fn unresolved_links_become_ghost_nodes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.md"), "see [[Ghost]] and [[B]]\n").unwrap();
        std::fs::write(dir.path().join("B.md"), "also [[Ghost]]\n").unwrap();
        let index = Index::scan(dir.path());
        let (mut nodes, mut edges) = build(&index);
        assert_eq!(nodes.len(), 2, "only real notes to begin with");

        with_ghosts(&index, &mut nodes, &mut edges);
        let ghosts: Vec<&GraphNode> = nodes.iter().filter(|n| n.ghost).collect();
        assert_eq!(ghosts.len(), 1, "both references share one ghost");
        assert!(ghosts[0].path.ends_with("Ghost.md"));
        assert_eq!(ghosts[0].degree, 2, "linked from both notes");
    }

    /// Only wiki links: a relative path that does not resolve is a
    /// typo, not a note somebody means to write.
    #[test]
    fn a_broken_relative_link_is_not_a_ghost() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.md"), "see [x](nope.md)\n").unwrap();
        let index = Index::scan(dir.path());
        let (mut nodes, mut edges) = build(&index);
        with_ghosts(&index, &mut nodes, &mut edges);
        assert!(!nodes.iter().any(|n| n.ghost), "no ghost for a broken path");
    }

    #[test]
    fn depth_walks_out_hop_by_hop() {
        // 0 - 1 - 2 - 3, plus an unconnected 4.
        let edges = vec![
            Edge { from: 0, to: 1, both: false },
            Edge { from: 1, to: 2, both: false },
            Edge { from: 2, to: 3, both: false },
        ];
        let set = |d| within_depth(&edges, 0, d).into_iter().collect::<Vec<_>>();
        assert_eq!(set(0), vec![0], "depth 0 is the note alone");
        assert_eq!(set(1), vec![0, 1]);
        assert_eq!(set(2), vec![0, 1, 2]);
        assert_eq!(set(3), vec![0, 1, 2, 3]);
        assert_eq!(set(9), vec![0, 1, 2, 3], "past the end it stops growing");
    }

    /// Backlinks are neighbours too: a note that links *to* you is one
    /// hop away, not unreachable.
    #[test]
    fn depth_follows_links_in_both_directions() {
        let edges = vec![Edge { from: 1, to: 0, both: false }];
        assert!(within_depth(&edges, 0, 1).contains(&1), "the note linking in is a neighbour");
    }

    /// A cycle must terminate rather than walking forever.
    #[test]
    fn depth_terminates_on_a_cycle() {
        let edges = vec![
            Edge { from: 0, to: 1, both: false },
            Edge { from: 1, to: 2, both: false },
            Edge { from: 2, to: 0, both: false },
        ];
        assert_eq!(within_depth(&edges, 0, 50).len(), 3);
    }

    #[test]
    fn local_scope_narrows_the_filter() {
        let edges = vec![Edge { from: 0, to: 1, both: false }];
        let mut f = Filter { local: Some((0, 1)), ..Default::default() };
        assert!(!f.is_empty());
        f.resolve_scope(&edges);
        let n = node_named("x", 1, None, None);
        assert!(f.matches_at(0, &n), "the centre is in scope");
        assert!(f.matches_at(1, &n), "and its neighbour");
        assert!(!f.matches_at(7, &n), "something two hops out is not");
    }

    #[test]
    fn an_empty_filter_matches_everything() {
        let f = Filter::default();
        assert!(f.is_empty());
        assert!(f.matches(&node_named("Anything", 0, None, None)));
    }

    #[test]
    fn the_query_matches_a_name_case_insensitively() {
        let f = Filter { query: "read".into(), ..Default::default() };
        assert!(f.matches(&node_named("Reading list", 2, None, None)));
        assert!(f.matches(&node_named("UNREADABLE", 2, None, None)), "substring, any case");
        assert!(!f.matches(&node_named("Editing", 2, None, None)));
    }

    /// Finding notes nothing links to is one of the genuinely useful
    /// things a graph does, and there was no way to ask for it.
    #[test]
    fn orphans_only_keeps_the_unlinked() {
        let f = Filter { orphans_only: true, ..Default::default() };
        assert!(f.matches(&node_named("Loner", 0, None, None)));
        assert!(!f.matches(&node_named("Hub", 3, None, None)));
    }

    #[test]
    fn tag_and_folder_filters_are_exact_and_combine() {
        let f = Filter {
            folder: Some("Guide".into()),
            tag: Some("links".into()),
            ..Default::default()
        };
        assert!(f.matches(&node_named("A", 1, Some("Guide"), Some("links"))));
        assert!(!f.matches(&node_named("B", 1, Some("Notes"), Some("links"))), "wrong folder");
        assert!(!f.matches(&node_named("C", 1, Some("Guide"), Some("guide"))), "wrong tag");
        assert!(!f.matches(&node_named("D", 1, None, None)), "ungrouped is not a match");
    }

    /// Every criterion has to hold at once, not any of them.
    #[test]
    fn criteria_are_conjunctive() {
        let f = Filter {
            query: "read".into(),
            orphans_only: true,
            ..Default::default()
        };
        assert!(f.matches(&node_named("Reading list", 0, None, None)));
        assert!(!f.matches(&node_named("Reading list", 4, None, None)), "matches name, not orphan");
        assert!(!f.matches(&node_named("Loner", 0, None, None)), "orphan, wrong name");
    }

    #[test]
    fn labels_fade_in_rather_than_snapping() {
        assert_eq!(label_opacity(0.3), 0.0, "hidden when zoomed out");
        assert_eq!(label_opacity(1.5), 1.0, "solid when zoomed in");
        let mid = label_opacity((LABEL_FADE_START + LABEL_FULL) / 2.0);
        assert!((mid - 0.5).abs() < 1e-5, "halfway through the fade: {mid}");
        // Monotonic, so a slow zoom never flickers.
        let mut last = -1.0;
        for i in 0..=40 {
            let o = label_opacity(i as f32 / 20.0);
            assert!(o >= last, "opacity went backwards at zoom {}", i as f32 / 20.0);
            last = o;
        }
    }

    #[test]
    fn folder_grouping_uses_the_top_level_folder() {
        let root = Path::new("/v");
        assert_eq!(
            group_key(Path::new("/v/Guide/Editing.md"), root, &[], ColorBy::Folder).as_deref(),
            Some("Guide")
        );
        assert_eq!(
            group_key(Path::new("/v/Notes/Daily/x.md"), root, &[], ColorBy::Folder).as_deref(),
            Some("Notes"),
            "the top level, not the deepest"
        );
        assert_eq!(
            group_key(Path::new("/v/README.md"), root, &[], ColorBy::Folder),
            None,
            "a note at the vault root is in no folder"
        );
    }

    #[test]
    fn tag_grouping_uses_the_first_tag() {
        let root = Path::new("/v");
        let tags = vec!["guide".to_string(), "links".to_string()];
        assert_eq!(
            group_key(Path::new("/v/a.md"), root, &tags, ColorBy::Tag).as_deref(),
            Some("guide")
        );
        assert_eq!(group_key(Path::new("/v/a.md"), root, &[], ColorBy::Tag), None);
        assert_eq!(group_key(Path::new("/v/a.md"), root, &tags, ColorBy::None), None);
    }

    /// Colours must be stable: the same vault paints the same way each
    /// time, or the graph looks different on every open for no reason.
    #[test]
    fn colour_slots_are_stable_and_wrap() {
        let keys: Vec<String> = ["Code", "Guide", "Notes"].iter().map(|s| s.to_string()).collect();
        assert_eq!(color_slot(Some("Guide"), &keys, 8), Some(1));
        assert_eq!(color_slot(Some("Guide"), &keys, 8), Some(1), "same answer twice");
        assert_eq!(color_slot(Some("Notes"), &keys, 2), Some(0), "wraps past the palette");
        assert_eq!(color_slot(None, &keys, 8), None);
        assert_eq!(color_slot(Some("Guide"), &keys, 0), None, "no palette, no colour");
        assert_eq!(color_slot(Some("Unknown"), &keys, 8), None);
    }

    #[test]
    fn bounds_cover_every_node() {
        let (_d, index) = fixture();
        let (nodes, _) = build(&index);
        let (x0, y0, x1, y1) = bounds(&nodes);
        assert!(nodes.iter().all(|n| n.x >= x0 && n.x <= x1 && n.y >= y0 && n.y <= y1));
        assert_eq!(bounds(&[]), (0.0, 0.0, 1.0, 1.0), "empty is the unit square");
    }

    /// Fitting puts every node on screen with room to spare. There was
    /// no way back from a pan before this: you closed the graph and
    /// reopened it.
    #[test]
    fn fitting_brings_every_node_on_screen() {
        let (_d, index) = fixture();
        let (mut nodes, edges) = build(&index);
        let mut sim = Simulation::new(std::mem::take(&mut nodes), edges);
        sim.run(300);
        let viewport = (1200.0, 800.0);
        let margin = 60.0;
        let (zoom, pan_x, pan_y) = fit_to(&sim.nodes, viewport, margin);
        for n in &sim.nodes {
            let sx = pan_x + n.x * 900.0 * zoom + 60.0;
            let sy = pan_y + n.y * 900.0 * zoom + 60.0;
            assert!(
                (0.0..=viewport.0).contains(&sx) && (0.0..=viewport.1).contains(&sy),
                "node at ({sx}, {sy}) is off a {viewport:?} screen"
            );
        }
    }

    /// A single node must not divide by a zero-sized layout.
    #[test]
    fn fitting_a_degenerate_layout_is_finite() {
        let one = vec![GraphNode {
            path: PathBuf::from("a.md"),
            x: 0.5,
            y: 0.5,
            vx: 0.0,
            vy: 0.0,
            pinned: false,
            degree: 0,
            folder: None,
            tag: None,
            ghost: false,
        }];
        let (zoom, px_, py) = fit_to(&one, (800.0, 600.0), 40.0);
        assert!(zoom.is_finite() && px_.is_finite() && py.is_finite());
        assert!(zoom > 0.0);
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
