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

/// Converts common CLI `--continue-on-error` flags into executor policy.
pub fn failure_policy(continue_on_error: bool) -> FailurePolicy {
    if continue_on_error {
        FailurePolicy::Continue
    } else {
        FailurePolicy::FailFast
    }
}

/// User-facing text used by the generic workflow executor.
///
/// The workflow crate owns execution mechanics but not product language policy.
/// Repositories pass the message set that matches their CLI, keeping template
/// output English-only while private product repositories can use Japanese.
#[derive(Debug, Clone)]
pub struct WorkflowMessages {
    pub plan_heading: &'static str,
    pub dependencies_heading: &'static str,
    pub execution_heading: &'static str,
    pub result_heading: &'static str,
    pub dry_run_message: &'static str,
    pub completed: &'static str,
    pub ok: &'static str,
    pub failed: &'static str,
    pub skipped: &'static str,
    pub planned: &'static str,
    pub dependency_skip_reason: &'static str,
}

impl WorkflowMessages {
    pub const ENGLISH: Self = Self {
        plan_heading: "Plan",
        dependencies_heading: "Dependencies",
        execution_heading: "Execution",
        result_heading: "Result",
        dry_run_message: "Nothing was executed because --dry-run was set.",
        completed: "completed",
        ok: "ok",
        failed: "failed",
        skipped: "skipped",
        planned: "planned",
        dependency_skip_reason: "Reason: dependency failed or was skipped",
    };

    pub const JAPANESE: Self = Self {
        plan_heading: "実行計画",
        dependencies_heading: "依存関係",
        execution_heading: "実行",
        result_heading: "結果",
        dry_run_message: "--dry-run が指定されているため、実行はスキップしました。",
        completed: "完了",
        ok: "成功",
        failed: "失敗",
        skipped: "スキップ",
        planned: "未実行",
        dependency_skip_reason: "理由: 依存タスクが失敗またはスキップされました",
    };

    fn status_label(&self, status: TaskStatus) -> &'static str {
        match status {
            TaskStatus::Planned => self.planned,
            TaskStatus::Ok => self.ok,
            TaskStatus::Failed => self.failed,
            TaskStatus::Skipped => self.skipped,
        }
    }
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
        let mut ids = self
            .graph
            .neighbors_directed(task, Direction::Incoming)
            .map(|dep| &self.graph[dep].id)
            .collect::<Vec<_>>();
        ids.sort_by(|left, right| left.0.cmp(&right.0));
        ids
    }

    pub fn failed_dependency_ids(
        &self,
        task: TaskNode,
        statuses: &HashMap<TaskNode, TaskStatus>,
    ) -> Vec<&TaskId> {
        let mut ids = self
            .graph
            .neighbors_directed(task, Direction::Incoming)
            .filter(|dep| {
                matches!(
                    statuses.get(dep),
                    Some(TaskStatus::Failed | TaskStatus::Skipped)
                )
            })
            .map(|dep| &self.graph[dep].id)
            .collect::<Vec<_>>();
        ids.sort_by(|left, right| left.0.cmp(&right.0));
        ids
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

/// Executes a workflow graph while keeping task semantics in the caller.
///
/// The graph executor handles stable ordering, dry-run output, failure policy,
/// downstream skipping, and summary reporting. The caller still owns the task
/// enum, labels, and dispatch to WRAC operations or product-specific work.
pub fn execute_plan<K, Label, Run>(
    graph: TaskGraph<K>,
    dry_run: bool,
    policy: FailurePolicy,
    messages: &WorkflowMessages,
    mut label_task: Label,
    mut run_task: Run,
) -> Result<()>
where
    Label: FnMut(&K) -> String,
    Run: FnMut(&K) -> Result<()>,
{
    let ordered = graph.ordered()?;
    print_plan(&graph, &ordered, dry_run, messages, &mut label_task);
    if dry_run {
        return Ok(());
    }

    let mut statuses = HashMap::<TaskNode, TaskStatus>::new();
    for index in &ordered {
        statuses.insert(*index, TaskStatus::Planned);
    }
    let mut failures = Vec::new();

    for index in &ordered {
        let index = *index;
        // A failed dependency makes the dependent task meaningless, so continuing
        // never tries to run downstream work with missing artifacts. Independent
        // branches still run under FailurePolicy::Continue.
        let failed_deps = graph
            .failed_dependency_ids(index, &statuses)
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !failed_deps.is_empty() {
            println!("{}\n  ⏭️ {}", graph.id(index), messages.skipped);
            println!(
                "  {} ({})",
                messages.dependency_skip_reason,
                failed_deps.join(", ")
            );
            println!();
            statuses.insert(index, TaskStatus::Skipped);
            continue;
        }

        println!("{}\n  {}", graph.id(index), label_task(graph.kind(index)));
        match run_task(graph.kind(index)) {
            Ok(()) => {
                println!("  ✅ {}", messages.completed);
                println!();
                statuses.insert(index, TaskStatus::Ok);
            }
            Err(err) => {
                println!("  ❌ {}", messages.failed);
                println!("  Error: {err}");
                statuses.insert(index, TaskStatus::Failed);
                failures.push(format!("{}: {err}", graph.id(index)));
                if matches!(policy, FailurePolicy::FailFast) {
                    print_summary(&graph, &ordered, &statuses, messages);
                    return Err(failures.join("\n").into());
                }
                println!();
            }
        }
    }

    print_summary(&graph, &ordered, &statuses, messages);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n").into())
    }
}

