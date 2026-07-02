//! Typed WRAC build operations for repository-local `xtask` crates.
//!
//! Repository-local `xtask` crates own command parsing, workflow construction,
//! and task execution policy. This crate only exposes WRAC-specific operations
//! that those workflows can call from their own task executors.

use std::{env, error::Error, path::PathBuf};

mod commands;
mod context;
mod metadata;
pub mod profile;
mod quality;
pub mod target_resolution;
pub mod targets;
mod util;
mod validation;

pub use commands::{
    RustPluginBuild, WrapperBuild, WrapperTarget, build_gui, build_rust_plugin,
    build_wrapper_target, check_install_dir, clean, configure_wrapper, install_plugin_target,
    launch, package_clap, print_build_outputs, uninstall_plugin_target, validate_plugin_target,
    validate_wrac_rules_for_targets,
};
pub use context::WracContext;
pub use profile::BuildProfile;
pub use target_resolution::{
    resolve_build_targets_from_metadata, resolve_plugin_targets_from_metadata,
    resolve_validate_targets_from_metadata,
};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Clone)]
pub struct XtaskConfig {
    pub root: PathBuf,
    pub wrapper_dir: PathBuf,
    pub target_namespace: String,
    pub default_aax_sdk_root: Option<PathBuf>,
    pub output_language: XtaskOutputLanguage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum XtaskOutputLanguage {
    #[default]
    English,
    Japanese,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WracPluginPackage {
    pub package_name: String,
    pub artifact_namespace: String,
    pub manifest_path: PathBuf,
    pub plugin_root: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InstallScope {
    #[default]
    Default,
    User,
    System,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UninstallScope {
    #[default]
    All,
    User,
    System,
}

/// Discovers WRAC plugin packages from workspace metadata.
///
/// Repository-local planners decide how to interpret these packages. This
/// function only applies WRAC manifest discovery and layout validation.
pub fn discover_plugin_packages(config: &XtaskConfig) -> Result<Vec<WracPluginPackage>> {
    context::available_packages(config)
}

/// Loads repository-local `.env` values without overriding existing variables.
pub fn load_workspace_dotenv(config: &XtaskConfig) -> Result<()> {
    let path = config.root.join(".env");
    if !path.exists() {
        return Ok(());
    }

    for entry in dotenvy::from_path_iter(&path)? {
        let (key, value) = entry?;
        if env::var_os(&key).is_none() {
            // xtask binaries call this before starting worker threads or child
            // processes, so the environment mutation is confined to startup.
            unsafe {
                env::set_var(key, value);
            }
        }
    }
    Ok(())
}

pub fn run_quality(root: &std::path::Path) -> Result<()> {
    quality::quality(root)
}
