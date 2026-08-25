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
    let mut nodes = vec![GraphNode { path: center.to_path_buf(), x: 0.5, y: 0.5, degree: neighbors.len() }];
    let n = neighbors.len().max(1) as f32;
    for (ix, path) in neighbors.into_iter().enumerate() {
        let angle = ix as f32 / n * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        nodes.push(GraphNode {
            path,
            x: 0.5 + 0.32 * angle.cos(),
            y: 0.5 + 0.32 * angle.sin(),
            degree: 1,
        });
    }
    let edges = (1..nodes.len()).map(|ix| (0, ix)).collect();
    (nodes, edges)
}

/// Deterministic force layout over the unit square: springs along
/// edges, repulsion between all pairs, `iterations` rounds.
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
