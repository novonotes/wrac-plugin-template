use std::{fs, path::Path, process::Command};

use crate::{
    BuildProfile, Result,
    context::Context,
    target_resolution::resolve_build_targets_from_metadata,
    targets::{Platform, Target},
    util::{ensure_exists, env_value_or, remove_if_exists, run_with_language},
};

use super::{
    WrapperBuild, WrapperBuildOptions, WrapperTarget, build_wrapper_target_in_dir,
    configure_wrapper_with_options,
};

const RUST_TARGETS: [(&str, &str); 2] = [
    ("x86_64-apple-darwin", "x86_64"),
    ("aarch64-apple-darwin", "arm64"),
];

/// Builds selected macOS wrappers as x86_64/arm64 universal Release bundles.
///
/// This operation owns both the cross-architecture Rust library and the CMake
/// architecture setting because wrapper output is only valid when they agree.
/// Product-level distribution code should only select packages and formats. Every returned
/// wrapper has been checked with `lipo`; success therefore guarantees that its executable contains
/// both architectures. The function intentionally accepts no profile because distributable
/// universal bundles are always Release artifacts.
pub fn build_macos_universal_wrappers(ctx: &Context, targets: &[WrapperTarget]) -> Result<()> {
    if ctx.platform != Platform::Macos {
        return Err("macOS universal wrappers can only be built on macOS".into());
    }
    if targets
        .iter()
        .any(|target| matches!(target, WrapperTarget::Standalone))
    {
        return Err("macOS universal standalone builds are not supported".into());
    }
    let requested = targets
        .iter()
        .map(|target| match target {
            WrapperTarget::Vst3 => Target::Vst3,
            WrapperTarget::Au => Target::Au,
            WrapperTarget::Aax => Target::Aax,
            WrapperTarget::Standalone => unreachable!(),
        })
        .collect::<Vec<_>>();
    resolve_build_targets_from_metadata(ctx, &requested)?;

    let profile = BuildProfile::Release;
    // Do not reuse the native target directory: Cargo may otherwise make a host-architecture
    // archive look current after switching between native and universal workflows.
    let cargo_root = ctx.wrac_dir().join("cargo/macos-universal");
    let deployment_target = env_value_or("MACOSX_DEPLOYMENT_TARGET", "11.0");
    let library_name = ctx.platform.static_library_name(&ctx.metadata.package_name);
    let mut architecture_libraries = Vec::new();

    for (rust_target, _) in RUST_TARGETS {
        run_with_language(
            Command::new("cargo")
                .arg("build")
                .arg("--target-dir")
                .arg(&cargo_root)
                .arg("--manifest-path")
                .arg(ctx.plugin_manifest())
                .arg("--release")
                .arg("--target")
                .arg(rust_target)
                .env("MACOSX_DEPLOYMENT_TARGET", &deployment_target)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
        let library = cargo_root
            .join(rust_target)
            .join(profile.cargo_dir())
            .join(&library_name);
        ensure_exists(&library, "architecture-specific static plugin library")?;
        architecture_libraries.push(library);
    }

    let universal_library = cargo_root.join(profile.cargo_dir()).join(&library_name);
    fs::create_dir_all(
        universal_library
            .parent()
            .ok_or("invalid universal static library path")?,
    )?;
    remove_if_exists(&universal_library)?;
    run_with_language(
        Command::new("lipo")
            .arg("-create")
            .args(&architecture_libraries)
            .arg("-output")
            .arg(&universal_library),
        ctx.output_language,
    )?;
    verify_universal_binary(ctx, &universal_library)?;

    let builds = [
        (
            WrapperBuild::Plugins {
                vst3: targets.contains(&WrapperTarget::Vst3),
                au: targets.contains(&WrapperTarget::Au),
            },
            targets.contains(&WrapperTarget::Vst3) || targets.contains(&WrapperTarget::Au),
        ),
        (WrapperBuild::Aax, targets.contains(&WrapperTarget::Aax)),
    ];
    for (build, enabled) in builds {
        if !enabled {
            continue;
        }
        configure_wrapper_with_options(
            ctx,
            profile,
            build,
            WrapperBuildOptions {
                purpose_suffix: "-macos-universal",
                static_library: Some(&universal_library),
                macos_architectures: Some("x86_64;arm64"),
            },
        )?;
        build_and_verify_wrappers(ctx, targets, profile, build)?;
    }
    Ok(())
}

fn build_and_verify_wrappers(
    ctx: &Context,
    targets: &[WrapperTarget],
    profile: BuildProfile,
    build: WrapperBuild,
) -> Result<()> {
    let purpose = format!("{}-macos-universal", build.purpose());
    for target in targets.iter().copied().filter(|target| match build {
        WrapperBuild::Plugins { .. } => {
            matches!(target, WrapperTarget::Vst3 | WrapperTarget::Au)
        }
        WrapperBuild::Aax => matches!(target, WrapperTarget::Aax),
        WrapperBuild::Standalone => false,
    }) {
        build_wrapper_target_in_dir(ctx, profile, build, target, None, &purpose)?;
        let executable = match target {
            WrapperTarget::Vst3 => ctx.vst3_bundle(profile),
            WrapperTarget::Au => ctx.au_bundle(profile),
            WrapperTarget::Aax => ctx.aax_bundle(profile),
            WrapperTarget::Standalone => unreachable!(),
        }
        .join("Contents/MacOS")
        .join(&ctx.metadata.bundle_name);
        verify_universal_binary(ctx, &executable)?;
    }
    Ok(())
}

fn verify_universal_binary(ctx: &Context, binary: &Path) -> Result<()> {
    run_with_language(
        Command::new("lipo")
            .arg(binary)
            .arg("-verify_arch")
            .args(RUST_TARGETS.map(|(_, architecture)| architecture)),
        ctx.output_language,
    )
}
