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
use context::{Context, available_plugins};
use profile::BuildProfile;
use targets::{Target, resolve_plugin_targets, resolve_validate_targets};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Clone)]
pub struct XtaskConfig {
    pub root: PathBuf,
    pub plugins_dir: PathBuf,
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
            for plugin in selected_plugins(&config, args.plugin.as_deref(), args.all)? {
                let ctx = Context::new(&config, &plugin)?;
                build(&ctx, args_for_build(&args))?;
            }
        }
        Commands::Install(args) => {
            for plugin in selected_plugins(&config, args.plugin.as_deref(), args.all)? {
                let ctx = Context::new(&config, &plugin)?;
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
            for plugin in selected_plugins(&config, args.plugin.as_deref(), args.all)? {
                let ctx = Context::new(&config, &plugin)?;
                uninstall(&ctx, args.scope, &args.target, args.dry_run)?;
            }
        }
        Commands::Validate(args) => {
            for plugin in selected_plugins(&config, args.plugin.as_deref(), args.all)? {
                let ctx = Context::new(&config, &plugin)?;
                build(&ctx, args_for_validate_build(&ctx, &args)?)?;
                validate(&ctx, BuildProfile::from_release(args.release), &args.target)?;
            }
        }
        Commands::Launch(args) => {
            let plugin = selected_plugin(&config, args.plugin.as_deref())?;
            let ctx = Context::new(&config, &plugin)?;
            build(&ctx, args_for_launch_build(&args))?;
            launch(&ctx, BuildProfile::from_release(args.release))?;
        }
        Commands::Clean(args) => {
            for plugin in selected_plugins(&config, args.plugin.as_deref(), args.all)? {
                let ctx = Context::new(&config, &plugin)?;
                clean(&ctx)?;
            }
        }
    }

    Ok(())
}

fn selected_plugins(config: &XtaskConfig, plugin: Option<&str>, all: bool) -> Result<Vec<String>> {
    if all {
        if plugin.is_some() {
            return Err("--plugin and --all cannot be used together".into());
        }
        return available_plugins(config);
    }
    if let Some(plugin) = plugin {
        return Ok(vec![plugin.to_string()]);
    }
    Ok(vec![selected_plugin(config, None)?])
}

fn selected_plugin(config: &XtaskConfig, plugin: Option<&str>) -> Result<String> {
    if let Some(plugin) = plugin {
        return Ok(plugin.to_string());
    }
    let plugins = available_plugins(config)?;
    match plugins.as_slice() {
        [] => Err(format!(
            "no plugin packages found under {}",
            config.plugins_dir.display()
        )
        .into()),
        [plugin] => Ok(plugin.clone()),
        _ => Err(format!(
            "multiple plugin packages found under {}: {}. Use --plugin <PLUGIN> or --all.",
            config.plugins_dir.display(),
            plugins.join(", ")
        )
        .into()),
    }
}

fn args_for_build(args: &cli::BuildArgs) -> cli::BuildArgs {
    // Build is the only command where the command object is passed onward. Strip the
    // repository-level selection flags before handing it to template-derived build code.
    cli::BuildArgs {
        plugin: None,
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
        plugin: None,
        all: false,
        release,
        clean: false,
        target,
    }
}
