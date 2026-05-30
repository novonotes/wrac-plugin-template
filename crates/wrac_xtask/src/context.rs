use std::fs;
use std::path::PathBuf;

use crate::metadata::PluginMetadata;
use crate::profile::BuildProfile;
use crate::targets::Platform;
use crate::{Result, XtaskConfig};

pub(crate) struct Context {
    pub(crate) root: PathBuf,
    pub(crate) plugin_slug: String,
    pub(crate) plugin_root: PathBuf,
    pub(crate) platform: Platform,
    pub(crate) target_dir: PathBuf,
    pub(crate) wrapper_dir: PathBuf,
    pub(crate) metadata: PluginMetadata,
}

impl Context {
    pub(crate) fn new(config: &XtaskConfig, plugin_slug: &str) -> Result<Self> {
        let plugin_root = config.plugins_dir.join(plugin_slug);
        if !plugin_root.join("src-plugin").join("Cargo.toml").exists() {
            return Err(format!("unknown plugin package: {plugin_slug}").into());
        }
        // CARGO_TARGET_DIR may be redirected to a shared cache in workspaces or CI.
        // Using the same target root as cargo keeps post-build library detection consistent.
        let target_root = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| config.root.join("target"));
        // Each plugin owns its own Cargo and CMake output tree. Wrapper builds create
        // format-specific projects with fixed target names, so sharing one target/wrac
        // directory across plugins would make artifacts overwrite or cross-contaminate.
        let target_dir = target_root.join(&config.target_namespace).join(plugin_slug);
        // CLAP_WRAPPER_DIR remains an escape hatch for testing SDK changes or a temporary external checkout.
        let wrapper_dir = std::env::var_os("CLAP_WRAPPER_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| config.wrapper_dir.clone());
        // Plugin identity is sourced from [package.metadata.wrac] in src-plugin/Cargo.toml.
        // Maintaining separate bundle names or wrapper arguments in xtask risks stale build artifacts on rename.
        let metadata = PluginMetadata::read(&plugin_root.join("src-plugin").join("Cargo.toml"))?;

        Ok(Self {
            root: config.root.clone(),
            plugin_slug: plugin_slug.to_string(),
            plugin_root,
            platform: Platform::detect()?,
            target_dir,
            wrapper_dir,
            metadata,
        })
    }

    pub(crate) fn gui_dir(&self) -> PathBuf {
        self.plugin_root.join("src-gui")
    }

    pub(crate) fn plugin_manifest(&self) -> PathBuf {
        self.plugin_root.join("src-plugin").join("Cargo.toml")
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
        // Keep the wrapper build directory short and stable.
        // The old hash-based path avoided Windows path length limits but changed between runs, which broke launch.json paths and made debugging harder.
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
        self.cargo_profile_dir(profile).join(
            self.platform
                .dynamic_library_name(&self.metadata.package_name),
        )
    }
}

pub(crate) fn available_plugins(config: &XtaskConfig) -> Result<Vec<String>> {
    let mut plugins = Vec::new();
    for entry in fs::read_dir(&config.plugins_dir)? {
        let entry = entry?;
        // A directory becomes a plugin package by containing src-plugin/Cargo.toml.
        // This keeps docs or future helper directories under plugins/ from becoming
        // implicit build targets.
        if entry.path().join("src-plugin").join("Cargo.toml").exists() {
            plugins.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    plugins.sort();
    Ok(plugins)
}
