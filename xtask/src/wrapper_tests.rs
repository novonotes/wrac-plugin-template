//! Run the template's native wrapper regression hosts through CTest.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use crate::Result;

pub(crate) fn run(root: &Path) -> Result<()> {
    let extension = if cfg!(target_os = "windows") {
        "vst3"
    } else if cfg!(target_os = "macos") {
        "component"
    } else {
        return Err("wrapper regression tests are supported on Windows and macOS".into());
    };

    // Match the plugin builder's CARGO_TARGET_DIR handling so the test uses
    // the same release artifact in local builds and shared CI caches.
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let target = if target.is_relative() {
        env::current_dir()?.join(target)
    } else {
        target
    };
    let plugin = target
        .join("wrac-plugins/wrac-gain/wrac/plugins/release")
        .join(format!("WRAC Gain.{extension}"));
    if !plugin.is_dir() {
        return Err(format!(
            "WRAC Gain bundle not found: {}. Run `cargo xtask build --release --target={}` first",
            plugin.display(),
            if extension == "component" {
                "au"
            } else {
                "vst3"
            }
        )
        .into());
    }

    let build = target.join("wrapper-regression-tests");
    let mut plugin_arg = OsString::from("-DWRAC_TEST_PLUGIN=");
    plugin_arg.push(&plugin);
    run_command(
        Command::new("cmake")
            .arg("-S")
            .arg(root.join("clap_wrapper_builder/tests"))
            .arg("-B")
            .arg(&build)
            .arg("-DCMAKE_BUILD_TYPE=Release")
            .arg(plugin_arg),
    )?;
    run_command(
        Command::new("cmake")
            .arg("--build")
            .arg(&build)
            .args(["--config", "Release"]),
    )?;
    run_command(Command::new("ctest").arg("--test-dir").arg(&build).args([
        "-C",
        "Release",
        // The AU tests can skip individual checks while the rest passes;
        // keep their SKIP messages visible in both local runs and CI logs.
        "--verbose",
        "--output-on-failure",
        "--no-tests=error",
    ]))
}

fn run_command(command: &mut Command) -> Result<()> {
    let status = command.status().map_err(|error| {
        format!(
            "failed to start {}: {error}",
            command.get_program().to_string_lossy()
        )
    })?;
    if !status.success() {
        return Err(format!("{command:?} failed with {status}").into());
    }
    Ok(())
}
