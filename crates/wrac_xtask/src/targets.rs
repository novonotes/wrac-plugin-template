use clap::ValueEnum;
use serde::Deserialize;

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub(crate) enum Target {
    Clap,
    Vst3,
    Au,
    Aax,
    Standalone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PluginFormat {
    Clap,
    Vst3,
    Au,
    Aax,
}

impl PluginFormat {
    pub(crate) fn display(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Au => "AU",
            Self::Aax => "AAX",
        }
    }

    pub(crate) fn target(self) -> Target {
        match self {
            Self::Clap => Target::Clap,
            Self::Vst3 => Target::Vst3,
            Self::Au => Target::Au,
            Self::Aax => Target::Aax,
        }
    }
}

impl Target {
    pub(crate) fn display(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Au => "AU",
            Self::Aax => "AAX",
            Self::Standalone => "Standalone",
        }
    }

    pub(crate) fn plugin_format(self) -> Option<PluginFormat> {
        match self {
            Self::Clap => Some(PluginFormat::Clap),
            Self::Vst3 => Some(PluginFormat::Vst3),
            Self::Au => Some(PluginFormat::Au),
            Self::Aax => Some(PluginFormat::Aax),
            Self::Standalone => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub(crate) enum PluginTarget {
    Clap,
    Vst3,
    Au,
    Aax,
}

impl PluginTarget {
    pub(crate) fn display(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Au => "AU",
            Self::Aax => "AAX",
        }
    }

    pub(crate) fn target(self) -> Target {
        self.format().target()
    }

    pub(crate) fn format(self) -> PluginFormat {
        match self {
            Self::Clap => PluginFormat::Clap,
            Self::Vst3 => PluginFormat::Vst3,
            Self::Au => PluginFormat::Au,
            Self::Aax => PluginFormat::Aax,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub(crate) enum ValidateTarget {
    Clap,
    Vst3,
    Au,
    Aax,
}

impl ValidateTarget {
    pub(crate) fn display(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Au => "AU",
            Self::Aax => "AAX",
        }
    }

    pub(crate) fn target(self) -> Target {
        self.format().target()
    }

    pub(crate) fn format(self) -> PluginFormat {
        match self {
            Self::Clap => PluginFormat::Clap,
            Self::Vst3 => PluginFormat::Vst3,
            Self::Au => PluginFormat::Au,
            Self::Aax => PluginFormat::Aax,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        self.supports_vst3() || self.supports_au() || self.supports_aax()
    }

    pub(crate) fn supports_au(self) -> bool {
        self == Self::Macos
    }

    pub(crate) fn supports_aax(self) -> bool {
        matches!(self, Self::Macos | Self::Windows)
    }

    pub(crate) fn supports_target(self, target: Target) -> bool {
        match target {
            Target::Clap => true,
            Target::Vst3 => self.supports_vst3(),
            Target::Au => self.supports_au(),
            Target::Aax => self.supports_aax(),
            Target::Standalone => matches!(self, Self::Macos | Self::Windows | Self::Linux),
        }
    }

    pub(crate) fn display(self) -> &'static str {
        match self {
            Self::Macos => "macOS",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
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
