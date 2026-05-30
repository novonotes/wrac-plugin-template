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
  Linux:   clap, vst3, standalone

Examples:
  cargo xtask build
  cargo xtask build --plugin=sine-synth
  cargo xtask build --all --target=clap
  cargo xtask build --plugin=gain-basic --release
  cargo xtask build --plugin=gain-basic --target=vst3
  cargo xtask build --plugin=gain-basic --target=au,standalone --release

Notes:
  --plugin can be omitted when plugins/ contains exactly one plugin package.
  `install`, `validate`, and `launch` build their required artifacts before use.
  VST3/AU/standalone targets require clap-wrapper dependencies.";

const INSTALL_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au

Default targets by platform:
  macOS:   clap, vst3, au
  Windows: clap, vst3
  Linux:   clap, vst3

Examples:
  cargo xtask install
  cargo xtask install --plugin=gain-basic
  cargo xtask install --all --release
  cargo xtask install --plugin=sine-synth --scope=system
  cargo xtask install --plugin=gain-basic --target=clap,vst3

Notes:
  --plugin can be omitted when plugins/ contains exactly one plugin package.
  install builds the selected plugin formats before copying artifacts.
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
  cargo xtask uninstall --plugin=gain-basic
  cargo xtask uninstall --all --target=vst3
  cargo xtask uninstall --plugin=sine-synth --scope=user
  cargo xtask uninstall --plugin=sine-synth --scope=system
  cargo xtask uninstall --plugin=gain-basic --dry-run

Notes:
  --plugin can be omitted when plugins/ contains exactly one plugin package.
  --scope defaults to all and removes both user-local and system-wide plugin artifacts.";

const VALIDATE_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au

Default targets by platform:
  macOS:   clap, vst3, au
  Windows: clap, vst3
  Linux:   clap, vst3

Examples:
  cargo xtask validate
  cargo xtask validate --plugin=gain-basic
  cargo xtask validate --all --release
  cargo xtask validate --all --target=clap
  cargo xtask validate --plugin=sine-synth --target=vst3

Notes:
  --plugin can be omitted when plugins/ contains exactly one plugin package.
  validate builds the selected plugin formats before running validators.
  CLAP validation downloads clap-validator 0.3.2 into target/tools if needed.
  VST3 validation uses the VST3 validator.
  AU validation is available only on macOS and installs the built AU before running auval.
  AU validation fails if the same AU bundle exists under /Library/Audio/Plug-Ins/Components.";

const LAUNCH_AFTER_HELP: &str = "\
Examples:
  cargo xtask launch
  cargo xtask launch --plugin=gain-basic
  cargo xtask launch --plugin=sine-synth
  cargo xtask launch --plugin=gain-basic --release

Notes:
  launch builds the standalone artifact before starting it.";

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
        about = "Build and install plugin artifacts.",
        after_help = INSTALL_AFTER_HELP
    )]
    Install(InstallArgs),
    #[command(
        about = "Remove installed plugin artifacts from user-local and system-wide paths.",
        after_help = UNINSTALL_AFTER_HELP
    )]
    Uninstall(UninstallArgs),
    #[command(
        about = "Build and validate CLAP/VST3/AU artifacts.",
        after_help = VALIDATE_AFTER_HELP
    )]
    Validate(ValidateArgs),
    #[command(
        about = "Build and launch the standalone artifact.",
        after_help = LAUNCH_AFTER_HELP
    )]
    Launch(LaunchArgs),
    #[command(about = "Remove generated build artifacts managed by xtask.")]
    Clean(CleanArgs),
}

#[derive(Debug, Args)]
pub(crate) struct BuildArgs {
    #[arg(
        short_alias = 'p',
        long,
        help = "Plugin directory name under plugins/, such as sine-synth."
    )]
    pub(crate) plugin: Option<String>,

    #[arg(short_alias = 'a', long, help = "Build every example plugin.")]
    pub(crate) all: bool,

    #[arg(long, help = "Build with the release profile.")]
    pub(crate) release: bool,

    #[arg(long, help = "Remove generated plugin artifacts before building.")]
    pub(crate) clean: bool,

    #[arg(
        short_alias = 't',
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
    #[arg(
        short_alias = 'p',
        long,
        help = "Plugin directory name under plugins/, such as sine-synth."
    )]
    pub(crate) plugin: Option<String>,

    #[arg(short_alias = 'a', long, help = "Install every example plugin.")]
    pub(crate) all: bool,

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
        short_alias = 't',
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
        short_alias = 'p',
        long,
        help = "Plugin directory name under plugins/, such as sine-synth."
    )]
    pub(crate) plugin: Option<String>,

    #[arg(short_alias = 'a', long, help = "Uninstall every example plugin.")]
    pub(crate) all: bool,

    #[arg(
        long,
        value_enum,
        default_value_t = UninstallScope::All,
        help = "Uninstall location scope."
    )]
    pub(crate) scope: UninstallScope,

    #[arg(
        short_alias = 't',
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
    #[arg(
        short_alias = 'p',
        long,
        help = "Plugin directory name under plugins/, such as sine-synth."
    )]
    pub(crate) plugin: Option<String>,

    #[arg(short_alias = 'a', long, help = "Validate every example plugin.")]
    pub(crate) all: bool,

    #[arg(long, help = "Validate release artifacts.")]
    pub(crate) release: bool,

    #[arg(
        short_alias = 't',
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Targets to validate, comma-separated.",
        long_help = "Targets to validate, comma-separated. Supported values are clap, vst3, and au. Defaults to every validation target supported by the current OS."
    )]
    pub(crate) target: Vec<ValidateTarget>,
}

#[derive(Debug, Args)]
pub(crate) struct LaunchArgs {
    #[arg(
        short_alias = 'p',
        long,
        help = "Plugin directory name under plugins/, such as sine-synth."
    )]
    pub(crate) plugin: Option<String>,

    #[arg(long, help = "Launch release artifact.")]
    pub(crate) release: bool,
}

#[derive(Debug, Args)]
pub(crate) struct CleanArgs {
    #[arg(
        short_alias = 'p',
        long,
        help = "Plugin directory name under plugins/, such as sine-synth."
    )]
    pub(crate) plugin: Option<String>,

    #[arg(short_alias = 'a', long, help = "Clean every example plugin.")]
    pub(crate) all: bool,
}
