use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::targets::{PluginTarget, Target, ValidateTarget};

const XTASK_AFTER_HELP: &str = "\
Run `cargo xtask <command> --help` for command-specific targets, platform support, and examples.";

const BUILD_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au, aax, standalone

Default targets:
  package.metadata.wrac.supported_formats supported on this platform, plus standalone

Examples:
  cargo xtask build
  cargo xtask build -p wrac_gain_plugin
  cargo xtask build --all --target=clap
  cargo xtask build --package=wrac_gain_plugin --release
  cargo xtask build -p wrac_gain_plugin --target=vst3
  cargo xtask build -p wrac_gain_plugin --target=au,standalone --release

Notes:
  -p/--package can be omitted when the workspace contains exactly one WRAC plugin package.
  xtask expands requested terminal tasks into a dependency graph before execution.
  Default target selection skips formats unsupported on the current platform and logs the reason.
  Explicit plugin-format targets must be listed in package.metadata.wrac.supported_formats.
  Explicit plugin-format targets fail when unsupported on the current platform.
  VST3/AU/AAX/standalone targets require clap-wrapper dependencies.";

const INSTALL_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au, aax

Default targets:
  package.metadata.wrac.supported_formats supported on this platform

Examples:
  cargo xtask install
  cargo xtask install -p wrac_gain_plugin
  cargo xtask install --all --release
  cargo xtask install -p wrac_gain_plugin --scope=system
  cargo xtask install -p wrac_gain_plugin --target=clap,vst3

Notes:
  -p/--package can be omitted when the workspace contains exactly one WRAC plugin package.
  install expands the selected plugin formats into a dependency graph before copying artifacts.
  Default target selection skips formats unsupported on the current platform and logs the reason.
  Explicit targets must be listed in package.metadata.wrac.supported_formats.
  Explicit targets fail when unsupported on the current platform.
  --scope=default installs AAX system-wide and CLAP/VST3/AU user-locally.
  standalone is not a plugin format and cannot be installed with this command.";

const UNINSTALL_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au, aax

Default targets:
  package.metadata.wrac.supported_formats supported on this platform

Examples:
  cargo xtask uninstall
  cargo xtask uninstall -p wrac_gain_plugin
  cargo xtask uninstall --all --target=vst3
  cargo xtask uninstall -p wrac_gain_plugin --scope=user
  cargo xtask uninstall -p wrac_gain_plugin --scope=system
  cargo xtask uninstall -p wrac_gain_plugin --dry-run

Notes:
  -p/--package can be omitted when the workspace contains exactly one WRAC plugin package.
  Default target selection skips formats unsupported on the current platform and logs the reason.
  Explicit targets must be listed in package.metadata.wrac.supported_formats.
  Explicit targets fail when unsupported on the current platform.
  --scope defaults to all and removes both user-local and system-wide plugin artifacts.
  AAX has no user-local install scope, so --scope=all removes only its system-wide Avid bundle.";

const VALIDATE_AFTER_HELP: &str = "\
Targets:
  clap, vst3, au, aax

Default targets:
  package.metadata.wrac.supported_formats supported on this platform

Examples:
  cargo xtask validate
  cargo xtask validate -p wrac_gain_plugin
  cargo xtask validate --all --release
  cargo xtask validate --all --target=clap
  cargo xtask validate -p wrac_gain_plugin --target=vst3

