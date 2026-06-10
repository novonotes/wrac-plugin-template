use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Result;
use crate::cli::{InstallScope, UninstallScope};
use crate::context::Context;
use crate::metadata::PluginMetadata;
use crate::profile::BuildProfile;
use crate::targets::{Platform, PluginFormat, PluginTarget, Target, ValidateTarget};
use crate::util::{
    common_program_files, copy_path, ensure_exists, env_value_or, home_dir, local_app_data, on_off,
    remove_if_exists, run, run_output, run_with_optional_xcbeautify,
};
use crate::validation::validate_wrac_rules;

const CLAP_VALIDATOR_VERSION: &str = "0.3.2";
// Keep the local AAX contract explicit instead of delegating to the validator's
// broad `runtests` collection. `runtests` includes hardware/DSP and page-table
// XML coverage that this source-built native template does not generate, so CI
// should fail only on the concrete native tests that the generated bundle is
// expected to pass. The skipped IDs are still logged at runtime to make that
// boundary visible in CI without turning docs/aax.md into a validation manual.
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
const AAX_VALIDATOR_TIMEOUT_SECS: u64 = 15 * 60;

pub(crate) fn build_gui(ctx: &Context) -> Result<()> {
    println!("Building GUI...");
    let package_json = ctx.gui_dir().join("package.json");
    if !package_json.exists() {
        println!("No src-gui/package.json found; skipping GUI build.");
        return Ok(());
    }
    if !has_package_script(&package_json, "build")? {
        println!(
            "No build script found in {}; skipping GUI build.",
            package_json.display()
        );
        return Ok(());
    }
    let package = read_package_json(&package_json)?;
    if !is_pnpm_workspace(ctx) {
        // Standalone template projects keep the frontend package under src-gui
        // without a repository-level package.json.
        run(Command::new(npm_command(ctx.platform))
            .arg("install")
            .current_dir(ctx.gui_dir()))?;
        run(Command::new(npm_command(ctx.platform))
            .args(["run", "build"])
            .current_dir(ctx.gui_dir()))?;
        return Ok(());
    }

    let package_name = package_name(&package, &package_json)?;
    let dependency_names = workspace_dependency_names(&package);
    // build.rs embeds src-gui/dist into the plugin binary. Workspace packages such as
    // @novonotes/webview-bridge also need their dist before the GUI typecheck runs.
    run(Command::new(pnpm_command(ctx.platform))
        .arg("install")
        .current_dir(&ctx.root))?;
    for dependency_name in dependency_names {
        run(Command::new(pnpm_command(ctx.platform))
            .args(["--filter", &dependency_name, "run", "--if-present", "build"])
            .current_dir(&ctx.root))?;
    }
    run(Command::new(pnpm_command(ctx.platform))
        .args(["--filter", &package_name, "run", "build"])
        .current_dir(&ctx.root))?;
    Ok(())
}

fn is_pnpm_workspace(ctx: &Context) -> bool {
    ctx.root.join("package.json").exists() && ctx.root.join("pnpm-workspace.yaml").exists()
}

fn has_package_script(package_json: &Path, script: &str) -> Result<bool> {
    let json = read_package_json(package_json)?;
    Ok(json
        .get("scripts")
        .and_then(Value::as_object)
        .and_then(|scripts| scripts.get(script))
        .and_then(Value::as_str)
        .is_some())
}

fn read_package_json(package_json: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(package_json)?)?)
}

fn package_name(json: &Value, package_json: &Path) -> Result<String> {
    json.get("name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("package name not found in {}", package_json.display()).into())
}

fn workspace_dependency_names(json: &Value) -> Vec<String> {
    json.get("dependencies")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|dependencies| dependencies.iter())
        .filter(|(_, version)| {
            version
                .as_str()
                .is_some_and(|version| version.starts_with("workspace:"))
        })
        .map(|(name, _)| name.to_owned())
        .collect()
}

fn pnpm_command(platform: Platform) -> &'static str {
    if platform == Platform::Windows {
        "pnpm.cmd"
    } else {
        "pnpm"
    }
}

