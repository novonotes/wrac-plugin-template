use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::context::Context;
use crate::util::remove_if_exists;

use super::build::WrapperBuild;

pub(super) fn prepare_plugin_resource_dir(ctx: &Context) -> Result<Option<PathBuf>> {
    let source_dirs = plugin_resource_dirs(ctx);
    let stage_dir = staged_plugin_resource_dir(ctx);
    remove_if_exists(&stage_dir)?;
    if source_dirs.is_empty() {
        return Ok(None);
    }

    fs::create_dir_all(&stage_dir)?;
    for source_dir in source_dirs {
        copy_resource_directory(&source_dir, &stage_dir)?;
    }
    Ok(Some(stage_dir))
}

fn plugin_resource_dirs(ctx: &Context) -> Vec<PathBuf> {
    [
        // Source resources are hand-authored plugin assets. They are copied first so generated
        // resources can intentionally replace them when both trees contain the same relative path.
        ctx.plugin_root.join("resources"),
        // Generated resources belong under the build output so large artifacts can be rebuilt and
        // omitted from source control while still flowing through the same packaging contract.
        ctx.wrac_dir().join("resources"),
    ]
    .into_iter()
    .filter(|resource_dir| resource_dir.exists())
    .collect()
}

fn staged_plugin_resource_dir(ctx: &Context) -> PathBuf {
    ctx.wrac_dir().join("staged-resources")
}

pub(super) fn copy_resource_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    copy_resource_directory_inner(source, destination)
}

fn copy_resource_directory_inner(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            copy_symlink(&source_path, &destination_path)?;
        } else if file_type.is_dir() {
            if destination_path.is_symlink()
                || (destination_path.exists() && !destination_path.is_dir())
            {
                remove_resource_destination(&destination_path)?;
            }
            fs::create_dir_all(&destination_path)?;
            copy_resource_directory_inner(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            if destination_path.exists() || destination_path.is_symlink() {
                remove_resource_destination(&destination_path)?;
            }
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = fs::read_link(source)?;
    if destination.exists() || destination.is_symlink() {
        remove_resource_destination(destination)?;
    }
    std::os::unix::fs::symlink(target, destination)?;
    Ok(())
}

#[cfg(windows)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = fs::read_link(source)?;
    if destination.exists() || destination.is_symlink() {
        remove_resource_destination(destination)?;
    }
    // Preserve symlinks produced by external packagers instead of flattening them, because their
    // relative loader paths can be part of the runtime's file-system contract.
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination)?;
    } else {
        std::os::windows::fs::symlink_file(target, destination)?;
    }
    Ok(())
}

fn remove_resource_destination(path: &Path) -> Result<()> {
    // Resource staging intentionally allows generated resources to replace source resources with
    // the same relative path. Use symlink_metadata so replacement never follows a symlink into the
    // source tree or a tool-generated runtime directory.
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

pub(super) fn ensure_wrapper_resources_supported(
    build: WrapperBuild,
    resource_dir: Option<&Path>,
) -> Result<()> {
    if resource_dir.is_none() {
        return Ok(());
    }

    // Wrapper resource packaging needs format-specific artifact/install semantics and rebuild
    // dependencies. Failing here avoids producing bundles that pass build steps while silently
    // omitting or staling required runtime files.
    match build {
        WrapperBuild::Plugins { .. } => Err(
            "plugin resources are not supported for wrapper plugin builds yet; build macOS CLAP or remove plugin resources".into(),
        ),
        WrapperBuild::Aax => Err(
            "plugin resources are not supported for AAX wrapper builds yet; build macOS CLAP or remove plugin resources".into(),
        ),
        WrapperBuild::Standalone => Err(
            "plugin resources are not supported for standalone app builds yet; build macOS CLAP or remove plugin resources".into(),
        ),
    }
}
