use std::collections::{HashMap, HashSet};
use std::fmt;

use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;

use crate::Result;
use crate::cli::{BuildArgs, InstallArgs, UninstallArgs, ValidateArgs};
use crate::commands::{
    RustPluginBuild, WrapperBuild, WrapperTarget, build_gui, build_rust_plugin,
    build_wrapper_target, clean, configure_wrapper, install_dir, install_plugin_target,
    package_clap, print_outputs, uninstall_plugin_target, validate_plugin_target,
    validate_wrac_rules_for_targets,
};
use crate::context::Context;
use crate::profile::BuildProfile;
use crate::targets::{PluginFormat, PluginTarget, Target, ValidateTarget};

/// How the executor treats a task failure after the graph has already been planned.
///
/// This is intentionally not a target-selection policy. Unsupported targets,
/// invalid scopes, missing SDKs, build failures, and validator failures are all
/// represented as task failures so the same downstream-skip rule applies.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FailurePolicy {
    FailFast,
    Continue,
}

#[derive(Debug, Clone, Copy)]
enum CommandKind {
    Build,
    Install,
    Validate,
}

/// Stable user-facing task identity.
///
/// `NodeIndex` is only an implementation detail of petgraph. Keeping reports,
/// skip reasons, and dry-run output on these string IDs prevents graph insertion
/// order changes from leaking into user-visible diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TaskId(String);

