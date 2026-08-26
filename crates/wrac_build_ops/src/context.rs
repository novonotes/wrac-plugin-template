use std::{fs, path::PathBuf};

use cargo_metadata::MetadataCommand;

use crate::BuildProfile;
use crate::metadata::{PluginMetadata, PluginProductMetadata};
use crate::targets::Platform;
use crate::targets::PluginFormat;
use crate::{Result, WracPluginPackage, XtaskConfig, XtaskOutputLanguage};

pub struct WracContext {
    pub(crate) root: PathBuf,
    pub package_name: String,
    pub(crate) plugin_root: PathBuf,
    pub(crate) manifest_path: PathBuf,
    pub platform: Platform,
    pub(crate) target_dir: PathBuf,
    pub(crate) wrapper_dir: PathBuf,
    pub(crate) default_aax_sdk_root: Option<PathBuf>,
    pub output_language: XtaskOutputLanguage,
    pub(crate) metadata: PluginMetadata,
}

impl WracContext {
    pub fn new(config: &XtaskConfig, package_name: &str) -> Result<Self> {
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
        // Plugin identity is sourced from wrac-plugin.toml so product metadata
        // stays out of Cargo package metadata.
        let metadata = PluginMetadata::read_discovered(&package.manifest_path)?;

        Ok(Self {
            root: config.root.clone(),
            package_name: package.package_name,
            plugin_root: package.plugin_root,
            manifest_path: package.manifest_path,
            platform: Platform::detect()?,
            target_dir,
            wrapper_dir,
            default_aax_sdk_root: config.default_aax_sdk_root.clone(),
            output_language: config.output_language,
            metadata,
        })
    }

    pub fn gui_dir(&self) -> PathBuf {
        self.plugin_root.join("src-gui")
    }

    pub fn supports_plugin_format(&self, format: PluginFormat) -> bool {
        self.metadata.supports_format(format)
    }

    pub fn publicly_distributes_plugin_format(&self, format: PluginFormat) -> bool {
        self.metadata.publicly_distributes_format(format)
    }

