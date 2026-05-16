use std::env;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::metadata::PluginMetadata;
use crate::profile::BuildProfile;
use crate::targets::Platform;

pub(crate) struct Context {
    pub(crate) root: PathBuf,
    pub(crate) platform: Platform,
    pub(crate) target_dir: PathBuf,
    pub(crate) wrapper_dir: PathBuf,
    pub(crate) metadata: PluginMetadata,
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
        // Plugin identity は src-plugin/Cargo.toml の [package.metadata.wrac] を SoT にする。
        // xtask が bundle 名や wrapper 引数を別に持つと、rename 時に build artifact だけ古い名前になる。
        let metadata = PluginMetadata::read(&root.join("src-plugin").join("Cargo.toml"))?;

        Ok(Self {
            root,
            platform: Platform::detect()?,
            target_dir,
            wrapper_dir,
            metadata,
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
        self.plugins_dir(profile)
            .join(self.metadata.clap_bundle_name())
    }

    pub(crate) fn vst3_bundle(&self, profile: BuildProfile) -> PathBuf {
        self.plugins_dir(profile)
            .join(self.metadata.vst3_bundle_name())
    }

    pub(crate) fn au_bundle(&self, profile: BuildProfile) -> PathBuf {
        self.plugins_dir(profile)
            .join(self.metadata.au_bundle_name())
    }

    pub(crate) fn standalone_artifact(&self, profile: BuildProfile) -> PathBuf {
        let filename = match self.platform {
            Platform::Macos => format!("{}.app", self.metadata.standalone_name),
            Platform::Windows => format!("{}.exe", self.metadata.standalone_name),
            Platform::Linux => self.metadata.standalone_name.clone(),
        };
        self.standalone_dir(profile).join(filename)
    }

    pub(crate) fn dynamic_library(&self, profile: BuildProfile) -> PathBuf {
        self.cargo_profile_dir(profile)
            .join(self.platform.dynamic_library_name())
    }
}
