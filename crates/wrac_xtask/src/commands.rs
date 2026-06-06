use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Result;
use crate::cli::{BuildArgs, InstallScope, UninstallScope};
use crate::context::Context;
use crate::metadata::PluginMetadata;
use crate::profile::BuildProfile;
use crate::targets::{
    Platform, PluginTarget, Target, ValidateTarget, resolve_build_targets, resolve_plugin_targets,
    resolve_validate_targets,
};
use crate::util::{
    common_program_files, copy_path, ensure_exists, env_value_or, home_dir, local_app_data, on_off,
    remove_if_exists, run,
};
use crate::validation::validate_wrac_rules;

const CLAP_VALIDATOR_VERSION: &str = "0.3.2";
// Keep the local AAX contract explicit instead of delegating to `runtests`.
// DSH exposes collection filters but no stable "all except these test IDs" switch,
// and this template intentionally does not cover HDX cycle-count or page-table XML validation.
const AAX_VALIDATOR_REQUIRED_TESTS: &[&str] = &[
    "info.productids",
    "info.support.audiosuite",
    "info.support.general",
    "info.support.s6_feature",
    "test.data_model",
    "test.describe_validation",
    "test.load_unload",
    "test.page_table.automation_list",
    "test.parameter_traversal.linear",
    "test.parameter_traversal.random",
    "test.parameter_traversal.random.fast",
    "test.parameters",
];
const AAX_VALIDATOR_SKIPPED_TESTS: &[(&str, &str)] = &[
    (
        "test.cycle_counts",
        "targets DSP/HDX cycle-count validation, which is outside this native local build target",
    ),
    (
        "test.page_table.load",
        "requires page-table XML resources, which this template does not generate",
    ),
];
const AAX_VALIDATOR_DSH_TIMEOUT_SECS: u64 = 15 * 60;

pub(crate) fn build(ctx: &Context, args: BuildArgs) -> Result<()> {
    let profile = BuildProfile::from_release(args.release);
    let targets = resolve_build_targets(ctx.platform, &args.target)?;

    // Missing wrapper inputs surface as CMake errors after npm/cargo have already run,
    // making the root cause hard to diagnose. Check wrapper inputs upfront only
    // when the selected targets require a wrapper.
    if targets.iter().any(|target| target.is_wrapper()) || targets.contains(&Target::Standalone) {
        ensure_wrapper_inputs(
            ctx,
            targets.contains(&Target::Vst3),
            targets.contains(&Target::Au),
            targets.contains(&Target::Aax),
        )?;
    }

    if args.clean {
        clean(ctx)?;
    }

    build_gui(ctx)?;

    let mut default_rust_plugin_built = false;
    if targets.contains(&Target::Clap) {
        build_rust_plugin(ctx, profile, RustPluginBuild::Default)?;
        default_rust_plugin_built = true;
        package_clap(ctx, profile)?;
    }

    if targets.iter().any(|target| target.is_wrapper()) {
        // In the old WRY_OBJC_SUFFIX era, VST3 and AU each required a separate Rust
        // staticlib so their Objective-C class names could differ per format. The current
        // wxp/wry embeds the wry source ID into objc2's auto-generated class names, so a
        // single staticlib can be shared by both VST3 and AU in the same product build.
        // Do not split again unless per-format compile-time inputs are reintroduced.
        if !default_rust_plugin_built {
            build_rust_plugin(ctx, profile, RustPluginBuild::Default)?;
        }
        build_wrapper_set(
            ctx,
            profile,
            WrapperBuild::Plugin {
                vst3: targets.contains(&Target::Vst3),
                au: targets.contains(&Target::Au),
                aax: targets.contains(&Target::Aax),
            },
        )?;
    }

    if targets.contains(&Target::Standalone) {
        build_rust_plugin(ctx, profile, RustPluginBuild::Standalone)?;
        build_wrapper_set(ctx, profile, WrapperBuild::Standalone)?;
    }

    print_outputs(ctx, profile, &targets);
    Ok(())
}

fn build_gui(ctx: &Context) -> Result<()> {
    println!("Building GUI...");
    // build.rs embeds src-gui/dist into the plugin binary, so the frontend must be
    // finalized here first. Reversing the order risks bundling a stale or empty dist.
    run(Command::new(npm_command(ctx.platform))
        .arg("install")
        .current_dir(ctx.gui_dir()))?;
    run(Command::new(npm_command(ctx.platform))
        .args(["run", "build"])
        .current_dir(ctx.gui_dir()))?;
    Ok(())
}

fn npm_command(platform: Platform) -> &'static str {
    if platform == Platform::Windows {
        "npm.cmd"
    } else {
        "npm"
    }
}

#[derive(Debug, Clone, Copy)]
enum RustPluginBuild {
    Default,
    Standalone,
}

impl RustPluginBuild {
    fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Standalone => "standalone",
        }
    }

    fn cargo_target_dir(self, ctx: &Context) -> PathBuf {
        match self {
            Self::Default => ctx.target_dir.clone(),
            Self::Standalone => ctx.wrac_dir().join("cargo").join(self.label()),
        }
    }

    fn dynamic_library(self, ctx: &Context, profile: BuildProfile) -> PathBuf {
        self.cargo_target_dir(ctx).join(profile.cargo_dir()).join(
            ctx.platform
                .dynamic_library_name(&ctx.metadata.package_name),
        )
    }

    fn static_library(self, ctx: &Context, profile: BuildProfile) -> PathBuf {
        self.cargo_target_dir(ctx)
            .join(profile.cargo_dir())
            .join(ctx.platform.static_library_name(&ctx.metadata.package_name))
    }
}

