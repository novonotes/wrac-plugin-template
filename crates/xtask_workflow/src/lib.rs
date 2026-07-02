//! Dependency-ordered workflow primitives for repository-local `xtask` crates.
//!
//! This crate deliberately knows nothing about WRAC, audio plugins, or build
//! targets. Product repositories keep their own task enum and executor, while
//! this crate provides the stable DAG mechanics those executors need.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;
pub type TaskNode = NodeIndex;

/// How a workflow executor should react after a task fails.
///
/// The policy is intentionally independent from task semantics. A product
/// executor can treat compiler failures, validator failures, or custom task
/// failures uniformly while still skipping tasks whose dependencies failed.
#[derive(Debug, Clone, Copy)]
pub enum FailurePolicy {
    FailFast,
    Continue,
}

/// Stable user-facing task identity.
///
/// `NodeIndex` is only a graph implementation detail. Keeping reports, skip
/// reasons, and dry-run output on these string IDs prevents graph insertion
/// order changes from leaking into user-visible diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone)]
struct Task<K> {
    id: TaskId,
    kind: K,
}

/// Execution status tracked by repository-local workflow executors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskStatus {
    Planned,
    Ok,
    Failed,
    Skipped,
}

/// A dependency graph whose task semantics are owned by the caller.
pub struct TaskGraph<K> {
    graph: DiGraph<Task<K>, ()>,
    nodes: HashMap<TaskId, TaskNode>,
}

impl<K> TaskGraph<K> {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            nodes: HashMap::new(),
        }
    }

    pub fn task(&mut self, id: TaskId, kind: K) -> TaskNode {
        // Multiple terminal tasks often share dependencies. Reusing an existing
        // node by ID keeps the workflow a DAG instead of a duplicated tree.
        if let Some(index) = self.nodes.get(&id) {
            return *index;
        }
        let index = self.graph.add_node(Task {
            id: id.clone(),
            kind,
        });
        self.nodes.insert(id, index);
        index
    }

    pub fn depends_on(&mut self, task: TaskNode, dependency: TaskNode) {
        // Edges point from dependency to dependent so the stable topological
        // order returned by `ordered` is directly executable.
        self.graph.add_edge(dependency, task, ());
    }

    pub fn id(&self, task: TaskNode) -> &TaskId {
        &self.graph[task].id
    }

    pub fn kind(&self, task: TaskNode) -> &K {
        &self.graph[task].kind
    }

    pub fn dependency_ids(&self, task: TaskNode) -> Vec<&TaskId> {
        self.graph
            .neighbors_directed(task, Direction::Incoming)
            .map(|dep| &self.graph[dep].id)
            .collect()
    }

    pub fn failed_dependency_ids(
        &self,
        task: TaskNode,
        statuses: &HashMap<TaskNode, TaskStatus>,
    ) -> Vec<&TaskId> {
        self.graph
            .neighbors_directed(task, Direction::Incoming)
            .filter(|dep| {
                matches!(
                    statuses.get(dep),
                    Some(TaskStatus::Failed | TaskStatus::Skipped)
                )
            })
            .map(|dep| &self.graph[dep].id)
            .collect()
    }

    pub fn ordered(&self) -> Result<Vec<TaskNode>> {
        // petgraph's generic toposort is correct but its peer ordering is tied
        // to traversal internals. Use a stable topological sort so dry-run
        // output and CI logs remain reviewable as task definitions evolve.
        let mut incoming = self
            .graph
            .node_indices()
            .map(|node| {
                (
                    node,
                    self.graph
                        .neighbors_directed(node, Direction::Incoming)
                        .count(),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut ready = incoming
            .iter()
            .filter_map(|(node, count)| (*count == 0).then_some(*node))
            .collect::<Vec<_>>();
        ready.sort_by_key(|node| node.index());

        let mut ordered = Vec::new();
        while let Some(node) = ready.first().copied() {
            ready.remove(0);
            ordered.push(node);
            for edge in self.graph.edges_directed(node, Direction::Outgoing) {
                let dependent = edge.target();
                let count = incoming
                    .get_mut(&dependent)
                    .expect("dependent node must have an incoming count");
                *count -= 1;
                if *count == 0 {
                    ready.push(dependent);
                    ready.sort_by_key(|node| node.index());
                }
            }
        }

        if ordered.len() != self.graph.node_count() {
            return Err("xtask workflow graph has a dependency cycle".into());
        }
        Ok(ordered)
    }
}

impl<K> Default for TaskGraph<K> {
    fn default() -> Self {
        Self::new()
    }
}