    pub fn public_plugin_formats(&self) -> impl Iterator<Item = PluginFormat> + '_ {
        self.metadata.public_formats()
    }

    pub fn plugin_manifest(&self) -> PathBuf {
        self.manifest_path.clone()
    }

    pub fn cargo_profile_dir(&self, profile: BuildProfile) -> PathBuf {
        self.target_dir.join(profile.cargo_dir())
    }

    pub fn wrac_dir(&self) -> PathBuf {
        self.target_dir.join("wrac")
    }

    pub fn plugins_dir(&self, profile: BuildProfile) -> PathBuf {
        self.wrac_dir().join("plugins").join(profile.artifact_dir())
    }

    pub fn cmake_dir(&self, purpose: &str, profile: BuildProfile) -> PathBuf {
        // Keep the wrapper build directory short and stable.
        // The old hash-based path avoided Windows path length limits but changed between runs, which broke launch.json paths and made debugging harder.
        self.wrac_dir()
            .join("cmake")
            .join(format!("{purpose}-{}", profile.cmake_suffix()))
    }

    pub fn standalone_dir(&self, profile: BuildProfile) -> PathBuf {
        self.wrac_dir()
            .join("standalone")
            .join(profile.artifact_dir())
    }

    pub fn clap_bundle(&self, profile: BuildProfile) -> PathBuf {
        self.plugins_dir(profile)
            .join(self.metadata.clap_bundle_name())
    }

    pub fn vst3_bundle(&self, profile: BuildProfile) -> PathBuf {
        self.plugins_dir(profile)
            .join(self.metadata.vst3_bundle_name())
    }

    pub fn aax_bundle(&self, profile: BuildProfile) -> PathBuf {
        self.plugins_dir(profile)
            .join(self.metadata.aax_bundle_name())
    }

    pub fn au_bundles(&self, profile: BuildProfile) -> Vec<PathBuf> {
        vec![self.au_bundle(profile)]
    }

    pub fn au_bundle(&self, profile: BuildProfile) -> PathBuf {
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

pub(crate) type Context = WracContext;

pub(crate) fn available_packages(config: &XtaskConfig) -> Result<Vec<WracPluginPackage>> {
    available_packages_at_root(&config.root)
}

pub(crate) fn available_packages_at_root(root: &std::path::Path) -> Result<Vec<WracPluginPackage>> {
    let metadata = MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .exec()?;

    let mut packages = Vec::new();
    for package in metadata.workspace_packages() {
        let manifest_path = package.manifest_path.clone().into_std_path_buf();
        let Some((package_dir, plugin_root)) = plugin_layout_from_manifest(&manifest_path)? else {
            continue;
        };
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
        validate_plugin_layout(&package_dir, &plugin_root)?;
        packages.push(WracPluginPackage {
            package_name: package.name.clone(),
            artifact_namespace,
            manifest_path,
            plugin_root,
        });
    }
    packages.sort_by(|a, b| a.package_name.cmp(&b.package_name));
    Ok(packages)
}

fn plugin_layout_from_manifest(
    manifest_path: &std::path::Path,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let package_dir = manifest_path
        .parent()
        .ok_or_else(|| {
            format!(
                "failed to derive package dir from manifest path: {}",
                manifest_path.display()
            )
        })?
        .to_path_buf();
    // The directory name is the workspace-level plugin marker. Manifest presence
    // cannot be the marker because missing and misplaced manifests must fail loudly.
    if package_dir.file_name().and_then(|name| name.to_str()) != Some("src-plugin") {
        return Ok(None);
    }
    let plugin_root = package_dir
        .parent()
        .ok_or_else(|| {
            format!(
                "failed to derive plugin root from manifest path: {}",
                manifest_path.display()
            )
        })?
        .to_path_buf();
    Ok(Some((package_dir, plugin_root)))
}

fn find_package(config: &XtaskConfig, package_name: &str) -> Result<WracPluginPackage> {
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
    let expected_manifest = package_dir.join("wrac-plugin.toml");
    let mut detected_manifests = Vec::new();
    collect_plugin_manifests(plugin_root, &mut detected_manifests)?;
    detected_manifests.sort();
    let misplaced_manifests = detected_manifests
        .iter()
        .filter(|path| *path != &expected_manifest)
        .collect::<Vec<_>>();
    if !misplaced_manifests.is_empty() {
        let detected = misplaced_manifests
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "WRAC plugin manifest must exist only at {}; found manifest at {detected}",
            expected_manifest.display()
        )
        .into());
    }
    if !expected_manifest.is_file() {
        return Err(format!(
            "WRAC plugin manifest must exist at {}, but no manifest was found there",
            expected_manifest.display()
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

fn collect_plugin_manifests(
    directory: &std::path::Path,
    manifests: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            // Generated dependency and build trees are not plugin source layout and can
            // contain unrelated fixtures; avoiding them also keeps discovery bounded.
            let generated = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| matches!(name, ".git" | "target" | "node_modules" | ".build"));
            if !generated {
                collect_plugin_manifests(&path, manifests)?;
            }
        } else if file_type.is_file()
            && path.file_name().and_then(|name| name.to_str()) == Some("wrac-plugin.toml")
        {
            manifests.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{plugin_layout_from_manifest, validate_plugin_layout};

    #[test]
    fn accepts_conventional_plugin_layout() {
        let root = temp_dir("conventional");
        let plugin_root = root.join("plugins").join("wrac-gain");
        let package_dir = plugin_root.join("src-plugin");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(package_dir.join("wrac-plugin.toml"), "").unwrap();

        validate_plugin_layout(&package_dir, &plugin_root).unwrap();
    }

    #[test]
    fn rejects_missing_plugin_manifest() {
        let root = temp_dir("missing-plugin-manifest");
        let plugin_root = root.join("plugins").join("wrac-gain");
        let package_dir = plugin_root.join("src-plugin");
        fs::create_dir_all(&package_dir).unwrap();

        let error = validate_plugin_layout(&package_dir, &plugin_root)
            .unwrap_err()
            .to_string();

        assert!(error.contains(&package_dir.join("wrac-plugin.toml").display().to_string()));
        assert!(error.contains("no manifest was found"));
    }

    #[test]
    fn rejects_plugin_manifest_at_plugin_root() {
        let root = temp_dir("manifest-at-plugin-root");
        let plugin_root = root.join("plugins").join("wrac-gain");
        let package_dir = plugin_root.join("src-plugin");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(package_dir.join("wrac-plugin.toml"), "").unwrap();
        let detected_manifest = plugin_root.join("wrac-plugin.toml");
        fs::write(&detected_manifest, "").unwrap();

        let error = validate_plugin_layout(&package_dir, &plugin_root)
            .unwrap_err()
            .to_string();

        assert!(error.contains(&package_dir.join("wrac-plugin.toml").display().to_string()));
        assert!(error.contains(&detected_manifest.display().to_string()));
    }

    #[test]
    fn rejects_plugin_manifest_in_other_directory() {
        let root = temp_dir("manifest-in-other-directory");
        let plugin_root = root.join("plugins").join("wrac-gain");
        let package_dir = plugin_root.join("src-plugin");
        let other_dir = plugin_root.join("other-directory");
        fs::create_dir_all(&package_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();
        let detected_manifest = other_dir.join("wrac-plugin.toml");
        fs::write(&detected_manifest, "").unwrap();

        let error = validate_plugin_layout(&package_dir, &plugin_root)
            .unwrap_err()
            .to_string();

        assert!(error.contains(&package_dir.join("wrac-plugin.toml").display().to_string()));
        assert!(error.contains(&detected_manifest.display().to_string()));
    }

    #[test]
    fn ignores_ordinary_cargo_package() {
        let root = temp_dir("ordinary-cargo-package");
        let manifest_path = root.join("crates").join("ordinary").join("Cargo.toml");
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();

        assert!(
            plugin_layout_from_manifest(&manifest_path)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn rejects_frontend_package_at_plugin_root() {
        let root = temp_dir("frontend-at-plugin-root");
        let plugin_root = root.join("plugins").join("wrac-gain");
        let package_dir = plugin_root.join("src-plugin");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(package_dir.join("wrac-plugin.toml"), "").unwrap();
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
            "wrac_build_ops_layout_test_{}_{}_{}",
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
