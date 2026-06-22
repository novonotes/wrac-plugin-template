use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Result;
use crate::XtaskOutputLanguage;
use crate::context::Context;
use crate::metadata::PluginMetadata;
use crate::targets::{Platform, PluginFormat, PluginTarget, Target};
use crate::util::{
    common_program_files, copy_path, ensure_exists, home_dir, local_app_data, print_section,
    print_success, remove_if_exists, run_with_language,
};
use crate::{BuildProfile, InstallScope, UninstallScope};

mod build;
mod validation;

pub(crate) use self::build::{
    RustPluginBuild, WrapperBuild, WrapperTarget, build_gui, build_rust_plugin,
    build_wrapper_target, configure_wrapper, package_clap, standalone_products,
};
pub(crate) use self::validation::{validate_plugin_target, validate_wrac_rules_for_targets};

pub(crate) fn launch(ctx: &Context, profile: BuildProfile, plugin_id: Option<&str>) -> Result<()> {
    let plugin = standalone_plugin_to_launch(ctx, plugin_id)?;
    let artifact = ctx.standalone_artifact_for(profile, plugin);
    if !artifact.exists() {
        let release = if profile == BuildProfile::Release {
            " --release"
        } else {
            ""
        };
        return Err(format!(
            "standalone artifact not found: {}\nRun `cargo xtask build -p {} --target=standalone{release}` first.",
            artifact.display(),
            ctx.package_name
        )
        .into());
    }

    print_section(ctx.output_language, "Launch standalone", "standalone 起動");
    print_success(
        ctx.output_language,
        &format!("Launching standalone artifact: {}", artifact.display()),
        &format!("standalone artifact: {}", artifact.display()),
    );
    match ctx.platform {
        Platform::Macos => run_with_language(
            Command::new("open").arg("-W").arg("-n").arg(&artifact),
            ctx.output_language,
        )?,
        Platform::Windows | Platform::Linux => {
            run_with_language(&mut Command::new(&artifact), ctx.output_language)?
        }
    }
    Ok(())
}

