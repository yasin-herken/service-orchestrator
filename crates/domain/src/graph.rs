//! Internal graph helpers shared by the workflow and workspace models.
//!
//! This module is crate-private. It exists so that cycle detection is defined
//! once and used by every aggregate that must be acyclic.

use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    White,
    Gray,
    Black,
}

/// Finds a cycle in a directed graph, if one exists.
///
/// `nodes` lists the vertices; `edges` maps a vertex to the vertices it points
/// at. Edges that reference unknown vertices are ignored. The returned vector
/// lists the vertices of one detected cycle, starting at the point where the
/// cycle was closed.
pub(crate) fn find_cycle(
    nodes: &[String],
    edges: &BTreeMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    let index: BTreeMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(position, node)| (node.as_str(), position))
        .collect();
    let adjacency: Vec<Vec<usize>> = nodes
        .iter()
        .map(|node| {
            edges
                .get(node)
                .map(|targets| {
                    targets
                        .iter()
                        .filter_map(|target| index.get(target.as_str()).copied())
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();

    let mut colors = vec![Color::White; nodes.len()];
    let mut path = Vec::new();

    for node in 0..nodes.len() {
        if colors[node] == Color::White {
            if let Some(cycle) = visit(node, &adjacency, &mut colors, &mut path, nodes) {
                return Some(cycle);
            }
        }
    }
    None
}

fn visit(
    node: usize,
    adjacency: &[Vec<usize>],
    colors: &mut [Color],
    path: &mut Vec<usize>,
    nodes: &[String],
) -> Option<Vec<String>> {
    colors[node] = Color::Gray;
    path.push(node);
    for &next in &adjacency[node] {
        match colors[next] {
            Color::Gray => {
                let start = path.iter().position(|&seen| seen == next).unwrap_or(0);
                return Some(
                    path[start..]
                        .iter()
                        .map(|&seen| nodes[seen].clone())
                        .collect(),
                );
            }
            Color::White => {
                if let Some(cycle) = visit(next, adjacency, colors, path, nodes) {
                    return Some(cycle);
                }
            }
            Color::Black => {}
        }
    }
    path.pop();
    colors[node] = Color::Black;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(node, targets)| {
                (
                    (*node).to_owned(),
                    targets.iter().map(|t| (*t).to_owned()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn returns_none_for_acyclic_graphs() {
        let nodes = ["a", "b", "c"].map(str::to_owned).to_vec();
        let graph = edges(&[("a", &["b"]), ("b", &["c"]), ("c", &[])]);
        assert!(find_cycle(&nodes, &graph).is_none());
    }

    #[test]
    fn detects_cycles() {
        let nodes = ["a", "b"].map(str::to_owned).to_vec();
        let graph = edges(&[("a", &["b"]), ("b", &["a"])]);
        let cycle = find_cycle(&nodes, &graph).unwrap();
        assert!(cycle.len() >= 2);
    }

    #[test]
    fn detects_self_cycles() {
        let nodes = ["a"].map(str::to_owned).to_vec();
        let graph = edges(&[("a", &["a"])]);
        assert_eq!(find_cycle(&nodes, &graph), Some(vec!["a".to_owned()]));
    }

    #[test]
    fn ignores_edges_to_unknown_nodes() {
        let nodes = ["a"].map(str::to_owned).to_vec();
        let graph = edges(&[("a", &["missing"])]);
        assert!(find_cycle(&nodes, &graph).is_none());
    }
}