impl TaskId {
    fn new(value: impl Into<String>) -> Self {
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
    fn label(&self) -> String {
        self.kind.label()
    }
}

#[derive(Debug, Clone)]
enum TaskKind {
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
    BuildStandaloneBundle,
    CheckInstallScope {
        target: PluginTarget,
        scope: crate::cli::InstallScope,
    },
    InstallBundle {
        target: PluginTarget,
        scope: crate::cli::InstallScope,
    },
    UninstallBundle {
        target: PluginTarget,
        scope: crate::cli::UninstallScope,
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
    fn label(&self) -> String {
        match self {
            Self::Clean => "clean generated artifacts".to_string(),
            Self::BuildGui => "build GUI".to_string(),
            Self::BuildRustDefault => "build Rust plugin library".to_string(),
            Self::BuildRustStandalone => "build Rust standalone library".to_string(),
            Self::PackageClap => "package CLAP bundle".to_string(),
            Self::ConfigureWrapperPlugins { vst3, au } => {
                let mut formats = Vec::new();
                if *vst3 {
                    formats.push("VST3");
                }
                if *au {
                    formats.push("AU");
                }
                format!("configure clap-wrapper ({})", formats.join(", "))
            }
            Self::ConfigureWrapperAax => "configure clap-wrapper (AAX)".to_string(),
            Self::ConfigureWrapperStandalone => "configure clap-wrapper (standalone)".to_string(),
            Self::BuildVst3Bundle => "build VST3 bundle".to_string(),
            Self::BuildAuBundle => "build AU bundle".to_string(),
            Self::BuildAaxBundle => "build AAX bundle".to_string(),
            Self::BuildStandaloneBundle => "build standalone artifact".to_string(),
            Self::CheckInstallScope { target, scope } => {
                format!("check install scope for {} ({scope:?})", target.display())
            }
            Self::InstallBundle { target, scope } => {
                format!("install {} ({scope:?})", target.display())
            }
            Self::UninstallBundle {
                target, dry_run, ..
            } => {
                if *dry_run {
                    format!("plan uninstall {}", target.display())
                } else {
                    format!("uninstall {}", target.display())
                }
            }
            Self::ValidateWracRules { targets } => {
                let targets = targets
                    .iter()
                    .map(|target| target.display())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("run WRAC production-readiness checks ({targets})")
            }
            Self::ValidateBundle { target } => format!("validate {}", target.display()),
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

struct TaskGraph {
    graph: DiGraph<Task, ()>,
    nodes: HashMap<TaskId, NodeIndex>,
}

impl TaskGraph {
    fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            nodes: HashMap::new(),
        }
    }

    fn task(&mut self, id: TaskId, kind: TaskKind) -> NodeIndex {
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

    fn depends_on(&mut self, task: NodeIndex, dependency: NodeIndex) {
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

pub(crate) fn run_build(ctx: &Context, args: &BuildArgs) -> Result<()> {
    let profile = BuildProfile::from_release(args.release);
    let targets = resolve_build_targets_from_metadata(ctx, &args.target)?;
    // The command only chooses terminal build tasks. The graph builder expands
    // those into Rust, wrapper-configure, and format-specific build tasks.
    let graph = build_graph(ctx, CommandKind::Build, &targets, args.clean, None)?;
    execute_plan(
        ctx,
        profile,
        graph,
        args.dry_run,
        failure_policy(args.continue_on_error),
    )?;
    if !args.dry_run {
        print_outputs(ctx, profile, &targets);
    }
    Ok(())
}

pub(crate) fn run_install(ctx: &Context, args: &InstallArgs) -> Result<()> {
    let profile = BuildProfile::from_release(args.release);
    let targets = resolve_plugin_targets_from_metadata(ctx, &args.target)?;
    let build_targets = targets
        .iter()
        .map(|target| target.target())
        .collect::<Vec<_>>();
    let graph = build_graph(
        ctx,
        CommandKind::Install,
        &build_targets,
        false,
        Some(InstallSelection {
            targets,
            scope: args.scope,
        }),
    )?;
    execute_plan(
        ctx,
        profile,
        graph,
        args.dry_run,
        failure_policy(args.continue_on_error),
    )
}

pub(crate) fn run_uninstall(ctx: &Context, args: &UninstallArgs) -> Result<()> {
    let targets = resolve_plugin_targets_from_metadata(ctx, &args.target)?;
    let mut graph = TaskGraph::new();
    for target in targets {
        graph.task(
            TaskId::new(format!("{}:uninstall:{target:?}", ctx.package_name)),
            TaskKind::UninstallBundle {
                target,
                scope: args.scope,
                dry_run: args.dry_run,
            },
        );
    }
    execute_plan(
        ctx,
        BuildProfile::Debug,
        graph,
        false,
        failure_policy(args.continue_on_error),
    )
}

pub(crate) fn run_validate(ctx: &Context, args: &ValidateArgs) -> Result<()> {
    let profile = BuildProfile::from_release(args.release);
    let targets = resolve_validate_targets_from_metadata(ctx, &args.target)?;
    let build_targets = targets
        .iter()
        .map(|target| target.target())
        .collect::<Vec<_>>();
    // Validate does not "call build" as a special case. It asks for validation
    // terminal tasks, and the dependency graph pulls in exactly the build and
    // install tasks those validators need.
    let graph = build_graph(
        ctx,
        CommandKind::Validate,
        &build_targets,
        false,
        Some(InstallSelection {
            targets: targets
                .iter()
                .map(|target| match target {
                    ValidateTarget::Clap => PluginTarget::Clap,
                    ValidateTarget::Vst3 => PluginTarget::Vst3,
                    ValidateTarget::Au => PluginTarget::Au,
                    ValidateTarget::Aax => PluginTarget::Aax,
                })
                .collect(),
            scope: crate::cli::InstallScope::Default,
        }),
    )?;
    execute_plan(
        ctx,
        profile,
        graph,
        args.dry_run,
        failure_policy(args.continue_on_error),
    )
}

#[derive(Clone)]
struct InstallSelection {
    targets: Vec<PluginTarget>,
    scope: crate::cli::InstallScope,
}

fn build_graph(
    ctx: &Context,
    command: CommandKind,
    targets: &[Target],
    clean_first: bool,
    install_selection: Option<InstallSelection>,
) -> Result<TaskGraph> {
    let mut graph = TaskGraph::new();
    // Install scope validation is modeled as a task, not an upfront global
    // preflight. With --continue-on-error, an invalid AAX user install scope can
    // skip only AAX while unrelated CLAP/VST3/AU installs continue.
    let install_checks = install_selection
        .as_ref()
        .filter(|_| matches!(command, CommandKind::Install))
        .map(|selection| {
            selection
                .targets
                .iter()
                .map(|target| {
                    let check = graph.task(
                        package_task_id(ctx, &format!("check-install-scope-{target:?}")),
                        TaskKind::CheckInstallScope {
                            target: *target,
                            scope: selection.scope,
                        },
                    );
                    (target.target(), check)
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let clean = clean_first.then(|| graph.task(package_task_id(ctx, "clean"), TaskKind::Clean));

    let needs_vst3 = targets.contains(&Target::Vst3);
    let needs_au = targets.contains(&Target::Au);
    let needs_aax = targets.contains(&Target::Aax);
    let needs_standalone = targets.contains(&Target::Standalone);
    // CLAP/VST3/AU/AAX all use the default Rust plugin build. CLAP consumes the
    // cdylib, while wrapper formats link the staticlib from the same cargo run.
    let needs_default = targets.iter().any(|target| {
        matches!(
            target,
            Target::Clap | Target::Vst3 | Target::Au | Target::Aax
        )
    });

    let build_gui = if needs_default || needs_standalone {
        let build_gui = graph.task(package_task_id(ctx, "build-gui"), TaskKind::BuildGui);
        if let Some(clean) = clean {
            graph.depends_on(build_gui, clean);
        }
        Some(build_gui)
    } else {
        None
    };

    let rust_default = if needs_default {
        let rust_default = graph.task(
            package_task_id(ctx, "build-rust-default"),
            TaskKind::BuildRustDefault,
        );
        graph.depends_on(
            rust_default,
            build_gui.expect("default Rust build needs GUI"),
        );
        Some(rust_default)
    } else {
        None
    };

    let rust_standalone = if needs_standalone {
        // Standalone uses a separate cargo target directory because wrapper app
        // dependencies and debug artifacts should not contaminate plugin builds.
        let rust_standalone = graph.task(
            package_task_id(ctx, "build-rust-standalone"),
            TaskKind::BuildRustStandalone,
        );
        graph.depends_on(
            rust_standalone,
            build_gui.expect("standalone Rust build needs GUI"),
        );
        Some(rust_standalone)
    } else {
        None
    };

    let mut build_by_target = HashMap::new();
    if targets.contains(&Target::Clap) || matches!(command, CommandKind::Validate) {
        // WRAC production-readiness checks read the CLAP schema, so validation
        // always packages CLAP even when the requested external validator is VST3,
        // AU, or AAX.
        let clap = graph.task(package_task_id(ctx, "package-clap"), TaskKind::PackageClap);
        graph.depends_on(
            clap,
            rust_default.expect("CLAP packaging needs default Rust build"),
        );
        if let Some(check) = install_checks.get(&Target::Clap) {
            graph.depends_on(clap, *check);
        }
        build_by_target.insert(Target::Clap, clap);
    }

    if needs_vst3 || needs_au {
        // VST3 and AU intentionally share the default Rust staticlib. Older
        // WRY_OBJC_SUFFIX-based builds needed per-format Rust builds for
        // Objective-C class names, but current wxp/wry embeds the source ID into
        // objc2-generated class names. Split this again only if per-format
        // compile-time inputs return.
        // Configure the private-SDK-free wrapper project with the full native
        // plugin target set for this platform, then build only the DAG-selected
        // CMake target below. This avoids flipping the same CMake cache between
        // "VST3 only" and "AU only" when developers alternate commands.
        let configure_vst3 = needs_vst3 || ctx.platform.supports_vst3();
        let configure_au = needs_au || ctx.platform.supports_au();
        // Keep AAX out of this configure group. AAX requires a private SDK, and
        // VST3/AU-only builds must not fail just because AAX inputs are absent.
        let configure = graph.task(
            package_task_id(ctx, "configure-wrapper-plugins"),
            TaskKind::ConfigureWrapperPlugins {
                vst3: configure_vst3,
                au: configure_au,
            },
        );
        graph.depends_on(
            configure,
            rust_default.expect("wrapper plugin builds need default Rust build"),
        );
        for target in [Target::Vst3, Target::Au] {
            if let Some(check) = install_checks.get(&target) {
                graph.depends_on(configure, *check);
            }
        }
        if needs_vst3 {
            let vst3 = graph.task(
                package_task_id(ctx, "build-vst3"),
                TaskKind::BuildVst3Bundle,
            );
            graph.depends_on(vst3, configure);
            build_by_target.insert(Target::Vst3, vst3);
        }
        if needs_au {
            let au = graph.task(package_task_id(ctx, "build-au"), TaskKind::BuildAuBundle);
            graph.depends_on(au, configure);
            build_by_target.insert(Target::Au, au);
        }
    }

    if needs_aax {
        // AAX gets its own CMake build directory because the target set and SDK
        // root are configure-time inputs. Sharing one wrapper cache with VST3/AU
        // would recreate the old "last command wins" CMake state problem.
        let configure = graph.task(
            package_task_id(ctx, "configure-wrapper-aax"),
            TaskKind::ConfigureWrapperAax,
        );
        graph.depends_on(
            configure,
            rust_default.expect("AAX wrapper builds need default Rust build"),
        );
        if let Some(check) = install_checks.get(&Target::Aax) {
            graph.depends_on(configure, *check);
        }
        let aax = graph.task(package_task_id(ctx, "build-aax"), TaskKind::BuildAaxBundle);
        graph.depends_on(aax, configure);
        build_by_target.insert(Target::Aax, aax);
    }

    if needs_standalone {
        let configure = graph.task(
            package_task_id(ctx, "configure-wrapper-standalone"),
            TaskKind::ConfigureWrapperStandalone,
        );
        graph.depends_on(
            configure,
            rust_standalone.expect("standalone wrapper needs standalone Rust build"),
        );
        let standalone = graph.task(
            package_task_id(ctx, "build-standalone"),
            TaskKind::BuildStandaloneBundle,
        );
        graph.depends_on(standalone, configure);
        build_by_target.insert(Target::Standalone, standalone);
    }

    match command {
        CommandKind::Build => {}
        CommandKind::Install => {
            let install_selection = install_selection.expect("install graph needs selection");
            for target in install_selection.targets {
                // Install tasks depend on their concrete format build task, not
                // on a broad "build all" node. This lets --continue-on-error skip
                // only the affected format when another format fails.
                let install = graph.task(
                    package_task_id(ctx, &format!("install-{target:?}")),
                    TaskKind::InstallBundle {
                        target,
                        scope: install_selection.scope,
                    },
                );
                let build = build_by_target[&target.target()];
                graph.depends_on(install, build);
            }
        }
        CommandKind::Validate => {
            let validate_targets = targets
                .iter()
                .filter_map(|target| match target {
                    Target::Clap => Some(ValidateTarget::Clap),
                    Target::Vst3 => Some(ValidateTarget::Vst3),
                    Target::Au => Some(ValidateTarget::Au),
                    Target::Aax => Some(ValidateTarget::Aax),
                    Target::Standalone => None,
                })
                .collect::<Vec<_>>();
            let rules = graph.task(
                package_task_id(ctx, "validate-wrac-rules"),
                TaskKind::ValidateWracRules {
                    targets: validate_targets.clone(),
                },
            );
            graph.depends_on(rules, build_by_target[&Target::Clap]);
            for target in validate_targets {
                let validate = graph.task(
                    package_task_id(ctx, &format!("validate-{target:?}")),
                    TaskKind::ValidateBundle { target },
                );
                graph.depends_on(validate, rules);
                if target == ValidateTarget::Au {
                    // auval discovers Audio Units through AudioComponentRegistrar
                    // instead of taking a bundle path. Model the user-local AU
                    // install as a real dependency so validate-AU cannot observe
                    // a stale or missing component.
                    let install = graph.task(
                        package_task_id(ctx, "install-Au-for-validation"),
                        TaskKind::InstallBundle {
                            target: PluginTarget::Au,
                            scope: crate::cli::InstallScope::User,
                        },
                    );
                    graph.depends_on(install, build_by_target[&Target::Au]);
                    graph.depends_on(validate, install);
                } else {
                    graph.depends_on(validate, build_by_target[&target.target()]);
                }
            }
        }
    }
    Ok(graph)
}

fn execute_plan(
    ctx: &Context,
    profile: BuildProfile,
    graph: TaskGraph,
    dry_run: bool,
    policy: FailurePolicy,
) -> Result<()> {
    let ordered = graph.ordered()?;
    print_plan(&graph, &ordered, dry_run);
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
                "SKIP {}: depends on {}",
                graph.graph[index].id,
                failed_deps.join(", ")
            );
            statuses.insert(index, TaskStatus::Skipped);
            continue;
        }

        println!(
            "TASK {}: {}",
            graph.graph[index].id,
            graph.graph[index].label()
        );
        match run_task(ctx, profile, &graph.graph[index].kind) {
            Ok(()) => {
                statuses.insert(index, TaskStatus::Ok);
            }
            Err(err) => {
                println!("FAILED {}: {err}", graph.graph[index].id);
                statuses.insert(index, TaskStatus::Failed);
                failures.push(format!("{}: {err}", graph.graph[index].id));
                if matches!(policy, FailurePolicy::FailFast) {
                    print_summary(&graph, &statuses);
                    return Err(failures.join("\n").into());
                }
            }
        }
    }

    print_summary(&graph, &statuses);
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
        ),
        TaskKind::BuildAuBundle => build_wrapper_target(
            ctx,
            profile,
            WrapperBuild::Plugins {
                vst3: false,
                au: true,
            },
            WrapperTarget::Au,
        ),
        TaskKind::BuildAaxBundle => {
            build_wrapper_target(ctx, profile, WrapperBuild::Aax, WrapperTarget::Aax)
        }
        TaskKind::BuildStandaloneBundle => build_wrapper_target(
            ctx,
            profile,
            WrapperBuild::Standalone,
            WrapperTarget::Standalone,
        ),
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
                    "Uninstall dry run complete for {}: {removed} would be removed, {missing} not found",
                    target.display()
                );
            } else {
                println!(
                    "Uninstall complete for {}: {removed} removed, {missing} not found",
                    target.display()
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

fn print_plan(graph: &TaskGraph, ordered: &[NodeIndex], dry_run: bool) {
    println!("Plan:");
    for (position, index) in ordered.iter().enumerate() {
        println!(
            "  {}. {} - {}",
            position + 1,
            graph.graph[*index].id,
            graph.graph[*index].label()
        );
    }
    println!("Dependencies:");
    for index in ordered {
        let deps = graph
            .graph
            .neighbors_directed(*index, Direction::Incoming)
            .map(|dep| graph.graph[dep].id.to_string())
            .collect::<Vec<_>>();
        if !deps.is_empty() {
            println!("  {} <- {}", graph.graph[*index].id, deps.join(", "));
        }
    }
    if dry_run {
        println!("Nothing was executed because --dry-run was set.");
    }
}

fn print_summary(graph: &TaskGraph, statuses: &HashMap<NodeIndex, TaskStatus>) {
    let mut counts = HashMap::<TaskStatus, usize>::new();
    for status in statuses.values() {
        *counts.entry(*status).or_default() += 1;
    }
    println!(
        "Task summary: {} ok, {} failed, {} skipped",
        counts.get(&TaskStatus::Ok).copied().unwrap_or(0),
        counts.get(&TaskStatus::Failed).copied().unwrap_or(0),
        counts.get(&TaskStatus::Skipped).copied().unwrap_or(0)
    );
    for (index, status) in statuses {
        if matches!(status, TaskStatus::Failed | TaskStatus::Skipped) {
            println!("  {status:?}: {}", graph.graph[*index].id);
        }
    }
}

fn resolve_build_targets_from_metadata(ctx: &Context, requested: &[Target]) -> Result<Vec<Target>> {
    let mut targets = if requested.is_empty() {
        // supported_formats is the product policy. The development standalone
        // remains outside that list because it is not a plugin format. Default
        // selection is platform-aware so a product can support AU without making
        // Windows/Linux builds fail unless AU was explicitly requested.
        let mut targets = ctx
            .metadata
            .supported_formats
            .iter()
            .map(|format| format.target())
            .collect::<Vec<_>>();
        targets.push(Target::Standalone);
        filter_platform_targets(ctx, targets)
    } else {
        requested.to_vec()
    };
    targets = dedup(targets);
    validate_target_support(ctx, &targets, !requested.is_empty())?;
    Ok(targets)
}

fn resolve_plugin_targets_from_metadata(
    ctx: &Context,
    requested: &[PluginTarget],
) -> Result<Vec<PluginTarget>> {
    let targets = if requested.is_empty() {
        filter_platform_targets(
            ctx,
            ctx.metadata
                .supported_formats
                .iter()
                .map(|format| format.target())
                .collect::<Vec<_>>(),
        )
        .into_iter()
        .filter_map(|target| match target {
            Target::Clap => Some(PluginTarget::Clap),
            Target::Vst3 => Some(PluginTarget::Vst3),
            Target::Au => Some(PluginTarget::Au),
            Target::Aax => Some(PluginTarget::Aax),
            Target::Standalone => None,
        })
        .collect::<Vec<_>>()
    } else {
        requested.to_vec()
    };
    let targets = dedup(targets);
    validate_plugin_format_support(
        ctx,
        &plugin_formats_for_plugin_targets(&targets),
        !requested.is_empty(),
    )?;
    Ok(targets)
}

fn resolve_validate_targets_from_metadata(
    ctx: &Context,
    requested: &[ValidateTarget],
) -> Result<Vec<ValidateTarget>> {
    let targets = if requested.is_empty() {
        filter_platform_targets(
            ctx,
            ctx.metadata
                .supported_formats
                .iter()
                .map(|format| format.target())
                .collect::<Vec<_>>(),
        )
        .into_iter()
        .filter_map(|target| match target {
            Target::Clap => Some(ValidateTarget::Clap),
            Target::Vst3 => Some(ValidateTarget::Vst3),
            Target::Au => Some(ValidateTarget::Au),
            Target::Aax => Some(ValidateTarget::Aax),
            Target::Standalone => None,
        })
        .collect::<Vec<_>>()
    } else {
        requested.to_vec()
    };
    let targets = dedup(targets);
    validate_plugin_format_support(
        ctx,
        &plugin_formats_for_validate_targets(&targets),
        !requested.is_empty(),
    )?;
    Ok(targets)
}

fn filter_platform_targets(ctx: &Context, targets: Vec<Target>) -> Vec<Target> {
    targets
        .into_iter()
        .filter(|target| {
            let supported = ctx.platform.supports_target(*target);
            if !supported {
                println!(
                    "Skipping {}: not supported on {}.",
                    target.display(),
                    ctx.platform.display()
                );
            }
            supported
        })
        .collect()
}

fn plugin_formats_for_targets(targets: &[Target]) -> Vec<PluginFormat> {
    targets
        .iter()
        .filter_map(|target| target.plugin_format())
        .collect()
}

fn plugin_formats_for_plugin_targets(targets: &[PluginTarget]) -> Vec<PluginFormat> {
    targets.iter().map(|target| target.format()).collect()
}

fn plugin_formats_for_validate_targets(targets: &[ValidateTarget]) -> Vec<PluginFormat> {
    targets.iter().map(|target| target.format()).collect()
}

fn validate_target_support(ctx: &Context, targets: &[Target], explicit: bool) -> Result<()> {
    validate_plugin_format_support(ctx, &plugin_formats_for_targets(targets), explicit)?;
    validate_platform_target_support(ctx, targets)
}

fn validate_platform_target_support(ctx: &Context, targets: &[Target]) -> Result<()> {
    for target in targets {
        if !ctx.platform.supports_target(*target) {
            return Err(format!(
                "{} is not supported on {}",
                target.display(),
                ctx.platform.display()
            )
            .into());
        }
    }
    Ok(())
}

fn validate_plugin_format_support(
    ctx: &Context,
    formats: &[PluginFormat],
    explicit: bool,
) -> Result<()> {
    let supported = ctx
        .metadata
        .supported_formats
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    for format in formats {
        // An explicit --target is a request, not a hint. If a package does not
        // advertise that format, fail instead of silently falling back to the
        // supported subset.
        if explicit && !supported.contains(format) {
            return Err(format!(
                "{} is not listed in package.metadata.wrac.supported_formats for {}",
                format.display(),
                ctx.package_name
            )
            .into());
        }
        if !ctx.platform.supports_target(format.target()) {
            return Err(format!(
                "{} is not supported on {}",
                format.display(),
                ctx.platform.display()
            )
            .into());
        }
    }
    Ok(())
}

fn dedup<T: Copy + Eq + std::hash::Hash>(values: Vec<T>) -> Vec<T> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(*value))
        .collect()
}

fn package_task_id(ctx: &Context, task: &str) -> TaskId {
    TaskId::new(format!("{}:{task}", ctx.package_name))
}

fn failure_policy(continue_on_error: bool) -> FailurePolicy {
    if continue_on_error {
        FailurePolicy::Continue
    } else {
        FailurePolicy::FailFast
    }
}
