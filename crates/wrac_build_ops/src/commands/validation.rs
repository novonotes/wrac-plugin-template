use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::ensure_vst3_sdk_input;
use crate::Result;
use crate::context::Context;
use crate::profile::BuildProfile;
use crate::targets::{Platform, ValidateTarget};
use crate::util::{
    ensure_exists, print_detail, run_output_with_language, run_with_language,
    run_with_optional_xcbeautify_language,
};

mod aax;

use aax::run_aax_validator;

/// Caller-owned choices for external format validator adapters.
#[derive(Debug, Clone)]
pub struct ExternalValidatorConfig {
    pub clap: ClapValidatorConfig,
    pub aax: AaxValidatorConfig,
}

/// clap-validator version and caller-approved test exclusions.
#[derive(Debug, Clone)]
pub struct ClapValidatorConfig {
    pub version: String,
    pub test_exclusions: Vec<ClapValidatorTestExclusion>,
}

/// A clap-validator test filter omitted by the caller, together with its reviewable reason.
#[derive(Debug, Clone)]
pub struct ClapValidatorTestExclusion {
    pub filter: String,
    pub reason: String,
}

/// Explicit AAX Validator test selection and process timeout.
#[derive(Debug, Clone)]
pub struct AaxValidatorConfig {
    pub required_tests: Vec<String>,
    pub skipped_tests: Vec<SkippedValidatorTest>,
    pub timeout_secs: u64,
}

/// A test omitted by the caller, together with its reviewable reason.
#[derive(Debug, Clone)]
pub struct SkippedValidatorTest {
    pub id: String,
    pub reason: String,
}

pub fn validate_plugin_target(
    ctx: &Context,
    profile: BuildProfile,
    target: ValidateTarget,
    config: &ExternalValidatorConfig,
) -> Result<()> {
    match target {
        ValidateTarget::Clap => {
            let clap = ctx.clap_bundle(profile);
            ensure_exists(&clap, "CLAP artifact")?;
            let validator = ensure_clap_validator(ctx, &config.clap.version)?;
            for exclusion in &config.clap.test_exclusions {
                print_detail(
                    ctx.output_language,
                    &format!(
                        "CLAP validator test exclusion: {} ({})",
                        exclusion.filter, exclusion.reason
                    ),
                    &format!(
                        "CLAP validator test exclusion: {} ({})",
                        exclusion.filter, exclusion.reason
                    ),
                );
            }
            let mut command = clap_validator_command(&validator, &clap, &config.clap);
            run_with_language(command.current_dir(&ctx.root), ctx.output_language)?;
        }
        ValidateTarget::Vst3 => {
            let vst3 = ctx.vst3_bundle(profile);
            ensure_exists(&vst3, "VST3 artifact")?;
            let validator = ensure_vst3_validator(ctx)?;
            let output = run_output_with_language(
                Command::new(validator)
                    .env("WRAC_PLUGIN_VALIDATOR", "1")
                    .arg(&vst3)
                    .current_dir(&ctx.root),
                ctx.output_language,
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
                run_with_language(
                    Command::new("/usr/bin/auval")
                        .args([
                            "-v",
                            &plugin.auv2_type,
                            &plugin.auv2_subtype,
                            &ctx.metadata.auv2_manufacturer_code,
                        ])
                        .current_dir(&ctx.root),
                    ctx.output_language,
                )?;
            }
        }
        ValidateTarget::Aax => {
            let aax = ctx.aax_bundle(profile);
            ensure_exists(&aax, "AAX artifact")?;
            run_aax_validator(ctx, &aax, &config.aax)?;
        }
    }
    Ok(())
}

fn clap_validator_command(validator: &Path, clap: &Path, config: &ClapValidatorConfig) -> Command {
    let mut command = Command::new(validator);
    command
        .env("WRAC_PLUGIN_VALIDATOR", "1")
        .arg("validate")
        .arg(clap)
        .arg("--only-failed");
    for exclusion in &config.test_exclusions {
        // clap-validator 0.4.1 accepts repeatable regular expressions through --exclude.
        command.arg("--exclude").arg(&exclusion.filter);
    }
    command
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

    print_detail(
        ctx.output_language,
        "VST3 component IDs match plugins.vst3_component_id",
        "VST3 component ID は plugins.vst3_component_id と一致",
    );
    Ok(())
}