Notes:
  -p/--package can be omitted when the workspace contains exactly one WRAC plugin package.
  validate expands the selected plugin formats into a dependency graph, runs WRAC checks, then runs external validators.
  Default target selection skips formats unsupported on the current platform and logs the reason.
  Explicit targets must be listed in package.metadata.wrac.supported_formats.
  Explicit targets fail when unsupported on the current platform.
  WRAC check violations are errors. See docs/production-readiness-checks.md for rule IDs and disable metadata.
  CLAP validation downloads clap-validator 0.3.2 into target/tools if needed.
  VST3 validation uses the VST3 validator.
  AU validation is available only on macOS and installs the built AU before running auval.
  AAX validation requires AAX_SDK_ROOT and AAX_VALIDATOR_DSH_ARCHIVE from .env or the process environment.
  AAX validation runs selected AAX Validator tests by test ID through Avid's bundled DTT runner.
  AAX validation saves official JSON results under target/wrac-plugins/<package>/wrac/validation/aax/.
  AAX validation intentionally skips DSP/HDX cycle-count and page-table XML load tests.
  --continue-on-error continues independent tasks after failures, but the final exit status remains non-zero.
  AU validation fails if the same AU bundle exists under /Library/Audio/Plug-Ins/Components.";

const LAUNCH_AFTER_HELP: &str = "\
Examples:
  cargo xtask launch
  cargo xtask launch -p wrac_gain_plugin
  cargo xtask launch -p wrac_gain_plugin --plugin-id=com.your-company.wrac-gain
  cargo xtask launch --package=wrac_gain_plugin
  cargo xtask launch -p wrac_gain_plugin --release

Notes:
  launch builds only the standalone target and its dependencies before starting the app.
  Use --plugin-id when a package exposes multiple plugin products; invalid IDs fail before building.";

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

    #[arg(long, help = "Print the task graph plan without executing it.")]
    pub(crate) dry_run: bool,

    #[arg(
        long,
        help = "Continue independent tasks after a task fails; final exit status remains non-zero."
    )]
    pub(crate) continue_on_error: bool,

    #[arg(
        short = 't',
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Targets to build, comma-separated.",
        long_help = "Targets to build, comma-separated. Supported values are clap, vst3, au, aax, and standalone. Defaults to package.metadata.wrac.supported_formats supported on this platform plus standalone."
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
        short = 's',
        long,
        value_enum,
        default_value_t = InstallScope::Default,
        help = "Install location scope."
    )]
    pub(crate) scope: InstallScope,

    #[arg(long, help = "Print the task graph plan without executing it.")]
    pub(crate) dry_run: bool,

    #[arg(
        long,
        help = "Continue independent tasks after a task fails; final exit status remains non-zero."
    )]
    pub(crate) continue_on_error: bool,

    #[arg(
        short = 't',
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Plugin formats to install, comma-separated.",
        long_help = "Plugin formats to install, comma-separated. Supported values are clap, vst3, au, and aax. Defaults to package.metadata.wrac.supported_formats supported on this platform. standalone is not supported here."
    )]
    pub(crate) target: Vec<PluginTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum InstallScope {
    Default,
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
        short = 's',
        long,
        value_enum,
        default_value_t = UninstallScope::All,
        help = "Uninstall location scope."
    )]
    pub(crate) scope: UninstallScope,

    #[arg(
        short = 't',
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Plugin formats to uninstall, comma-separated.",
        long_help = "Plugin formats to uninstall, comma-separated. Supported values are clap, vst3, au, and aax. Defaults to package.metadata.wrac.supported_formats supported on this platform. standalone is not supported here."
    )]
    pub(crate) target: Vec<PluginTarget>,

    #[arg(
        long,
        help = "Print paths that would be removed without deleting them."
    )]
    pub(crate) dry_run: bool,

    #[arg(
        long,
        help = "Continue independent tasks after a task fails; final exit status remains non-zero."
    )]
    pub(crate) continue_on_error: bool,
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

    #[arg(long, help = "Print the task graph plan without executing it.")]
    pub(crate) dry_run: bool,

    #[arg(
        long,
        help = "Continue independent tasks after a task fails; final exit status remains non-zero."
    )]
    pub(crate) continue_on_error: bool,

    #[arg(
        short = 't',
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 1..,
        help = "Targets to validate, comma-separated.",
        long_help = "Targets to validate, comma-separated. Supported values are clap, vst3, au, and aax. Defaults to package.metadata.wrac.supported_formats supported on this platform."
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
