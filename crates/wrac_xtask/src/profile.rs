#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    pub(crate) fn from_release(release: bool) -> Self {
        if release { Self::Release } else { Self::Debug }
    }

    pub(crate) fn cargo_flag(self) -> Option<&'static str> {
        match self {
            Self::Debug => None,
            Self::Release => Some("--release"),
        }
    }

    pub(crate) fn cargo_dir(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    pub(crate) fn artifact_dir(self) -> &'static str {
        self.cargo_dir()
    }

    pub(crate) fn cmake_config(self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Release => "Release",
        }
    }

    pub(crate) fn cmake_suffix(self) -> &'static str {
        self.cargo_dir()
    }
}
