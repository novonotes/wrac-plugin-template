use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::targets::{PluginTarget, Target, ValidateTarget};

const XTASK_AFTER_HELP: &str = "\
Run `cargo xtask <command> --help` for command-specific targets, platform support, and examples.";

const BUILD_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au, aax, standalone

Default targets by platform:
  macOS:   clap, vst3, au, standalone
  Windows: clap, vst3, standalone
  Linux:   clap, vst3, standalone

Examples:
  cargo xtask build
  cargo xtask build -p wrac_gain_plugin
  cargo xtask build --all --target=clap
  cargo xtask build --package=wrac_gain_plugin --release
  cargo xtask build -p wrac_gain_plugin --target=vst3
  cargo xtask build -p wrac_gain_plugin --target=au,standalone --release

Notes:
  -p/--package can be omitted when the workspace contains exactly one WRAC plugin package.
  `install`, `validate`, and `launch` build their required artifacts before use.
  VST3/AU/AAX/standalone targets require clap-wrapper dependencies.";

const INSTALL_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au, aax

Default targets by platform:
  macOS:   clap, vst3, au
  Windows: clap, vst3
  Linux:   clap, vst3

Examples:
  cargo xtask install
  cargo xtask install -p wrac_gain_plugin
  cargo xtask install --all --release
  cargo xtask install -p wrac_gain_plugin --scope=system
  cargo xtask install -p wrac_gain_plugin --target=clap,vst3

Notes:
  -p/--package can be omitted when the workspace contains exactly one WRAC plugin package.
  install builds the selected plugin formats before copying artifacts.
  --scope defaults to user. Use --scope=system for hosts that only scan system-wide plugin folders.
  standalone is not a plugin format and cannot be installed with this command.";

const UNINSTALL_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au, aax

Default targets by platform:
  macOS:   clap, vst3, au
  Windows: clap, vst3
  Linux:   clap, vst3

Examples:
  cargo xtask uninstall
  cargo xtask uninstall -p wrac_gain_plugin
  cargo xtask uninstall --all --target=vst3
  cargo xtask uninstall -p wrac_gain_plugin --scope=user
  cargo xtask uninstall -p wrac_gain_plugin --scope=system
  cargo xtask uninstall -p wrac_gain_plugin --dry-run

Notes:
  -p/--package can be omitted when the workspace contains exactly one WRAC plugin package.
  --scope defaults to all and removes both user-local and system-wide plugin artifacts.";

const VALIDATE_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au, aax

Default targets by platform:
  macOS:   clap, vst3, au
  Windows: clap, vst3
  Linux:   clap, vst3

Examples:
  cargo xtask validate
  cargo xtask validate -p wrac_gain_plugin
  cargo xtask validate --all --release
  cargo xtask validate --all --target=clap
  cargo xtask validate -p wrac_gain_plugin --target=vst3

Notes:
  -p/--package can be omitted when the workspace contains exactly one WRAC plugin package.
  validate builds the selected plugin formats, runs WRAC production-readiness checks, then runs external validators.
  WRAC check violations are errors. See docs/production-readiness-checks.md for rule IDs and disable metadata.
  CLAP validation downloads clap-validator 0.3.2 into target/tools if needed.
  VST3 validation uses the VST3 validator.
  AU validation is available only on macOS and installs the built AU before running auval.
  AAX validation requires the AAX SDK plus AAX validator/DSH and is run only when explicitly targeted.
  AU validation fails if the same AU bundle exists under /Library/Audio/Plug-Ins/Components.";

const LAUNCH_AFTER_HELP: &str = "\
Examples:
  cargo xtask launch
  cargo xtask launch -p wrac_gain_plugin
  cargo xtask launch -p wrac_gain_plugin --plugin-id=com.your-company.wrac-gain
  cargo xtask launch --package=wrac_gain_plugin
  cargo xtask launch -p wrac_gain_plugin --release

Notes:
  launch builds standalone artifacts before starting one. Use --plugin-id when a package exposes multiple plugin products.";

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
        about = "Build and validate plugin artifacts.",
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
        short = 'p',
        long = "package",
        help = "WRAC plugin package name, such as wrac_gain_plugin."
    )]
    pub(crate) package: Option<String>,

    #[arg(short = 'a', long, help = "Build every WRAC plugin package.")]
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
        long_help = "Targets to build, comma-separated. Supported values are clap, vst3, au, aax, and standalone. Defaults to every target supported by the current OS except AAX, which must be requested explicitly."
    )]
    pub(crate) target: Vec<Target>,
}

#[derive(Debug, Args)]
pub(crate) struct InstallArgs {
    #[arg(
        short = 'p',
        long = "package",
        help = "WRAC plugin package name, such as wrac_gain_plugin."
    )]
    pub(crate) package: Option<String>,

    #[arg(short = 'a', long, help = "Install every WRAC plugin package.")]
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
        long_help = "Plugin formats to install, comma-separated. Supported values are clap, vst3, au, and aax. Defaults to every plugin format supported by the current OS except AAX, which must be requested explicitly. standalone is not supported here."
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
        short = 'p',
        long = "package",
        help = "WRAC plugin package name, such as wrac_gain_plugin."
    )]
    pub(crate) package: Option<String>,

    #[arg(short = 'a', long, help = "Uninstall every WRAC plugin package.")]
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
        long_help = "Plugin formats to uninstall, comma-separated. Supported values are clap, vst3, au, and aax. Defaults to every plugin format supported by the current OS except AAX, which must be requested explicitly. standalone is not supported here."
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
        short = 'p',
        long = "package",
        help = "WRAC plugin package name, such as wrac_gain_plugin."
    )]
    pub(crate) package: Option<String>,

    #[arg(short = 'a', long, help = "Validate every WRAC plugin package.")]
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
        long_help = "Targets to validate, comma-separated. Supported values are clap, vst3, au, and aax. Defaults to every validation target supported by the current OS except AAX, which must be requested explicitly."
    )]
    pub(crate) target: Vec<ValidateTarget>,
}

#[derive(Debug, Args)]
pub(crate) struct LaunchArgs {
    #[arg(
        short = 'p',
        long = "package",
        help = "WRAC plugin package name, such as wrac_gain_plugin."
    )]
    pub(crate) package: Option<String>,

    #[arg(long, help = "Launch release artifact.")]
    pub(crate) release: bool,

    #[arg(
        long,
        help = "Plugin ID to launch when the package has multiple products."
    )]
    pub(crate) plugin_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct CleanArgs {
    #[arg(
        short = 'p',
        long = "package",
        help = "WRAC plugin package name, such as wrac_gain_plugin."
    )]
    pub(crate) package: Option<String>,

    #[arg(short = 'a', long, help = "Clean every WRAC plugin package.")]
    pub(crate) all: bool,
}
