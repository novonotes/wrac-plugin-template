//! AAX Validator (DSH/DTT) execution for `validate_plugin_target`.
//!
//! Kept apart from the CLAP/VST3/AU validator adapters because DTT needs its own
//! archive staging, per-test process handling and result collection.

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::super::config_path;
use super::AaxValidatorConfig;
use crate::Result;
use crate::context::Context;
use crate::targets::Platform;
use crate::util::{
    copy_path, ensure_exists, print_detail, print_section, remove_if_exists, run_with_language,
};

pub(super) fn run_aax_validator(
    ctx: &Context,
    aax: &Path,
    config: &AaxValidatorConfig,
) -> Result<()> {
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
        config.required_tests.len()
    );
    for skipped in &config.skipped_tests {
        print_detail(
            ctx.output_language,
            &format!("Skipping {}: {}.", skipped.id, skipped.reason),
            &format!("{}: {}", skipped.id, skipped.reason),
        );
    }

    run_aax_validator_dtt(ctx, &aax, &results_dir, config)?;

    assert_aax_validator_results(ctx, &results_dir, &config.required_tests)
}

fn run_aax_validator_dtt(
    ctx: &Context,
    aax: &Path,
    results_dir: &Path,
    config: &AaxValidatorConfig,
) -> Result<()> {
    let dtt = ensure_aax_validator_dtt(ctx)?;
    let aax_search_dir = aax
        .parent()
        .ok_or_else(|| format!("AAX bundle path has no parent directory: {}", aax.display()))?;
    println!("  $ {}", dtt.display());

    for (index, test_id) in config.required_tests.iter().enumerate() {
        let test_dir =
            results_dir
                .join("dtt")
                .join(format!("{:02}-{}", index + 1, test_id.replace('.', "_")));
        let log_dir = test_dir.join("logs");
        let started_at = Instant::now();
        let mut attempt = 1;
        let output = loop {
            // Each attempt starts from empty directories so a retry cannot pick up
            // result files or logs left by the failed attempt.
            remove_if_exists(&test_dir)?;
            fs::create_dir_all(&log_dir)?;
            let output = run_aax_validator_dtt_test(
                ctx,
                &dtt,
                aax_search_dir,
                &test_dir,
                &log_dir,
                test_id,
                config,
            )?;
            if attempt < AAX_VALIDATOR_DISH_LOAD_ATTEMPTS
                && find_aax_validator_dtt_result(&test_dir, test_id).is_err()
                && is_aax_validator_dish_load_failure(&output.stdout)
            {
                // DigiShell occasionally fails to load Avid's own `aaxval.dish` before
                // any plug-in is opened. Observed on a self-hosted macOS runner right
                // after a cancelled job had its validator processes killed, and not
                // reproducible by killing the validator locally. The failure happens
                // before the plug-in under test is touched, so retrying only this case
                // cannot hide a plug-in defect.
                print_aax_validator_output(&output.stdout, &output.stderr);
                // Keep the retry visible in CI logs so its frequency can be tracked.
                println!(
                    "  AAX validator could not load aaxval.dish for {test_id}; retrying \
                     (attempt {} of {AAX_VALIDATOR_DISH_LOAD_ATTEMPTS})",
                    attempt + 1
                );
                attempt += 1;
                thread::sleep(AAX_VALIDATOR_DISH_LOAD_RETRY_DELAY);
                continue;
            }
            break output;
        };
        let stdout_path = test_dir.join("dtt-stdout.log");
        let stderr_path = test_dir.join("dtt-stderr.log");
        fs::write(&stdout_path, &output.stdout)?;
        fs::write(&stderr_path, &output.stderr)?;

        let result_path = aax_validator_result_path(results_dir, index, test_id);
        let dtt_result = match find_aax_validator_dtt_result(&test_dir, test_id) {
            Ok(path) => path,
            Err(err) => {
                // DTT can exit before writing any result, for example when its Ruby
                // runtime aborts. Its own output is then the only clue in CI logs.
                print_aax_validator_output(&output.stdout, &output.stderr);
                print_aax_validator_dtt_logs(&log_dir)?;
                return Err(err);
            }
        };
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
        print_aax_validator_test_timing(ctx, test_id, started_at.elapsed(), &result_path);

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

/// Total DTT runs allowed per test when DigiShell fails to load `aaxval.dish`.
const AAX_VALIDATOR_DISH_LOAD_ATTEMPTS: u32 = 2;
/// Pause before retrying so a transient system condition has time to clear.
const AAX_VALIDATOR_DISH_LOAD_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Returns whether DTT stopped because DigiShell could not load `aaxval.dish`.
///
/// DTT reports this as a `load_dish` command failure whose exception names the dish.
fn is_aax_validator_dish_load_failure(stdout: &[u8]) -> bool {
    let stdout = String::from_utf8_lossy(stdout);
    stdout.contains("DSH command 'load_dish' failed") && stdout.contains("\"aaxval.dish\"")
}

fn run_aax_validator_dtt_test(
    ctx: &Context,
    dtt: &Path,
    aax_search_dir: &Path,
    test_dir: &Path,
    log_dir: &Path,
    test_id: &str,
    config: &AaxValidatorConfig,
) -> Result<Output> {
    // Avid ships DTT as the automatable scripting layer for DigiShell. Use the
    // bundled ValidatorRunAllTests script instead of scripting DigiShell stdin
    // directly because Windows hosted CI can launch DigiShell while dropping
    // scripted stdin. The script expects a search directory for `findaaxplugins`;
    // passing the bundle path itself gives a different result shape on some packages.
    let child = Command::new(dtt)
        .arg("--script")
        .arg("ValidatorRunAllTests")
        .arg("--no_pref_delete")
        .arg("--no_move_options")
        .arg("--disable_digitrace")
        .arg("--verbose")
        .arg("--logdir")
        .arg(log_dir)
        .arg("--arg")
        .arg(format!(
            "pi_path={}",
            aax_validator_path_arg(aax_search_dir, ctx.platform)
        ))
        .arg("--arg")
        .arg(format!(
            "out_path={}",
            aax_validator_path_arg(test_dir, ctx.platform)
        ))
        .arg("--arg")
        .arg("result_format=json")
        .arg("--arg")
        .arg(format!("test_id={test_id}"))
        // DTT's Ruby scripts parse system profiler output that can contain non-ASCII
        // text. Without a UTF-8 locale, Ruby reads it as US-ASCII and aborts before
        // writing any result, which happens under CI services that leave LANG unset.
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(dtt.parent().unwrap_or(&ctx.root))
        .spawn()?;
    wait_for_aax_validator_process(child, aax_validator_timeout(config.timeout_secs)?)
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

fn assert_aax_validator_results(
    ctx: &Context,
    results_dir: &Path,
    required_tests: &[String],
) -> Result<()> {
    let mut failed = Vec::new();
    for (index, test_id) in required_tests.iter().enumerate() {
        let result_path = aax_validator_result_path(results_dir, index, test_id);
        // DTT's process exit is not enough for reviewable validation: the official
        // JSON result records the test ID and validator result_status that CI logs
        // and artifacts can be audited against.
        let status = aax_validator_result_status(&result_path)?;
        if status == "E_COMPLETED_PASS" {
            print_detail(
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
    // Drain stdout/stderr while DTT runs instead of after it exits. With `--verbose`,
    // tests such as `test.parameters` can print more than the OS pipe buffer holds
    // (64 KiB on macOS); reading only after exit leaves DTT blocked on a write, so a
    // passing test would end as a timeout.
    let stdout_reader = spawn_aax_validator_pipe_reader(child.stdout.take());
    let stderr_reader = spawn_aax_validator_pipe_reader(child.stderr.take());
    let started_at = Instant::now();
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, false);
        }
        if started_at.elapsed() >= timeout {
            child.kill()?;
            break (child.wait()?, true);
        }
        thread::sleep(Duration::from_secs(1));
    };
    let output = Output {
        status,
        stdout: join_aax_validator_pipe_reader(stdout_reader)?,
        stderr: join_aax_validator_pipe_reader(stderr_reader)?,
    };
    if timed_out {
        // Keep timeouts outside `run_with_language()` so failed DTT processes still have their
        // stdout/stderr printed. That output is usually the only clue when the
        // validator hangs while loading a bundle.
        print_aax_validator_output(&output.stdout, &output.stderr);
        return Err(format!(
            "AAX validator process timed out after {} seconds",
            timeout.as_secs()
        )
        .into());
    }
    Ok(output)
}

fn spawn_aax_validator_pipe_reader<R>(pipe: Option<R>) -> JoinHandle<io::Result<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut pipe) = pipe {
            pipe.read_to_end(&mut buffer)?;
        }
        Ok(buffer)
    })
}

