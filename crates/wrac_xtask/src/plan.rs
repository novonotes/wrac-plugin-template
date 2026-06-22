use std::collections::HashMap;
use std::fmt;

use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;

use crate::Result;
use crate::XtaskOutputLanguage;
use crate::commands::{
    RustPluginBuild, WrapperBuild, WrapperTarget, build_gui, build_rust_plugin,
    build_wrapper_target, clean, configure_wrapper, install_dir, install_plugin_target, launch,
    package_clap, print_outputs, uninstall_plugin_target, validate_plugin_target,
    validate_wrac_rules_for_targets,
};
use crate::context::Context;
use crate::targets::{PluginTarget, Target, ValidateTarget};
use crate::{BuildProfile, InstallScope, UninstallScope};

mod output;
mod target_resolution;

use self::output::{
    completed_label, dependencies_heading, dry_run_message, execution_heading, failed_label,
    plan_heading, result_heading, skip_reason, skipped_label, status_label, success_label,
    uninstall_summary,
};
pub use self::target_resolution::{
    resolve_build_targets_from_metadata, resolve_plugin_targets_from_metadata,
    resolve_validate_targets_from_metadata,
};

/// How the executor treats a task failure after the graph has already been planned.
///
/// This is intentionally not a target-selection policy. Unsupported targets,
/// invalid scopes, missing SDKs, build failures, and validator failures are all
/// represented as task failures so the same downstream-skip rule applies.
#[derive(Debug, Clone, Copy)]
pub enum FailurePolicy {
    FailFast,
    Continue,
}

pub type TaskNode = NodeIndex;

/// Stable user-facing task identity.
///
/// `NodeIndex` is only an implementation detail of petgraph. Keeping reports,
/// skip reasons, and dry-run output on these string IDs prevents graph insertion
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
struct Task {
    id: TaskId,
    kind: TaskKind,
}

impl Task {
    fn label(&self, language: XtaskOutputLanguage) -> String {
        self.kind.label(language)
    }
}

#[derive(Debug, Clone)]
pub enum TaskKind {
    Clean,
    BuildGui,
    BuildRustDefault,
    BuildRustStandalone,
    PackageClap,
    ConfigureWrapperPlugins {
        vst3: bool,
        au: bool,
    },
    ConfigureWrapperAax,
    ConfigureWrapperStandalone,
    BuildVst3Bundle,
    BuildAuBundle,
    BuildAaxBundle,
    BuildStandaloneBundle {
        plugin_id: Option<String>,
    },
    LaunchStandalone {
        plugin_id: Option<String>,
    },
    CheckInstallScope {
        target: PluginTarget,
        scope: InstallScope,
    },
    InstallBundle {
        target: PluginTarget,
        scope: InstallScope,
    },
    UninstallBundle {
        target: PluginTarget,
        scope: UninstallScope,
        dry_run: bool,
    },
    ValidateWracRules {
        targets: Vec<ValidateTarget>,
    },
    ValidateBundle {
        target: ValidateTarget,
    },
}