fn standalone_plugin_to_launch<'a>(
    ctx: &'a Context,
    plugin_id: Option<&str>,
) -> Result<&'a crate::metadata::PluginProductMetadata> {
    if let Some(plugin_id) = plugin_id {
        return ctx
            .metadata
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
            .ok_or_else(|| format!("plugin ID not found in WRAC metadata: {plugin_id}").into());
    }
    match ctx.metadata.plugins.as_slice() {
        [plugin] => Ok(plugin),
        // Avoid silently launching the first product from a package whose
        // metadata intentionally exposes more than one standalone artifact.
        plugins => Err(format!(
            "multiple plugin products found: {}. Use --plugin-id <PLUGIN_ID>.",
            plugins
                .iter()
                .map(|plugin| plugin.plugin_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into()),
    }
}

pub(crate) fn install_plugin_target(
    ctx: &Context,
    profile: BuildProfile,
    scope: InstallScope,
    target: PluginTarget,
) -> Result<()> {
    match target {
        PluginTarget::Clap => install_artifact(
            &ctx.clap_bundle(profile),
            &install_dir(ctx, scope, PluginFormat::Clap)?,
            ctx.output_language,
        )?,
        PluginTarget::Vst3 => install_artifact(
            &ctx.vst3_bundle(profile),
            &install_dir(ctx, scope, PluginFormat::Vst3)?,
            ctx.output_language,
        )?,
        PluginTarget::Aax => install_artifact(
            &ctx.aax_bundle(profile),
            &install_dir(ctx, scope, PluginFormat::Aax)?,
            ctx.output_language,
        )?,
        PluginTarget::Au => {
            let install_dir = install_dir(ctx, scope, PluginFormat::Au)?;
            for artifact in ctx.au_bundles(profile) {
                install_artifact(&artifact, &install_dir, ctx.output_language)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn uninstall_plugin_target(
    ctx: &Context,
    scope: UninstallScope,
    target: PluginTarget,
    dry_run: bool,
) -> Result<(usize, usize)> {
    let mut removed = 0usize;
    let mut missing = 0usize;
    for path in installed_artifacts(ctx, scope, target)? {
        if !path.exists() {
            println!(
                "  {}: {}",
                missing_label(ctx.output_language),
                path.display()
            );
            missing += 1;
            continue;
        }

        if dry_run {
            println!(
                "  {}: {}",
                would_remove_label(ctx.output_language),
                path.display()
            );
        } else {
            println!(
                "  {}: {}",
                removing_label(ctx.output_language),
                path.display()
            );
            remove_if_exists(&path)?;
        }
        removed += 1;
    }
    Ok((removed, missing))
}

fn missing_label(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "Not found",
        XtaskOutputLanguage::Japanese => "なし",
    }
}

fn would_remove_label(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "Would remove",
        XtaskOutputLanguage::Japanese => "削除予定",
    }
}

fn removing_label(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "Removing",
        XtaskOutputLanguage::Japanese => "削除",
    }
}

pub(crate) fn install_dir(
    ctx: &Context,
    scope: InstallScope,
    format: PluginFormat,
) -> Result<PathBuf> {
    let scope = effective_install_scope(scope, format);
    let dir = match (ctx.platform, scope, format) {
        (Platform::Macos, InstallScope::User, PluginFormat::Clap) => {
            home_dir()?.join("Library/Audio/Plug-Ins/CLAP")
        }
        (Platform::Macos, InstallScope::User, PluginFormat::Vst3) => {
            home_dir()?.join("Library/Audio/Plug-Ins/VST3")
        }
        (Platform::Macos, InstallScope::User, PluginFormat::Au) => {
            home_dir()?.join("Library/Audio/Plug-Ins/Components")
        }
        (Platform::Macos, InstallScope::User, PluginFormat::Aax) => {
            return Err(
                "AAX plugins install to the system-wide Avid folder on macOS; use --scope=system"
                    .into(),
            );
        }
        (Platform::Macos, InstallScope::System, PluginFormat::Clap) => {
            PathBuf::from("/Library/Audio/Plug-Ins/CLAP")
        }
        (Platform::Macos, InstallScope::System, PluginFormat::Vst3) => {
            PathBuf::from("/Library/Audio/Plug-Ins/VST3")
        }
        (Platform::Macos, InstallScope::System, PluginFormat::Au) => {
            PathBuf::from("/Library/Audio/Plug-Ins/Components")
        }
        (Platform::Macos, InstallScope::System, PluginFormat::Aax) => {
            PathBuf::from("/Library/Application Support/Avid/Audio/Plug-Ins")
        }
        (Platform::Windows, InstallScope::User, PluginFormat::Clap) => local_app_data()?
            .join("Programs")
            .join("Common")
            .join("CLAP"),
        (Platform::Windows, InstallScope::User, PluginFormat::Vst3) => local_app_data()?
            .join("Programs")
            .join("Common")
            .join("VST3"),
        (Platform::Windows, InstallScope::User, PluginFormat::Aax) => {
            return Err(
                "AAX plugins install to the system-wide Avid folder on Windows; use --scope=system"
                    .into(),
            );
        }
        (Platform::Windows, InstallScope::System, PluginFormat::Clap) => {
            common_program_files()?.join("CLAP")
        }
        (Platform::Windows, InstallScope::System, PluginFormat::Vst3) => {
            common_program_files()?.join("VST3")
        }
        (Platform::Windows, InstallScope::System, PluginFormat::Aax) => common_program_files()?
            .join("Avid")
            .join("Audio")
            .join("Plug-Ins"),
        (Platform::Windows, _, PluginFormat::Au) => {
            return Err("AU is not supported on Windows".into());
        }
        (Platform::Linux, InstallScope::User, PluginFormat::Clap) => home_dir()?.join(".clap"),
        (Platform::Linux, InstallScope::User, PluginFormat::Vst3) => home_dir()?.join(".vst3"),
        (Platform::Linux, _, PluginFormat::Aax) => {
            return Err("AAX is not supported on Linux".into());
        }
        (Platform::Linux, InstallScope::System, PluginFormat::Clap) => {
            PathBuf::from("/usr/lib/clap")
        }
        (Platform::Linux, InstallScope::System, PluginFormat::Vst3) => {
            PathBuf::from("/usr/lib/vst3")
        }
        (Platform::Linux, _, PluginFormat::Au) => {
            return Err("AU is not supported on Linux".into());
        }
        (_, InstallScope::Default, _) => {
            unreachable!("InstallScope::Default must be resolved before install_dir matching")
        }
    };
    Ok(dir)
}

pub(crate) fn install_artifact(
    artifact: &Path,
    destination_dir: &Path,
    language: XtaskOutputLanguage,
) -> Result<()> {
    ensure_exists(artifact, "install artifact")?;
    fs::create_dir_all(destination_dir)?;
    let destination = destination_dir.join(
        artifact
            .file_name()
            .ok_or_else(|| format!("artifact has no file name: {}", artifact.display()))?,
    );
    // Merging over an existing bundle can leave behind stale binaries or resources.
    // Remove the destination first, then copy the whole artifact so the installed result matches the build output exactly.
    remove_if_exists(&destination)?;
    copy_path(artifact, &destination)?;
    print_success(
        language,
        &format!("Installed: {}", destination.display()),
        &format!("インストール: {}", destination.display()),
    );
    Ok(())
}

pub(crate) fn installed_artifacts(
    ctx: &Context,
    scope: UninstallScope,
    target: PluginTarget,
) -> Result<Vec<PathBuf>> {
    let format = target.format();
    let bundle_names = match target {
        PluginTarget::Clap => vec![ctx.metadata.clap_bundle_name()],
        PluginTarget::Vst3 => vec![ctx.metadata.vst3_bundle_name()],
        PluginTarget::Aax => vec![ctx.metadata.aax_bundle_name()],
        PluginTarget::Au => vec![ctx.metadata.au_bundle_name()],
    };
    let mut artifacts = Vec::new();
    for install_scope in uninstall_scopes(ctx.platform, scope, format)? {
        let dir = install_dir(ctx, *install_scope, format)?;
        artifacts.extend(bundle_names.iter().map(|bundle_name| dir.join(bundle_name)));
    }
    Ok(artifacts)
}

fn uninstall_scopes(
    platform: Platform,
    scope: UninstallScope,
    format: PluginFormat,
) -> Result<&'static [InstallScope]> {
    match scope {
        UninstallScope::All => {
            if matches!(
                (platform, format),
                (Platform::Macos | Platform::Windows, PluginFormat::Aax)
            ) {
                // AAX has no user-local install location on macOS or Windows.
                // Keep the default broad cleanup useful by targeting the only valid
                // install scope instead of failing before it can remove system bundles.
                Ok(&[InstallScope::System])
            } else {
                Ok(&[InstallScope::User, InstallScope::System])
            }
        }
        UninstallScope::User => Ok(&[InstallScope::User]),
        UninstallScope::System => Ok(&[InstallScope::System]),
    }
}

pub(crate) fn effective_install_scope(scope: InstallScope, format: PluginFormat) -> InstallScope {
    match (scope, format) {
        (InstallScope::Default, PluginFormat::Aax) => InstallScope::System,
        (InstallScope::Default, _) => InstallScope::User,
        _ => scope,
    }
}

pub(crate) fn clean(ctx: &Context) -> Result<()> {
    remove_if_exists(&ctx.wrac_dir())?;
    Ok(())
}

fn ensure_common_wrapper_inputs(ctx: &Context) -> Result<()> {
    // Missing subtree files or uninitialized SDK submodules otherwise surface as opaque CMake errors.
    // Check the sentinel files the wrapper actually reads.
    ensure_exists(&ctx.wrapper_dir, "clap_wrapper_builder directory")?;
    ensure_exists(
        &ctx.wrapper_dir.join("clap-wrapper").join("CMakeLists.txt"),
        "clap-wrapper subtree",
    )?;
    ensure_exists(
        &ctx.wrapper_dir
            .join("clap")
            .join("include")
            .join("clap")
            .join("clap.h"),
        "CLAP SDK submodule",
    )?;
    Ok(())
}

fn ensure_vst3_sdk_input(ctx: &Context) -> Result<()> {
    ensure_exists(
        &ctx.wrapper_dir.join("vst3sdk").join("CMakeLists.txt"),
        "VST3 SDK submodule",
    )
}

fn ensure_au_sdk_input(ctx: &Context) -> Result<()> {
    ensure_exists(
        &ctx.wrapper_dir
            .join("AudioUnitSDK")
            .join("include")
            .join("AudioUnitSDK")
            .join("AudioUnitSDK.h"),
        "AudioUnitSDK submodule",
    )
}

fn ensure_aax_sdk_input(ctx: &Context) -> Result<()> {
    let root = aax_sdk_root(ctx)?;
    ensure_aax_sdk_exists(&root)
}

fn aax_sdk_root(ctx: &Context) -> Result<PathBuf> {
    if let Some(root) = env_path(ctx, "AAX_SDK_ROOT")? {
        // clap-wrapper evaluates AAX_SDK_ROOT inside its CMake project, so a relative
        // path would be resolved against clap_wrapper_builder rather than this repo.
        // Resolve relative .env and CI paths from the repository root instead.
        ensure_aax_sdk_exists(&root)?;
        return Ok(root);
    }

    if let Some(root) = config_path(ctx, ctx.default_aax_sdk_root.as_ref()) {
        ensure_aax_sdk_exists(&root)?;
        return Ok(root);
    }

    Err("AAX SDK not found.\nRun `cargo xtask setup` to download the repository-local AAX SDK, then retry.".into())
}

fn ensure_aax_sdk_exists(root: &Path) -> Result<()> {
    let header = root.join("Interfaces").join("AAX.h");
    if header.exists() {
        return Ok(());
    }
    Err(format!(
        "AAX SDK not found: {}\nRun `cargo xtask setup` to download the repository-local AAX SDK, then retry.",
        header.display()
    )
    .into())
}

fn env_path(ctx: &Context, key: &str) -> Result<Option<PathBuf>> {
    let Some(value) = env::var_os(key) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(Some(path))
    } else {
        // `.env` lives at the workspace root, and CI also runs xtask from that
        // root. Using one base directory avoids CMake resolving relative AAX
        // paths from clap_wrapper_builder or another subprocess directory.
        Ok(Some(ctx.root.join(path)))
    }
}