pub(super) fn parse_vst3_validator_cids(output: &str) -> Vec<String> {
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

fn ensure_clap_validator(ctx: &Context, version: &str) -> Result<PathBuf> {
    let validator_dir = ctx
        .target_dir
        .join("tools")
        .join("clap-validator")
        .join(version);
    let validator = clap_validator_executable(ctx.platform, &validator_dir);
    if validator.exists() {
        return Ok(validator);
    }

    fs::create_dir_all(&validator_dir)?;
    let archives = clap_validator_archives(ctx.platform, version)?;
    let archive = validator_dir.join(archives.download);
    if !archive.exists() {
        let url = format!(
            "https://github.com/free-audio/clap-validator/releases/download/{version}/{}",
            archives.download
        );
        // Download into a temporary file and rename it only after curl succeeds.
        // The existence check above would otherwise reuse a partial or empty archive
        // left by an interrupted or failed download on every later run. A leftover
        // temporary file is removed here before the next attempt.
        let partial = validator_dir.join(format!("{}.download", archives.download));
        if partial.exists() {
            fs::remove_file(&partial)?;
        }
        run_with_language(
            Command::new("curl")
                .args(["-L", "--fail"])
                // GitHub release downloads occasionally fail with transient errors
                // such as HTTP 504. Retry them instead of failing the whole validation.
                .args(["--retry", "5", "--retry-all-errors", "--retry-delay", "5"])
                .arg("-o")
                .arg(&partial)
                .arg(url)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
        fs::rename(&partial, &archive)?;
    }

    if ctx.platform == Platform::Linux {
        // GNU tar does not extract zip archives, while unzip is available on the
        // supported Ubuntu development and CI environments.
        run_with_language(
            Command::new("unzip")
                .arg("-o")
                .arg(&archive)
                .arg("-d")
                .arg(&validator_dir)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
    } else {
        // macOS and Windows provide bsdtar as `tar`, including zip support.
        run_with_language(
            Command::new("tar")
                .arg("-xf")
                .arg(&archive)
                .arg("-C")
                .arg(&validator_dir)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
    }
    if let Some(nested) = archives.nested {
        let nested = validator_dir.join(nested);
        ensure_exists(&nested, "nested CLAP validator archive")?;
        run_with_language(
            Command::new("tar")
                .args(["-xzf"])
                .arg(nested)
                .arg("-C")
                .arg(&validator_dir)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
    }

    ensure_exists(&validator, "CLAP validator")?;
    if ctx.platform != Platform::Windows {
        run_with_language(
            Command::new("chmod")
                .arg("+x")
                .arg(&validator)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
    }
    Ok(validator)
}

#[derive(Clone, Copy)]
struct ClapValidatorArchives {
    download: &'static str,
    nested: Option<&'static str>,
}

fn clap_validator_archives(platform: Platform, version: &str) -> Result<ClapValidatorArchives> {
    // clap-validator release asset names and archive nesting are not stable across tags.
    // Pinning the exact published layout avoids silently downloading a different build.
    if version != "0.4.1" {
        return Err(format!("unsupported clap-validator release: {version}").into());
    }
    Ok(match platform {
        Platform::Macos => ClapValidatorArchives {
            download: "clap-validator-0.4.1-127-g152b982-macos-universal.zip",
            nested: Some("clap-validator-0.3.2-127-g152b982-macos-universal.tar.gz"),
        },
        Platform::Windows => ClapValidatorArchives {
            download: "clap-validator-0.4.1-127-g152b982-windows.zip",
            nested: None,
        },
        Platform::Linux => ClapValidatorArchives {
            download: "clap-validator-0.4.1-127-g152b982-ubuntu-22.04.zip",
            nested: Some("clap-validator-0.3.2-127-g152b982-ubuntu-22.04.tar.gz"),
        },
    })
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
    let shared_validator_dir = ctx
        .target_dir
        .parent()
        .map(|path| path.join("vst3sdk-validator"))
        .unwrap_or_else(|| ctx.target_dir.join("vst3sdk-validator"));
    let validator_bin_dir = shared_validator_dir.join("bin");
    let validator = validator_bin_dir.join("Debug").join(executable);
    let validator_without_config = validator_bin_dir.join(executable);

    if validator.exists() {
        return Ok(validator);
    }
    if validator_without_config.exists() {
        return Ok(validator_without_config);
    }

    // The validator is a verification tool, not a shipping artifact.
    // It is independent of the plugin and release/debug profile, so one Debug build is
    // shared by all plugin validations in the same target namespace.
    let build_dir = shared_validator_dir;
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
        // The SDK defaults to macOS 10.13, which current Xcode releases reject before the
        // compiler probe. Keep validation aligned with the template's supported minimum.
        configure
            .arg("-DCMAKE_OSX_DEPLOYMENT_TARGET=12.0")
            .arg("-G")
            .arg("Xcode");
    }
    run_with_language(configure.current_dir(&ctx.root), ctx.output_language)?;

    let mut build = Command::new("cmake");
    build
        .arg("--build")
        .arg(&build_dir)
        .arg("--target")
        .arg("validator")
        .arg("--config")
        .arg("Debug");
    if ctx.platform == Platform::Macos {
        build.args([
            "--",
            "-quiet",
            "OTHER_CPLUSPLUSFLAGS=$(inherited) -Wno-unknown-warning-option -Wno-gnu-statement-expression-from-macro-expansion -Wno-shorten-64-to-32 -Wno-perf-constraint-implies-noexcept",
        ]);
    }

    let build = build.current_dir(&ctx.root);
    if ctx.platform == Platform::Macos {
        run_with_optional_xcbeautify_language(build, ctx.output_language)?;
    } else {
        run_with_language(build, ctx.output_language)?;
    }

    if validator.exists() {
        Ok(validator)
    } else {
        ensure_exists(&validator_without_config, "VST3 validator")?;
        Ok(validator_without_config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_validator_command_uses_supported_exclusion_arguments() {
        let config = ClapValidatorConfig {
            version: "0.4.1".to_string(),
            test_exclusions: vec![ClapValidatorTestExclusion {
                filter: "^param-conversions$".to_string(),
                reason: "upstream validator defect".to_string(),
            }],
        };

        let command = clap_validator_command(
            Path::new("clap-validator"),
            Path::new("Example.clap"),
            &config,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            args,
            [
                "validate",
                "Example.clap",
                "--only-failed",
                "--exclude",
                "^param-conversions$",
            ]
        );
    }
}