fn build_rust_plugin(ctx: &Context, profile: BuildProfile, build: RustPluginBuild) -> Result<()> {
    println!("Building Rust plugin ({})...", build.label());
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--target-dir")
        .arg(build.cargo_target_dir(ctx))
        .arg("--manifest-path")
        .arg(ctx.plugin_manifest());
    if let Some(flag) = profile.cargo_flag() {
        command.arg(flag);
    }
    if ctx.platform == Platform::Macos {
        // Respect CI and user environment variables; inject the template's safe default only when unset.
        command.env(
            "MACOSX_DEPLOYMENT_TARGET",
            env_value_or("MACOSX_DEPLOYMENT_TARGET", "11.0"),
        );
    }
    run(command.current_dir(&ctx.root))?;

    ensure_exists(
        &build.dynamic_library(ctx, profile),
        "dynamic plugin library",
    )?;
    if ctx.platform.supports_wrappers() {
        // clap-wrapper links the Rust staticlib directly rather than consuming a CLAP bundle.
        // Not needed on CLAP-only platforms, so check only on OS targets that support wrappers.
        ensure_exists(&build.static_library(ctx, profile), "static plugin library")?;
    }
    Ok(())
}

fn package_clap(ctx: &Context, profile: BuildProfile) -> Result<()> {
    println!("Packaging CLAP...");
    let bundle = ctx.clap_bundle(profile);
    remove_if_exists(&bundle)?;
    fs::create_dir_all(ctx.plugins_dir(profile))?;

    match ctx.platform {
        Platform::Macos => {
            // macOS distributes CLAP plugins as bundles, not bare dylibs.
            // The host reads bundle metadata, so the plugin ID must match Info.plist.
            // Set install_name to a bundle-relative path so the plugin loads regardless of install location.
            let contents = bundle.join("Contents");
            let macos = contents.join("MacOS");
            fs::create_dir_all(&macos)?;
            fs::write(
                contents.join("Info.plist"),
                macos_clap_info_plist(&ctx.metadata),
            )?;
            fs::write(contents.join("PkgInfo"), "BNDL????")?;
            fs::copy(
                ctx.dynamic_library(profile),
                macos.join(&ctx.metadata.bundle_name),
            )?;
            run(Command::new("install_name_tool")
                .arg("-id")
                .arg(format!("@loader_path/{}", ctx.metadata.bundle_name))
                .arg(macos.join(&ctx.metadata.bundle_name))
                .current_dir(&ctx.root))?;
            codesign(&bundle)?;
        }
        Platform::Windows | Platform::Linux => {
            // On Windows/Linux the CLAP artifact is a dynamic library with the .clap extension.
            // Skipping the bundle structure keeps it compatible with each OS's existing host scan conventions.
            fs::copy(ctx.dynamic_library(profile), &bundle)?;
        }
    }

    ensure_exists(&bundle, "CLAP artifact")?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum WrapperBuild {
    Plugin { vst3: bool, au: bool, aax: bool },
    Standalone,
}

impl WrapperBuild {
    fn purpose(self) -> &'static str {
        match self {
            Self::Plugin { .. } => "wrap",
            Self::Standalone => "standalone",
        }
    }

    fn rust_build(self) -> RustPluginBuild {
        match self {
            Self::Plugin { .. } => RustPluginBuild::Default,
            Self::Standalone => RustPluginBuild::Standalone,
        }
    }
}

fn build_wrapper_set(ctx: &Context, profile: BuildProfile, build: WrapperBuild) -> Result<()> {
    let rust_build = build.rust_build();
    let static_library = rust_build.static_library(ctx, profile);
    ensure_exists(&static_library, "static plugin library")?;

    let build_dir = ctx.cmake_dir(build.purpose(), profile);
    let stage_dir = match build {
        WrapperBuild::Plugin { .. } => ctx.plugins_dir(profile),
        WrapperBuild::Standalone => ctx.standalone_dir(profile),
    };
    fs::create_dir_all(&stage_dir)?;

    let mut configure = Command::new("cmake");
    // Build the wrapper directly from the Rust staticlib. Locating a pre-built CLAP bundle
    // instead would tie reproducibility to clean/install ordering and stale artifacts.
    // Pass the same stage path that xtask uses for downstream validation checks.
    configure
        .arg("-S")
        .arg(&ctx.wrapper_dir)
        .arg("-B")
        .arg(&build_dir)
        .arg(format!(
            "-DCLAP_WRAPPER_BUILDER_TARGET_LIB={}",
            static_library.display()
        ))
        .arg(format!(
            "-DCLAP_WRAPPER_BUILDER_OUTPUT_NAME={}",
            ctx.metadata.bundle_name
        ))
        .arg(format!(
            "-DCLAP_WRAPPER_BUILDER_TARGET_NAME={}_{}",
            ctx.metadata.package_name,
            build.purpose()
        ))
        .arg(format!(
            "-DCLAP_WRAPPER_BUILDER_STAGE_DIR={}",
            stage_dir.display()
        ))
        .arg(format!(
            "-DCLAP_WRAPPER_BUILDER_BUNDLE_VERSION={}",
            ctx.metadata.version
        ))
        .arg(format!("-DCMAKE_BUILD_TYPE={}", profile.cmake_config()))
        .arg("-DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=OFF")
        .arg("-DCLAP_WRAPPER_CXX_STANDARD=23");
    add_wrapper_product_args(ctx, &mut configure, build);

    match build {
        WrapperBuild::Plugin { vst3, au, aax } => {
            configure
                .arg(format!(
                    "-DCLAP_WRAPPER_BUILDER_BUILD_VST3={}",
                    on_off(vst3)
                ))
                .arg(format!("-DCLAP_WRAPPER_BUILDER_BUILD_AUV2={}", on_off(au)))
                .arg(format!("-DCLAP_WRAPPER_BUILDER_BUILD_AAX={}", on_off(aax)))
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=OFF");
            if aax {
                configure.arg(format!("-DAAX_SDK_ROOT={}", aax_sdk_root(ctx)?.display()));
            }
        }
        WrapperBuild::Standalone => {
            // standalone requires additional app-side dependencies that plugin wrappers do not.
            // Delegate fetching to clap-wrapper's own download logic while keeping downloads
            // disabled for plugin wrapper builds.
            configure
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_VST3=OFF")
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_AUV2=OFF")
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_AAX=OFF")
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=ON")
                .arg("-DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=ON");
        }
    }

    if ctx.platform == Platform::Macos {
        // AUv2 uses 4-character type/manufacturer/subtype codes as the host discovery key.
        // Drive them from the template's constants rather than inferring from the Rust descriptor.
        configure
            .arg(format!(
                "-DAUDIOUNIT_SDK_ROOT={}",
                ctx.wrapper_dir.join("AudioUnitSDK").display()
            ))
            .arg(format!(
                "-DCLAP_WRAPPER_AUV2_MANUFACTURER_NAME={}",
                ctx.metadata.company_name
            ))
            .arg(format!(
                "-DCLAP_WRAPPER_AUV2_MANUFACTURER_CODE={}",
                ctx.metadata.auv2_manufacturer_code
            ));
    }

    if let Some(generator) = ctx.platform.cmake_generator() {
        configure.arg("-G").arg(generator);
    }

    run(configure.current_dir(&ctx.root))?;

    let mut build_cmd = Command::new("cmake");
    build_cmd
        .arg("--build")
        .arg(&build_dir)
        .arg("--config")
        .arg(profile.cmake_config());

    if ctx.platform == Platform::Macos {
        // AudioUnitSDK emits GNU statement-expression and narrowing warnings in Xcode.
        // Suppress them here so template users are not pulled into wrapper SDK warnings.
        build_cmd.args([
            "--",
            "OTHER_CPLUSPLUSFLAGS=$(inherited) -Wno-unknown-warning-option -Wno-gnu-statement-expression-from-macro-expansion -Wno-shorten-64-to-32 -Wno-perf-constraint-implies-noexcept",
        ]);
    }

    run(build_cmd.current_dir(&ctx.root))?;

    match build {
        WrapperBuild::Plugin { vst3, au, aax } => {
            if vst3 {
                ensure_exists(&ctx.vst3_bundle(profile), "VST3 artifact")?;
                if ctx.platform == Platform::Macos {
                    // macOS hosts may reject unsigned bundles; apply an ad-hoc signature for development.
                    codesign_nested_macos_bundle(&ctx.vst3_bundle(profile))?;
                }
            }
            if au {
                for artifact in ctx.au_bundles(profile) {
                    ensure_exists(&artifact, "AU artifact")?;
                    // AU components are loaded via AudioComponentRegistrar, so they must be signed even for local builds.
                    codesign_nested_macos_bundle(&artifact)?;
                }
            }
            if aax {
                ensure_exists(&ctx.aax_bundle(profile), "AAX artifact")?;
                if ctx.platform == Platform::Macos {
                    // AAX developer validation loads the bundle directly via DSH, so keep
                    // the local artifact ad-hoc signed before the validator sees it.
                    codesign_nested_macos_bundle(&ctx.aax_bundle(profile))?;
                }
            }
        }
        WrapperBuild::Standalone => {
            for artifact in ctx.standalone_artifacts(profile) {
                ensure_exists(&artifact, "standalone artifact")?;
                if ctx.platform == Platform::Macos {
                    // Apply the same Gatekeeper/loader treatment to the standalone app as to plugin bundles.
                    codesign_nested_macos_bundle(&artifact)?;
                }
            }
        }
    }

    Ok(())
}

