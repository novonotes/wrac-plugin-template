use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Result;
use crate::context::Context;
use crate::profile::BuildProfile;
use crate::targets::{Platform, ValidateTarget};
use crate::util::{
    copy_path, ensure_exists, print_section, print_skip, print_success, remove_if_exists,
    run_output_with_language, run_with_language, run_with_optional_xcbeautify_language,
};
use crate::validation::validate_wrac_rules;

use super::{ensure_vst3_sdk_input, env_path};

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
                let reason = ctx
                    .metadata
                    .validation
                    .clap_validator
                    .skip_reason
                    .as_deref()
                    .unwrap_or("no reason provided");
                print_skip(
                    ctx.output_language,
                    &format!("CLAP validator skip filter: {filter} ({reason})"),
                    &format!("CLAP validator skip filter: {filter} ({reason})"),
                );
                command
                    .arg("--test-filter")
                    .arg(filter)
                    .arg("--invert-filter");
            }
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

    print_success(
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

fn run_aax_validator(ctx: &Context, aax: &Path) -> Result<()> {
    let results_dir = ctx.wrac_dir().join("validation").join("aax");
    // A fresh directory prevents a previous pass result from masking a missing
    // validator output if DTT exits early or changes a result reference.
    remove_if_exists(&results_dir)?;
    fs::create_dir_all(&results_dir)?;
    let aax = stage_aax_for_validator(&results_dir, aax)?;

    print_section(ctx.output_language, "AAX validator", "AAX validator");
    println!(
        "  {}: {}",
        match ctx.output_language {
            crate::XtaskOutputLanguage::English => "Target",
            crate::XtaskOutputLanguage::Japanese => "対象",
        },
        aax.display()
    );
    println!(
        "  {}: {}",
        match ctx.output_language {
            crate::XtaskOutputLanguage::English => "Selected tests",
            crate::XtaskOutputLanguage::Japanese => "実行 test",
        },
        AAX_VALIDATOR_REQUIRED_TESTS.len()
    );
    for (test_id, reason) in AAX_VALIDATOR_SKIPPED_TESTS {
        print_skip(
            ctx.output_language,
            &format!("Skipping {test_id}: {reason}."),
            &format!("{test_id}: {reason}"),
        );
    }

    run_aax_validator_dtt(ctx, &aax, &results_dir)?;

    assert_aax_validator_results(ctx, &results_dir)
}

fn run_aax_validator_dtt(ctx: &Context, aax: &Path, results_dir: &Path) -> Result<()> {
    let dtt = ensure_aax_validator_dtt(ctx)?;
    let aax_search_dir = aax
        .parent()
        .ok_or_else(|| format!("AAX bundle path has no parent directory: {}", aax.display()))?;
    print_section(ctx.output_language, "Running command", "コマンド実行");
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

fn assert_aax_validator_results(ctx: &Context, results_dir: &Path) -> Result<()> {
    let mut failed = Vec::new();
    for (index, test_id) in AAX_VALIDATOR_REQUIRED_TESTS.iter().enumerate() {
        let result_path = aax_validator_result_path(results_dir, index, test_id);
        // DTT's process exit is not enough for reviewable validation: the official
        // JSON result records the test ID and validator result_status that CI logs
        // and artifacts can be audited against.
        let status = aax_validator_result_status(&result_path)?;
        if status == "E_COMPLETED_PASS" {
            print_success(
                ctx.output_language,
                &format!("AAX validator PASS: {test_id}"),
                &format!("AAX validator 成功: {test_id}"),
            );
        } else {
            println!(
                "  ❌ {}",
                match ctx.output_language {
                    crate::XtaskOutputLanguage::English => format!(
                        "AAX validator FAIL: {test_id} ({status}); see {}",
                        result_path.display()
                    ),
                    crate::XtaskOutputLanguage::Japanese => format!(
                        "AAX validator 失敗: {test_id} ({status}); 詳細 {}",
                        result_path.display()
                    ),
                }
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
            // Keep timeouts outside `run_with_language()` so failed DTT processes still have their
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
        run_with_language(
            Command::new("chmod")
                .arg("+x")
                .arg(&dtt)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
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
        run_with_language(
            Command::new("7z")
                .arg("x")
                .arg(&archive)
                .arg(format!("-o{}", extracted_root.display()))
                .arg("-y")
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
    } else {
        run_with_language(
            Command::new("tar")
                .arg("-xf")
                .arg(&archive)
                .arg("--strip-components=1")
                .arg("-C")
                .arg(&extracted_root)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
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
        run_with_language(
            Command::new("curl")
                .args(["-L", "--fail", "-o"])
                .arg(&archive)
                .arg(url)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
    }

    if archive_name.ends_with(".zip") {
        // Windows runners provide bsdtar as `tar`, and it can extract zip files.
        // Using it here keeps argument passing identical to the tar.gz path.
        run_with_language(
            Command::new("tar")
                .arg("-xf")
                .arg(&archive)
                .arg("-C")
                .arg(&validator_dir)
                .current_dir(&ctx.root),
            ctx.output_language,
        )?;
    } else {
        run_with_language(
            Command::new("tar")
                .args(["-xzf"])
                .arg(&archive)
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
        configure.arg("-G").arg("Xcode");
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
