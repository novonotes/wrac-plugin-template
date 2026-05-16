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
}

impl ValidateTarget {
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
        // 無指定 build は「その OS で開発者が期待する全部」を作る。
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
        // validate は既に build 済みの plugin artifact を外部 validator で確認する command。
        match self {
            Self::Macos => vec![
                ValidateTarget::Clap,
                ValidateTarget::Vst3,
                ValidateTarget::Au,
            ],
            Self::Windows => vec![ValidateTarget::Clap, ValidateTarget::Vst3],
            Self::Linux => vec![ValidateTarget::Clap, ValidateTarget::Vst3],
        }
    }

    pub(crate) fn cmake_generator(self) -> Option<&'static str> {
        match self {
            Self::Macos => Some("Xcode"),
            Self::Windows => Some("Visual Studio 17 2022"),
            Self::Linux => None,
        }
    }

    pub(crate) fn dynamic_library_name(self) -> &'static str {
        match self {
            Self::Macos => concat!("lib", "wrac_gain_plugin", ".dylib"),
            Self::Windows => concat!("wrac_gain_plugin", ".dll"),
            Self::Linux => concat!("lib", "wrac_gain_plugin", ".so"),
        }
    }

    pub(crate) fn static_library_name(self) -> &'static str {
        match self {
            Self::Windows => concat!("wrac_gain_plugin", ".lib"),
            Self::Macos | Self::Linux => concat!("lib", "wrac_gain_plugin", ".a"),
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
    // CLI では `--target=vst3,vst3` のような重複入力を許す。
    // エラーにせず順序だけ保って重複排除し、script からの呼び出しを寛容にする。
    let mut unique = Vec::new();
    for target in targets {
        if !unique.contains(&target) {
            unique.push(target);
        }
    }
    unique
}
