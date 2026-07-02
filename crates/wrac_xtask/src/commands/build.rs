use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::context::Context;
use crate::metadata::PluginProductMetadata;
use crate::profile::BuildProfile;
use crate::targets::Platform;
use crate::util::{
    ensure_exists, env_value_or, on_off, print_section, print_skip, remove_if_exists,
    run_output_with_language, run_with_language, run_with_optional_xcbeautify_language,
};
use crate::{Result, XtaskOutputLanguage};

use super::plugin_resources::{
    copy_resource_directory, ensure_wrapper_resources_supported, prepare_plugin_resource_dir,
};
use super::{
    aax_sdk_root, codesign, codesign_nested_macos_bundle, ensure_aax_sdk_input,
    ensure_au_sdk_input, ensure_common_wrapper_inputs, ensure_vst3_sdk_input,
    macos_clap_info_plist,
};

pub(crate) fn build_gui(ctx: &Context) -> Result<()> {
    print_section(ctx.output_language, "Build GUI", "GUI ビルド");
    let package_json = ctx.gui_dir().join("package.json");
    if !package_json.exists() {
        print_skip(
            ctx.output_language,
            "No src-gui/package.json found; skipping GUI build.",
            "src-gui/package.json がないため GUI ビルドをスキップ",
        );
        return Ok(());
    }
    if !has_package_script(&package_json, "build")? {
        print_skip(
            ctx.output_language,
            &format!(
                "No build script found in {}; skipping GUI build.",
                package_json.display()
            ),
            &format!(
                "{} に build script がないため GUI ビルドをスキップ",
                package_json.display()
            ),
        );
        return Ok(());
    }
    let package = read_package_json(&package_json)?;
    if !is_pnpm_workspace(ctx) {
        // Standalone template projects keep the frontend package under src-gui
        // without a repository-level package.json.
        run_with_language(
            Command::new(command_for_platform(ctx.platform, "npm"))
                .arg("install")
                .current_dir(ctx.gui_dir()),
            ctx.output_language,
        )?;
        run_with_language(
            Command::new(command_for_platform(ctx.platform, "npm"))
                .args(["run", "build"])
                .current_dir(ctx.gui_dir()),
            ctx.output_language,
        )?;
        return Ok(());
    }

    let package_name = package_name(&package, &package_json)?;
    let dependency_names = workspace_dependency_names(&package);
    // build.rs embeds src-gui/dist into the plugin binary. Workspace packages such as
    // @novonotes/webview-bridge also need their dist before the GUI typecheck runs.
    run_with_language(
        Command::new(command_for_platform(ctx.platform, "pnpm"))
            .arg("install")
            .current_dir(&ctx.root),
        ctx.output_language,
    )?;
    for dependency_name in dependency_names {
        run_with_language(
            Command::new(command_for_platform(ctx.platform, "pnpm"))
                .args(["--filter", &dependency_name, "run", "--if-present", "build"])
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
    }
    run_with_language(
        Command::new(command_for_platform(ctx.platform, "pnpm"))
            .args(["--filter", &package_name, "run", "build"])
            .current_dir(&ctx.root),
        ctx.output_language,
    )?;
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

fn command_for_platform(platform: Platform, command: &'static str) -> OsString {
    if platform == Platform::Windows {
        let candidates = [
            format!("{command}.cmd"),
            format!("{command}.exe"),
            command.to_string(),
        ];
        for candidate in candidates {
            let candidate = OsString::from(candidate);
            if command_exists_on_path(&candidate) {
                return candidate;
            }
        }
    }
    OsString::from(command)
}

fn command_exists_on_path(command: &OsStr) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    command_exists_in_paths(command, env::split_paths(&paths))
}

pub(super) fn command_exists_in_paths(
    command: &OsStr,
    paths: impl IntoIterator<Item = PathBuf>,
) -> bool {
    paths.into_iter().any(|path| path.join(command).is_file())
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
    print_section(
        ctx.output_language,
        &format!("Build Rust plugin ({})", build.label()),
        &format!("Rust plugin ビルド ({})", build.label()),
    );
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
    run_with_language(command.current_dir(&ctx.root), ctx.output_language)?;

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
    print_section(ctx.output_language, "Package CLAP", "CLAP packaging");
    let bundle = ctx.clap_bundle(profile);
    remove_if_exists(&bundle)?;
    fs::create_dir_all(ctx.plugins_dir(profile))?;
    let resource_dir = prepare_plugin_resource_dir(ctx)?;

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
            if let Some(resource_dir) = &resource_dir {
                copy_resource_directory(resource_dir, &contents.join("Resources"))?;
            }
            run_with_language(
                Command::new("install_name_tool")
                    .arg("-id")
                    .arg(format!("@loader_path/{}", ctx.metadata.bundle_name))
                    .arg(macos.join(&ctx.metadata.bundle_name))
                    .current_dir(&ctx.root),
                ctx.output_language,
            )?;
            codesign(&bundle, ctx.output_language)?;
        }
        Platform::Windows | Platform::Linux => {
            // On Windows/Linux the CLAP artifact is a dynamic library with the .clap extension.
            // Skipping the bundle structure keeps it compatible with each OS's existing host scan conventions.
            fs::copy(ctx.dynamic_library(profile), &bundle)?;
            if let (Some(resource_dir), Some(parent)) = (&resource_dir, bundle.parent()) {
                copy_resource_directory(resource_dir, parent)?;
            }
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
    let resource_dir = prepare_plugin_resource_dir(ctx)?;
    ensure_wrapper_resources_supported(ctx, build, resource_dir.as_deref())?;

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
    if let Some(resource_dir) = &resource_dir {
        push_cmake_arg(
            &mut args,
            format!(
                "-DCLAP_WRAPPER_BUILDER_RESOURCE_DIRECTORY={}",
                resource_dir.display()
            ),
        );
    }
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

    if let Some(generator) = cmake_generator(ctx.platform)? {
        push_cmake_arg(&mut args, "-G");
        push_cmake_arg(&mut args, generator);
    }

    if cmake_configure_is_current(&build_dir, &args, &ctx.wrapper_dir)? {
        print_skip(
            ctx.output_language,
            &format!(
                "CMake configure is up to date for {} ({})",
                build.purpose(),
                profile.cmake_config()
            ),
            &format!(
                "CMake configure は最新です: {} ({})",
                build.purpose(),
                profile.cmake_config()
            ),
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
    run_with_language(configure.current_dir(&ctx.root), ctx.output_language)?;
    write_cmake_configure_stamp(&build_dir, &args, &ctx.wrapper_dir)?;
    Ok(())
}

pub(crate) fn build_wrapper_target(
    ctx: &Context,
    profile: BuildProfile,
    build: WrapperBuild,
    target: WrapperTarget,
    standalone_plugin_id: Option<&str>,
) -> Result<()> {
    let build_dir = ctx.cmake_dir(build.purpose(), profile);
    for cmake_target in cmake_wrapper_targets(ctx, build, target, standalone_plugin_id)? {
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
            run_with_optional_xcbeautify_language(build_cmd, ctx.output_language)?;
        } else {
            run_with_language(build_cmd, ctx.output_language)?;
        }
    }

    match target {
        WrapperTarget::Vst3 => {
            ensure_exists(&ctx.vst3_bundle(profile), "VST3 artifact")?;
            if ctx.platform == Platform::Macos {
                // macOS hosts may reject unsigned bundles; apply an ad-hoc signature for development.
                codesign_nested_macos_bundle(&ctx.vst3_bundle(profile), ctx.output_language)?;
            }
        }
        WrapperTarget::Au => {
            for artifact in ctx.au_bundles(profile) {
                ensure_exists(&artifact, "AU artifact")?;
                // AU components are loaded via AudioComponentRegistrar, so they must be signed even for local builds.
                codesign_nested_macos_bundle(&artifact, ctx.output_language)?;
            }
        }
        WrapperTarget::Aax => {
            ensure_exists(&ctx.aax_bundle(profile), "AAX artifact")?;
            if ctx.platform == Platform::Macos {
                // AAX developer validation loads the bundle directly, so keep the
                // local artifact ad-hoc signed before the validator sees it.
                codesign_nested_macos_bundle(&ctx.aax_bundle(profile), ctx.output_language)?;
            }
        }
        WrapperTarget::Standalone => {
            for (_, plugin) in standalone_products(ctx, standalone_plugin_id)? {
                let artifact = ctx.standalone_artifact_for(profile, plugin);
                ensure_exists(&artifact, "standalone artifact")?;
                if ctx.platform == Platform::Macos {
                    // Apply the same Gatekeeper/loader treatment to the standalone app as to plugin bundles.
                    codesign_nested_macos_bundle(&artifact, ctx.output_language)?;
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

fn cmake_wrapper_targets(
    ctx: &Context,
    build: WrapperBuild,
    target: WrapperTarget,
    standalone_plugin_id: Option<&str>,
) -> Result<Vec<String>> {
    let base = build.target_name_base(ctx);
    Ok(match target {
        WrapperTarget::Vst3 => vec![format!("{base}_vst3")],
        WrapperTarget::Aax => vec![format!("{base}_aax")],
        WrapperTarget::Au => vec![format!("{base}_auv2")],
        WrapperTarget::Standalone => standalone_products(ctx, standalone_plugin_id)?
            .iter()
            .map(|(index, _)| format!("{base}_product_{index}_standalone"))
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn standalone_products<'a>(
    ctx: &'a Context,
    plugin_id: Option<&str>,
) -> Result<Vec<(usize, &'a PluginProductMetadata)>> {
    if let Some(plugin_id) = plugin_id {
        return ctx
            .metadata
            .plugins
            .iter()
            .enumerate()
            .find(|(_, plugin)| plugin.plugin_id == plugin_id)
            .map(|(index, plugin)| vec![(index, plugin)])
            .ok_or_else(|| format!("plugin ID not found in WRAC metadata: {plugin_id}").into());
    }
    Ok(ctx.metadata.plugins.iter().enumerate().collect())
}

fn push_cmake_arg(args: &mut Vec<OsString>, arg: impl Into<OsString>) {
    args.push(arg.into());
}

fn cmake_generator(platform: Platform) -> Result<Option<String>> {
    match platform {
        Platform::Macos => Ok(platform.cmake_generator().map(ToOwned::to_owned)),
        Platform::Windows => Ok(Some(windows_cmake_generator()?)),
        Platform::Linux => Ok(None),
    }
}

fn windows_cmake_generator() -> Result<String> {
    if let Ok(generator) = env::var("WRAC_CMAKE_GENERATOR") {
        let generator = generator.trim();
        if !generator.is_empty() {
            return Ok(generator.to_owned());
        }
    }

    ensure_visual_studio_msbuild_available()?;
    let generator = latest_cmake_visual_studio_generator()?;
    println!("  ✅ CMake generator: {generator}");
    Ok(generator)
}

fn ensure_visual_studio_msbuild_available() -> Result<()> {
    let mut vswhere = Command::new(vswhere_command());
    vswhere.args([
        "-products",
        "*",
        "-requires",
        "Microsoft.Component.MSBuild",
        "-latest",
        "-property",
        "installationPath",
    ]);
    let output = run_output_with_language(&mut vswhere, XtaskOutputLanguage::English)?;
    let installation_path = String::from_utf8(output.stdout)?;
    if installation_path.trim().is_empty() {
        return Err("Visual Studio with MSBuild was not found by vswhere".into());
    }
    Ok(())
}

fn vswhere_command() -> PathBuf {
    env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .map(|program_files_x86| {
            program_files_x86
                .join("Microsoft Visual Studio")
                .join("Installer")
                .join("vswhere.exe")
        })
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("vswhere"))
}

fn latest_cmake_visual_studio_generator() -> Result<String> {
    let output = run_output_with_language(
        Command::new("cmake").arg("--help"),
        XtaskOutputLanguage::English,
    )?;
    let help = String::from_utf8(output.stdout)?;
    select_latest_visual_studio_generator(&help)
        .ok_or_else(|| "CMake does not list any Visual Studio generator".into())
}

#[cfg(test)]
pub(super) fn cmake_help_lists_generator(help: &str, generator: &str) -> bool {
    cmake_visual_studio_generators(help)
        .into_iter()
        .any(|available| available == generator)
}

pub(super) fn select_latest_visual_studio_generator(help: &str) -> Option<String> {
    cmake_visual_studio_generators(help)
        .into_iter()
        .filter_map(|generator| {
            visual_studio_generator_version(&generator).map(|version| (version, generator))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, generator)| generator)
}

fn visual_studio_generator_version(generator: &str) -> Option<u32> {
    let mut words = generator.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some("Visual"), Some("Studio"), Some(version)) => version.parse().ok(),
        _ => None,
    }
}

pub(super) fn cmake_visual_studio_generators(help: &str) -> Vec<String> {
    help.lines()
        .filter_map(|line| {
            let line = line
                .trim_start()
                .strip_prefix("* ")
                .unwrap_or(line.trim_start());
            let (name, _) = line.split_once(" = ")?;
            let name = name.trim();
            name.starts_with("Visual Studio").then(|| name.to_owned())
        })
        .collect()
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