fn add_wrapper_product_args(ctx: &Context, command: &mut Command, build: WrapperBuild) {
    command.arg(format!(
        "-DCLAP_WRAPPER_BUILDER_PRODUCT_COUNT={}",
        ctx.metadata.plugins.len()
    ));
    for (index, plugin) in ctx.metadata.plugins.iter().enumerate() {
        match build {
            WrapperBuild::Plugin { au: true, .. } => {
                // CLAP/VST3 read product descriptors from the Rust plugin factory.
                // AUv2 cannot, so only AUv2 builds need per-product output and
                // four-character AudioComponent identity values from xtask.
                command
                    .arg(format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_OUTPUT_NAME={}",
                        plugin.plugin_name
                    ))
                    .arg(format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_AUV2_TYPE={}",
                        plugin.auv2_type
                    ))
                    .arg(format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_AUV2_SUBTYPE={}",
                        plugin.auv2_subtype
                    ));
            }
            WrapperBuild::Standalone => {
                // Each standalone app embeds the product ID it should host at
                // compile time; passing all standalone metadata keeps CMake from
                // choosing an implicit primary product.
                command
                    .arg(format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_OUTPUT_NAME={}",
                        plugin.plugin_name
                    ))
                    .arg(format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_PLUGIN_ID={}",
                        plugin.plugin_id
                    ))
                    .arg(format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_STANDALONE_NAME={}",
                        plugin.standalone_name
                    ));
            }
            WrapperBuild::Plugin { au: false, .. } => {}
        }
    }
}

