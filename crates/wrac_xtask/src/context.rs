use std::path::PathBuf;

use cargo_metadata::MetadataCommand;

use crate::metadata::{PluginMetadata, PluginProductMetadata};
use crate::profile::BuildProfile;
use crate::targets::Platform;
use crate::{Result, XtaskConfig};

#[derive(Debug, Clone)]
pub(crate) struct PluginPackage {
    pub(crate) package_name: String,
    pub(crate) artifact_namespace: String,
    pub(crate) manifest_path: PathBuf,
    pub(crate) plugin_root: PathBuf,
}

pub(crate) struct Context {
    pub(crate) root: PathBuf,
    pub(crate) package_name: String,
    pub(crate) plugin_root: PathBuf,
    pub(crate) manifest_path: PathBuf,
    pub(crate) platform: Platform,
    pub(crate) target_dir: PathBuf,
    pub(crate) wrapper_dir: PathBuf,
    pub(crate) metadata: PluginMetadata,
}

impl Context {
    pub(crate) fn new(config: &XtaskConfig, package_name: &str) -> Result<Self> {
        let package = find_package(config, package_name)?;
        // CARGO_TARGET_DIR may be redirected to a shared cache in workspaces or CI.
        // Using the same target root as cargo keeps post-build library detection consistent.
        let target_root = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| config.root.join("target"));
        // Each plugin owns its own Cargo and CMake output tree. Wrapper builds create
        // format-specific projects with fixed target names, so sharing one target/wrac
        // directory across plugins would make artifacts overwrite or cross-contaminate.
        let target_dir = target_root
            .join(&config.target_namespace)
            .join(&package.artifact_namespace);
        // CLAP_WRAPPER_DIR lets wrapper developers point xtask at another clap_wrapper_builder checkout.
        let wrapper_dir = std::env::var_os("CLAP_WRAPPER_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| config.wrapper_dir.clone());
        // Plugin identity is sourced from wrac-plugin.toml, with legacy Cargo
        // metadata supported only as a migration fallback.
        let metadata =
            PluginMetadata::read_discovered(&package.manifest_path, &package.plugin_root)?;

        Ok(Self {
            root: config.root.clone(),
            package_name: package.package_name,
            plugin_root: package.plugin_root,
            manifest_path: package.manifest_path,
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
        self.manifest_path.clone()
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

    pub(crate) fn aax_bundle(&self, profile: BuildProfile) -> PathBuf {
        self.plugins_dir(profile)
            .join(self.metadata.aax_bundle_name())
    }

    pub(crate) fn au_bundles(&self, profile: BuildProfile) -> Vec<PathBuf> {
        vec![self.au_bundle(profile)]
    }

    pub(crate) fn au_bundle(&self, profile: BuildProfile) -> PathBuf {
        // AUv2 keeps multiple AudioComponents inside one component bundle.
        // The wrapper reads per-product type/subtype metadata from the CLAP
        // factory's AUv2 extension, so xtask tracks the artifact at bundle level.
        self.plugins_dir(profile)
            .join(self.metadata.au_bundle_name())
    }

    pub(crate) fn standalone_artifact_for(
        &self,
        profile: BuildProfile,
        plugin: &PluginProductMetadata,
    ) -> PathBuf {
        // Standalone app names are product metadata so multi-product templates
        // can expose distinct launchable artifacts without deriving names from
        // bundle-level metadata.
        let filename = match self.platform {
            Platform::Macos => format!("{}.app", plugin.standalone_name),
            Platform::Windows => format!("{}.exe", plugin.standalone_name),
            Platform::Linux => plugin.standalone_name.clone(),
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

pub(crate) fn available_packages(config: &XtaskConfig) -> Result<Vec<PluginPackage>> {
    let metadata = MetadataCommand::new()
        .manifest_path(config.root.join("Cargo.toml"))
        .exec()?;

    let mut packages = Vec::new();
    for package in metadata.workspace_packages() {
        let manifest_path = package.manifest_path.clone().into_std_path_buf();
        let package_dir = manifest_path
            .parent()
            .ok_or_else(|| {
                format!(
                    "failed to derive package dir from manifest path: {}",
                    manifest_path.display()
                )
            })?
            .to_path_buf();
        let plugin_root = package_dir
            .parent()
            .ok_or_else(|| {
                format!(
                    "failed to derive plugin root from manifest path: {}",
                    manifest_path.display()
                )
            })?
            .to_path_buf();
        let artifact_namespace = plugin_root
            .file_name()
            .ok_or_else(|| {
                format!(
                    "failed to derive artifact namespace from plugin root: {}",
                    plugin_root.display()
                )
            })?
            .to_string_lossy()
            .into_owned();
        let has_manifest = wrac_manifest::discover_manifest(&manifest_path, &plugin_root)
            .map(|source| match source {
                wrac_manifest::ManifestSource::Dedicated(path) => path.exists(),
                wrac_manifest::ManifestSource::LegacyCargoMetadata(_) => {
                    package.metadata.get("wrac").is_some()
                }
            })
            .unwrap_or(false);
        if !has_manifest {
            continue;
        }
        validate_plugin_layout(&package_dir, &plugin_root)?;
        packages.push(PluginPackage {
            package_name: package.name.clone(),
            artifact_namespace,
            manifest_path,
            plugin_root,
        });
    }
    packages.sort_by(|a, b| a.package_name.cmp(&b.package_name));
    Ok(packages)
}

fn find_package(config: &XtaskConfig, package_name: &str) -> Result<PluginPackage> {
    let packages = available_packages(config)?;
    for package in &packages {
        if package.package_name == package_name {
            return Ok(package.clone());
        }
    }
    let available = packages
        .iter()
        .map(|package| package.package_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if available.is_empty() {
        Err(format!("unknown WRAC plugin package: {package_name}").into())
    } else {
        Err(format!("unknown WRAC plugin package: {package_name}. Available: {available}").into())
    }
}

fn validate_plugin_layout(
    package_dir: &std::path::Path,
    plugin_root: &std::path::Path,
) -> Result<()> {
    if package_dir.file_name().and_then(|name| name.to_str()) != Some("src-plugin") {
        return Err(format!(
            "WRAC plugin package must live at <plugin-root>/src-plugin, but found {}",
            package_dir.display()
        )
        .into());
    }

    let root_package_json = plugin_root.join("package.json");
    if root_package_json.exists() {
        return Err(format!(
            "WRAC plugin frontend package must live at <plugin-root>/src-gui/package.json, but found {}",
            root_package_json.display()
        )
        .into());
    }

    let nested_package_json = package_dir.join("src-gui").join("package.json");
    if nested_package_json.exists() {
        return Err(format!(
            "WRAC plugin frontend package must live at <plugin-root>/src-gui/package.json, but found {}",
            nested_package_json.display()
        )
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::validate_plugin_layout;

    #[test]
    fn accepts_conventional_plugin_layout() {
        let root = temp_dir("conventional");
        let plugin_root = root.join("plugins").join("wrac-gain");
        let package_dir = plugin_root.join("src-plugin");
        fs::create_dir_all(&package_dir).unwrap();

        validate_plugin_layout(&package_dir, &plugin_root).unwrap();
    }

    #[test]
    fn rejects_plugin_crate_outside_src_plugin() {
        let root = temp_dir("plugin-crate-outside-src-plugin");
        let plugin_root = root.join("plugins").join("wrac-gain");
        let package_dir = plugin_root.clone();
        fs::create_dir_all(&package_dir).unwrap();

        let error = validate_plugin_layout(&package_dir, &plugin_root)
            .unwrap_err()
            .to_string();

        assert!(error.contains("<plugin-root>/src-plugin"));
    }

    #[test]
    fn rejects_frontend_package_at_plugin_root() {
        let root = temp_dir("frontend-at-plugin-root");
        let plugin_root = root.join("plugins").join("wrac-gain");
        let package_dir = plugin_root.join("src-plugin");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(plugin_root.join("package.json"), "{}").unwrap();

        let error = validate_plugin_layout(&package_dir, &plugin_root)
            .unwrap_err()
            .to_string();

        assert!(error.contains("<plugin-root>/src-gui/package.json"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wrac_xtask_layout_test_{}_{}_{}",
            std::process::id(),
            nanos,
            name
        ));
        reset_dir(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn reset_dir(path: &Path) {
        if path.exists() {
            fs::remove_dir_all(path).unwrap();
        }
    }
}
