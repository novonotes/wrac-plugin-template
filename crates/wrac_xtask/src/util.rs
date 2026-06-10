use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::Result;

pub(crate) fn copy_path(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        copy_dir(from, to)?;
    } else {
        fs::copy(from, to)?;
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            copy_dir(&source, &destination)?;
        } else {
            fs::copy(&source, &destination)?;
        }
    }
    Ok(())
}

pub(crate) fn ensure_exists(path: &Path, description: &str) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(format!("{description} not found: {}", path.display()).into())
    }
}

pub(crate) fn run(command: &mut Command) -> Result<()> {
    // xtask is a build orchestrator, so seeing the exact external command on failure is important.
    // Run directly via Command without a shell, but print in a form that humans can re-run easily.
    println!();
    println!("========== Running command ==========");
    println!("$ {}", format_command(command));
    let status = command.status()?;
    if !status.success() {
        return Err(format!(
            "command failed with status {status}: {}",
            format_command(command)
        )
        .into());
    }
    Ok(())
}

pub(crate) fn run_with_optional_xcbeautify(command: &mut Command) -> Result<()> {
    if !command_exists("xcbeautify") {
        return run(command);
    }

    println!();
    println!("========== Running command ==========");
    println!("$ {} 2>&1 | xcbeautify", format_command(command));

    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture command stdout")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("failed to capture command stderr")?;

    let mut formatter = Command::new("xcbeautify").stdin(Stdio::piped()).spawn()?;
    let formatter_stdin = formatter
        .stdin
        .take()
        .ok_or("failed to open xcbeautify stdin")?;
    let formatter_stdin = Arc::new(Mutex::new(formatter_stdin));

    let stdout_thread = pipe_to_formatter(stdout, Arc::clone(&formatter_stdin));
    let stderr_thread = pipe_to_formatter(stderr, Arc::clone(&formatter_stdin));

    let status = child.wait()?;
    stdout_thread
        .join()
        .map_err(|_| "stdout pipe thread panicked")??;
    stderr_thread
        .join()
        .map_err(|_| "stderr pipe thread panicked")??;
    drop(formatter_stdin);

    let formatter_status = formatter.wait()?;
    if !status.success() {
        return Err(format!(
            "command failed with status {status}: {}",
            format_command(command)
        )
        .into());
    }
    if !formatter_status.success() {
        return Err(format!("xcbeautify failed with status {formatter_status}").into());
    }
    Ok(())
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {command} >/dev/null 2>&1")])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn pipe_to_formatter<R>(
    mut reader: R,
    formatter_stdin: Arc<Mutex<std::process::ChildStdin>>,
) -> thread::JoinHandle<io::Result<()>>
where
    R: io::Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0; 8192];
        loop {
            let bytes_read = io::Read::read(&mut reader, &mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            let mut writer = formatter_stdin
                .lock()
                .map_err(|_| io::Error::other("xcbeautify stdin lock poisoned"))?;
            writer.write_all(&buffer[..bytes_read])?;
        }
        Ok(())
    })
}

pub(crate) fn run_output(command: &mut Command) -> Result<Output> {
    println!();
    println!("========== Running command ==========");
    println!("$ {}", format_command(command));
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "command failed with status {}: {}",
            output.status,
            format_command(command)
        )
        .into());
    }
    Ok(output)
}

fn format_command(command: &Command) -> String {
    let mut parts = Vec::new();
    parts.push(shell_display(command.get_program()));
    parts.extend(command.get_args().map(shell_display));
    parts.join(" ")
}

fn shell_display(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.into_owned()
    }
}

pub(crate) fn remove_if_exists(path: &Path) -> Result<()> {
    // clean/install should be idempotent regardless of how many times they are run.
    // Treating a missing path as success makes re-runs after partial failures and dry environments straightforward.
    if !path.exists() {
        return Ok(());
    }

    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(crate) fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}

pub(crate) fn local_app_data() -> Result<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA is not set".into())
}

pub(crate) fn common_program_files() -> Result<PathBuf> {
    env::var_os("CommonProgramFiles")
        .map(PathBuf::from)
        .ok_or_else(|| "CommonProgramFiles is not set".into())
}

pub(crate) fn env_value_or(name: &str, fallback: &str) -> String {
    env::var(name).unwrap_or_else(|_| fallback.to_owned())
}

pub(crate) fn on_off(value: bool) -> &'static str {
    if value { "ON" } else { "OFF" }
}
