use std::env;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::constants::{AU_BUNDLE_NAME, CLAP_BUNDLE_NAME, STANDALONE_NAME, VST3_BUNDLE_NAME};
use crate::profile::BuildProfile;
use crate::targets::Platform;

pub(crate) struct Context {
    pub(crate) root: PathBuf,
    pub(crate) platform: Platform,
    pub(crate) target_dir: PathBuf,
    pub(crate) wrapper_dir: PathBuf,
}

impl Context {
    pub(crate) fn new() -> Result<Self> {
        // cargo xtask は xtask crate の manifest から起動されるため、親 directory を repo root とする。
        // current_dir に依存すると、別 directory から実行した時に artifact path がずれる。
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(1)
            .ok_or("failed to locate repository root")?
            .to_path_buf();
        // CARGO_TARGET_DIR は workspace や CI で共有 cache に向けられることがある。
        // xtask が cargo と同じ target root を見ることで、build 後の library 検出を一致させる。
        let target_dir = env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("target"));
        // wrapper の fork/patch を最小限に保つため、通常は repo 内 submodule を使う。
        // CLAP_WRAPPER_DIR は SDK 検証や一時的な外部 checkout を試すための escape hatch。
        let wrapper_dir = env::var_os("CLAP_WRAPPER_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("clap_wrapper_builder"));

        Ok(Self {
            root,
            platform: Platform::detect()?,
            target_dir,
            wrapper_dir,
        })
    }

    pub(crate) fn gui_dir(&self) -> PathBuf {
        self.root.join("src-gui")
    }

    pub(crate) fn plugin_manifest(&self) -> PathBuf {
        self.root.join("src-plugin").join("Cargo.toml")
    }

    pub(crate) fn cargo_profile_dir(&self, profile: BuildProfile) -> PathBuf {
        self.target_dir.join(profile.cargo_dir())
    }

    pub(crate) fn wrac_dir(&self) -> PathBuf {
        self.target_dir.join("wrac")
    }

    pub(crate) fn plugins_dir(&self, profile: BuildProfile) -> PathBuf {
        self.wrac_dir().join("plugins").join(profile.artifact_dir())
    }

    pub(crate) fn cmake_dir(&self, purpose: &str, profile: BuildProfile) -> PathBuf {
        // wrapper build directory は短く固定する。
        // 旧 script の hash path は Windows path limit 回避には有効だが、launch.json や調査時の再現性が落ちる。
        self.wrac_dir()
            .join("cmake")
            .join(format!("{purpose}-{}", profile.cmake_suffix()))
    }

    pub(crate) fn standalone_dir(&self, profile: BuildProfile) -> PathBuf {
        self.wrac_dir()
            .join("standalone")
            .join(profile.artifact_dir())
    }

    pub(crate) fn clap_bundle(&self, profile: BuildProfile) -> PathBuf {
        self.plugins_dir(profile).join(CLAP_BUNDLE_NAME)
    }

    pub(crate) fn vst3_bundle(&self, profile: BuildProfile) -> PathBuf {
        self.plugins_dir(profile).join(VST3_BUNDLE_NAME)
    }

    pub(crate) fn au_bundle(&self, profile: BuildProfile) -> PathBuf {
        self.plugins_dir(profile).join(AU_BUNDLE_NAME)
    }

    pub(crate) fn standalone_artifact(&self, profile: BuildProfile) -> PathBuf {
        let filename = if self.platform == Platform::Macos {
            format!("{STANDALONE_NAME}.app")
        } else {
            format!("{STANDALONE_NAME}.exe")
        };
        self.standalone_dir(profile).join(filename)
    }

    pub(crate) fn dynamic_library(&self, profile: BuildProfile) -> PathBuf {
        self.cargo_profile_dir(profile)
            .join(self.platform.dynamic_library_name())
    }
}
