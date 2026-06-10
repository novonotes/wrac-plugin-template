use clap::{Args, Parser, Subcommand, ValueEnum};
use wrac_xtask::targets::{PluginTarget, Target, ValidateTarget};
use wrac_xtask::{
    BuildOptions, CleanOptions, InstallOptions, LaunchOptions, UninstallOptions, ValidateOptions,
    WracCommand, WracInstallScope, WracUninstallScope,
};

pub(crate) fn command() -> WracCommand {
    Cli::parse().command.into()
}

#[derive(Debug, Parser)]
#[command(
    name = "xtask",
    about = "Build, install, validate, and clean WRAC plugin artifacts.",
    after_help = "\
This repository-local CLI is intentionally thin. wrac_xtask provides the typed
building blocks; the template owns only argument parsing and workspace wiring."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(
        about = "Build plugin and standalone artifacts.",
        after_help = "Use --dry-run to print the task graph without executing it."
    )]
    Build(BuildArgs),
    #[command(
        about = "Build and install plugin artifacts.",
        after_help = "Default scope installs CLAP/VST3/AU user-locally and AAX system-wide."
    )]
    Install(InstallArgs),
    #[command(
        about = "Remove installed plugin artifacts.",
        after_help = "Default scope removes both user-local and system-wide plugin artifacts."
    )]
    Uninstall(UninstallArgs),
    #[command(
        about = "Build and validate plugin artifacts.",
        after_help = "Use --continue-on-error to keep independent validation tasks running after a failure."
    )]
    Validate(ValidateArgs),
    #[command(about = "Build and launch the standalone artifact.")]
    Launch(LaunchArgs),
    #[command(about = "Remove generated build artifacts managed by xtask.")]
    Clean(CleanArgs),
}

impl From<Commands> for WracCommand {
    fn from(command: Commands) -> Self {
        match command {
            Commands::Build(args) => Self::Build(args.into()),
            Commands::Install(args) => Self::Install(args.into()),
            Commands::Uninstall(args) => Self::Uninstall(args.into()),
            Commands::Validate(args) => Self::Validate(args.into()),
            Commands::Launch(args) => Self::Launch(args.into()),
            Commands::Clean(args) => Self::Clean(args.into()),
        }
    }
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
}

impl From<BuildArgs> for BuildOptions {
    fn from(args: BuildArgs) -> Self {
        Self {
            package: args.package,
            all: args.all,
            release: args.release,
            clean: args.clean,
            dry_run: args.dry_run,
            continue_on_error: args.continue_on_error,
            target: args.target,
        }
    }
}

#[derive(Debug, Args)]
struct InstallArgs {
    #[arg(short = 'p', long = "package")]
    package: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
    #[arg(long)]
    release: bool,
    #[arg(short = 's', long, value_enum, default_value_t = InstallScope::Default)]
    scope: InstallScope,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    continue_on_error: bool,
    #[arg(short = 't', long, value_enum, value_delimiter = ',', num_args = 1..)]
    target: Vec<PluginTarget>,
}

impl From<InstallArgs> for InstallOptions {
    fn from(args: InstallArgs) -> Self {
        Self {
            package: args.package,
            all: args.all,
            release: args.release,
            scope: args.scope.into(),
            dry_run: args.dry_run,
            continue_on_error: args.continue_on_error,
            target: args.target,
        }
    }
}

#[derive(Debug, Args)]
struct UninstallArgs {
    #[arg(short = 'p', long = "package")]
    package: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
    #[arg(short = 's', long, value_enum, default_value_t = UninstallScope::All)]
    scope: UninstallScope,
    #[arg(short = 't', long, value_enum, value_delimiter = ',', num_args = 1..)]
    target: Vec<PluginTarget>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    continue_on_error: bool,
}

impl From<UninstallArgs> for UninstallOptions {
    fn from(args: UninstallArgs) -> Self {
        Self {
            package: args.package,
            all: args.all,
            scope: args.scope.into(),
            target: args.target,
            dry_run: args.dry_run,
            continue_on_error: args.continue_on_error,
        }
    }
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
    #[arg(short = 't', long, value_enum, value_delimiter = ',', num_args = 1..)]
    target: Vec<ValidateTarget>,
}

impl From<ValidateArgs> for ValidateOptions {
    fn from(args: ValidateArgs) -> Self {
        Self {
            package: args.package,
            all: args.all,
            release: args.release,
            dry_run: args.dry_run,
            continue_on_error: args.continue_on_error,
            target: args.target,
        }
    }
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

impl From<LaunchArgs> for LaunchOptions {
    fn from(args: LaunchArgs) -> Self {
        Self {
            package: args.package,
            release: args.release,
            plugin_id: args.plugin_id,
        }
    }
}

#[derive(Debug, Args)]
struct CleanArgs {
    #[arg(short = 'p', long = "package")]
    package: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
}

impl From<CleanArgs> for CleanOptions {
    fn from(args: CleanArgs) -> Self {
        Self {
            package: args.package,
            all: args.all,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum InstallScope {
    Default,
    User,
    System,
}

impl From<InstallScope> for WracInstallScope {
    fn from(scope: InstallScope) -> Self {
        match scope {
            InstallScope::Default => Self::Default,
            InstallScope::User => Self::User,
            InstallScope::System => Self::System,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum UninstallScope {
    All,
    User,
    System,
}

impl From<UninstallScope> for WracUninstallScope {
    fn from(scope: UninstallScope) -> Self {
        match scope {
            UninstallScope::All => Self::All,
            UninstallScope::User => Self::User,
            UninstallScope::System => Self::System,
        }
    }
}