pub(crate) fn install(
    ctx: &Context,
    profile: BuildProfile,
    scope: InstallScope,
    requested: &[PluginTarget],
) -> Result<()> {
    let targets = resolve_plugin_targets(ctx.platform, requested)?;
    install_plugin_targets(ctx, profile, scope, &targets)
}

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

    println!("Launching standalone artifact: {}", artifact.display());
    match ctx.platform {
        Platform::Macos => run(Command::new("open").arg("-W").arg("-n").arg(&artifact))?,
        Platform::Windows | Platform::Linux => run(&mut Command::new(&artifact))?,
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

fn install_plugin_targets(
    ctx: &Context,
    profile: BuildProfile,
    scope: InstallScope,
    targets: &[PluginTarget],
) -> Result<()> {
    for target in targets {
        match target {
            PluginTarget::Clap => install_artifact(
                &ctx.clap_bundle(profile),
                &install_dir(ctx, scope, PluginFormat::Clap)?,
            )?,
            PluginTarget::Vst3 => install_artifact(
                &ctx.vst3_bundle(profile),
                &install_dir(ctx, scope, PluginFormat::Vst3)?,
            )?,
            PluginTarget::Aax => install_artifact(
                &ctx.aax_bundle(profile),
                &install_dir(ctx, scope, PluginFormat::Aax)?,
            )?,
            PluginTarget::Au => {
                let install_dir = install_dir(ctx, scope, PluginFormat::Au)?;
                for artifact in ctx.au_bundles(profile) {
                    install_artifact(&artifact, &install_dir)?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn uninstall(
    ctx: &Context,
    scope: UninstallScope,
    requested: &[PluginTarget],
    dry_run: bool,
) -> Result<()> {
    let targets = resolve_plugin_targets(ctx.platform, requested)?;

    let mut removed = 0usize;
    let mut missing = 0usize;
    for target in targets {
        for path in installed_artifacts(ctx, scope, target)? {
            if !path.exists() {
                println!("Not found: {}", path.display());
                missing += 1;
                continue;
            }

            if dry_run {
                println!("Would remove: {}", path.display());
            } else {
                println!("Removing: {}", path.display());
                remove_if_exists(&path)?;
            }
            removed += 1;
        }
    }

    if dry_run {
        println!("Uninstall dry run complete: {removed} would be removed, {missing} not found");
    } else {
        println!("Uninstall complete: {removed} removed, {missing} not found");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum PluginFormat {
    Clap,
    Vst3,
    Au,
    Aax,
}

fn install_dir(ctx: &Context, scope: InstallScope, format: PluginFormat) -> Result<PathBuf> {
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
    };
    Ok(dir)
}

fn install_artifact(artifact: &Path, destination_dir: &Path) -> Result<()> {
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
    println!("Installed: {}", destination.display());
    Ok(())
}

fn installed_artifacts(
    ctx: &Context,
    scope: UninstallScope,
    target: PluginTarget,
) -> Result<Vec<PathBuf>> {
    let format = match target {
        PluginTarget::Clap => PluginFormat::Clap,
        PluginTarget::Vst3 => PluginFormat::Vst3,
        PluginTarget::Au => PluginFormat::Au,
        PluginTarget::Aax => PluginFormat::Aax,
    };
    let bundle_names = match target {
        PluginTarget::Clap => vec![ctx.metadata.clap_bundle_name()],
        PluginTarget::Vst3 => vec![ctx.metadata.vst3_bundle_name()],
        PluginTarget::Aax => vec![ctx.metadata.aax_bundle_name()],
        PluginTarget::Au => ctx
            .metadata
            .plugins
            .iter()
            .map(|plugin| ctx.metadata.au_bundle_name(plugin))
            .collect(),
    };
    let mut artifacts = Vec::new();
    for install_scope in uninstall_scopes(scope) {
        let dir = install_dir(ctx, *install_scope, format)?;
        artifacts.extend(bundle_names.iter().map(|bundle_name| dir.join(bundle_name)));
    }
    Ok(artifacts)
}

fn uninstall_scopes(scope: UninstallScope) -> &'static [InstallScope] {
    match scope {
        UninstallScope::All => &[InstallScope::User, InstallScope::System],
        UninstallScope::User => &[InstallScope::User],
        UninstallScope::System => &[InstallScope::System],
    }
}

pub(crate) fn validate(
    ctx: &Context,
    profile: BuildProfile,
    requested: &[ValidateTarget],
) -> Result<()> {
    let targets = resolve_validate_targets(ctx.platform, requested)?;
    if targets.contains(&ValidateTarget::Vst3) {
        // The validator is built on-demand from the VST3 SDK, so verify the SDK before checking
        // the artifact. Proceeding to CMake with an empty submodule directory produces an opaque error.
        ensure_vst3_sdk_input(ctx)?;
    }
    if targets.contains(&ValidateTarget::Aax) {
        ensure_aax_sdk_input(ctx)?;
    }
    validate_targets(ctx, profile, &targets)
}

fn validate_targets(
    ctx: &Context,
    profile: BuildProfile,
    targets: &[ValidateTarget],
) -> Result<()> {
    if targets.is_empty() {
        println!("No CLAP/VST3/AU targets to validate.");
        return Ok(());
    }

    validate_wrac_rules(ctx, profile, targets)?;

    if targets.contains(&ValidateTarget::Clap) {
        let clap = ctx.clap_bundle(profile);
        ensure_exists(&clap, "CLAP artifact")?;
        let validator = ensure_clap_validator(ctx)?;
        run(Command::new(validator)
            .arg("validate")
            .arg(&clap)
            .arg("--only-failed")
            .current_dir(&ctx.root))?;
    }

    if targets.contains(&ValidateTarget::Vst3) {
        let vst3 = ctx.vst3_bundle(profile);
        ensure_exists(&vst3, "VST3 artifact")?;
        let validator = ensure_vst3_validator(ctx)?;
        run(Command::new(validator).arg(&vst3).current_dir(&ctx.root))?;
    }

    if targets.contains(&ValidateTarget::Au) {
        ensure_no_system_au_conflict(ctx)?;

        // auval resolves its target via AudioComponentRegistrar rather than a direct path,
        // so the freshly built AU must be installed user-locally before running validation.
        let install_dir = install_dir(ctx, InstallScope::User, PluginFormat::Au)?;
        for artifact in ctx.au_bundles(profile) {
            ensure_exists(&artifact, "AU artifact")?;
            install_artifact(&artifact, &install_dir)?;
        }

        // The registrar caches component metadata, so it must be restarted to expose the newly placed AU.
        // If killall fails, auval may still detect the component, so treat this as best-effort.
        let _ = Command::new("killall")
            .args(["-9", "AudioComponentRegistrar"])
            .status();

        for plugin in &ctx.metadata.plugins {
            run(Command::new("/usr/bin/auval")
                .args([
                    "-v",
                    &plugin.auv2_type,
                    &plugin.auv2_subtype,
                    &ctx.metadata.auv2_manufacturer_code,
                ])
                .current_dir(&ctx.root))?;
        }
    }

    if targets.contains(&ValidateTarget::Aax) {
        let aax = ctx.aax_bundle(profile);
        ensure_exists(&aax, "AAX artifact")?;
        run_aax_validator(ctx, &aax)?;
    }

    Ok(())
}

fn run_aax_validator(ctx: &Context, aax: &Path) -> Result<()> {
    let results_dir = ctx.wrac_dir().join("validation").join("aax");
    // A fresh directory prevents a previous pass result from masking a missing
    // validator output if DSH exits early or changes a result reference.
    remove_if_exists(&results_dir)?;
    fs::create_dir_all(&results_dir)?;
    let aax = stage_aax_for_validator(&results_dir, aax)?;

    println!("Running AAX validator for: {}", aax.display());
    println!(
        "AAX validation runs {} selected validator tests.",
        AAX_VALIDATOR_REQUIRED_TESTS.len()
    );
    for (test_id, reason) in AAX_VALIDATOR_SKIPPED_TESTS {
        println!("Skipping {test_id}: {reason}.");
    }
    println!();

    if ctx.platform == Platform::Windows {
        run_aax_validator_dtt(ctx, &aax, &results_dir)?;
    } else {
        run_aax_validator_dsh(ctx, &aax, &results_dir)?;
    }

    assert_aax_validator_results(&results_dir)
}

fn run_aax_validator_dsh(ctx: &Context, aax: &Path, results_dir: &Path) -> Result<()> {
    let dsh = ensure_aax_validator_dsh(ctx)?;
    println!("========== Running command ==========");
    println!("$ {}", dsh.display());

    let mut child = Command::new(&dsh)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(dsh.parent().unwrap_or(&ctx.root))
        .spawn()?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or("failed to open DSH stdin for AAX validation")?;
        write_dsh_command(&mut stdin, "load_dish aaxval")?;
        for (index, test_id) in AAX_VALIDATOR_REQUIRED_TESTS.iter().enumerate() {
            let result_ref = format!("r{}", index + 1);
            let result_path = aax_validator_result_path(results_dir, index, test_id);
            // The aaxval dish has no documented per-test exclude list for `runtests`.
            // Running explicit `runtest` commands keeps the CI contract stable as
            // Avid updates collections, while still using DSH's own JSON result writer.
            write_dsh_command(
                &mut stdin,
                format!(
                    "runtest {{test: {test_id}, path: {}, stringformat: json, detail: min}}",
                    dsh_string(aax)?
                ),
            )?;
            write_dsh_command(
                &mut stdin,
                format!(
                    "saveresult {{result_ref: {result_ref}, result_path: {}, stringformat: json}}",
                    dsh_string(&result_path)?
                ),
            )?;
        }
        write_dsh_command(&mut stdin, "quit")?;
    }

    let output = wait_for_aax_validator_process(child, aax_validator_dsh_timeout()?)?;
    let stdout_path = results_dir.join("dsh-stdout.log");
    let stderr_path = results_dir.join("dsh-stderr.log");
    fs::write(&stdout_path, &output.stdout)?;
    fs::write(&stderr_path, &output.stderr)?;
    println!("AAX validator DSH stdout: {}", stdout_path.display());
    if !output.stderr.is_empty() {
        println!("AAX validator DSH stderr: {}", stderr_path.display());
    }

    let status = output.status;
    if !status.success() {
        print_aax_validator_output(&output.stdout, &output.stderr);
        return Err(format!(
            "AAX validator/DSH failed with status {status}; see {}",
            stdout_path.display()
        )
        .into());
    }

    Ok(())
}

fn run_aax_validator_dtt(ctx: &Context, aax: &Path, results_dir: &Path) -> Result<()> {
    let dtt = ensure_aax_validator_dtt(ctx)?;
    let aax_search_dir = aax
        .parent()
        .ok_or_else(|| format!("AAX bundle path has no parent directory: {}", aax.display()))?;
    println!("========== Running command ==========");
    println!("$ {}", dtt.display());

    for (index, test_id) in AAX_VALIDATOR_REQUIRED_TESTS.iter().enumerate() {
        let test_dir =
            results_dir
                .join("dtt")
                .join(format!("{:02}-{}", index + 1, test_id.replace('.', "_")));
        fs::create_dir_all(&test_dir)?;

        // Avid ships DTT as the automatable scripting layer for DigiShell. The
        // Windows DSH process can launch in hosted CI while ignoring direct stdin,
        // so Windows validation goes through DTT instead of treating DSH like a
        // plain pipe-oriented CLI. The bundled ValidatorRunAllTests script discovers
        // plug-ins via `findaaxplugins`, which returns the expected list shape when
        // given a search directory rather than the `.aaxplugin` bundle path itself.
        let child = Command::new(&dtt)
            .arg("--script")
            .arg("ValidatorRunAllTests")
            .arg("--no_pref_delete")
            .arg("--no_move_options")
            .arg("--disable_digitrace")
            .arg("--arg")
            .arg(format!("pi_path={}", aax_search_dir.display()))
            .arg("--arg")
            .arg(format!("out_path={}", test_dir.display()))
            .arg("--arg")
            .arg("result_format=json")
            .arg("--arg")
            .arg(format!("test_id={test_id}"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(dtt.parent().unwrap_or(&ctx.root))
            .spawn()?;
        let output = wait_for_aax_validator_process(child, aax_validator_dsh_timeout()?)?;
        let stdout_path = test_dir.join("dtt-stdout.log");
        let stderr_path = test_dir.join("dtt-stderr.log");
        fs::write(&stdout_path, &output.stdout)?;
        fs::write(&stderr_path, &output.stderr)?;

        if !output.status.success() {
            print_aax_validator_output(&output.stdout, &output.stderr);
            return Err(format!(
                "AAX validator/DTT failed while running {test_id}; see {}",
                stdout_path.display()
            )
            .into());
        }

        let result_path = aax_validator_result_path(results_dir, index, test_id);
        let dtt_result = find_aax_validator_dtt_result(&test_dir, test_id)?;
        fs::copy(&dtt_result, &result_path).map_err(|err| {
            format!(
                "failed to copy AAX validator result {} to {}: {err}",
                dtt_result.display(),
                result_path.display()
            )
        })?;
    }

    Ok(())
}

fn find_aax_validator_dtt_result(test_dir: &Path, test_id: &str) -> Result<PathBuf> {
    let result_dir = test_dir.join("run_all_tests_result");
    let expected_prefix = format!("{test_id}__");
    let mut matches = Vec::new();
    for entry in fs::read_dir(&result_dir).map_err(|err| {
        format!(
            "failed to read AAX validator DTT result directory {}: {err}",
            result_dir.display()
        )
    })? {
        let path = entry?.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if file_name.starts_with(&expected_prefix)
            && path.extension().is_some_and(|ext| ext == "json")
        {
            matches.push(path);
        }
    }
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(format!(
            "AAX validator/DTT did not write a JSON result for {test_id} under {}",
            result_dir.display()
        )
        .into()),
        _ => Err(format!(
            "AAX validator/DTT wrote multiple JSON results for {test_id} under {}",
            result_dir.display()
        )
        .into()),
    }
}

fn assert_aax_validator_results(results_dir: &Path) -> Result<()> {
    let mut failed = Vec::new();
    for (index, test_id) in AAX_VALIDATOR_REQUIRED_TESTS.iter().enumerate() {
        let result_path = aax_validator_result_path(results_dir, index, test_id);
        let status = aax_validator_result_status(&result_path)?;
        if status == "E_COMPLETED_PASS" {
            println!("AAX validator PASS: {test_id}");
        } else {
            println!(
                "AAX validator FAIL: {test_id} ({status}); see {}",
                result_path.display()
            );
            failed.push(format!("{test_id} ({status})"));
        }
    }
    if !failed.is_empty() {
        return Err(format!(
            "AAX validator reported failed validation results: {}",
            failed.join(", ")
        )
        .into());
    }
    Ok(())
}

fn write_dsh_command(stdin: &mut impl Write, command: impl AsRef<str>) -> Result<()> {
    let command = command.as_ref();
    writeln!(stdin, "{command}")?;
    Ok(())
}

fn wait_for_aax_validator_process(mut child: Child, timeout: Duration) -> Result<Output> {
    let started_at = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if started_at.elapsed() >= timeout {
            child.kill()?;
            let output = child.wait_with_output()?;
            print_aax_validator_output(&output.stdout, &output.stderr);
            return Err(format!(
                "AAX validator process timed out after {} seconds",
                timeout.as_secs()
            )
            .into());
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn aax_validator_dsh_timeout() -> Result<Duration> {
    let seconds = match env::var("AAX_VALIDATOR_DSH_TIMEOUT_SECS") {
        Ok(value) => value.parse::<u64>().map_err(|err| {
            format!("failed to parse AAX_VALIDATOR_DSH_TIMEOUT_SECS={value}: {err}")
        })?,
        Err(env::VarError::NotPresent) => AAX_VALIDATOR_DSH_TIMEOUT_SECS,
        Err(err) => {
            return Err(format!("failed to read AAX_VALIDATOR_DSH_TIMEOUT_SECS: {err}").into());
        }
    };
    Ok(Duration::from_secs(seconds))
}

fn stage_aax_for_validator(results_dir: &Path, aax: &Path) -> Result<PathBuf> {
    let bundle_name = aax
        .file_name()
        .ok_or_else(|| format!("AAX bundle path has no file name: {}", aax.display()))?;
    let staged_aax = results_dir.join("input").join(bundle_name);
    // DSH/DTT path handling is easier to keep stable when the search directory has
    // no spaces, but the `.aaxplugin` bundle name itself should stay product-facing.
    // Avid's DTT discovery inspects bundle structure, so renaming the bundle during
    // staging can make `findaaxplugins` miss an otherwise valid plug-in.
    remove_if_exists(&staged_aax)?;
    if let Some(parent) = staged_aax.parent() {
        fs::create_dir_all(parent)?;
    }
    copy_path(aax, &staged_aax)?;
    Ok(staged_aax)
}

fn print_aax_validator_output(stdout: &[u8], stderr: &[u8]) {
    let stdout = String::from_utf8_lossy(stdout);
    if !stdout.trim().is_empty() {
        println!("========== AAX validator stdout ==========");
        println!("{stdout}");
    }
    let stderr = String::from_utf8_lossy(stderr);
    if !stderr.trim().is_empty() {
        println!("========== AAX validator stderr ==========");
        println!("{stderr}");
    }
}

fn aax_validator_result_path(results_dir: &Path, index: usize, test_id: &str) -> PathBuf {
    results_dir.join(format!(
        "{:02}-{}.json",
        index + 1,
        test_id.replace('.', "_")
    ))
}

fn aax_validator_result_status(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read AAX validator result {}: {err}",
            path.display()
        )
    })?;
    let json: Value = serde_json::from_str(&content).map_err(|err| {
        format!(
            "failed to parse AAX validator result {}: {err}",
            path.display()
        )
    })?;
    json.get("result_status")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "AAX validator result did not include result_status: {}",
                path.display()
            )
            .into()
        })
}

fn dsh_string(path: &Path) -> Result<String> {
    let value = path.display().to_string();
    if value.contains('"') {
        return Err(format!("DSH paths cannot contain double quotes: {value}").into());
    }
    // DSH command input is not a shell, but quoted paths are still required for
    // bundle names such as "WRAC Gain.aaxplugin".
    Ok(format!("\"{value}\""))
}

fn ensure_aax_validator_dsh(ctx: &Context) -> Result<PathBuf> {
    let root = aax_validator_dsh_root(ctx)?;
    let dsh = aax_validator_dsh_executable(&root, ctx.platform)?;
    ensure_exists(&dsh, "AAX validator DSH")?;
    if ctx.platform == Platform::Macos {
        // Avid validator archives downloaded through a browser may carry quarantine
        // attributes. Clearing them here keeps first-run validation deterministic;
        // failure is non-fatal because previously cleared archives work without it.
        let _ = Command::new("xattr")
            .args(["-dr", "com.apple.quarantine"])
            .arg(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        run(Command::new("chmod")
            .arg("+x")
            .arg(&dsh)
            .current_dir(&ctx.root))?;
    }
    Ok(dsh)
}

fn ensure_aax_validator_dtt(ctx: &Context) -> Result<PathBuf> {
    let root = aax_validator_dsh_root(ctx)?;
    let dtt = aax_validator_dtt_runner(&root, ctx.platform)?;
    ensure_exists(&dtt, "AAX validator DTT runner")?;
    Ok(dtt)
}

fn aax_validator_dsh_root(ctx: &Context) -> Result<PathBuf> {
    if let Some(root) = env::var_os("AAX_VALIDATOR_DSH_ROOT").map(PathBuf::from) {
        return Ok(root);
    }

    let extracted_root = ctx.target_dir.join("tools").join("aax-validator-dsh");
    if aax_validator_dsh_executable(&extracted_root, ctx.platform).is_ok() {
        return Ok(extracted_root);
    }

    let archive = aax_validator_dsh_archive()?;
    // Extract into target/ so CI caches or local builds can reuse the private
    // validator without committing Avid binaries to the template repository.
    remove_if_exists(&extracted_root)?;
    fs::create_dir_all(&extracted_root)?;
    if archive
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        // Windows validator downloads are zip archives. GitHub-hosted Windows runners
        // provide 7-Zip, and using it here avoids relying on tar implementations that
        // only support tar streams.
        run(Command::new("7z")
            .arg("x")
            .arg(&archive)
            .arg(format!("-o{}", extracted_root.display()))
            .arg("-y")
            .current_dir(&ctx.root))?;
    } else {
        run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("--strip-components=1")
            .arg("-C")
            .arg(&extracted_root)
            .current_dir(&ctx.root))?;
    }
    Ok(extracted_root)
}

fn aax_validator_dsh_archive() -> Result<PathBuf> {
    if let Some(archive) = env::var_os("AAX_VALIDATOR_DSH_ARCHIVE").map(PathBuf::from) {
        ensure_exists(&archive, "AAX validator/DSH archive")?;
        return Ok(archive);
    }
    // Environment variables are the reproducible path for CI. The Downloads fallback
    // is only a local developer convenience for the exact Avid archive names.
    let downloads = home_dir()?.join("Downloads");
    for name in [
        "aax-validator-dsh-2024-6-0-138bab0d-mac-arm64.tar.gz",
        "aax-validator-dsh-2024-6-0-138bab0d-mac-x64.tar.gz",
        "aax-validator-dsh-2024-6-0-dc68c2dd-win-x86_64.zip",
    ] {
        let archive = downloads.join(name);
        if archive.exists() {
            return Ok(archive);
        }
    }
    Err("AAX validator/DSH not found. Set AAX_VALIDATOR_DSH_ROOT to the extracted validator directory or AAX_VALIDATOR_DSH_ARCHIVE to the downloaded archive.".into())
}

fn aax_validator_dsh_executable(root: &Path, platform: Platform) -> Result<PathBuf> {
    let executable = executable_name("dsh", platform);
    for candidate in [
        // Avid's Windows validator ReadMe starts DigiShell from the package root.
        // The Tools copy can launch but may not consume scripted stdin reliably on
        // hosted CI, so prefer the root executable for Windows zip archives.
        root.join("DigiShell").join(&executable),
        // Zip extraction keeps the archive's top-level DigiShell directory. Include the
        // nested Windows paths so CI can consume Avid's downloaded archive directly.
        root.join("DigiShell")
            .join("AAXValidatorResources")
            .join("Tools")
            .join(&executable),
        // Windows validator archives place the runnable validator dish and helper
        // executables under AAXValidatorResources/Tools. Keep it as a fallback for
        // extracted roots supplied by users.
        root.join("AAXValidatorResources")
            .join("Tools")
            .join(&executable),
        // macOS archives expose CommandLineTools at the package root.
        root.join("CommandLineTools").join(&executable),
        // Some Windows archives also include a top-level dsh.exe. Keep it as a fallback
        // for extracted roots supplied by users, but do not rely on it in CI.
        root.join(&executable),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "AAX validator DSH executable not found under {}",
        root.display()
    )
    .into())
}

