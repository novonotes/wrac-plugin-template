use std::env;
use std::error::Error;
use std::path::PathBuf;

mod cli;
mod commands;
mod context;
mod metadata;
mod plan;
mod profile;
pub mod targets;
mod util;
mod validation;

use cli::{CleanArgs, InstallScope, UninstallScope};
use commands::{clean, launch};
use context::{Context, available_packages};
use profile::BuildProfile;
use targets::{PluginTarget, Target, ValidateTarget};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Clone)]
pub struct XtaskConfig {
    pub root: PathBuf,
    pub wrapper_dir: PathBuf,
    pub target_namespace: String,
}

#[derive(Debug, Clone)]
pub struct WracWorkspace {
    config: XtaskConfig,
}

#[derive(Debug, Clone)]
pub enum WracCommand {
    Build(BuildOptions),
    Install(InstallOptions),
    Uninstall(UninstallOptions),
    Validate(ValidateOptions),
    Launch(LaunchOptions),
    Clean(CleanOptions),
}

#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    pub package: Option<String>,
    pub all: bool,
    pub release: bool,
    pub clean: bool,
    pub dry_run: bool,
    pub continue_on_error: bool,
    pub target: Vec<Target>,
}

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    pub package: Option<String>,
    pub all: bool,
    pub release: bool,
    pub scope: WracInstallScope,
    pub dry_run: bool,
    pub continue_on_error: bool,
    pub target: Vec<PluginTarget>,
}