fn join_aax_validator_pipe_reader(reader: JoinHandle<io::Result<Vec<u8>>>) -> Result<Vec<u8>> {
    let buffer = reader
        .join()
        .map_err(|_| "AAX validator output reader thread panicked")??;
    Ok(buffer)
}

fn aax_validator_timeout(default_seconds: u64) -> Result<Duration> {
    let seconds = match env::var("AAX_VALIDATOR_TIMEOUT_SECS") {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|err| format!("failed to parse AAX_VALIDATOR_TIMEOUT_SECS={value}: {err}"))?,
        Err(env::VarError::NotPresent) => default_seconds,
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
        // DTT logs are not guaranteed to be UTF-8 on Windows. Decode them lossily so a
        // log encoding never replaces the validation error these logs are printed for.
        let bytes = fs::read(&path).map_err(|err| {
            format!(
                "failed to read AAX validator DTT log {}: {err}",
                path.display()
            )
        })?;
        let content = String::from_utf8_lossy(&bytes);
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

/// Prints how long one DTT test took, split into the whole DTT run and the time the
/// validator itself reports for the test.
///
/// The gap between the two is DTT/DigiShell start-up and tear-down. Logging both per test
/// lets slow runners be diagnosed from CI logs alone, without access to the runner's
/// result directory.
fn print_aax_validator_test_timing(
    ctx: &Context,
    test_id: &str,
    process: Duration,
    result_path: &Path,
) {
    // Timing is diagnostic only. A result that cannot be read here still fails the run
    // in `assert_aax_validator_results`, so report the reason instead of failing twice.
    let reported = match aax_validator_reported_seconds(result_path) {
        Ok(seconds) if seconds.is_empty() => "not reported".to_string(),
        Ok(seconds) => seconds
            .iter()
            .map(|seconds| format!("{seconds:.1}s"))
            .collect::<Vec<_>>()
            .join(" + "),
        Err(err) => format!("unavailable ({err})"),
    };
    let process = process.as_secs_f64();
    print_detail(
        ctx.output_language,
        &format!("AAX validator timing: {test_id}: DTT {process:.1}s, test {reported}"),
        &format!("AAX validator 所要時間: {test_id}: DTT {process:.1}s, test {reported}"),
    );
}

/// Returns every `performance_data.elapsed_time_sec` in a DTT result.
///
/// Some tests report one result per effect, and info tests report none, so the
/// values are collected wherever they appear instead of from a fixed path.
fn aax_validator_reported_seconds(path: &Path) -> Result<Vec<f64>> {
    fn collect(value: &Value, seconds: &mut Vec<f64>) -> Result<()> {
        match value {
            Value::Object(map) => {
                if let Some(elapsed) = map
                    .get("performance_data")
                    .and_then(|data| data.get("elapsed_time_sec"))
                {
                    // DTT writes numbers as strings, e.g. "300.000031".
                    let text = elapsed.as_str().ok_or("elapsed_time_sec is not a string")?;
                    seconds.push(
                        text.parse::<f64>()
                            .map_err(|err| format!("invalid elapsed_time_sec {text:?}: {err}"))?,
                    );
                }
                for child in map.values() {
                    collect(child, seconds)?;
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect(item, seconds)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut seconds = Vec::new();
    collect(&read_aax_validator_result(path)?, &mut seconds)?;
    Ok(seconds)
}

fn read_aax_validator_result(path: &Path) -> Result<Value> {
    let content = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read AAX validator result {}: {err}",
            path.display()
        )
    })?;
    let json = serde_json::from_str(&content).map_err(|err| {
        format!(
            "failed to parse AAX validator result {}: {err}",
            path.display()
        )
    })?;
    Ok(json)
}

fn aax_validator_result_status(path: &Path) -> Result<String> {
    let json = read_aax_validator_result(path)?;
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

/// Formats a path for DTT script arguments.
///
/// CI target directories such as `C:/ci-target` produce mixed separators once joined on
/// Windows, and DigiShell's `findaaxplugins` returns no plug-ins for such paths.
fn aax_validator_path_arg(path: &Path, platform: Platform) -> String {
    let path = path.display().to_string();
    if platform == Platform::Windows {
        path.replace('/', "\\")
    } else {
        path
    }
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
        // Windows validator downloads are zip archives. Windows ships bsdtar as `tar`
        // with zip support, the same tool that extracts clap-validator, so self-hosted
        // runners do not need 7-Zip installed.
        run_with_language(
            Command::new("tar")
                .arg("-xf")
                .arg(&archive)
                .arg("-C")
                .arg(&extracted_root)
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
    // The caller resolves its own configuration so this operation has a single archive source.
    let archive = config_path(ctx, ctx.aax_validator_dsh_archive.as_ref())
        .ok_or("AAX validator/DSH archive path is not configured")?;
    if !archive.is_file() {
        return Err(format!("AAX validator/DSH archive not found: {}", archive.display()).into());
    }
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

#[cfg(test)]
mod tests {
    use super::is_aax_validator_dish_load_failure;

    // The retry condition can only be observed on a flaky runner, so pin it to the
    // DTT output recorded there and make sure plug-in failures never match it.
    #[test]
    fn dish_load_failure_matches_only_aaxval_dish_load_errors() {
        let dish_load_failure =
            b"FAILED: ValidatorRunAllTests - DSH command 'load_dish' failed with exception:\n\
            ---\n\
            exception_message: ' (-23) while loading \"aaxval.dish\"'\n";
        assert!(is_aax_validator_dish_load_failure(dish_load_failure));

        let plugin_failure =
            b"FAILED: ValidatorRunAllTests - DSH command 'runtest' failed with exception:\n";
        assert!(!is_aax_validator_dish_load_failure(plugin_failure));
    }
}