fn print_plan<K, Label>(
    graph: &TaskGraph<K>,
    ordered: &[TaskNode],
    dry_run: bool,
    messages: &WorkflowMessages,
    label_task: &mut Label,
) where
    Label: FnMut(&K) -> String,
{
    println!("== {} ==\n", messages.plan_heading);
    for (position, index) in ordered.iter().enumerate() {
        println!(
            "{}. {}  {}",
            position + 1,
            graph.id(*index),
            label_task(graph.kind(*index))
        );
    }
    let dependencies = ordered
        .iter()
        .filter_map(|index| {
            let deps = graph
                .dependency_ids(*index)
                .into_iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            (!deps.is_empty()).then_some((index, deps))
        })
        .collect::<Vec<_>>();
    if !dependencies.is_empty() {
        println!("\n== {} ==\n", messages.dependencies_heading);
    }
    for (index, deps) in dependencies {
        println!("{} <- {}", graph.id(*index), deps.join(", "));
    }
    if dry_run {
        println!("\n{}", messages.dry_run_message);
    } else {
        println!("\n== {} ==\n", messages.execution_heading);
    }
}

fn print_summary<K>(
    graph: &TaskGraph<K>,
    ordered: &[TaskNode],
    statuses: &HashMap<TaskNode, TaskStatus>,
    messages: &WorkflowMessages,
) {
    let mut counts = HashMap::<TaskStatus, usize>::new();
    for status in statuses.values() {
        *counts.entry(*status).or_default() += 1;
    }
    println!(
        "== {} ==\n\n✅ {} {} / ❌ {} {} / ⏭️ {} {}",
        messages.result_heading,
        messages.ok,
        counts.get(&TaskStatus::Ok).copied().unwrap_or(0),
        messages.failed,
        counts.get(&TaskStatus::Failed).copied().unwrap_or(0),
        messages.skipped,
        counts.get(&TaskStatus::Skipped).copied().unwrap_or(0)
    );
    for index in ordered {
        let Some(status) = statuses.get(index) else {
            continue;
        };
        if matches!(status, TaskStatus::Failed | TaskStatus::Skipped) {
            println!("{}: {}", messages.status_label(*status), graph.id(*index));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestTask {
        Root,
        Dependent,
        Independent,
    }

    #[test]
    fn ordered_returns_dependencies_before_dependents() {
        let mut graph = TaskGraph::new();
        let dependent = graph.task(TaskId::new("dependent"), TestTask::Dependent);
        let root = graph.task(TaskId::new("root"), TestTask::Root);
        graph.depends_on(dependent, root);

        let ordered = graph.ordered().unwrap();
        let ids = ordered
            .iter()
            .map(|node| graph.id(*node).to_string())
            .collect::<Vec<_>>();

        assert_eq!(ids, ["root", "dependent"]);
    }

    #[test]
    fn ordered_rejects_cycles() {
        let mut graph = TaskGraph::new();
        let first = graph.task(TaskId::new("first"), TestTask::Root);
        let second = graph.task(TaskId::new("second"), TestTask::Dependent);
        graph.depends_on(first, second);
        graph.depends_on(second, first);

        let error = graph.ordered().unwrap_err().to_string();

        assert!(error.contains("dependency cycle"));
    }

    #[test]
    fn execute_plan_skips_dependents_after_failed_dependencies() {
        let mut graph = TaskGraph::new();
        let root = graph.task(TaskId::new("root"), TestTask::Root);
        let dependent = graph.task(TaskId::new("dependent"), TestTask::Dependent);
        let _independent = graph.task(TaskId::new("independent"), TestTask::Independent);
        graph.depends_on(dependent, root);

        let mut executed = Vec::new();
        let result = execute_plan(
            graph,
            false,
            FailurePolicy::Continue,
            &WorkflowMessages::ENGLISH,
            |task| format!("{task:?}"),
            |task| {
                executed.push(*task);
                match task {
                    TestTask::Root => Err("root failed".into()),
                    TestTask::Dependent | TestTask::Independent => Ok(()),
                }
            },
        );

        let error = result.unwrap_err().to_string();

        assert!(error.contains("root failed"));
        assert_eq!(executed, [TestTask::Root, TestTask::Independent]);
    }
}