#[derive(Debug, Clone, Default)]
pub struct UninstallOptions {
    pub package: Option<String>,
    pub all: bool,
    pub scope: WracUninstallScope,
    pub target: Vec<PluginTarget>,
    pub dry_run: bool,
    pub continue_on_error: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ValidateOptions {
    pub package: Option<String>,
    pub all: bool,
    pub release: bool,
    pub dry_run: bool,
    pub continue_on_error: bool,
    pub target: Vec<ValidateTarget>,
}

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub package: Option<String>,
    pub release: bool,
    pub plugin_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CleanOptions {
    pub package: Option<String>,
    pub all: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WracInstallScope {
    Default,
    User,
    System,
}

impl Default for WracInstallScope {
    fn default() -> Self {
        Self::Default
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WracUninstallScope {
    All,
    User,
    System,
}

impl Default for WracUninstallScope {
    fn default() -> Self {
        Self::All
    }
}

impl WracWorkspace {
    pub fn new(config: XtaskConfig) -> Result<Self> {
        load_workspace_dotenv(&config)?;
        Ok(Self { config })
    }

    pub fn run(&self, command: WracCommand) -> Result<()> {
        match command {
            WracCommand::Build(options) => {
                self.run_build(options)?;
            }
            WracCommand::Install(options) => {
                self.run_install(options)?;
            }
            WracCommand::Uninstall(options) => {
                self.run_uninstall(options)?;
            }
            WracCommand::Validate(options) => {
                self.run_validate(options)?;
            }
            WracCommand::Launch(options) => {
                self.run_launch(options)?;
            }
            WracCommand::Clean(options) => {
                self.run_clean(options)?;
            }
        }
        Ok(())
    }

    fn run_build(&self, options: BuildOptions) -> Result<()> {
        let args = cli::BuildArgs {
            package: options.package,
            all: options.all,
            release: options.release,
            clean: options.clean,
            dry_run: options.dry_run,
            continue_on_error: options.continue_on_error,
            target: options.target,
        };
        // Keep build/install logic scoped to one plugin package at a time. A package may
        // export multiple plugin products; the shared Context is still the correct unit for
        // metadata, GUI assets, wrapper staging, and install paths.
        let mut failures = Vec::new();
        for package in selected_packages(&self.config, args.package.as_deref(), args.all)? {
            let ctx = Context::new(&self.config, &package)?;
            if let Err(err) = plan::run_build(&ctx, &args) {
                if args.continue_on_error {
                    failures.push(format!("{package}: {err}"));
                } else {
                    return Err(err);
                }
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("\n").into());
        }
        Ok(())
    }

    fn run_install(&self, options: InstallOptions) -> Result<()> {
        let args = cli::InstallArgs {
            package: options.package,
            all: options.all,
            release: options.release,
            scope: options.scope.into(),
            dry_run: options.dry_run,
            continue_on_error: options.continue_on_error,
            target: options.target,
        };
        let mut failures = Vec::new();
        for package in selected_packages(&self.config, args.package.as_deref(), args.all)? {
            let ctx = Context::new(&self.config, &package)?;
            if let Err(err) = plan::run_install(&ctx, &args) {
                if args.continue_on_error {
                    failures.push(format!("{package}: {err}"));
                } else {
                    return Err(err);
                }
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("\n").into());
        }
        Ok(())
    }

    fn run_uninstall(&self, options: UninstallOptions) -> Result<()> {
        let args = cli::UninstallArgs {
            package: options.package,
            all: options.all,
            scope: options.scope.into(),
            target: options.target,
            dry_run: options.dry_run,
            continue_on_error: options.continue_on_error,
        };
        let mut failures = Vec::new();
        for package in selected_packages(&self.config, args.package.as_deref(), args.all)? {
            let ctx = Context::new(&self.config, &package)?;
            if let Err(err) = plan::run_uninstall(&ctx, &args) {
                if args.continue_on_error {
                    failures.push(format!("{package}: {err}"));
                } else {
                    return Err(err);
                }
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("\n").into());
        }
        Ok(())
    }

    fn run_validate(&self, options: ValidateOptions) -> Result<()> {
        let args = cli::ValidateArgs {
            package: options.package,
            all: options.all,
            release: options.release,
            dry_run: options.dry_run,
            continue_on_error: options.continue_on_error,
            target: options.target,
        };
        let mut failures = Vec::new();
        for package in selected_packages(&self.config, args.package.as_deref(), args.all)? {
            let ctx = Context::new(&self.config, &package)?;
            if let Err(err) = plan::run_validate(&ctx, &args) {
                if args.continue_on_error {
                    failures.push(format!("{package}: {err}"));
                } else {
                    return Err(err);
                }
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("\n").into());
        }
        Ok(())
    }

    fn run_launch(&self, options: LaunchOptions) -> Result<()> {
        let args = cli::LaunchArgs {
            package: options.package,
            release: options.release,
            plugin_id: options.plugin_id,
        };
        let package = selected_package(&self.config, args.package.as_deref())?;
        let ctx = Context::new(&self.config, &package)?;
        // Validate product selection before the implicit standalone build.
        // A typo in --plugin-id is independent of artifacts and should not
        // spend time configuring CMake or building wrapper dependencies.
        commands::ensure_launch_target_exists(&ctx, args.plugin_id.as_deref())?;
        plan::run_build(&ctx, &args_for_launch_build(&args))?;
        launch(
            &ctx,
            BuildProfile::from_release(args.release),
            args.plugin_id.as_deref(),
        )?;
        Ok(())
    }

    fn run_clean(&self, options: CleanOptions) -> Result<()> {
        let args = CleanArgs {
            package: options.package,
            all: options.all,
        };
        for package in selected_packages(&self.config, args.package.as_deref(), args.all)? {
            let ctx = Context::new(&self.config, &package)?;
            clean(&ctx)?;
        }
        Ok(())
    }
}

fn load_workspace_dotenv(config: &XtaskConfig) -> Result<()> {
    let path = config.root.join(".env");
    if !path.exists() {
        return Ok(());
    }

    // `.env` is for project-local machine paths such as the AAX SDK. Do not
    // override the process environment so CI variables and one-off shell
    // overrides keep higher precedence than the repository-local file.
    for entry in dotenvy::from_path_iter(&path)? {
        let (key, value) = entry?;
        if env::var_os(&key).is_none() {
            // xtask loads .env before starting worker threads or subprocesses.
            // Mutating the process environment at this point lets the existing
            // command code and child processes consume one consistent source.
            unsafe {
                env::set_var(key, value);
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

fn args_for_launch_build(args: &cli::LaunchArgs) -> cli::BuildArgs {
    // launch is not "build the package defaults, then open an app"; it needs
    // exactly the standalone terminal task and its dependencies. Using the same
    // DAG entrypoint as `xtask build` keeps dependency behavior aligned without
    // accidentally pulling in supported plugin formats such as AAX.
    cli::BuildArgs {
        package: None,
        all: false,
        release: args.release,
        clean: false,
        dry_run: false,
        continue_on_error: false,
        target: vec![targets::Target::Standalone],
    }
}

impl From<WracInstallScope> for InstallScope {
    fn from(scope: WracInstallScope) -> Self {
        match scope {
            WracInstallScope::Default => Self::Default,
            WracInstallScope::User => Self::User,
            WracInstallScope::System => Self::System,
        }
    }
}

impl From<WracUninstallScope> for UninstallScope {
    fn from(scope: WracUninstallScope) -> Self {
        match scope {
            WracUninstallScope::All => Self::All,
            WracUninstallScope::User => Self::User,
            WracUninstallScope::System => Self::System,
        }
    }
}
