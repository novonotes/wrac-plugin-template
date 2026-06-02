use std::error::Error;
use std::path::PathBuf;

use clap::Parser;

mod cli;
mod commands;
mod context;
mod metadata;
mod profile;
mod targets;
mod util;

use cli::{Cli, Commands};
use commands::{build, clean, install, launch, uninstall, validate};
use context::{Context, available_packages};
use profile::BuildProfile;
use targets::{Target, resolve_plugin_targets, resolve_validate_targets};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Clone)]
pub struct XtaskConfig {
    pub root: PathBuf,
    pub wrapper_dir: PathBuf,
    pub target_namespace: String,
}

pub fn run(config: XtaskConfig) -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build(args) => {
            // Keep build/install logic scoped to one plugin package at a time. A package may
            // export multiple plugin products; the shared Context is still the correct unit for
            // metadata, GUI assets, wrapper staging, and install paths.
            for package in selected_packages(&config, args.package.as_deref(), args.all)? {
                let ctx = Context::new(&config, &package)?;
                build(&ctx, args_for_build(&args))?;
            }
        }
        Commands::Install(args) => {
            for package in selected_packages(&config, args.package.as_deref(), args.all)? {
                let ctx = Context::new(&config, &package)?;
                build(&ctx, args_for_install_build(&ctx, &args)?)?;
                install(
                    &ctx,
                    BuildProfile::from_release(args.release),
                    args.scope,
                    &args.target,
                )?;
            }
        }
        Commands::Uninstall(args) => {
            for package in selected_packages(&config, args.package.as_deref(), args.all)? {
                let ctx = Context::new(&config, &package)?;
                uninstall(&ctx, args.scope, &args.target, args.dry_run)?;
            }
        }
        Commands::Validate(args) => {
            for package in selected_packages(&config, args.package.as_deref(), args.all)? {
                let ctx = Context::new(&config, &package)?;
                build(&ctx, args_for_validate_build(&ctx, &args)?)?;
                validate(&ctx, BuildProfile::from_release(args.release), &args.target)?;
            }
        }
        Commands::Launch(args) => {
            let package = selected_package(&config, args.package.as_deref())?;
            let ctx = Context::new(&config, &package)?;
            build(&ctx, args_for_launch_build(&args))?;
            launch(&ctx, BuildProfile::from_release(args.release))?;
        }
        Commands::Clean(args) => {
            for package in selected_packages(&config, args.package.as_deref(), args.all)? {
                let ctx = Context::new(&config, &package)?;
                clean(&ctx)?;
            }
        }
    }

    Ok(())
}

fn selected_packages(
    config: &XtaskConfig,
    package: Option<&str>,
    all: bool,
) -> Result<Vec<String>> {
    if all {
        if package.is_some() {
            return Err("--package and --all cannot be used together".into());
        }
        let packages = available_packages(config)?
            .into_iter()
            .map(|package| package.package_name)
            .collect::<Vec<_>>();
        if packages.is_empty() {
            return Err("no WRAC plugin packages found in workspace members".into());
        }
        return Ok(packages);
    }
    if let Some(package) = package {
        return Ok(vec![package.to_string()]);
    }
    Ok(vec![selected_package(config, None)?])
}

fn selected_package(config: &XtaskConfig, package: Option<&str>) -> Result<String> {
    if let Some(package) = package {
        return Ok(package.to_string());
    }
    let packages = available_packages(config)?;
    match packages.as_slice() {
        [] => Err("no WRAC plugin packages found in workspace members".into()),
        [package] => Ok(package.package_name.clone()),
        _ => Err(format!(
            "multiple WRAC plugin packages found: {}. Use -p <PACKAGE> or --all.",
            packages
                .iter()
                .map(|package| package.package_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into()),
    }
}

fn args_for_build(args: &cli::BuildArgs) -> cli::BuildArgs {
    // Build is the only command where the command object is passed onward. Strip the
    // repository-level selection flags before handing it to template-derived build code.
    cli::BuildArgs {
        package: None,
        all: false,
        release: args.release,
        clean: args.clean,
        target: args.target.clone(),
    }
}

fn args_for_install_build(ctx: &Context, args: &cli::InstallArgs) -> Result<cli::BuildArgs> {
    let targets = resolve_plugin_targets(ctx.platform, &args.target)?
        .into_iter()
        .map(|target| target.target())
        .collect();
    Ok(args_for_implicit_build(args.release, targets))
}

fn args_for_validate_build(ctx: &Context, args: &cli::ValidateArgs) -> Result<cli::BuildArgs> {
    let targets = resolve_validate_targets(ctx.platform, &args.target)?
        .into_iter()
        .map(|target| target.target())
        .collect();
    Ok(args_for_implicit_build(args.release, targets))
}

fn args_for_launch_build(args: &cli::LaunchArgs) -> cli::BuildArgs {
    args_for_implicit_build(args.release, vec![Target::Standalone])
}

fn args_for_implicit_build(release: bool, target: Vec<Target>) -> cli::BuildArgs {
    cli::BuildArgs {
        package: None,
        all: false,
        release,
        clean: false,
        target,
    }
}
