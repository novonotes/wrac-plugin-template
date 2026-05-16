use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Result;
use crate::cli::{BuildArgs, InstallScope, UninstallScope};
use crate::constants::CRATE_NAME;
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

const CLAP_VALIDATOR_VERSION: &str = "0.3.2";

pub(crate) fn build(ctx: &Context, args: BuildArgs) -> Result<()> {
    let profile = BuildProfile::from_release(args.release);
    let targets = resolve_build_targets(ctx.platform, &args.target)?;

    // wrapper 系の不足は npm/cargo のビルド後に CMake エラーとして出ると原因が追いにくい。
    // 対象が wrapper を必要とする時だけ、先にサブモジュールの実体まで確認する。
    if targets.iter().any(|target| target.is_wrapper()) || targets.contains(&Target::Standalone) {
        ensure_wrapper_inputs(
            ctx,
            targets.contains(&Target::Vst3),
            targets.contains(&Target::Au),
        )?;
    }

    if args.clean {
        clean(ctx)?;
    }

    build_gui(ctx)?;

    if targets.contains(&Target::Clap) {
        build_rust_plugin(ctx, profile, RustPluginBuild::Default)?;
        package_clap(ctx, profile)?;
    }

    if ctx.platform == Platform::Macos {
        if targets.contains(&Target::Vst3) {
            build_rust_plugin(ctx, profile, RustPluginBuild::Vst3)?;
            build_wrapper_set(ctx, profile, WrapperBuild::Vst3)?;
        }
        if targets.contains(&Target::Au) {
            build_rust_plugin(ctx, profile, RustPluginBuild::Au)?;
            build_wrapper_set(ctx, profile, WrapperBuild::Au)?;
        }
    } else if targets.iter().any(|target| target.is_wrapper()) {
        build_rust_plugin(ctx, profile, RustPluginBuild::Default)?;
        build_wrapper_set(
            ctx,
            profile,
            WrapperBuild::Plugin {
                vst3: targets.contains(&Target::Vst3),
                au: targets.contains(&Target::Au),
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
    // Rust 側の build.rs が src-gui/dist を埋め込むため、ここで先に frontend を確定させる。
    // 順序を逆にすると古い dist や空の dist を plugin に含める可能性がある。
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
    Vst3,
    Au,
    Standalone,
}

impl RustPluginBuild {
    fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Vst3 => "vst3",
            Self::Au => "au",
            Self::Standalone => "standalone",
        }
    }

    fn cargo_target_dir(self, ctx: &Context) -> PathBuf {
        match self {
            Self::Default => ctx.target_dir.clone(),
            Self::Vst3 | Self::Au | Self::Standalone => {
                ctx.wrac_dir().join("cargo").join(self.label())
            }
        }
    }

    fn dynamic_library(self, ctx: &Context, profile: BuildProfile) -> PathBuf {
        self.cargo_target_dir(ctx)
            .join(profile.cargo_dir())
            .join(ctx.platform.dynamic_library_name())
    }

    fn static_library(self, ctx: &Context, profile: BuildProfile) -> PathBuf {
        self.cargo_target_dir(ctx)
            .join(profile.cargo_dir())
            .join(ctx.platform.static_library_name())
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
        // CI や利用者環境の env を尊重しつつ、未指定時だけ template の安全な既定値を入れる。
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
        // clap-wrapper は CLAP bundle ではなく Rust staticlib を直接 link する。
        // CLAP だけの platform では不要なので、wrapper 対応 OS の時だけ確認する。
        ensure_exists(&build.static_library(ctx, profile), "static plugin library")?;
    }
    Ok(())
}

fn package_clap(ctx: &Context, profile: BuildProfile) -> Result<()> {
    println!("Packaging CLAP...");
    let bundle = ctx.clap_bundle(profile);
    let version = plugin_version(ctx)?;
    remove_if_exists(&bundle)?;
    fs::create_dir_all(ctx.plugins_dir(profile))?;

    match ctx.platform {
        Platform::Macos => {
            // macOS の CLAP は裸の dylib ではなく bundle として配布される。
            // host が bundle metadata を読むため、plugin id と Info.plist をここで一致させる。
            // install_name も bundle 内相対にして、install 先を変えてもロードできるようにする。
            let contents = bundle.join("Contents");
            let macos = contents.join("MacOS");
            fs::create_dir_all(&macos)?;
            fs::write(
                contents.join("Info.plist"),
                macos_clap_info_plist(&ctx.metadata, &version),
            )?;
            fs::write(contents.join("PkgInfo"), "BNDL????")?;
            fs::copy(
                ctx.dynamic_library(profile),
                macos.join(&ctx.metadata.plugin_name),
            )?;
            run(Command::new("install_name_tool")
                .arg("-id")
                .arg(format!("@loader_path/{}", ctx.metadata.plugin_name))
                .arg(macos.join(&ctx.metadata.plugin_name))
                .current_dir(&ctx.root))?;
            codesign(&bundle)?;
        }
        Platform::Windows | Platform::Linux => {
            // Windows/Linux では CLAP artifact は拡張子が .clap の dynamic library として扱う。
            // bundle 構造を作らないことで各 OS の既存 host 探索規約に合わせる。
            fs::copy(ctx.dynamic_library(profile), &bundle)?;
        }
    }

    ensure_exists(&bundle, "CLAP artifact")?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum WrapperBuild {
    Plugin { vst3: bool, au: bool },
    Vst3,
    Au,
    Standalone,
}

impl WrapperBuild {
    fn purpose(self) -> &'static str {
        match self {
            Self::Plugin { .. } => "wrap",
            Self::Vst3 => "wrap-vst3",
            Self::Au => "wrap-au",
            Self::Standalone => "standalone",
        }
    }

    fn rust_build(self) -> RustPluginBuild {
        match self {
            Self::Plugin { .. } => RustPluginBuild::Default,
            Self::Vst3 => RustPluginBuild::Vst3,
            Self::Au => RustPluginBuild::Au,
            Self::Standalone => RustPluginBuild::Standalone,
        }
    }
}

fn build_wrapper_set(ctx: &Context, profile: BuildProfile, build: WrapperBuild) -> Result<()> {
    let rust_build = build.rust_build();
    let static_library = rust_build.static_library(ctx, profile);
    ensure_exists(&static_library, "static plugin library")?;
    let version = plugin_version(ctx)?;

    let build_dir = ctx.cmake_dir(build.purpose(), profile);
    let stage_dir = match build {
        WrapperBuild::Plugin { .. } | WrapperBuild::Vst3 | WrapperBuild::Au => {
            ctx.plugins_dir(profile)
        }
        WrapperBuild::Standalone => ctx.standalone_dir(profile),
    };
    fs::create_dir_all(&stage_dir)?;

    let mut configure = Command::new("cmake");
    // wrapper は Rust staticlib から直接作る。既に生成済みの CLAP bundle を探す方式にすると、
    // clean/install の順序や古い artifact に依存して再現性が落ちるため。
    // stage 先も xtask が後続で検証する path と同じ値を CMake に渡す。
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
            ctx.metadata.plugin_name
        ))
        .arg(format!(
            "-DCLAP_WRAPPER_BUILDER_TARGET_NAME={CRATE_NAME}_{}",
            build.purpose()
        ))
        .arg(format!(
            "-DCLAP_WRAPPER_BUILDER_STAGE_DIR={}",
            stage_dir.display()
        ))
        .arg(format!("-DCLAP_WRAPPER_BUILDER_BUNDLE_VERSION={version}"))
        .arg(format!("-DCMAKE_BUILD_TYPE={}", profile.cmake_config()))
        .arg("-DCLAP_WRAPPER_BUILDER_BUILD_AAX=OFF")
        .arg("-DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=OFF")
        .arg("-DCLAP_WRAPPER_CXX_STANDARD=23");

    match build {
        WrapperBuild::Plugin { vst3, au } => {
            configure
                .arg(format!(
                    "-DCLAP_WRAPPER_BUILDER_BUILD_VST3={}",
                    on_off(vst3)
                ))
                .arg(format!("-DCLAP_WRAPPER_BUILDER_BUILD_AUV2={}", on_off(au)))
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=OFF");
        }
        WrapperBuild::Vst3 => {
            configure
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_VST3=ON")
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_AUV2=OFF")
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=OFF");
        }
        WrapperBuild::Au => {
            configure
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_VST3=OFF")
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_AUV2=ON")
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=OFF");
        }
        WrapperBuild::Standalone => {
            // standalone は plugin wrapper と違い、アプリ側の補助依存が必要になる。
            // clap-wrapper 側の取得ロジックに任せ、plugin wrapper では依存 download を無効のまま保つ。
            configure
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_VST3=OFF")
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_AUV2=OFF")
                .arg("-DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=ON")
                .arg(format!(
                    "-DCLAP_WRAPPER_BUILDER_STANDALONE_PLUGIN_ID={}",
                    ctx.metadata.plugin_id
                ))
                .arg(format!(
                    "-DCLAP_WRAPPER_BUILDER_STANDALONE_OUTPUT_NAME={}",
                    ctx.metadata.standalone_name
                ))
                .arg("-DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=ON");
        }
    }

    if ctx.platform == Platform::Macos {
        // AUv2 は type/manufacturer/subtype の 4 文字コードが host discovery の key になる。
        // Rust 側の descriptor から推測させず、template の constants を単一の入力にする。
        configure
            .arg(format!(
                "-DAUDIOUNIT_SDK_ROOT={}",
                ctx.wrapper_dir.join("AudioUnitSDK").display()
            ))
            .arg(format!(
                "-DCLAP_WRAPPER_AUV2_INSTRUMENT_TYPE={}",
                ctx.metadata.auv2_type
            ))
            .arg(format!(
                "-DCLAP_WRAPPER_AUV2_MANUFACTURER_NAME={}",
                ctx.metadata.company_name
            ))
            .arg(format!(
                "-DCLAP_WRAPPER_AUV2_MANUFACTURER_CODE={}",
                ctx.metadata.auv2_manufacturer_code
            ))
            .arg(format!(
                "-DCLAP_WRAPPER_AUV2_SUBTYPE_CODE={}",
                ctx.metadata.auv2_subtype
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
        // AudioUnitSDK は Xcode で warning-only な GNU statement-expression / narrowing を出す。
        // template 利用者が wrapper SDK の warning に引きずられず build できるよう、ここだけ抑制する。
        build_cmd.args([
            "--",
            "OTHER_CPLUSPLUSFLAGS=$(inherited) -Wno-unknown-warning-option -Wno-gnu-statement-expression-from-macro-expansion -Wno-shorten-64-to-32 -Wno-perf-constraint-implies-noexcept",
        ]);
    }

    run(build_cmd.current_dir(&ctx.root))?;

    match build {
        WrapperBuild::Plugin { vst3, au } => {
            if vst3 {
                ensure_exists(&ctx.vst3_bundle(profile), "VST3 artifact")?;
                if ctx.platform == Platform::Macos {
                    // macOS host は未署名 bundle を拒否することがあるため、開発用に ad-hoc 署名する。
                    codesign_nested_macos_bundle(ctx, &ctx.vst3_bundle(profile))?;
                }
            }
            if au {
                ensure_exists(&ctx.au_bundle(profile), "AU artifact")?;
                // AU は AudioComponentRegistrar 経由で読むため、local build でも署名済みにしておく。
                codesign_nested_macos_bundle(ctx, &ctx.au_bundle(profile))?;
            }
        }
        WrapperBuild::Vst3 => {
            ensure_exists(&ctx.vst3_bundle(profile), "VST3 artifact")?;
            codesign_nested_macos_bundle(ctx, &ctx.vst3_bundle(profile))?;
        }
        WrapperBuild::Au => {
            ensure_exists(&ctx.au_bundle(profile), "AU artifact")?;
            codesign_nested_macos_bundle(ctx, &ctx.au_bundle(profile))?;
        }
        WrapperBuild::Standalone => {
            ensure_exists(&ctx.standalone_artifact(profile), "standalone artifact")?;
            if ctx.platform == Platform::Macos {
                // standalone app も Gatekeeper/loader の扱いを plugin bundle と揃える。
                codesign_nested_macos_bundle(ctx, &ctx.standalone_artifact(profile))?;
            }
        }
    }

    Ok(())
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
            PluginTarget::Au => install_artifact(
                &ctx.au_bundle(profile),
                &install_dir(ctx, scope, PluginFormat::Au)?,
            )?,
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
        (Platform::Macos, InstallScope::System, PluginFormat::Clap) => {
            PathBuf::from("/Library/Audio/Plug-Ins/CLAP")
        }
        (Platform::Macos, InstallScope::System, PluginFormat::Vst3) => {
            PathBuf::from("/Library/Audio/Plug-Ins/VST3")
        }
        (Platform::Macos, InstallScope::System, PluginFormat::Au) => {
            PathBuf::from("/Library/Audio/Plug-Ins/Components")
        }
        (Platform::Windows, InstallScope::User, PluginFormat::Clap) => local_app_data()?
            .join("Programs")
            .join("Common")
            .join("CLAP"),
        (Platform::Windows, InstallScope::User, PluginFormat::Vst3) => local_app_data()?
            .join("Programs")
            .join("Common")
            .join("VST3"),
        (Platform::Windows, InstallScope::System, PluginFormat::Clap) => {
            common_program_files()?.join("CLAP")
        }
        (Platform::Windows, InstallScope::System, PluginFormat::Vst3) => {
            common_program_files()?.join("VST3")
        }
        (Platform::Windows, _, PluginFormat::Au) => {
            return Err("AU is not supported on Windows".into());
        }
        (Platform::Linux, InstallScope::User, PluginFormat::Clap) => home_dir()?.join(".clap"),
        (Platform::Linux, InstallScope::User, PluginFormat::Vst3) => home_dir()?.join(".vst3"),
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
    // bundle の中身を上書き merge すると古い binary/resource が残り得る。
    // 一度消してから丸ごとコピーし、install 結果を build artifact と一致させる。
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
    };
    let bundle_name = match target {
        PluginTarget::Clap => ctx.metadata.clap_bundle_name(),
        PluginTarget::Vst3 => ctx.metadata.vst3_bundle_name(),
        PluginTarget::Au => ctx.metadata.au_bundle_name(),
    };
    uninstall_scopes(scope)
        .iter()
        .copied()
        .map(|install_scope| {
            install_dir(ctx, install_scope, format).map(|dir| dir.join(&bundle_name))
        })
        .collect::<Result<Vec<_>>>()
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
        // validator は VST3 SDK からその場で build するため、artifact 確認より先に SDK を検証する。
        // 空のサブモジュール directory だけで CMake まで進むと、利用者に原因が伝わりにくい。
        ensure_vst3_sdk_input(ctx)?;
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
        let au = ctx.au_bundle(profile);
        ensure_exists(&au, "AU artifact")?;
        ensure_no_system_au_conflict(ctx)?;

        // auval は path 指定ではなく AudioComponentRegistrar から対象を解決する。
        // そのため freshly built な AU を user-local に置いてから validation する。
        let install_dir = install_dir(ctx, InstallScope::User, PluginFormat::Au)?;
        install_artifact(&au, &install_dir)?;

        // registrar は component 情報を cache するため、直前に置いた AU を見せるには再起動が必要。
        // killall が失敗しても auval 側で検出できる可能性があるので、ここでは best-effort にする。
        let _ = Command::new("killall")
            .args(["-9", "AudioComponentRegistrar"])
            .status();

        run(Command::new("/usr/bin/auval")
            .args([
                "-v",
                &ctx.metadata.auv2_type,
                &ctx.metadata.auv2_subtype,
                &ctx.metadata.auv2_manufacturer_code,
            ])
            .current_dir(&ctx.root))?;
    }

    Ok(())
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

    // validator は出荷 artifact ではなく検証用 tool。
    // plugin の release/debug とは独立なので、Debug の 1 build を両 profile で使い回す。
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

