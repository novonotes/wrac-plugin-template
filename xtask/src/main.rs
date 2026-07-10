//! Repository-local `cargo xtask` entry point for the WRAC template.
//!
//! This crate owns the template command planner and task executor. Shared
//! crates provide graph primitives and typed WRAC build operations, but the
//! task enum and dependencies live here so products can extend the workflow
//! without changing the template libraries.

use std::{collections::HashMap, path::Path};

use clap::{Args, Parser, Subcommand, ValueEnum};
use wrac_build_ops::{
    BuildProfile, InstallScope, RustPluginBuild, UninstallScope, WracContext, WrapperBuild,
    WrapperTarget, XtaskConfig, XtaskOutputLanguage, build_gui, build_rust_plugin,
    build_wrapper_target, check_install_dir, clean, configure_wrapper, install_plugin_target,
    launch, load_workspace_dotenv, package_clap, package_task_id, print_build_outputs,
    resolve_build_targets_from_metadata, resolve_plugin_targets_from_metadata,
    resolve_validate_targets_from_metadata, select_packages, select_single_package,
    targets::{PluginTarget, Target, ValidateTarget},
    uninstall_plugin_target, validate_plugin_target, validate_wrac_rules_for_targets,
};
use xtask_workflow::{
    FailurePolicy, TaskGraph, TaskNode, TaskOutcome, WorkflowMessages, execute_plan, failure_policy,
};

type Result<T> = wrac_build_ops::Result<T>;
type TaskPlan = TaskGraph<TaskKind>;

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be a direct child of the repository root")
        .to_path_buf();
    let config = XtaskConfig {
        wrapper_dir: root.join("clap_wrapper_builder"),
        target_namespace: "wrac-plugins".to_string(),
        default_aax_sdk_root: None,
        output_language: XtaskOutputLanguage::English,
        root,
    };
    load_workspace_dotenv(&config)?;
    let cli = Cli::parse();
    match cli.command {
        Command::Build(args) => execute_build(&config, args),
        Command::Install(args) => execute_install(&config, args),
        Command::Uninstall(args) => execute_uninstall(&config, args),
        Command::Validate(args) => execute_validate(&config, args),
        Command::Launch(args) => execute_launch(&config, args),
        Command::Clean(args) => execute_clean(&config, args),
        Command::Quality => wrac_build_ops::run_quality(&config.root),
    }
}

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "WRAC template build tasks.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Build(BuildArgs),
    Install(InstallArgs),
    Uninstall(UninstallArgs),
    #[command(after_help = "\
When --checks is omitted, validate builds artifacts, runs WRAC production-readiness checks,
and runs external format validators.

Examples:
  xtask validate --target=clap,vst3
  xtask validate --target=clap,vst3 --checks build-artifacts
  xtask validate --target=clap,vst3 --checks build-artifacts,external-validators,production-readiness")]
    Validate(ValidateArgs),
    Launch(LaunchArgs),
    Clean(CleanArgs),
    Quality,
}

#[derive(Debug, Args)]
struct BuildArgs {
    #[arg(short = 'p', long = "package")]
    package: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
    #[arg(long)]
    release: bool,
    #[arg(long)]
    clean: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    continue_on_error: bool,
    #[arg(short = 't', long, value_enum, value_delimiter = ',', num_args = 1..)]
    target: Vec<Target>,
    #[arg(long = "plugin-id")]
    plugin_id: Option<String>,
}

#[derive(Debug, Args)]
struct InstallArgs {
    #[arg(short = 'p', long = "package")]
    package: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
    #[arg(long)]
    release: bool,
    #[arg(short = 's', long, value_enum, default_value_t = InstallScopeArg::Default)]
    scope: InstallScopeArg,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    continue_on_error: bool,
    #[arg(short = 't', long, value_enum, value_delimiter = ',', num_args = 1..)]
    target: Vec<PluginTarget>,
}

#[derive(Debug, Args)]
struct UninstallArgs {
    #[arg(short = 'p', long = "package")]
    package: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
    #[arg(short = 's', long, value_enum, default_value_t = UninstallScopeArg::All)]
    scope: UninstallScopeArg,
    #[arg(short = 't', long, value_enum, value_delimiter = ',', num_args = 1..)]
    target: Vec<PluginTarget>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    continue_on_error: bool,
}