impl TaskKind {
    fn label(&self, language: XtaskOutputLanguage) -> String {
        match (language, self) {
            (XtaskOutputLanguage::English, Self::Clean) => "clean generated artifacts".to_string(),
            (XtaskOutputLanguage::Japanese, Self::Clean) => "生成済み成果物を削除".to_string(),
            (XtaskOutputLanguage::English, Self::BuildGui) => "build GUI".to_string(),
            (XtaskOutputLanguage::Japanese, Self::BuildGui) => "GUI をビルド".to_string(),
            (XtaskOutputLanguage::English, Self::BuildRustDefault) => {
                "build Rust plugin library".to_string()
            }
            (XtaskOutputLanguage::Japanese, Self::BuildRustDefault) => {
                "Rust プラグインライブラリをビルド".to_string()
            }
            (XtaskOutputLanguage::English, Self::BuildRustStandalone) => {
                "build Rust standalone library".to_string()
            }
            (XtaskOutputLanguage::Japanese, Self::BuildRustStandalone) => {
                "Rust standalone ライブラリをビルド".to_string()
            }
            (XtaskOutputLanguage::English, Self::PackageClap) => "package CLAP bundle".to_string(),
            (XtaskOutputLanguage::Japanese, Self::PackageClap) => {
                "CLAP bundle をパッケージ".to_string()
            }
            (language, Self::ConfigureWrapperPlugins { vst3, au }) => {
                let mut formats = Vec::new();
                if *vst3 {
                    formats.push("VST3");
                }
                if *au {
                    formats.push("AU");
                }
                match language {
                    XtaskOutputLanguage::English => {
                        format!("configure clap-wrapper ({})", formats.join(", "))
                    }
                    XtaskOutputLanguage::Japanese => {
                        format!("clap-wrapper を設定 ({})", formats.join(", "))
                    }
                }
            }
            (XtaskOutputLanguage::English, Self::ConfigureWrapperAax) => {
                "configure clap-wrapper (AAX)".to_string()
            }
            (XtaskOutputLanguage::Japanese, Self::ConfigureWrapperAax) => {
                "clap-wrapper を設定 (AAX)".to_string()
            }
            (XtaskOutputLanguage::English, Self::ConfigureWrapperStandalone) => {
                "configure clap-wrapper (standalone)".to_string()
            }
            (XtaskOutputLanguage::Japanese, Self::ConfigureWrapperStandalone) => {
                "clap-wrapper を設定 (standalone)".to_string()
            }
            (XtaskOutputLanguage::English, Self::BuildVst3Bundle) => {
                "build VST3 bundle".to_string()
            }
            (XtaskOutputLanguage::Japanese, Self::BuildVst3Bundle) => {
                "VST3 bundle をビルド".to_string()
            }
            (XtaskOutputLanguage::English, Self::BuildAuBundle) => "build AU bundle".to_string(),
            (XtaskOutputLanguage::Japanese, Self::BuildAuBundle) => {
                "AU bundle をビルド".to_string()
            }
            (XtaskOutputLanguage::English, Self::BuildAaxBundle) => "build AAX bundle".to_string(),
            (XtaskOutputLanguage::Japanese, Self::BuildAaxBundle) => {
                "AAX bundle をビルド".to_string()
            }
            (XtaskOutputLanguage::English, Self::BuildStandaloneBundle { plugin_id }) => {
                match plugin_id {
                    Some(plugin_id) => format!("build standalone artifact ({plugin_id})"),
                    None => "build standalone artifact".to_string(),
                }
            }
            (XtaskOutputLanguage::Japanese, Self::BuildStandaloneBundle { plugin_id }) => {
                match plugin_id {
                    Some(plugin_id) => format!("standalone 成果物をビルド ({plugin_id})"),
                    None => "standalone 成果物をビルド".to_string(),
                }
            }
            (XtaskOutputLanguage::English, Self::LaunchStandalone { plugin_id }) => match plugin_id
            {
                Some(plugin_id) => format!("launch standalone artifact ({plugin_id})"),
                None => "launch standalone artifact".to_string(),
            },
            (XtaskOutputLanguage::Japanese, Self::LaunchStandalone { plugin_id }) => {
                match plugin_id {
                    Some(plugin_id) => format!("standalone artifact を起動 ({plugin_id})"),
                    None => "standalone artifact を起動".to_string(),
                }
            }
            (XtaskOutputLanguage::English, Self::CheckInstallScope { target, scope }) => {
                format!("check install scope for {} ({scope:?})", target.display())
            }
            (XtaskOutputLanguage::Japanese, Self::CheckInstallScope { target, scope }) => {
                format!("{} のインストール先を確認 ({scope:?})", target.display())
            }
            (XtaskOutputLanguage::English, Self::InstallBundle { target, scope }) => {
                format!("install {} ({scope:?})", target.display())
            }
            (XtaskOutputLanguage::Japanese, Self::InstallBundle { target, scope }) => {
                format!("{} をインストール ({scope:?})", target.display())
            }
            (
                XtaskOutputLanguage::English,
                Self::UninstallBundle {
                    target, dry_run, ..
                },
            ) => {
                if *dry_run {
                    format!("plan uninstall {}", target.display())
                } else {
                    format!("uninstall {}", target.display())
                }
            }
            (
                XtaskOutputLanguage::Japanese,
                Self::UninstallBundle {
                    target, dry_run, ..
                },
            ) => {
                if *dry_run {
                    format!("{} のアンインストール内容を確認", target.display())
                } else {
                    format!("{} をアンインストール", target.display())
                }
            }
            (language, Self::ValidateWracRules { targets }) => {
                let targets = targets
                    .iter()
                    .map(|target| target.display())
                    .collect::<Vec<_>>()
                    .join(", ");
                match language {
                    XtaskOutputLanguage::English => {
                        format!("run WRAC production-readiness checks ({targets})")
                    }
                    XtaskOutputLanguage::Japanese => {
                        format!("WRAC production-readiness checks を実行 ({targets})")
                    }
                }
            }
            (XtaskOutputLanguage::English, Self::ValidateBundle { target }) => {
                format!("validate {}", target.display())
            }
            (XtaskOutputLanguage::Japanese, Self::ValidateBundle { target }) => {
                format!("{} を検証", target.display())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TaskStatus {
    Planned,
    Ok,
    Failed,
    Skipped,
}

pub struct TaskPlan {
    graph: DiGraph<Task, ()>,
    nodes: HashMap<TaskId, NodeIndex>,
}

impl TaskPlan {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            nodes: HashMap::new(),
        }
    }

    pub fn task(&mut self, id: TaskId, kind: TaskKind) -> NodeIndex {
        // Multiple terminal tasks often share dependencies, for example VST3
        // and AAX both need the default Rust staticlib. Reusing the existing
        // node here is what keeps the plan a DAG instead of a duplicated tree.
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

    pub fn depends_on(&mut self, task: NodeIndex, dependency: NodeIndex) {
        // Edges point from dependency to dependent so petgraph's topological
        // order is directly executable. Keeping that convention local makes
        // later task additions much easier to review.
        self.graph.add_edge(dependency, task, ());
    }

    fn ordered(&self) -> Result<Vec<NodeIndex>> {
        // petgraph's generic toposort is correct but its peer ordering is tied
        // to traversal internals. Use a tiny stable topological sort so dry-run
        // output and CI logs stay reviewable as task definitions evolve.
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
            return Err("internal xtask task graph has a dependency cycle".into());
        }
        Ok(ordered)
    }
}

impl Default for TaskPlan {
    fn default() -> Self {
        Self::new()
    }
}

pub fn execute_plan(
    ctx: &Context,
    profile: BuildProfile,
    graph: TaskPlan,
    dry_run: bool,
    policy: FailurePolicy,
) -> Result<()> {
    let ordered = graph.ordered()?;
    let language = ctx.output_language;
    print_plan(&graph, &ordered, dry_run, language);
    if dry_run {
        return Ok(());
    }

    let mut statuses = HashMap::<NodeIndex, TaskStatus>::new();
    for index in &ordered {
        statuses.insert(*index, TaskStatus::Planned);
    }
    let mut failures = Vec::new();

    for index in ordered {
        // A failed dependency makes the dependent task meaningless, so continuing
        // never tries to run downstream work with missing artifacts. Independent
        // branches still run under FailurePolicy::Continue.
        let failed_deps = graph
            .graph
            .neighbors_directed(index, Direction::Incoming)
            .filter(|dep| {
                matches!(
                    statuses.get(dep),
                    Some(TaskStatus::Failed | TaskStatus::Skipped)
                )
            })
            .map(|dep| graph.graph[dep].id.to_string())
            .collect::<Vec<_>>();
        if !failed_deps.is_empty() {
            println!(
                "{}\n  ⏭️ {}",
                graph.graph[index].id,
                skipped_label(language)
            );
            println!("  {}", skip_reason(language, &failed_deps));
            println!();
            statuses.insert(index, TaskStatus::Skipped);
            continue;
        }

        println!(
            "{}\n  {}",
            graph.graph[index].id,
            graph.graph[index].label(language)
        );
        match run_task(ctx, profile, &graph.graph[index].kind) {
            Ok(()) => {
                println!("  ✅ {}", completed_label(language));
                println!();
                statuses.insert(index, TaskStatus::Ok);
            }
            Err(err) => {
                println!("  ❌ {}", failed_label(language));
                println!("  Error: {err}");
                statuses.insert(index, TaskStatus::Failed);
                failures.push(format!("{}: {err}", graph.graph[index].id));
                if matches!(policy, FailurePolicy::FailFast) {
                    print_summary(&graph, &statuses, language);
                    return Err(failures.join("\n").into());
                }
                println!();
            }
        }
    }

    print_summary(&graph, &statuses, language);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n").into())
    }
}

fn run_task(ctx: &Context, profile: BuildProfile, kind: &TaskKind) -> Result<()> {
    match kind {
        TaskKind::Clean => clean(ctx),
        TaskKind::BuildGui => build_gui(ctx),
        TaskKind::BuildRustDefault => build_rust_plugin(ctx, profile, RustPluginBuild::Default),
        TaskKind::BuildRustStandalone => {
            build_rust_plugin(ctx, profile, RustPluginBuild::Standalone)
        }
        TaskKind::PackageClap => package_clap(ctx, profile),
        TaskKind::ConfigureWrapperPlugins { vst3, au } => configure_wrapper(
            ctx,
            profile,
            WrapperBuild::Plugins {
                vst3: *vst3,
                au: *au,
            },
        ),
        TaskKind::ConfigureWrapperAax => configure_wrapper(ctx, profile, WrapperBuild::Aax),
        TaskKind::ConfigureWrapperStandalone => {
            configure_wrapper(ctx, profile, WrapperBuild::Standalone)
        }
        TaskKind::BuildVst3Bundle => build_wrapper_target(
            ctx,
            profile,
            WrapperBuild::Plugins {
                vst3: true,
                au: false,
            },
            WrapperTarget::Vst3,
            None,
        ),
        TaskKind::BuildAuBundle => build_wrapper_target(
            ctx,
            profile,
            WrapperBuild::Plugins {
                vst3: false,
                au: true,
            },
            WrapperTarget::Au,
            None,
        ),
        TaskKind::BuildAaxBundle => {
            build_wrapper_target(ctx, profile, WrapperBuild::Aax, WrapperTarget::Aax, None)
        }
        TaskKind::BuildStandaloneBundle { plugin_id } => build_wrapper_target(
            ctx,
            profile,
            WrapperBuild::Standalone,
            WrapperTarget::Standalone,
            plugin_id.as_deref(),
        ),
        TaskKind::LaunchStandalone { plugin_id } => launch(ctx, profile, plugin_id.as_deref()),
        TaskKind::CheckInstallScope { target, scope } => {
            install_dir(ctx, *scope, target.format()).map(|_| ())
        }
        TaskKind::InstallBundle { target, scope } => {
            install_plugin_target(ctx, profile, *scope, *target)
        }
        TaskKind::UninstallBundle {
            target,
            scope,
            dry_run,
        } => {
            let (removed, missing) = uninstall_plugin_target(ctx, *scope, *target, *dry_run)?;
            if *dry_run {
                println!(
                    "  {}",
                    uninstall_summary(ctx.output_language, *target, removed, missing, true)
                );
            } else {
                println!(
                    "  {}",
                    uninstall_summary(ctx.output_language, *target, removed, missing, false)
                );
            }
            Ok(())
        }
        TaskKind::ValidateWracRules { targets } => {
            validate_wrac_rules_for_targets(ctx, profile, targets)
        }
        TaskKind::ValidateBundle { target } => validate_plugin_target(ctx, profile, *target),
    }
}

fn print_plan(
    graph: &TaskPlan,
    ordered: &[NodeIndex],
    dry_run: bool,
    language: XtaskOutputLanguage,
) {
    println!("== {} ==\n", plan_heading(language));
    for (position, index) in ordered.iter().enumerate() {
        println!(
            "{}. {}  {}",
            position + 1,
            graph.graph[*index].id,
            graph.graph[*index].label(language)
        );
    }
    let dependencies = ordered
        .iter()
        .filter_map(|index| {
            let deps = graph
                .graph
                .neighbors_directed(*index, Direction::Incoming)
                .map(|dep| graph.graph[dep].id.to_string())
                .collect::<Vec<_>>();
            (!deps.is_empty()).then_some((index, deps))
        })
        .collect::<Vec<_>>();
    if !dependencies.is_empty() {
        println!("\n== {} ==\n", dependencies_heading(language));
    }
    for (index, deps) in dependencies {
        println!("{} <- {}", graph.graph[*index].id, deps.join(", "));
    }
    if dry_run {
        println!("\n{}", dry_run_message(language));
    } else {
        println!("\n== {} ==\n", execution_heading(language));
    }
}

fn print_summary(
    graph: &TaskPlan,
    statuses: &HashMap<NodeIndex, TaskStatus>,
    language: XtaskOutputLanguage,
) {
    let mut counts = HashMap::<TaskStatus, usize>::new();
    for status in statuses.values() {
        *counts.entry(*status).or_default() += 1;
    }
    println!(
        "== {} ==\n\n✅ {} {} / ❌ {} {} / ⏭️ {} {}",
        result_heading(language),
        success_label(language),
        counts.get(&TaskStatus::Ok).copied().unwrap_or(0),
        failed_label(language),
        counts.get(&TaskStatus::Failed).copied().unwrap_or(0),
        skipped_label(language),
        counts.get(&TaskStatus::Skipped).copied().unwrap_or(0)
    );
    for (index, status) in statuses {
        if matches!(status, TaskStatus::Failed | TaskStatus::Skipped) {
            println!(
                "{}: {}",
                status_label(language, *status),
                graph.graph[*index].id
            );
        }
    }
}

pub fn failure_policy(continue_on_error: bool) -> FailurePolicy {
    if continue_on_error {
        FailurePolicy::Continue
    } else {
        FailurePolicy::FailFast
    }
}

pub fn print_build_outputs(
    ctx: &Context,
    profile: BuildProfile,
    targets: &[Target],
    standalone_plugin_id: Option<&str>,
) -> Result<()> {
    print_outputs(ctx, profile, targets, standalone_plugin_id)
}