fn npm_command(platform: Platform) -> &'static str {
    if platform == Platform::Windows {
        "npm.cmd"
    } else {
        "npm"
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RustPluginBuild {
    Default,
    Standalone,
}

impl RustPluginBuild {
    pub(crate) fn label(self) -> &'static str {
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

pub(crate) fn build_rust_plugin(
    ctx: &Context,
    profile: BuildProfile,
    build: RustPluginBuild,
) -> Result<()> {
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

pub(crate) fn package_clap(ctx: &Context, profile: BuildProfile) -> Result<()> {
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
pub(crate) enum WrapperBuild {
    // VST3 and AU share the same private-SDK-free wrapper configure. AAX is
    // deliberately separate so VST3/AU builds do not require AAX_SDK_ROOT.
    Plugins { vst3: bool, au: bool },
    Aax,
    Standalone,
}

impl WrapperBuild {
    pub(crate) fn purpose(self) -> &'static str {
        match self {
            Self::Plugins { .. } => "wrap-plugins",
            Self::Aax => "wrap-aax",
            Self::Standalone => "standalone",
        }
    }

    fn target_name_base(self, ctx: &Context) -> String {
        // clap_wrapper_builder derives its CMake target names from this cache
        // variable. Keep xtask's derivation in one place so DAG task names can
        // map predictably to concrete CMake targets.
        format!(
            "{}_{}",
            ctx.metadata.package_name,
            self.purpose().replace('-', "_")
        )
    }

    fn rust_build(self) -> RustPluginBuild {
        match self {
            Self::Plugins { .. } | Self::Aax => RustPluginBuild::Default,
            Self::Standalone => RustPluginBuild::Standalone,
        }
    }
}

pub(crate) fn configure_wrapper(
    ctx: &Context,
    profile: BuildProfile,
    build: WrapperBuild,
) -> Result<()> {
    // Keep SDK/submodule diagnostics close to the configure task even when the
    // DAG was created by install, validate, or launch. Checking before the CMake
    // stamp shortcut avoids silently relying on a stale cache after an SDK
    // directory was removed or a submodule was never initialized on this machine.
    ensure_common_wrapper_inputs(ctx)?;
    match build {
        WrapperBuild::Plugins { vst3, au } => {
            if vst3 {
                ensure_vst3_sdk_input(ctx)?;
            }
            if au {
                ensure_au_sdk_input(ctx)?;
            }
        }
        WrapperBuild::Aax => ensure_aax_sdk_input(ctx)?,
        WrapperBuild::Standalone => {}
    }

    let rust_build = build.rust_build();
    let static_library = rust_build.static_library(ctx, profile);
    ensure_exists(&static_library, "static plugin library")?;

    let build_dir = ctx.cmake_dir(build.purpose(), profile);
    let stage_dir = match build {
        WrapperBuild::Plugins { .. } | WrapperBuild::Aax => ctx.plugins_dir(profile),
        WrapperBuild::Standalone => ctx.standalone_dir(profile),
    };
    fs::create_dir_all(&stage_dir)?;

    let mut args = Vec::<OsString>::new();
    push_cmake_arg(&mut args, "-S");
    args.push(ctx.wrapper_dir.as_os_str().to_owned());
    push_cmake_arg(&mut args, "-B");
    args.push(build_dir.as_os_str().to_owned());
    // Build the wrapper directly from the Rust staticlib. Locating a pre-built CLAP bundle
    // instead would tie reproducibility to clean/install ordering and stale artifacts.
    // Pass the same stage path that xtask uses for downstream validation checks.
    push_cmake_arg(
        &mut args,
        format!(
            "-DCLAP_WRAPPER_BUILDER_TARGET_LIB={}",
            static_library.display()
        ),
    );
    push_cmake_arg(
        &mut args,
        format!(
            "-DCLAP_WRAPPER_BUILDER_OUTPUT_NAME={}",
            ctx.metadata.bundle_name
        ),
    );
    push_cmake_arg(
        &mut args,
        format!(
            "-DCLAP_WRAPPER_BUILDER_TARGET_NAME={}_{}",
            ctx.metadata.package_name,
            build.purpose().replace('-', "_")
        ),
    );
    push_cmake_arg(
        &mut args,
        format!("-DCLAP_WRAPPER_BUILDER_STAGE_DIR={}", stage_dir.display()),
    );
    push_cmake_arg(
        &mut args,
        format!(
            "-DCLAP_WRAPPER_BUILDER_BUNDLE_VERSION={}",
            ctx.metadata.version
        ),
    );
    push_cmake_arg(
        &mut args,
        format!("-DCMAKE_BUILD_TYPE={}", profile.cmake_config()),
    );
    push_cmake_arg(&mut args, "-DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=OFF");
    push_cmake_arg(&mut args, "-DCLAP_WRAPPER_CXX_STANDARD=23");
    add_wrapper_product_args(ctx, &mut args, build);

    match build {
        WrapperBuild::Plugins { vst3, au } => {
            push_cmake_arg(
                &mut args,
                format!("-DCLAP_WRAPPER_BUILDER_BUILD_VST3={}", on_off(vst3)),
            );
            push_cmake_arg(
                &mut args,
                format!("-DCLAP_WRAPPER_BUILDER_BUILD_AUV2={}", on_off(au)),
            );
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_AAX=OFF");
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=OFF");
        }
        WrapperBuild::Aax => {
            // AAX target creation happens during CMake configure and requires
            // the Avid SDK root. Keeping this in wrap-aax-* avoids rewriting the
            // VST3/AU CMake cache when users switch between targets.
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_VST3=OFF");
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_AUV2=OFF");
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_AAX=ON");
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=OFF");
            push_cmake_arg(
                &mut args,
                format!("-DAAX_SDK_ROOT={}", aax_sdk_root(ctx)?.display()),
            );
        }
        WrapperBuild::Standalone => {
            // standalone requires additional app-side dependencies that plugin wrappers do not.
            // Delegate fetching to clap-wrapper's own download logic while keeping downloads
            // disabled for plugin wrapper builds.
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_VST3=OFF");
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_AUV2=OFF");
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_AAX=OFF");
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=ON");
            push_cmake_arg(&mut args, "-DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=ON");
        }
    }

    if ctx.platform == Platform::Macos {
        let macos_deployment_target = env_value_or("MACOSX_DEPLOYMENT_TARGET", "11.0");
        push_cmake_arg(
            &mut args,
            format!("-DCMAKE_OSX_DEPLOYMENT_TARGET={macos_deployment_target}"),
        );
        // AUv2 uses 4-character type/manufacturer/subtype codes as the host discovery key.
        // Drive them from the template's constants rather than inferring from the Rust descriptor.
        push_cmake_arg(
            &mut args,
            format!(
                "-DAUDIOUNIT_SDK_ROOT={}",
                ctx.wrapper_dir.join("AudioUnitSDK").display()
            ),
        );
        push_cmake_arg(
            &mut args,
            format!(
                "-DCLAP_WRAPPER_AUV2_MANUFACTURER_NAME={}",
                ctx.metadata.company_name
            ),
        );
        push_cmake_arg(
            &mut args,
            format!(
                "-DCLAP_WRAPPER_AUV2_MANUFACTURER_CODE={}",
                ctx.metadata.auv2_manufacturer_code
            ),
        );
    }

    if let Some(generator) = ctx.platform.cmake_generator() {
        push_cmake_arg(&mut args, "-G");
        push_cmake_arg(&mut args, generator);
    }

    if cmake_configure_is_current(&build_dir, &args, &ctx.wrapper_dir)? {
        println!(
            "CMake configure is up to date for {} ({})",
            build.purpose(),
            profile.cmake_config()
        );
        return Ok(());
    }

    let mut configure = Command::new("cmake");
    configure.args(&args);
    if ctx.platform == Platform::Macos {
        configure.env(
            "MACOSX_DEPLOYMENT_TARGET",
            env_value_or("MACOSX_DEPLOYMENT_TARGET", "11.0"),
        );
    }
    run(configure.current_dir(&ctx.root))?;
    write_cmake_configure_stamp(&build_dir, &args, &ctx.wrapper_dir)?;
    Ok(())
}

pub(crate) fn build_wrapper_target(
    ctx: &Context,
    profile: BuildProfile,
    build: WrapperBuild,
    target: WrapperTarget,
) -> Result<()> {
    let build_dir = ctx.cmake_dir(build.purpose(), profile);
    for cmake_target in cmake_wrapper_targets(ctx, build, target) {
        // Build the concrete CMake target for this DAG node instead of ALL_BUILD.
        // That keeps dry-run output aligned with the actual work and lets
        // independent format tasks fail or pass separately.
        let mut build_cmd = Command::new("cmake");
        build_cmd
            .arg("--build")
            .arg(&build_dir)
            .arg("--target")
            .arg(cmake_target)
            .arg("--config")
            .arg(profile.cmake_config());

        if ctx.platform == Platform::Macos {
            // AudioUnitSDK emits GNU statement-expression and narrowing warnings in Xcode.
            // Suppress them here so template users are not pulled into wrapper SDK warnings.
            build_cmd.args([
                "--",
                "-quiet",
                "OTHER_CPLUSPLUSFLAGS=$(inherited) -Wno-unknown-warning-option -Wno-gnu-statement-expression-from-macro-expansion -Wno-shorten-64-to-32 -Wno-perf-constraint-implies-noexcept",
            ]);
        }

        let build_cmd = build_cmd.current_dir(&ctx.root);
        if ctx.platform == Platform::Macos {
            run_with_optional_xcbeautify(build_cmd)?;
        } else {
            run(build_cmd)?;
        }
    }

    match target {
        WrapperTarget::Vst3 => {
            ensure_exists(&ctx.vst3_bundle(profile), "VST3 artifact")?;
            if ctx.platform == Platform::Macos {
                // macOS hosts may reject unsigned bundles; apply an ad-hoc signature for development.
                codesign_nested_macos_bundle(&ctx.vst3_bundle(profile))?;
            }
        }
        WrapperTarget::Au => {
            for artifact in ctx.au_bundles(profile) {
                ensure_exists(&artifact, "AU artifact")?;
                // AU components are loaded via AudioComponentRegistrar, so they must be signed even for local builds.
                codesign_nested_macos_bundle(&artifact)?;
            }
        }
        WrapperTarget::Aax => {
            ensure_exists(&ctx.aax_bundle(profile), "AAX artifact")?;
            if ctx.platform == Platform::Macos {
                // AAX developer validation loads the bundle directly, so keep the
                // local artifact ad-hoc signed before the validator sees it.
                codesign_nested_macos_bundle(&ctx.aax_bundle(profile))?;
            }
        }
        WrapperTarget::Standalone => {
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum WrapperTarget {
    Vst3,
    Au,
    Aax,
    Standalone,
}

fn cmake_wrapper_targets(ctx: &Context, build: WrapperBuild, target: WrapperTarget) -> Vec<String> {
    let base = build.target_name_base(ctx);
    match target {
        WrapperTarget::Vst3 => vec![format!("{base}_vst3")],
        WrapperTarget::Aax => vec![format!("{base}_aax")],
        WrapperTarget::Au => vec![format!("{base}_auv2")],
        WrapperTarget::Standalone => ctx
            .metadata
            .plugins
            .iter()
            .enumerate()
            .map(|(index, _)| format!("{base}_product_{index}_standalone"))
            .collect::<Vec<_>>(),
    }
}

fn push_cmake_arg(args: &mut Vec<OsString>, arg: impl Into<OsString>) {
    args.push(arg.into());
}

fn cmake_configure_stamp_path(build_dir: &Path) -> PathBuf {
    build_dir.join(".wrac-configure-args")
}

fn cmake_configure_stamp(args: &[OsString], wrapper_dir: &Path) -> Result<String> {
    let mut lines = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    for relative_path in [
        "CMakeLists.txt",
        "clap-wrapper/cmake/make_clapfirst.cmake",
        "clap-wrapper/cmake/wrap_auv2.cmake",
    ] {
        let path = wrapper_dir.join(relative_path);
        let modified = fs::metadata(&path)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        lines.push(format!("cmake-input:{relative_path}:{modified}"));
    }
    Ok(lines.join("\n"))
}

fn cmake_configure_is_current(
    build_dir: &Path,
    args: &[OsString],
    wrapper_dir: &Path,
) -> Result<bool> {
    let cache = build_dir.join("CMakeCache.txt");
    let stamp_path = cmake_configure_stamp_path(build_dir);
    if !cache.exists() || !stamp_path.exists() {
        return Ok(false);
    }

    // Running CMake configure on every xtask invocation rewrites generated
    // wrapper entry files, which then forces Xcode/MSBuild to relink even when
    // the selected CMake target is unchanged. The stamp tracks xtask-owned
    // configure inputs plus the wrapper CMake files that define the generated
    // target graph.
    Ok(fs::read_to_string(stamp_path)? == cmake_configure_stamp(args, wrapper_dir)?)
}

fn write_cmake_configure_stamp(
    build_dir: &Path,
    args: &[OsString],
    wrapper_dir: &Path,
) -> Result<()> {
    fs::write(
        cmake_configure_stamp_path(build_dir),
        cmake_configure_stamp(args, wrapper_dir)?,
    )?;
    Ok(())
}

fn add_wrapper_product_args(ctx: &Context, args: &mut Vec<OsString>, build: WrapperBuild) {
    push_cmake_arg(
        args,
        format!(
            "-DCLAP_WRAPPER_BUILDER_PRODUCT_COUNT={}",
            ctx.metadata.plugins.len()
        ),
    );
    for (index, plugin) in ctx.metadata.plugins.iter().enumerate() {
        match build {
            WrapperBuild::Plugins { au: true, .. } => {
                // CLAP/VST3/AAX read product descriptors from the Rust plugin factory.
                // AUv2 cannot, so only AUv2 builds need per-product output and
                // four-character AudioComponent identity values from xtask.
                push_cmake_arg(
                    args,
                    format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_OUTPUT_NAME={}",
                        plugin.plugin_name
                    ),
                );
                push_cmake_arg(
                    args,
                    format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_AUV2_TYPE={}",
                        plugin.auv2_type
                    ),
                );
                push_cmake_arg(
                    args,
                    format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_AUV2_SUBTYPE={}",
                        plugin.auv2_subtype
                    ),
                );
            }
            WrapperBuild::Standalone => {
                // Each standalone app embeds the product ID it should host at
                // compile time; passing all standalone metadata keeps CMake from
                // choosing an implicit primary product.
                push_cmake_arg(
                    args,
                    format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_OUTPUT_NAME={}",
                        plugin.plugin_name
                    ),
                );
                push_cmake_arg(
                    args,
                    format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_PLUGIN_ID={}",
                        plugin.plugin_id
                    ),
                );
                push_cmake_arg(
                    args,
                    format!(
                        "-DCLAP_WRAPPER_BUILDER_PRODUCT_{index}_STANDALONE_NAME={}",
                        plugin.standalone_name
                    ),
                );
            }
            WrapperBuild::Plugins { au: false, .. } | WrapperBuild::Aax => {}
        }
    }
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

pub(crate) fn ensure_launch_target_exists(ctx: &Context, plugin_id: Option<&str>) -> Result<()> {
    // Launch has to build the standalone app before opening it, but an invalid
    // product selection is independent of build artifacts. Check it upfront so
    // typos in --plugin-id do not trigger a full standalone build.
    standalone_plugin_to_launch(ctx, plugin_id).map(|_| ())
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
    Ok((removed, missing))
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

pub(crate) fn install_artifact(artifact: &Path, destination_dir: &Path) -> Result<()> {
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

pub(crate) fn validate_wrac_rules_for_targets(
    ctx: &Context,
    profile: BuildProfile,
    targets: &[ValidateTarget],
) -> Result<()> {
    validate_wrac_rules(ctx, profile, targets)
}

pub(crate) fn validate_plugin_target(
    ctx: &Context,
    profile: BuildProfile,
    target: ValidateTarget,
) -> Result<()> {
    match target {
        ValidateTarget::Clap => {
            let clap = ctx.clap_bundle(profile);
            ensure_exists(&clap, "CLAP artifact")?;
            let validator = ensure_clap_validator(ctx)?;
            let mut command = Command::new(validator);
            command
                .env("WRAC_PLUGIN_VALIDATOR", "1")
                .arg("validate")
                .arg(&clap)
                .arg("--only-failed");
            if let Some(filter) = ctx
                .metadata
                .validation
                .clap_validator
                .skip_test_filter
                .as_deref()
            {
                println!(
                    "CLAP validator skip filter: {filter} ({})",
                    ctx.metadata
                        .validation
                        .clap_validator
                        .skip_reason
                        .as_deref()
                        .unwrap_or("no reason provided")
                );
                command
                    .arg("--test-filter")
                    .arg(filter)
                    .arg("--invert-filter");
            }
            run(command.current_dir(&ctx.root))?;
        }
        ValidateTarget::Vst3 => {
            let vst3 = ctx.vst3_bundle(profile);
            ensure_exists(&vst3, "VST3 artifact")?;
            let validator = ensure_vst3_validator(ctx)?;
            let output = run_output(
                Command::new(validator)
                    .env("WRAC_PLUGIN_VALIDATOR", "1")
                    .arg(&vst3)
                    .current_dir(&ctx.root),
            )?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            print!("{stdout}");
            eprint!("{stderr}");
            // The VST3 validator checks format behavior and prints the host-visible class IDs
            // while scanning the built bundle. Reusing that output keeps the artifact-boundary
            // byte-order check without running Steinberg's moduleinfotool, which can keep WRAC
            // Windows GUI/runtime dependencies alive after validation and hang CI.
            validate_vst3_component_ids(ctx, &vst3, &stdout, &stderr)?;
        }
        ValidateTarget::Au => {
            ensure_no_system_au_conflict(ctx)?;
            for artifact in ctx.au_bundles(profile) {
                ensure_exists(&artifact, "AU artifact")?;
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
        ValidateTarget::Aax => {
            let aax = ctx.aax_bundle(profile);
            ensure_exists(&aax, "AAX artifact")?;
            run_aax_validator(ctx, &aax)?;
        }
    }
    Ok(())
}

fn validate_vst3_component_ids(
    ctx: &Context,
    vst3: &Path,
    stdout: &str,
    stderr: &str,
) -> Result<()> {
    let actual = parse_vst3_validator_cids(stdout)
        .into_iter()
        .chain(parse_vst3_validator_cids(stderr))
        .collect::<Vec<_>>();
    let expected = ctx
        .metadata
        .plugins
        .iter()
        .map(|plugin| normalize_vst3_cid(&plugin.vst3_component_id))
        .collect::<Vec<_>>();

    if actual != expected {
        return Err(format!(
            "VST3 component ID mismatch for {}: metadata={expected:?}, validator={actual:?}",
            vst3.display()
        )
        .into());
    }

    println!("VST3 component IDs match package.metadata.wrac.plugins.vst3_component_id");
    Ok(())
}

fn parse_vst3_validator_cids(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.trim_start().split_once("cid = "))
        .map(|(_, cid)| normalize_vst3_cid(cid))
        .collect()
}

fn normalize_vst3_cid(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .flat_map(char::to_uppercase)
        .collect()
}

fn run_aax_validator(ctx: &Context, aax: &Path) -> Result<()> {
    let results_dir = ctx.wrac_dir().join("validation").join("aax");
    // A fresh directory prevents a previous pass result from masking a missing
    // validator output if DTT exits early or changes a result reference.
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

    run_aax_validator_dtt(ctx, &aax, &results_dir)?;

    assert_aax_validator_results(&results_dir)
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
        let log_dir = test_dir.join("logs");
        fs::create_dir_all(&test_dir)?;
        fs::create_dir_all(&log_dir)?;

        // Avid ships DTT as the automatable scripting layer for DigiShell. Use the
        // bundled ValidatorRunAllTests script instead of scripting DigiShell stdin
        // directly because Windows hosted CI can launch DigiShell while dropping
        // scripted stdin. The script expects a search directory for `findaaxplugins`;
        // passing the bundle path itself gives a different result shape on some packages.
        let child = Command::new(&dtt)
            .arg("--script")
            .arg("ValidatorRunAllTests")
            .arg("--no_pref_delete")
            .arg("--no_move_options")
            .arg("--disable_digitrace")
            .arg("--verbose")
            .arg("--logdir")
            .arg(&log_dir)
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
        let output = wait_for_aax_validator_process(child, aax_validator_timeout()?)?;
        let stdout_path = test_dir.join("dtt-stdout.log");
        let stderr_path = test_dir.join("dtt-stderr.log");
        fs::write(&stdout_path, &output.stdout)?;
        fs::write(&stderr_path, &output.stderr)?;

        let result_path = aax_validator_result_path(results_dir, index, test_id);
        let dtt_result = find_aax_validator_dtt_result(&test_dir, test_id)?;
        // DTT writes result files with connection-specific suffixes. Copy each one to
        // a deterministic per-test path so CI artifacts and final pass/fail checks do
        // not depend on DigiShell connection IDs.
        fs::copy(&dtt_result, &result_path).map_err(|err| {
            format!(
                "failed to copy AAX validator result {} to {}: {err}",
                dtt_result.display(),
                result_path.display()
            )
        })?;

        if !output.status.success() {
            print_aax_validator_output(&output.stdout, &output.stderr);
            print_aax_validator_result(&result_path)?;
            print_aax_validator_dtt_logs(&log_dir)?;
            return Err(format!(
                "AAX validator/DTT failed while running {test_id}; see {} and {}",
                stdout_path.display(),
                result_path.display()
            )
            .into());
        }
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
        // DTT's process exit is not enough for reviewable validation: the official
        // JSON result records the test ID and validator result_status that CI logs
        // and artifacts can be audited against.
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

fn wait_for_aax_validator_process(mut child: Child, timeout: Duration) -> Result<Output> {
    let started_at = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if started_at.elapsed() >= timeout {
            // Keep timeouts outside `run()` so failed DTT processes still have their
            // stdout/stderr printed. That output is usually the only clue when the
            // validator hangs while loading a bundle.
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

fn aax_validator_timeout() -> Result<Duration> {
    let seconds = match env::var("AAX_VALIDATOR_TIMEOUT_SECS") {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|err| format!("failed to parse AAX_VALIDATOR_TIMEOUT_SECS={value}: {err}"))?,
        Err(env::VarError::NotPresent) => AAX_VALIDATOR_TIMEOUT_SECS,
        Err(err) => {
            return Err(format!("failed to read AAX_VALIDATOR_TIMEOUT_SECS: {err}").into());
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

fn print_aax_validator_result(path: &Path) -> Result<()> {
    let content = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read AAX validator result {}: {err}",
            path.display()
        )
    })?;
    println!(
        "========== AAX validator result ({}) ==========",
        path.display()
    );
    println!("{content}");
    Ok(())
}

fn print_aax_validator_dtt_logs(log_dir: &Path) -> Result<()> {
    for path in collect_files(log_dir)? {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.ends_with(".txt") {
            continue;
        }
        let content = fs::read_to_string(&path).map_err(|err| {
            format!(
                "failed to read AAX validator DTT log {}: {err}",
                path.display()
            )
        })?;
        println!(
            "========== AAX validator DTT log ({}) ==========",
            path.display()
        );
        let max_len = 64 * 1024;
        if content.len() > max_len {
            let split = content
                .char_indices()
                .map(|(index, _)| index)
                .take_while(|index| *index <= max_len)
                .last()
                .unwrap_or(0);
            println!("{}", &content[..split]);
            println!(
                "... truncated {} bytes from {}",
                content.len() - split,
                path.display()
            );
        } else {
            println!("{content}");
        }
    }
    Ok(())
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    collect_files_inner(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files_inner(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files_inner(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
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

fn ensure_aax_validator_dtt(ctx: &Context) -> Result<PathBuf> {
    let root = aax_validator_dsh_root(ctx)?;
    let dtt = aax_validator_dtt_runner(&root, ctx.platform)?;
    ensure_exists(&dtt, "AAX validator DTT runner")?;
    if ctx.platform == Platform::Windows {
        normalize_windows_aax_validator_dtt_config(&root)?;
    }
    if ctx.platform == Platform::Macos {
        // Browser-downloaded Avid archives may carry quarantine attributes, and
        // `run_test.command` is not guaranteed to preserve its executable bit after
        // extraction. Normalize both here so first-run local validation behaves like CI.
        let _ = Command::new("xattr")
            .args(["-dr", "com.apple.quarantine"])
            .arg(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        run(Command::new("chmod")
            .arg("+x")
            .arg(&dtt)
            .current_dir(&ctx.root))?;
    }
    Ok(dtt)
}

fn normalize_windows_aax_validator_dtt_config(root: &Path) -> Result<()> {
    // User-supplied roots may point either at the archive root or directly at the
    // AAXValidatorResources root. Normalize every matching extracted config so both
    // layouts behave the same without asking users to repack Avid's archive.
    for candidate in [
        root.join("DigiShell")
            .join("AAXValidatorResources")
            .join("Main.valconfig"),
        root.join("AAXValidatorResources").join("Main.valconfig"),
    ] {
        if candidate.exists() {
            normalize_windows_aax_validator_main_config(&candidate)?;
        }
    }
    Ok(())
}

fn normalize_windows_aax_validator_main_config(path: &Path) -> Result<()> {
    let content = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read AAX validator config {}: {err}",
            path.display()
        )
    })?;
    // Avid's Windows 2024.6 validator package uses POSIX single quotes for the
    // DTT process arguments in Main.valconfig. `cmd.exe` passes those quotes
    // through literally, so DTT does not receive `bundle_path` and its helper
    // scripts fall back to sample plug-in names such as `Trim.aaxplugin`. Patch
    // only the extracted target/ copy and use the same escaped double-quote style
    // already used by the validator's other Windows process definitions.
    let normalized = content
        .replace(
            "elem: \"\\'bundle_path=$AAXVAL_PARAM_AAXPLUGIN$\\'\"",
            "elem: \"\\\"bundle_path=$AAXVAL_PARAM_AAXPLUGIN$\\\"\"",
        )
        .replace(
            "elem: \"\\'uniq_id=$AAXVAL_PARAM_UNIQ_ID$\\'\"",
            "elem: \"\\\"uniq_id=$AAXVAL_PARAM_UNIQ_ID$\\\"\"",
        );
    if normalized != content {
        fs::write(path, normalized).map_err(|err| {
            format!(
                "failed to write normalized AAX validator config {}: {err}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn aax_validator_dsh_root(ctx: &Context) -> Result<PathBuf> {
    let archive = aax_validator_dsh_archive(ctx)?;
    let extracted_root = ctx.target_dir.join("tools").join("aax-validator-dsh");
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

fn aax_validator_dsh_archive(ctx: &Context) -> Result<PathBuf> {
    let Some(archive) = env_path(ctx, "AAX_VALIDATOR_DSH_ARCHIVE")? else {
        return Err(
            "AAX validator/DSH archive not found. Set AAX_VALIDATOR_DSH_ARCHIVE in .env or the process environment."
                .into(),
        );
    };
    ensure_exists(&archive, "AAX validator/DSH archive")?;
    Ok(archive)
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
    let system_au =
        Path::new("/Library/Audio/Plug-Ins/Components").join(ctx.metadata.au_bundle_name());
    if system_au.exists() {
        return Err(format!(
            "system-wide AU already exists at {}. auval may validate that copy instead of the freshly built user-local AU. Remove the system-wide component and run validation again.",
            system_au.display()
        )
        .into());
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
    ensure_exists(&root.join("Interfaces").join("AAX.h"), "AAX SDK")
}

fn aax_sdk_root(ctx: &Context) -> Result<PathBuf> {
    if let Some(root) = env_path(ctx, "AAX_SDK_ROOT")? {
        // clap-wrapper evaluates AAX_SDK_ROOT inside its CMake project, so a relative
        // path would be resolved against clap_wrapper_builder rather than this repo.
        // Resolve relative .env and CI paths from the repository root instead.
        ensure_exists(&root.join("Interfaces").join("AAX.h"), "AAX SDK")?;
        return Ok(root);
    }

    Err("AAX SDK not found. Set AAX_SDK_ROOT in .env or the process environment.".into())
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

pub(crate) fn print_outputs(ctx: &Context, profile: BuildProfile, targets: &[Target]) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
