//! Repository-wide static quality checks.
//!
//! The default rules live here so each crate only documents deliberate exceptions
//! in its local `quality.toml`.

use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use cargo_metadata::MetadataCommand;
use serde::Deserialize;

use crate::Result;

const MAX_FILE_LINES: usize = 1000;

pub(crate) fn quality(root: &Path) -> Result<()> {
    // CI runs quality before plugin builds, so layout violations must be validated here
    // instead of depending on a later format-specific command to discover them.
    super::context::available_packages_at_root(root)?;
    let package_roots = workspace_package_roots(root)?;
    let mut errors = Vec::new();
    for package_root in package_roots {
        let config = QualityConfig::load(&package_root, root, &mut errors);
        check_package(root, &package_root, &config, &mut errors)?;
    }
    if errors.is_empty() {
        return Ok(());
    }

    let mut message = String::from("repository quality checks failed:\n");
    for error in errors {
        message.push_str("  - ");
        message.push_str(&error);
        message.push('\n');
    }
    Err(message.into())
}

fn workspace_package_roots(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let metadata = MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .exec()?;
    let workspace_members = metadata.workspace_members.iter().collect::<BTreeSet<_>>();
    let mut roots = BTreeSet::new();
    for package in metadata.packages {
        if !workspace_members.contains(&package.id) {
            continue;
        }
        let manifest_path = package.manifest_path.into_std_path_buf();
        let Some(package_root) = manifest_path.parent() else {
            continue;
        };
        roots.insert(package_root.to_path_buf());
    }
    Ok(roots)
}

fn check_package(
    root: &Path,
    package_root: &Path,
    config: &QualityConfig,
    errors: &mut Vec<String>,
) -> Result<()> {
    check_dir(root, package_root, package_root, config, errors)
}

fn check_dir(
    root: &Path,
    package_root: &Path,
    dir: &Path,
    config: &QualityConfig,
    errors: &mut Vec<String>,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if should_descend(&path) {
                check_dir(root, package_root, &path, config, errors)?;
            }
            continue;
        }
        if file_type.is_file() {
            check_file_length(root, package_root, &path, config, errors)?;
        }
    }
    Ok(())
}

fn should_descend(path: &Path) -> bool {
    !path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "target" | "node_modules" | ".build"))
}

fn check_file_length(
    root: &Path,
    package_root: &Path,
    path: &Path,
    config: &QualityConfig,
    errors: &mut Vec<String>,
) -> Result<()> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
        return Ok(());
    }

    let relative_to_package = relative_slash(package_root, path)?;
    if config.file_length.is_allowed(&relative_to_package) {
        return Ok(());
    }

    let file = fs::File::open(path)?;
    let line_count = BufReader::new(file).lines().count();
    if line_count > MAX_FILE_LINES {
        let relative_to_root = relative_slash(root, path)?;
        errors.push(format!(
            "{relative_to_root} has {line_count} lines; split files at or below {MAX_FILE_LINES} lines, or add a reasoned [file_length] allow entry"
        ));
    }
    Ok(())
}

fn relative_slash(base: &Path, path: &Path) -> Result<String> {
    Ok(path
        .strip_prefix(base)?
        .to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))?
        .replace('\\', "/"))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct QualityConfig {
    file_length: AllowRule,
}

impl QualityConfig {
    fn load(package_root: &Path, root: &Path, errors: &mut Vec<String>) -> Self {
        let path = package_root.join("quality.toml");
        if !path.exists() {
            return Self::default();
        }

        let display_path =
            relative_slash(root, &path).unwrap_or_else(|_| path.display().to_string());
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                errors.push(format!("failed to read {display_path}: {error}"));
                return Self::default();
            }
        };
        let config = match toml::from_str::<Self>(&content) {
            Ok(config) => config,
            Err(error) => {
                errors.push(format!("failed to parse {display_path}: {error}"));
                return Self::default();
            }
        };
        config.validate(&display_path, errors);
        config
    }

    fn validate(&self, config_path: &str, errors: &mut Vec<String>) {
        self.file_length
            .validate(config_path, "file_length", errors);
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AllowRule {
    allow: Vec<AllowEntry>,
}

impl AllowRule {
    fn is_allowed(&self, relative_path: &str) -> bool {
        self.allow
            .iter()
            .any(|entry| entry.path.as_deref() == Some(relative_path))
    }

    fn validate(&self, config_path: &str, rule_name: &str, errors: &mut Vec<String>) {
        for entry in &self.allow {
            entry.validate(config_path, rule_name, errors);
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowEntry {
    path: Option<String>,
    reason: String,
}

impl AllowEntry {
    fn validate(&self, config_path: &str, rule_name: &str, errors: &mut Vec<String>) {
        if self.path.is_none() {
            errors.push(format!(
                "{config_path} [{rule_name}] allow entry must include path"
            ));
        }
        if self.reason.trim().is_empty() {
            errors.push(format!(
                "{config_path} [{rule_name}] allow entry must include a non-empty reason"
            ));
        }
    }
}
