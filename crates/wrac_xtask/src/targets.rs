use clap::ValueEnum;

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Target {
    Clap,
    Vst3,
    Au,
    Standalone,
}

impl Target {
    pub(crate) fn display(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Au => "AU",
            Self::Standalone => "Standalone",
        }
    }

    pub(crate) fn is_wrapper(self) -> bool {
        matches!(self, Self::Vst3 | Self::Au)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PluginTarget {
    Clap,
    Vst3,
    Au,
}

impl PluginTarget {
    pub(crate) fn display(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Au => "AU",
        }
    }

    pub(crate) fn target(self) -> Target {
        match self {
            Self::Clap => Target::Clap,
            Self::Vst3 => Target::Vst3,
            Self::Au => Target::Au,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ValidateTarget {
    Clap,
    Vst3,
    Au,
    Standalone,
}

impl ValidateTarget {
    pub(crate) fn display(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Au => "AU",
            Self::Standalone => "Standalone",
        }
    }

    pub(crate) fn target(self) -> Target {
        match self {
            Self::Clap => Target::Clap,
            Self::Vst3 => Target::Vst3,
            Self::Au => Target::Au,
            Self::Standalone => Target::Standalone,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Macos,
    Windows,
    Linux,
}

impl Platform {
    pub(crate) fn detect() -> Result<Self> {
        if cfg!(target_os = "macos") {
            Ok(Self::Macos)
        } else if cfg!(target_os = "windows") {
            Ok(Self::Windows)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Linux)
        } else {
            Err("unsupported operating system".into())
        }
    }

    pub(crate) fn supports_vst3(self) -> bool {
        matches!(self, Self::Macos | Self::Windows | Self::Linux)
    }

    pub(crate) fn supports_wrappers(self) -> bool {
        self.supports_vst3() || self.supports_au()
    }

    pub(crate) fn supports_au(self) -> bool {
        self == Self::Macos
    }

    pub(crate) fn supports_target(self, target: Target) -> bool {
        match target {
            Target::Clap => true,
            Target::Vst3 => self.supports_vst3(),
            Target::Au => self.supports_au(),
            Target::Standalone => matches!(self, Self::Macos | Self::Windows | Self::Linux),
        }
    }

    pub(crate) fn default_build_targets(self) -> Vec<Target> {
        // An unspecified build produces everything a developer would expect for that OS.
        match self {
            Self::Macos => vec![Target::Clap, Target::Vst3, Target::Au, Target::Standalone],
            Self::Windows => vec![Target::Clap, Target::Vst3, Target::Standalone],
            Self::Linux => vec![Target::Clap, Target::Vst3, Target::Standalone],
        }
    }

    pub(crate) fn default_plugin_targets(self) -> Vec<PluginTarget> {
        match self {
            Self::Macos => vec![PluginTarget::Clap, PluginTarget::Vst3, PluginTarget::Au],
            Self::Windows => vec![PluginTarget::Clap, PluginTarget::Vst3],
            Self::Linux => vec![PluginTarget::Clap, PluginTarget::Vst3],
        }
    }

    pub(crate) fn default_validate_targets(self) -> Vec<ValidateTarget> {
        // validate runs external validators against already-built plugin artifacts.
        match self {
            Self::Macos => vec![
                ValidateTarget::Clap,
                ValidateTarget::Vst3,
                ValidateTarget::Au,
                ValidateTarget::Standalone,
            ],
            Self::Windows => vec![
                ValidateTarget::Clap,
                ValidateTarget::Vst3,
                ValidateTarget::Standalone,
            ],
            Self::Linux => vec![
                ValidateTarget::Clap,
                ValidateTarget::Vst3,
                ValidateTarget::Standalone,
            ],
        }
    }

    pub(crate) fn cmake_generator(self) -> Option<&'static str> {
        match self {
            Self::Macos => Some("Xcode"),
            Self::Windows => Some("Visual Studio 17 2022"),
            Self::Linux => None,
        }
    }

    pub(crate) fn dynamic_library_name(self, crate_name: &str) -> String {
        match self {
            Self::Macos => format!("lib{crate_name}.dylib"),
            Self::Windows => format!("{crate_name}.dll"),
            Self::Linux => format!("lib{crate_name}.so"),
        }
    }

    pub(crate) fn static_library_name(self, crate_name: &str) -> String {
        match self {
            Self::Windows => format!("{crate_name}.lib"),
            Self::Macos | Self::Linux => format!("lib{crate_name}.a"),
        }
    }
}

pub(crate) fn resolve_build_targets(
    platform: Platform,
    requested: &[Target],
) -> Result<Vec<Target>> {
    let targets = if requested.is_empty() {
        platform.default_build_targets()
    } else {
        requested.to_vec()
    };

    for target in &targets {
        if !platform.supports_target(*target) {
            return Err(format!(
                "{} is not supported on this operating system",
                target.display()
            )
            .into());
        }
    }

    Ok(dedup(targets))
}

pub(crate) fn resolve_plugin_targets(
    platform: Platform,
    requested: &[PluginTarget],
) -> Result<Vec<PluginTarget>> {
    let targets = if requested.is_empty() {
        platform.default_plugin_targets()
    } else {
        requested.to_vec()
    };

    for target in &targets {
        if !platform.supports_target(target.target()) {
            return Err(format!(
                "{} is not supported on this operating system",
                target.display()
            )
            .into());
        }
    }

    Ok(dedup(targets))
}

pub(crate) fn resolve_validate_targets(
    platform: Platform,
    requested: &[ValidateTarget],
) -> Result<Vec<ValidateTarget>> {
    let targets = if requested.is_empty() {
        platform.default_validate_targets()
    } else {
        requested.to_vec()
    };

    for target in &targets {
        if !platform.supports_target(target.target()) {
            return Err(format!(
                "{} is not supported on this operating system",
                target.display()
            )
            .into());
        }
    }

    Ok(dedup(targets))
}

fn dedup<T: Copy + PartialEq>(targets: Vec<T>) -> Vec<T> {
    // Allow duplicate inputs such as `--target=vst3,vst3` from the CLI.
    // Deduplicate while preserving order rather than erroring, to be lenient toward script callers.
    let mut unique = Vec::new();
    for target in targets {
        if !unique.contains(&target) {
            unique.push(target);
        }
    }
    unique
}