fn config_path(ctx: &Context, path: Option<&PathBuf>) -> Option<PathBuf> {
    let path = path?;
    if path.is_absolute() {
        Some(path.clone())
    } else {
        Some(ctx.root.join(path))
    }
}

pub(crate) fn print_outputs(
    ctx: &Context,
    profile: BuildProfile,
    targets: &[Target],
    standalone_plugin_id: Option<&str>,
) -> Result<()> {
    print_section(ctx.output_language, "Artifacts", "成果物");
    for target in targets {
        match target {
            Target::Clap => println!("  ✅ CLAP: {}", ctx.clap_bundle(profile).display()),
            Target::Vst3 => println!("  ✅ VST3: {}", ctx.vst3_bundle(profile).display()),
            Target::Aax => println!("  ✅ AAX: {}", ctx.aax_bundle(profile).display()),
            Target::Au => {
                for artifact in ctx.au_bundles(profile) {
                    println!("  ✅ AU: {}", artifact.display());
                }
            }
            Target::Standalone => {
                for (_, plugin) in standalone_products(ctx, standalone_plugin_id)? {
                    let artifact = ctx.standalone_artifact_for(profile, plugin);
                    println!("  ✅ Standalone: {}", artifact.display());
                }
            }
        }
    }
    Ok(())
}

