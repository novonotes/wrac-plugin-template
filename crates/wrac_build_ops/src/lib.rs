//! Typed WRAC build operations for repository-local `xtask` crates.
//!
//! Repository-local `xtask` crates own command parsing, workflow construction,
//! and task execution policy. This crate only exposes WRAC-specific operations
//! that those workflows can call from their own task executors.

use std::{env, error::Error, path::PathBuf};

use xtask_workflow::TaskId;

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

/// Selects WRAC packages using the standard `-p/--package` and `--all` rules.
///
/// Package discovery is WRAC-specific, so this helper lives with the build
/// operations. The caller still decides which selected packages become workflow
/// roots and how product-specific tasks depend on them.
pub fn select_packages(
    config: &XtaskConfig,
    package: Option<&str>,
    all: bool,
) -> Result<Vec<String>> {
    if all {
        if package.is_some() {
            return Err(select_package_error(
                config.output_language,
                "--package and --all cannot be used together",
                "--package と --all は同時に指定できません",
            ));
        }
        let packages = discover_plugin_packages(config)?
            .into_iter()
            .map(|package| package.package_name)
            .collect::<Vec<_>>();
        if packages.is_empty() {
            return Err(select_package_error(
                config.output_language,
                "no WRAC plugin packages found in workspace members",
                "workspace member に WRAC plugin package が見つかりません",
            ));
        }
        return Ok(packages);
    }
    if let Some(package) = package {
        return Ok(vec![package.to_string()]);
    }
    Ok(vec![select_single_package(config, None)?])
}

/// Selects exactly one WRAC package for commands that cannot act on many roots.
pub fn select_single_package(config: &XtaskConfig, package: Option<&str>) -> Result<String> {
    if let Some(package) = package {
        return Ok(package.to_string());
    }
    let packages = discover_plugin_packages(config)?;
    match packages.as_slice() {
        [] => Err(select_package_error(
            config.output_language,
            "no WRAC plugin packages found in workspace members",
            "workspace member に WRAC plugin package が見つかりません",
        )),
        [package] => Ok(package.package_name.clone()),
        _ => {
            let packages = packages
                .iter()
                .map(|package| package.package_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            match config.output_language {
                XtaskOutputLanguage::English => Err(format!(
                    "multiple WRAC plugin packages found: {packages}. Use -p <PACKAGE> or --all."
                )
                .into()),
                XtaskOutputLanguage::Japanese => Err(format!(
                    "複数の WRAC plugin package が見つかりました: {packages}。-p <PACKAGE> または --all を指定してください。"
                )
                .into()),
            }
        }
    }
}

/// Creates a stable per-package task ID without prescribing task semantics.
///
/// Product repositories can add custom task kinds after standard WRAC build
/// operations while keeping plan output namespaced by package.
pub fn package_task_id(ctx: &WracContext, suffix: &str) -> TaskId {
    TaskId::new(format!("{}:{suffix}", ctx.package_name))
}

fn select_package_error(
    language: XtaskOutputLanguage,
    english: &'static str,
    japanese: &'static str,
) -> Box<dyn Error> {
    match language {
        XtaskOutputLanguage::English => english.into(),
        XtaskOutputLanguage::Japanese => japanese.into(),
    }
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