fn ensure_wrapper_inputs(ctx: &Context, needs_vst3: bool, needs_au: bool) -> Result<()> {
    // git submodule が未 init の場合、directory だけ存在して中身が空のことがある。
    // CMake の抽象的な失敗に進ませず、wrapper が実際に読む sentinel file を見る。
    ensure_exists(&ctx.wrapper_dir, "clap_wrapper_builder directory")?;
    ensure_exists(
        &ctx.wrapper_dir.join("clap-wrapper").join("CMakeLists.txt"),
        "clap-wrapper submodule",
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
    Ok(())
}

fn ensure_vst3_sdk_input(ctx: &Context) -> Result<()> {
    ensure_exists(
        &ctx.wrapper_dir.join("vst3sdk").join("CMakeLists.txt"),
        "VST3 SDK submodule",
    )
}

fn print_outputs(ctx: &Context, profile: BuildProfile, targets: &[Target]) {
    for target in targets {
        match target {
            Target::Clap => println!("CLAP: {}", ctx.clap_bundle(profile).display()),
            Target::Vst3 => println!("VST3: {}", ctx.vst3_bundle(profile).display()),
            Target::Au => println!("AU: {}", ctx.au_bundle(profile).display()),
            Target::Standalone => {
                println!("Standalone: {}", ctx.standalone_artifact(profile).display())
            }
        }
    }
}

fn macos_clap_info_plist(metadata: &PluginMetadata, version: &str) -> String {
    let plugin_name = &metadata.plugin_name;
    let plugin_id = &metadata.plugin_id;
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
    <string>{plugin_id}</string>
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
    <string></string>
    <key>NSHighResolutionCapable</key>
    <true/>
  </dict>
</plist>
"#
    )
}

fn plugin_version(ctx: &Context) -> Result<String> {
    let manifest = fs::read_to_string(ctx.plugin_manifest())?;
    let mut in_package_section = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package_section = line == "[package]";
            continue;
        }
        if !in_package_section {
            continue;
        }
        if let Some(value) = line.strip_prefix("version") {
            let Some((_, value)) = value.split_once('=') else {
                continue;
            };
            let version = value.trim().trim_matches('"');
            if !version.is_empty() {
                return Ok(version.to_string());
            }
        }
    }
    Err("failed to read plugin version from src-plugin/Cargo.toml".into())
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

fn codesign_nested_macos_bundle(ctx: &Context, bundle: &Path) -> Result<()> {
    let nested_clap = bundle
        .join("Contents")
        .join("PlugIns")
        .join(ctx.metadata.clap_bundle_name());
    if nested_clap.exists() {
        codesign(&nested_clap)?;
    }
    codesign(bundle)?;
    Ok(())
}