fn aax_validator_dtt_runner(root: &Path, platform: Platform) -> Result<PathBuf> {
    let runner = if platform == Platform::Windows {
        "run_test.bat"
    } else {
        "run_test.command"
    };
    for candidate in [
        root.join("DigiShell").join("DTT").join(runner),
        root.join("DTT").join(runner),
        root.join("DigiShell")
            .join("AAXValidatorResources")
            .join("Tools")
            .join("DTT")
            .join(runner),
        root.join("AAXValidatorResources")
            .join("Tools")
            .join("DTT")
            .join(runner),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "AAX validator DTT runner not found under {}",
        root.display()
    )
    .into())
}

fn ensure_clap_validator(ctx: &Context) -> Result<PathBuf> {
    let validator_dir = ctx
        .target_dir
        .join("tools")
        .join("clap-validator")
        .join(CLAP_VALIDATOR_VERSION);
    let validator = clap_validator_executable(ctx.platform, &validator_dir);
    if validator.exists() {
        return Ok(validator);
    }

    fs::create_dir_all(&validator_dir)?;
    let archive_name = clap_validator_archive_name(ctx.platform);
    let archive = validator_dir.join(archive_name);
    if !archive.exists() {
        let url = format!(
            "https://github.com/free-audio/clap-validator/releases/download/{CLAP_VALIDATOR_VERSION}/{archive_name}"
        );
        run(Command::new("curl")
            .args(["-L", "--fail", "-o"])
            .arg(&archive)
            .arg(url)
            .current_dir(&ctx.root))?;
    }

    if archive_name.ends_with(".zip") {
        // Windows runners provide bsdtar as `tar`, and it can extract zip files.
        // Using it here keeps argument passing identical to the tar.gz path.
        run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&validator_dir)
            .current_dir(&ctx.root))?;
    } else {
        run(Command::new("tar")
            .args(["-xzf"])
            .arg(&archive)
            .arg("-C")
            .arg(&validator_dir)
            .current_dir(&ctx.root))?;
    }

    ensure_exists(&validator, "CLAP validator")?;
    if ctx.platform != Platform::Windows {
        run(Command::new("chmod")
            .arg("+x")
            .arg(&validator)
            .current_dir(&ctx.root))?;
    }
    Ok(validator)
}

