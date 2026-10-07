//! Dependency graph helpers.
//!
//! A task may depend on other tasks, and a workflow submits many tasks at once.
//! Before any of them are scheduled the engine verifies that the submitted
//! graph is a DAG: unknown dependencies and cycles are rejected up front, which
//! is what prevents the scheduler from deadlocking on tasks that can never
//! become runnable.
//!
//! This is the engine's own graph helper. The domain has a private one for
//! workspace and workflow validation; the engine cannot reuse it and must not
//! depend on its internals, so cycle detection is implemented here for task
//! identifiers.

use std::collections::BTreeMap;

use service_orchestrator_domain::TaskId;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    White,
    Gray,
    Black,
}

/// Finds a cycle in the task dependency graph, if one exists.
///
/// `nodes` lists every task in the submission and `edges` maps a task to the
/// tasks it depends on. Edges to unknown nodes are ignored. The returned vector
/// lists the identifiers of one detected cycle, ordered from where the cycle
/// was closed.
pub(crate) fn find_cycle(
    nodes: &[TaskId],
    edges: &BTreeMap<TaskId, Vec<TaskId>>,
) -> Option<Vec<TaskId>> {
    let index: BTreeMap<&TaskId, usize> = nodes
        .iter()
        .enumerate()
        .map(|(position, node)| (node, position))
        .collect();
    let adjacency: Vec<Vec<usize>> = nodes
        .iter()
        .map(|node| {
            edges
                .get(node)
                .map(|targets| {
                    targets
                        .iter()
                        .filter_map(|target| index.get(target).copied())
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
    nodes: &[TaskId],
) -> Option<Vec<TaskId>> {
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

/// Validates the shape of a submitted dependency graph.
///
/// `existing` are identifiers already known to the engine; dependencies may
/// point at them as well as at nodes in the submission.
///
/// # Errors
///
/// Returns a [`TaskEngineError`](crate::TaskEngineError) if a task is
/// duplicated, if a dependency is neither in the submission nor already known,
/// or if the graph contains a cycle.
pub(crate) fn validate(
    nodes: &[TaskId],
    edges: &BTreeMap<TaskId, Vec<TaskId>>,
    existing: &dyn Fn(&TaskId) -> bool,
) -> Result<(), crate::TaskEngineError> {
    use crate::TaskEngineError;

    let mut seen = std::collections::BTreeSet::new();
    for node in nodes {
        if !seen.insert(node.clone()) {
            return Err(TaskEngineError::DuplicateTask(node.clone()));
        }
    }

    for node in nodes {
        if let Some(dependencies) = edges.get(node) {
            for dependency in dependencies {
                if !seen.contains(dependency) && !existing(dependency) {
                    return Err(TaskEngineError::DependencyNotFound {
                        task: node.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }
    }

    if let Some(cycle) = find_cycle(nodes, edges) {
        let nodes = cycle
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(TaskEngineError::DependencyCycle { nodes });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TaskEngineError;

    fn id(value: &str) -> TaskId {
        TaskId::new(value).unwrap()
    }

    fn edge_map(entries: &[(&str, &[&str])]) -> BTreeMap<TaskId, Vec<TaskId>> {
        entries
            .iter()
            .map(|(node, targets)| (id(node), targets.iter().map(|target| id(target)).collect()))
            .collect()
    }

    fn no_existing(_: &TaskId) -> bool {
        false
    }

    #[test]
    fn accepts_an_acyclic_graph() {
        let nodes = vec![id("a"), id("b"), id("c")];
        let edges = edge_map(&[("a", &["b"]), ("b", &["c"]), ("c", &[])]);
        assert!(validate(&nodes, &edges, &no_existing).is_ok());
    }

    #[test]
    fn rejects_duplicate_nodes() {
        let nodes = vec![id("a"), id("a")];
        let edges = edge_map(&[("a", &[])]);
        let error = validate(&nodes, &edges, &no_existing).unwrap_err();
        assert!(matches!(error, TaskEngineError::DuplicateTask(_)));
    }

    #[test]
    fn rejects_unknown_dependencies() {
        let nodes = vec![id("a")];
        let edges = edge_map(&[("a", &["missing"])]);
        let error = validate(&nodes, &edges, &no_existing).unwrap_err();
        assert!(matches!(error, TaskEngineError::DependencyNotFound { .. }));
    }

    #[test]
    fn accepts_dependencies_on_existing_tasks() {
        let nodes = vec![id("a")];
        let edges = edge_map(&[("a", &["existing"])]);
        let existing = |candidate: &TaskId| candidate.as_str() == "existing";
        assert!(validate(&nodes, &edges, &existing).is_ok());
    }

    #[test]
    fn detects_cycles() {
        let nodes = vec![id("a"), id("b"), id("c")];
        let edges = edge_map(&[("a", &["b"]), ("b", &["c"]), ("c", &["a"])]);
        let error = validate(&nodes, &edges, &no_existing).unwrap_err();
        assert!(matches!(error, TaskEngineError::DependencyCycle { .. }));
    }

    #[test]
    fn detects_self_dependency() {
        let nodes = vec![id("a")];
        let edges = edge_map(&[("a", &["a"])]);
        assert!(find_cycle(&nodes, &edges).is_some());
    }

    #[test]
    fn supports_multiple_independent_branches() {
        let nodes = vec![id("a"), id("b"), id("c"), id("d")];
        let edges = edge_map(&[("a", &[]), ("b", &["a"]), ("c", &["a"]), ("d", &["b", "c"])]);
        assert!(validate(&nodes, &edges, &no_existing).is_ok());
    }
}