fn macos_clap_info_plist(metadata: &PluginMetadata) -> String {
    let plugin_name = &metadata.bundle_name;
    // A CLAP bundle has one CFBundleIdentifier even when the factory exposes
    // multiple products. Keep macOS bundle identity separate from product IDs so
    // adding another product does not silently change the installed bundle.
    let bundle_identifier = &metadata.bundle_identifier;
    let version = &metadata.version;
    let copyright = &metadata.copyright;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist>
  <dict>
    <key>CFBundleExecutable</key>
    <string>{plugin_name}</string>
    <key>CFBundleIconFile</key>
    <string></string>
    <key>CFBundleIdentifier</key>
    <string>{bundle_identifier}</string>
    <key>CFBundleName</key>
    <string>{plugin_name}</string>
    <key>CFBundleDisplayName</key>
    <string>{plugin_name}</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>NSHumanReadableCopyright</key>
    <string>{copyright}</string>
    <key>NSHighResolutionCapable</key>
    <true/>
  </dict>
</plist>
"#
    )
}

fn codesign(path: &Path, language: XtaskOutputLanguage) -> Result<()> {
    run_with_language(
        Command::new("codesign")
            .arg("--force")
            .arg("--sign")
            .arg("-")
            .arg("--timestamp=none")
            .arg(path),
        language,
    )?;
    Ok(())
}