fn clap_validator_archive_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Macos => "clap-validator-0.3.2-macos-universal.tar.gz",
        Platform::Windows => "clap-validator-0.3.2-windows.zip",
        Platform::Linux => "clap-validator-0.3.2-ubuntu-18.04.tar.gz",
    }
}

fn clap_validator_executable(platform: Platform, validator_dir: &Path) -> PathBuf {
    match platform {
        Platform::Macos => validator_dir.join("binaries").join("clap-validator"),
        Platform::Windows => validator_dir.join("clap-validator.exe"),
        Platform::Linux => validator_dir.join("clap-validator"),
    }
}

fn ensure_no_system_au_conflict(ctx: &Context) -> Result<()> {
    for plugin in &ctx.metadata.plugins {
        let system_au = Path::new("/Library/Audio/Plug-Ins/Components")
            .join(ctx.metadata.au_bundle_name(plugin));
        if system_au.exists() {
            return Err(format!(
                "system-wide AU already exists at {}. auval may validate that copy instead of the freshly built user-local AU. Remove the system-wide component and run validation again.",
                system_au.display()
            )
            .into());
        }
    }
    Ok(())
}

fn ensure_vst3_validator(ctx: &Context) -> Result<PathBuf> {
    ensure_vst3_sdk_input(ctx)?;

    let executable = if ctx.platform == Platform::Windows {
        "validator.exe"
    } else {
        "validator"
    };
    let validator_bin_dir = ctx.target_dir.join("vst3sdk-validator").join("bin");
    let validator = validator_bin_dir.join("Debug").join(executable);
    let validator_without_config = validator_bin_dir.join(executable);

    if validator.exists() {
        return Ok(validator);
    }
    if validator_without_config.exists() {
        return Ok(validator_without_config);
    }

    // The validator is a verification tool, not a shipping artifact.
    // It is independent of the plugin's release/debug profile, so a single Debug build is reused for both profiles.
    let build_dir = ctx.target_dir.join("vst3sdk-validator");
    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(ctx.wrapper_dir.join("vst3sdk"))
        .arg("-B")
        .arg(&build_dir)
        .arg("-DSMTG_ENABLE_VST3_HOSTING_EXAMPLES=ON")
        .arg("-DSMTG_ENABLE_VST3_PLUGIN_EXAMPLES=OFF")
        .arg("-DSMTG_ENABLE_VSTGUI_SUPPORT=OFF");
    if ctx.platform == Platform::Macos {
        configure.arg("-G").arg("Xcode");
    }
    run(configure.current_dir(&ctx.root))?;

    run(Command::new("cmake")
        .arg("--build")
        .arg(&build_dir)
        .arg("--target")
        .arg("validator")
        .arg("--config")
        .arg("Debug")
        .current_dir(&ctx.root))?;

    if validator.exists() {
        Ok(validator)
    } else {
        ensure_exists(&validator_without_config, "VST3 validator")?;
        Ok(validator_without_config)
    }
}

