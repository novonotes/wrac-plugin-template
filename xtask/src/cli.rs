use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::targets::{PluginTarget, Target, ValidateTarget};

const XTASK_AFTER_HELP: &str = "\
Run `cargo xtask <command> --help` for command-specific targets, platform support, and examples.";

const BUILD_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au, standalone

Default targets by platform:
  macOS:   clap, vst3, au, standalone
  Windows: clap, vst3, standalone
  Linux:   clap, vst3

Examples:
  cargo xtask build
  cargo xtask build --release
  cargo xtask build --target=vst3
  cargo xtask build --target=au,standalone --release

Notes:
  Run `cargo xtask install` after building to install plugin artifacts.
  Run `cargo xtask validate` after building to validate VST3/AU artifacts.
  VST3/AU wrapper targets require clap-wrapper dependencies.";

const INSTALL_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au

Default targets by platform:
  macOS:   clap, vst3, au
  Windows: clap, vst3
  Linux:   clap, vst3

Examples:
  cargo xtask install
  cargo xtask install --release
  cargo xtask install --scope=system
  cargo xtask install --target=clap,vst3

Notes:
  install copies previously built plugin artifacts.
  --scope defaults to user. Use --scope=system for hosts that only scan system-wide plugin folders.
  standalone is not a plugin format and cannot be installed with this command.";

const UNINSTALL_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au

Default targets by platform:
  macOS:   clap, vst3, au
  Windows: clap, vst3
  Linux:   clap, vst3

Examples:
  cargo xtask uninstall
  cargo xtask uninstall --target=vst3
  cargo xtask uninstall --scope=user
  cargo xtask uninstall --scope=system
  cargo xtask uninstall --dry-run

Notes:
  --scope defaults to all and removes both user-local and system-wide plugin artifacts.";

const VALIDATE_AFTER_HELP: &str = "\
Targets:
  vst3, au

Default targets by platform:
  macOS:   vst3, au
  Windows: vst3
  Linux:   vst3

Examples:
  cargo xtask validate
  cargo xtask validate --release
  cargo xtask validate --target=vst3

Notes:
  VST3 validation uses the VST3 validator.
  AU validation is available only on macOS and installs the built AU before running auval.
  AU validation fails if the same AU bundle exists under /Library/Audio/Plug-Ins/Components.";

#[derive(Debug, Parser)]
#[command(
    name = "xtask",
    about = "Build, install, validate, and clean WRAC plugin artifacts.",
    after_help = XTASK_AFTER_HELP
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    #[command(
        about = "Build plugin and standalone artifacts.",
        after_help = BUILD_AFTER_HELP
    )]
    Build(BuildArgs),
    #[command(
        about = "Install previously built plugin artifacts.",
        after_help = INSTALL_AFTER_HELP
    )]
    Install(InstallArgs),
    #[command(
        about = "Remove installed plugin artifacts from user-local and system-wide paths.",
        after_help = UNINSTALL_AFTER_HELP
    )]
    Uninstall(UninstallArgs),
    #[command(
        about = "Validate previously built VST3/AU artifacts.",
        after_help = VALIDATE_AFTER_HELP
    )]
    Validate(ValidateArgs),
    #[command(about = "Remove generated build artifacts managed by xtask.")]
    Clean,
}

#[derive(Debug, Args)]
pub(crate) struct BuildArgs {
    #[arg(long, help = "Build with the release profile.")]
    pub(crate) release: bool,

    #[arg(long, help = "Remove generated plugin artifacts before building.")]
    pub(crate) clean: bool,

    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Targets to build, comma-separated.",
        long_help = "Targets to build, comma-separated. Supported values are clap, vst3, au, and standalone. Defaults to every target supported by the current OS."
    )]
    pub(crate) target: Vec<Target>,
}

#[derive(Debug, Args)]
pub(crate) struct InstallArgs {
    #[arg(long, help = "Install release artifacts.")]
    pub(crate) release: bool,

    #[arg(
        long,
        value_enum,
        default_value_t = InstallScope::User,
        help = "Install location scope."
    )]
    pub(crate) scope: InstallScope,

    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Plugin formats to install, comma-separated.",
        long_help = "Plugin formats to install, comma-separated. Supported values are clap, vst3, and au. Defaults to every plugin format supported by the current OS. standalone is not supported here."
    )]
    pub(crate) target: Vec<PluginTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum InstallScope {
    User,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum UninstallScope {
    All,
    User,
    System,
}

#[derive(Debug, Args)]
pub(crate) struct UninstallArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = UninstallScope::All,
        help = "Uninstall location scope."
    )]
    pub(crate) scope: UninstallScope,

    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Plugin formats to uninstall, comma-separated.",
        long_help = "Plugin formats to uninstall, comma-separated. Supported values are clap, vst3, and au. Defaults to every plugin format supported by the current OS. standalone is not supported here."
    )]
    pub(crate) target: Vec<PluginTarget>,

    #[arg(
        long,
        help = "Print paths that would be removed without deleting them."
    )]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ValidateArgs {
    #[arg(long, help = "Validate release artifacts.")]
    pub(crate) release: bool,

    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Targets to validate, comma-separated.",
        long_help = "Targets to validate, comma-separated. Supported values are vst3 and au. Defaults to every validation target supported by the current OS."
    )]
    pub(crate) target: Vec<ValidateTarget>,
}