fn codesign_nested_macos_bundle(bundle: &Path, language: XtaskOutputLanguage) -> Result<()> {
    let plugins_dir = bundle.join("Contents").join("PlugIns");
    if plugins_dir.exists() {
        for entry in fs::read_dir(&plugins_dir)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "clap")
            {
                codesign(&path, language)?;
            }
        }
    }
    codesign(bundle, language)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::build::{
        cmake_help_lists_generator, cmake_visual_studio_generators, command_exists_in_paths,
        select_latest_visual_studio_generator,
    };
    use super::validation::parse_vst3_validator_cids;
    use super::*;

    #[test]
    fn command_exists_in_paths_checks_exact_candidate_files() {
        let temp_dir = std::env::temp_dir().join(format!(
            "wrac_xtask_command_path_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        fs::write(temp_dir.join("pnpm.exe"), "").unwrap();

        assert!(command_exists_in_paths(
            OsStr::new("pnpm.exe"),
            [temp_dir.clone()]
        ));
        assert!(!command_exists_in_paths(
            OsStr::new("pnpm.cmd"),
            [temp_dir.clone()]
        ));

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn parses_vst3_validator_cids_from_logged_output() {
        let output = r#"
* Scanning classes...
  Class Info 0:
    name = WRAC Gain
    cid = 822011CA37EC5CEF92D7EC7E67207195
  Class Info 1:
    name = Companion Controller
    cid = ffff664c-b963-53e6-87cc-2a7ceb29674b
"#;

        assert_eq!(
            parse_vst3_validator_cids(output),
            vec![
                "822011CA37EC5CEF92D7EC7E67207195".to_string(),
                "FFFF664CB96353E687CC2A7CEB29674B".to_string(),
            ]
        );
    }

    #[test]
    fn parses_visual_studio_generators_from_cmake_help() {
        let help = r#"
Generators

The following generators are available on this platform (* marks default):
* Visual Studio 18 2026        = Generates Visual Studio 2026 project files.
  Visual Studio 17 2022        = Generates Visual Studio 2022 project files.
  Ninja                        = Generates build.ninja files.
"#;

        assert!(cmake_help_lists_generator(help, "Visual Studio 18 2026"));
        assert!(cmake_help_lists_generator(help, "Visual Studio 17 2022"));
        assert!(!cmake_help_lists_generator(help, "Visual Studio 16 2019"));
        assert_eq!(
            cmake_visual_studio_generators(help),
            vec![
                "Visual Studio 18 2026".to_owned(),
                "Visual Studio 17 2022".to_owned(),
            ]
        );
    }

    #[test]
    fn selects_latest_visual_studio_generator_from_cmake_help() {
        let help = r#"
Generators

The following generators are available on this platform (* marks default):
  Visual Studio 17 2022        = Generates Visual Studio 2022 project files.
* Visual Studio 16 2019        = Generates Visual Studio 2019 project files.
  Visual Studio 15 2017 [arch] = Generates Visual Studio 2017 project files.
  Ninja                        = Generates build.ninja files.
"#;

        assert_eq!(
            select_latest_visual_studio_generator(help),
            Some("Visual Studio 17 2022".to_owned())
        );
    }
}