pub(crate) fn clean(ctx: &Context) -> Result<()> {
    remove_if_exists(&ctx.wrac_dir())?;
    Ok(())
}

fn ensure_wrapper_inputs(
    ctx: &Context,
    needs_vst3: bool,
    needs_au: bool,
    needs_aax: bool,
) -> Result<()> {
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
    if needs_vst3 {
        ensure_vst3_sdk_input(ctx)?;
    }
    if needs_au {
        ensure_exists(
            &ctx.wrapper_dir
                .join("AudioUnitSDK")
                .join("include")
                .join("AudioUnitSDK")
                .join("AudioUnitSDK.h"),
            "AudioUnitSDK submodule",
        )?;
    }
    if needs_aax {
        ensure_aax_sdk_input(ctx)?;
    }
    Ok(())
}

fn ensure_vst3_sdk_input(ctx: &Context) -> Result<()> {
    ensure_exists(
        &ctx.wrapper_dir.join("vst3sdk").join("CMakeLists.txt"),
        "VST3 SDK submodule",
    )
}

fn ensure_aax_sdk_input(ctx: &Context) -> Result<()> {
    let root = aax_sdk_root(ctx)?;
    ensure_exists(&root.join("Interfaces").join("AAX.h"), "AAX SDK")
}

fn aax_sdk_root(ctx: &Context) -> Result<PathBuf> {
    if let Some(root) = env::var_os("AAX_SDK_ROOT").map(PathBuf::from) {
        // clap-wrapper evaluates AAX_SDK_ROOT inside its CMake project, so a relative
        // path would be resolved against clap_wrapper_builder rather than this repo.
        // Normalize here so CI and local shells can both use repo-relative paths.
        return Ok(if root.is_absolute() {
            root
        } else {
            ctx.root.join(root)
        });
    }

    // Local Avid downloads are commonly unpacked under Downloads. Environment
    // variables remain the deterministic CI path; this fallback keeps first-run
    // developer validation from requiring shell-profile edits.
    let downloads = home_dir()?.join("Downloads");
    for name in ["aax-sdk-2-9-0", "aax-sdk-2-8-1"] {
        let root = downloads.join(name);
        if root.join("Interfaces").join("AAX.h").exists() {
            return Ok(root);
        }
    }

    Err("AAX SDK not found. Set AAX_SDK_ROOT to the extracted AAX SDK root directory.".into())
}

