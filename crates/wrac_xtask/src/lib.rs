//! Low-level WRAC xtask building blocks.
//!
//! Repository-local `xtask` crates own command parsing, package selection, and
//! build/validation planning. This crate provides typed task primitives,
//! metadata discovery, and shared execution helpers for those planners.

use std::{env, error::Error, path::PathBuf};

mod commands;
mod context;
mod metadata;
pub mod plan;
pub mod profile;
mod quality;
pub mod targets;
mod util;
mod validation;

pub use context::WracContext;
pub use plan::{FailurePolicy, TaskId, TaskKind, TaskNode, TaskPlan};
pub use profile::BuildProfile;

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