#[derive(Debug, Args)]
struct ValidateArgs {
    #[arg(short = 'p', long = "package")]
    package: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
    #[arg(long)]
    release: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    continue_on_error: bool,
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Validation checks to run. Omit to run every validation check."
    )]
    checks: Vec<ValidateCheckArg>,
    #[arg(short = 't', long, value_enum, value_delimiter = ',', num_args = 1..)]
    target: Vec<ValidateTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ValidateCheckArg {
    BuildArtifacts,
    ExternalValidators,
    ProductionReadiness,
}

#[derive(Debug, Args)]
struct LaunchArgs {
    #[arg(short = 'p', long = "package")]
    package: Option<String>,
    #[arg(long)]
    release: bool,
    #[arg(long)]
    plugin_id: Option<String>,
}

#[derive(Debug, Args)]
struct CleanArgs {
    #[arg(short = 'p', long = "package")]
    package: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InstallScopeArg {
    Default,
    User,
    System,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum UninstallScopeArg {
    All,
    User,
    System,
}

impl From<InstallScopeArg> for InstallScope {
    fn from(scope: InstallScopeArg) -> Self {
        match scope {
            InstallScopeArg::Default => Self::Default,
            InstallScopeArg::User => Self::User,
            InstallScopeArg::System => Self::System,
        }
    }
}

impl From<UninstallScopeArg> for UninstallScope {
    fn from(scope: UninstallScopeArg) -> Self {
        match scope {
            UninstallScopeArg::All => Self::All,
            UninstallScopeArg::User => Self::User,
            UninstallScopeArg::System => Self::System,
        }
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
            Self::BuildStandaloneBundle { plugin_id } => match plugin_id {
                Some(plugin_id) => format!("build standalone artifact ({plugin_id})"),
                None => "build standalone artifact".to_string(),
            },
            Self::LaunchStandalone { plugin_id } => match plugin_id {
                Some(plugin_id) => format!("launch standalone artifact ({plugin_id})"),
                None => "launch standalone artifact".to_string(),
            },
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

fn execute_package_plan(
    ctx: &WracContext,
    profile: BuildProfile,
    graph: TaskPlan,
    dry_run: bool,
    policy: FailurePolicy,
) -> Result<()> {
    execute_plan(
        graph,
        dry_run,
        policy,
        &WorkflowMessages::ENGLISH,
        |kind| kind.label(),
        |kind| run_task(ctx, profile, kind),
    )
}

fn run_task(ctx: &WracContext, profile: BuildProfile, kind: &TaskKind) -> Result<TaskOutcome> {
    match kind {
        TaskKind::Clean => completed(clean(ctx)),
        TaskKind::BuildGui => build_gui(ctx),
        TaskKind::BuildRustDefault => {
            completed(build_rust_plugin(ctx, profile, RustPluginBuild::Default))
        }
        TaskKind::BuildRustStandalone => {
            completed(build_rust_plugin(ctx, profile, RustPluginBuild::Standalone))
        }
        TaskKind::PackageClap => completed(package_clap(ctx, profile)),
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
        TaskKind::BuildVst3Bundle => completed(build_wrapper_target(
            ctx,
            profile,
            WrapperBuild::Plugins {
                vst3: true,
                au: false,
            },
            WrapperTarget::Vst3,
            None,
        )),
        TaskKind::BuildAuBundle => completed(build_wrapper_target(
            ctx,
            profile,
            WrapperBuild::Plugins {
                vst3: false,
                au: true,
            },
            WrapperTarget::Au,
            None,
        )),
        TaskKind::BuildAaxBundle => completed(build_wrapper_target(
            ctx,
            profile,
            WrapperBuild::Aax,
            WrapperTarget::Aax,
            None,
        )),
        TaskKind::BuildStandaloneBundle { plugin_id } => completed(build_wrapper_target(
            ctx,
            profile,
            WrapperBuild::Standalone,
            WrapperTarget::Standalone,
            plugin_id.as_deref(),
        )),
        TaskKind::LaunchStandalone { plugin_id } => {
            completed(launch(ctx, profile, plugin_id.as_deref()))
        }
        TaskKind::CheckInstallScope { target, scope } => {
            completed(check_install_dir(ctx, *scope, target.format()))
        }
        TaskKind::InstallBundle { target, scope } => {
            completed(install_plugin_target(ctx, profile, *scope, *target))
        }
        TaskKind::UninstallBundle {
            target,
            scope,
            dry_run,
        } => {
            let (removed, missing) = uninstall_plugin_target(ctx, *scope, *target, *dry_run)?;
            if *dry_run {
                println!("  {}", uninstall_summary(*target, removed, missing, true));
            } else {
                println!("  {}", uninstall_summary(*target, removed, missing, false));
            }
            Ok(TaskOutcome::Completed)
        }
        TaskKind::ValidateWracRules { targets } => {
            completed(validate_wrac_rules_for_targets(ctx, profile, targets))
        }
        TaskKind::ValidateBundle { target } => {
            completed(validate_plugin_target(ctx, profile, *target))
        }
    }
}

fn completed(result: Result<()>) -> Result<TaskOutcome> {
    // Operations that cannot skip still return a workflow outcome so the
    // workflow remains the single owner of the user-facing task result.
    result.map(|()| TaskOutcome::Completed)
}

fn execute_build(config: &XtaskConfig, args: BuildArgs) -> Result<()> {
    for package in select_packages(config, args.package.as_deref(), args.all)? {
        let ctx = WracContext::new(config, &package)?;
        let profile = BuildProfile::from_release(args.release);
        let targets = resolve_build_targets_from_metadata(&ctx, &args.target)?;
        let artifact_plan =
            build_artifact_plan(&ctx, &targets, args.clean, args.plugin_id.clone(), None);
        execute_package_plan(
            &ctx,
            profile,
            artifact_plan.graph,
            args.dry_run,
            failure_policy(args.continue_on_error),
        )?;
        if !args.dry_run {
            print_build_outputs(&ctx, profile, &targets, args.plugin_id.as_deref())?;
        }
    }
    Ok(())
}

fn execute_install(config: &XtaskConfig, args: InstallArgs) -> Result<()> {
    for package in select_packages(config, args.package.as_deref(), args.all)? {
        let ctx = WracContext::new(config, &package)?;
        let profile = BuildProfile::from_release(args.release);
        let targets = resolve_plugin_targets_from_metadata(&ctx, &args.target)?;
        let build_targets = targets
            .iter()
            .map(|target| target.target())
            .collect::<Vec<_>>();
        let scope: InstallScope = args.scope.into();
        let mut artifact_plan =
            build_artifact_plan(&ctx, &build_targets, false, None, Some((&targets, scope)));
        for target in targets {
            let install = artifact_plan.graph.task(
                package_task_id(&ctx, &format!("install-{target:?}")),
                TaskKind::InstallBundle { target, scope },
            );
            artifact_plan
                .graph
                .depends_on(install, artifact_plan.build_by_target[&target.target()]);
        }
        execute_package_plan(
            &ctx,
            profile,
            artifact_plan.graph,
            args.dry_run,
            failure_policy(args.continue_on_error),
        )?;
    }
    Ok(())
}

fn execute_uninstall(config: &XtaskConfig, args: UninstallArgs) -> Result<()> {
    for package in select_packages(config, args.package.as_deref(), args.all)? {
        let ctx = WracContext::new(config, &package)?;
        let targets = resolve_plugin_targets_from_metadata(&ctx, &args.target)?;
        let mut plan = TaskPlan::new();
        for target in targets {
            plan.task(
                package_task_id(&ctx, &format!("uninstall-{target:?}")),
                TaskKind::UninstallBundle {
                    target,
                    scope: args.scope.into(),
                    dry_run: args.dry_run,
                },
            );
        }
        execute_package_plan(
            &ctx,
            BuildProfile::Debug,
            plan,
            false,
            failure_policy(args.continue_on_error),
        )?;
    }
    Ok(())
}

fn execute_validate(config: &XtaskConfig, args: ValidateArgs) -> Result<()> {
    for package in select_packages(config, args.package.as_deref(), args.all)? {
        let ctx = WracContext::new(config, &package)?;
        let profile = BuildProfile::from_release(args.release);
        let validate_targets = resolve_validate_targets_from_metadata(&ctx, &args.target)?;
        let mut build_targets = validate_targets
            .iter()
            .map(|target| target.target())
            .collect::<Vec<_>>();
        let run_readiness = validate_checks(&args).run_production_readiness;
        if run_readiness && !build_targets.contains(&Target::Clap) {
            build_targets.push(Target::Clap);
        }
        let mut artifact_plan = build_artifact_plan(&ctx, &build_targets, false, None, None);
        let rules = if run_readiness {
            let rules = artifact_plan.graph.task(
                package_task_id(&ctx, "validate-wrac-rules"),
                TaskKind::ValidateWracRules {
                    targets: validate_targets.clone(),
                },
            );
            artifact_plan
                .graph
                .depends_on(rules, artifact_plan.build_by_target[&Target::Clap]);
            Some(rules)
        } else {
            None
        };
        if validate_checks(&args).run_external_validators {
            for target in validate_targets {
                let validate = artifact_plan.graph.task(
                    package_task_id(&ctx, &format!("validate-{target:?}")),
                    TaskKind::ValidateBundle { target },
                );
                if let Some(rules) = rules {
                    artifact_plan.graph.depends_on(validate, rules);
                }
                if target == ValidateTarget::Au {
                    let install = artifact_plan.graph.task(
                        package_task_id(&ctx, "install-Au-for-validation"),
                        TaskKind::InstallBundle {
                            target: PluginTarget::Au,
                            scope: InstallScope::User,
                        },
                    );
                    artifact_plan
                        .graph
                        .depends_on(install, artifact_plan.build_by_target[&Target::Au]);
                    artifact_plan.graph.depends_on(validate, install);
                } else {
                    artifact_plan
                        .graph
                        .depends_on(validate, artifact_plan.build_by_target[&target.target()]);
                }
            }
        }
        execute_package_plan(
            &ctx,
            profile,
            artifact_plan.graph,
            args.dry_run,
            failure_policy(args.continue_on_error),
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ValidateChecks {
    run_external_validators: bool,
    run_production_readiness: bool,
}

fn validate_checks(args: &ValidateArgs) -> ValidateChecks {
    if args.checks.is_empty() {
        return ValidateChecks {
            run_external_validators: true,
            run_production_readiness: true,
        };
    }
    ValidateChecks {
        run_external_validators: args.checks.contains(&ValidateCheckArg::ExternalValidators),
        run_production_readiness: args.checks.contains(&ValidateCheckArg::ProductionReadiness),
    }
}

fn execute_launch(config: &XtaskConfig, args: LaunchArgs) -> Result<()> {
    let package = select_single_package(config, args.package.as_deref())?;
    let ctx = WracContext::new(config, &package)?;
    let profile = BuildProfile::from_release(args.release);
    let mut artifact_plan = build_artifact_plan(
        &ctx,
        &[Target::Standalone],
        false,
        args.plugin_id.clone(),
        None,
    );
    let launch = artifact_plan.graph.task(
        package_task_id(&ctx, "launch-standalone"),
        TaskKind::LaunchStandalone {
            plugin_id: args.plugin_id,
        },
    );
    artifact_plan
        .graph
        .depends_on(launch, artifact_plan.build_by_target[&Target::Standalone]);
    execute_package_plan(
        &ctx,
        profile,
        artifact_plan.graph,
        false,
        FailurePolicy::FailFast,
    )
}

fn execute_clean(config: &XtaskConfig, args: CleanArgs) -> Result<()> {
    for package in select_packages(config, args.package.as_deref(), args.all)? {
        let ctx = WracContext::new(config, &package)?;
        let mut plan = TaskPlan::new();
        plan.task(package_task_id(&ctx, "clean"), TaskKind::Clean);
        execute_package_plan(
            &ctx,
            BuildProfile::Debug,
            plan,
            false,
            FailurePolicy::FailFast,
        )?;
    }
    Ok(())
}

fn uninstall_summary(
    target: PluginTarget,
    removed: usize,
    missing: usize,
    dry_run: bool,
) -> String {
    match dry_run {
        true => format!(
            "Summary: {} {} would be removed, {} not found",
            target.display(),
            removed,
            missing
        ),
        false => format!(
            "Summary: {} {} removed, {} not found",
            target.display(),
            removed,
            missing
        ),
    }
}

struct ArtifactPlan {
    graph: TaskPlan,
    build_by_target: HashMap<Target, TaskNode>,
}

fn build_artifact_plan(
    ctx: &WracContext,
    targets: &[Target],
    clean_first: bool,
    standalone_plugin_id: Option<String>,
    install_checks: Option<(&[PluginTarget], InstallScope)>,
) -> ArtifactPlan {
    let mut graph = TaskPlan::new();
    let mut build_by_target = HashMap::new();
    let clean = clean_first.then(|| graph.task(package_task_id(ctx, "clean"), TaskKind::Clean));
    let checks = install_checks
        .map(|(targets, scope)| {
            targets
                .iter()
                .map(|target| {
                    let check = graph.task(
                        package_task_id(ctx, &format!("check-install-scope-{target:?}")),
                        TaskKind::CheckInstallScope {
                            target: *target,
                            scope,
                        },
                    );
                    (target.target(), check)
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let needs_default = targets.iter().any(|target| {
        matches!(
            target,
            Target::Clap | Target::Vst3 | Target::Au | Target::Aax
        )
    });
    let needs_standalone = targets.contains(&Target::Standalone);
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
        let rust = graph.task(
            package_task_id(ctx, "build-rust-default"),
            TaskKind::BuildRustDefault,
        );
        graph.depends_on(rust, build_gui.expect("default Rust build needs GUI"));
        Some(rust)
    } else {
        None
    };
    let rust_standalone = if needs_standalone {
        let rust = graph.task(
            package_task_id(ctx, "build-rust-standalone"),
            TaskKind::BuildRustStandalone,
        );
        graph.depends_on(rust, build_gui.expect("standalone Rust build needs GUI"));
        Some(rust)
    } else {
        None
    };
    if targets.contains(&Target::Clap) {
        let clap = graph.task(package_task_id(ctx, "package-clap"), TaskKind::PackageClap);
        graph.depends_on(
            clap,
            rust_default.expect("CLAP packaging needs default Rust build"),
        );
        if let Some(check) = checks.get(&Target::Clap) {
            graph.depends_on(clap, *check);
        }
        build_by_target.insert(Target::Clap, clap);
    }
    let needs_vst3 = targets.contains(&Target::Vst3);
    let needs_au = targets.contains(&Target::Au);
    if needs_vst3 || needs_au {
        let configure = graph.task(
            package_task_id(ctx, "configure-wrapper-plugins"),
            TaskKind::ConfigureWrapperPlugins {
                vst3: needs_vst3 || ctx.platform.supports_vst3(),
                au: needs_au || ctx.platform.supports_au(),
            },
        );
        graph.depends_on(
            configure,
            rust_default.expect("wrapper builds need default Rust build"),
        );
        for target in [Target::Vst3, Target::Au] {
            if let Some(check) = checks.get(&target) {
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
    if targets.contains(&Target::Aax) {
        let configure = graph.task(
            package_task_id(ctx, "configure-wrapper-aax"),
            TaskKind::ConfigureWrapperAax,
        );
        graph.depends_on(
            configure,
            rust_default.expect("AAX wrapper needs default Rust build"),
        );
        if let Some(check) = checks.get(&Target::Aax) {
            graph.depends_on(configure, *check);
        }
        let aax = graph.task(package_task_id(ctx, "build-aax"), TaskKind::BuildAaxBundle);
        graph.depends_on(aax, configure);
        build_by_target.insert(Target::Aax, aax);
    }
    if targets.contains(&Target::Standalone) {
        let configure = graph.task(
            package_task_id(ctx, "configure-wrapper-standalone"),
            TaskKind::ConfigureWrapperStandalone,
        );
        graph.depends_on(
            configure,
            rust_standalone.expect("standalone wrapper needs Rust build"),
        );
        let standalone = graph.task(
            package_task_id(ctx, "build-standalone"),
            TaskKind::BuildStandaloneBundle {
                plugin_id: standalone_plugin_id,
            },
        );
        graph.depends_on(standalone, configure);
        build_by_target.insert(Target::Standalone, standalone);
    }
    ArtifactPlan {
        graph,
        build_by_target,
    }
}