fn executable_name(name: &str, platform: Platform) -> String {
    if platform == Platform::Windows {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn print_outputs(ctx: &Context, profile: BuildProfile, targets: &[Target]) {
    for target in targets {
        match target {
            Target::Clap => println!("CLAP: {}", ctx.clap_bundle(profile).display()),
            Target::Vst3 => println!("VST3: {}", ctx.vst3_bundle(profile).display()),
            Target::Aax => println!("AAX: {}", ctx.aax_bundle(profile).display()),
            Target::Au => {
                for artifact in ctx.au_bundles(profile) {
                    println!("AU: {}", artifact.display());
                }
            }
            Target::Standalone => {
                for artifact in ctx.standalone_artifacts(profile) {
                    println!("Standalone: {}", artifact.display());
                }
            }
        }
    }
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

fn codesign(path: &Path) -> Result<()> {
    run(Command::new("codesign")
        .arg("--force")
        .arg("--sign")
        .arg("-")
        .arg("--timestamp=none")
        .arg(path))?;
    Ok(())
}

fn codesign_nested_macos_bundle(bundle: &Path) -> Result<()> {
    let plugins_dir = bundle.join("Contents").join("PlugIns");
    if plugins_dir.exists() {
        for entry in fs::read_dir(&plugins_dir)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "clap")
            {
                codesign(&path)?;
            }
        }
    }
    codesign(bundle)?;
    Ok(())
}
