use std::path::{Path, PathBuf};

pub(super) const MAX_LOG_FILES: usize = 30;
pub(super) const DEFAULT_RECENT_LOG_MAX_FILES: usize = 30;
pub(super) const DEFAULT_RECENT_LOG_MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
const MAX_UNIQUE_ARCHIVED_LOG_FILE_ATTEMPTS: u32 = 1000;

use super::get_timestamp;

pub(super) fn collect_recent_log_files_from_current(
    current_log_file: &Path,
    options: &super::RecentLogFilesOptions,
) -> std::io::Result<Vec<PathBuf>> {
    let Some(log_dir) = current_log_file.parent() else {
        return Ok(Vec::new());
    };
    let Some(current_log_file_name) = current_log_file.file_name().and_then(|name| name.to_str())
    else {
        return Ok(Vec::new());
    };
    let Some(file_stem) = current_log_file_name.strip_suffix(" Latest.log") else {
        return Ok(vec![current_log_file.to_path_buf()]);
    };

    let mut archived_logs = Vec::new();
    for entry in std::fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path == current_log_file {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_archived_log_file_name(file_name, file_stem) {
            continue;
        }
        let modified = entry.metadata()?.modified()?;
        archived_logs.push((modified, path));
    }
    archived_logs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));

    // After a host crash, the crashed session's previous Latest log becomes an archived
    // log on the next launch. Include recent archives so diagnostic bundles can still
    // capture the failure that happened before the current session.
    let mut selected = vec![current_log_file.to_path_buf()];
    selected.extend(archived_logs.into_iter().map(|(_, path)| path));
    selected.truncate(options.max_files.max(1));

    // The current session describes the user's current state and is always included.
    // Older sessions are included newest first while respecting the total size limit.
    let mut total_bytes = 0_u64;
    let mut limited = Vec::new();
    for path in selected {
        let size = std::fs::metadata(&path)?.len();
        if limited.is_empty() || total_bytes.saturating_add(size) <= options.max_total_bytes {
            total_bytes = total_bytes.saturating_add(size);
            limited.push(path);
        }
    }
    Ok(limited)
}

pub(super) fn rotate_logs(log_dir: &Path, file_stem: &str) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };

    let mut log_files = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| is_archived_log_file_name(&entry.file_name().to_string_lossy(), file_stem))
        .collect::<Vec<_>>();
    if log_files.len() <= MAX_LOG_FILES {
        return;
    }

    // Rotate by modification time so the newest archived logs survive even if a
    // timestamped filename was created by a system clock with low precision.
    log_files.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    let files_to_delete = log_files.len() - MAX_LOG_FILES;
    for entry in log_files.into_iter().take(files_to_delete) {
        let _ = std::fs::remove_file(entry.path());
    }
}

pub(super) fn latest_log_file_path(log_dir: &Path, file_stem: &str) -> PathBuf {
    log_dir.join(format!("{file_stem} Latest.log"))
}

pub(super) fn archive_existing_latest_log(
    latest_log_file: &Path,
    file_stem: &str,
) -> std::io::Result<()> {
    if !latest_log_file.exists() {
        return Ok(());
    }

    let Some(log_dir) = latest_log_file.parent() else {
        return Ok(());
    };
    match std::fs::rename(
        latest_log_file,
        unique_archived_log_file_path(log_dir, file_stem)?,
    ) {
        Ok(()) => Ok(()),
        // Validators and plugin scanners can create multiple short-lived plugin
        // processes at once. Another process may archive the same Latest log after
        // our exists check, which is already a successful outcome for this session.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn unique_archived_log_file_path(log_dir: &Path, file_stem: &str) -> std::io::Result<PathBuf> {
    let timestamp = get_timestamp();
    let first = log_dir.join(format!("{file_stem} {timestamp}.log"));
    if !first.exists() {
        return Ok(first);
    }

    // Fast restarts or coarse system clocks can collide on the same timestamp. Bound
    // the suffix search so an abnormal directory state cannot turn archive creation
    // into an infinite loop.
    for index in 1..MAX_UNIQUE_ARCHIVED_LOG_FILE_ATTEMPTS {
        let candidate = log_dir.join(format!("{file_stem} {timestamp}-{index}.log"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!(
            "failed to find a unique archived log file name for '{file_stem}' after {MAX_UNIQUE_ARCHIVED_LOG_FILE_ATTEMPTS} attempts",
        ),
    ))
}

pub(super) fn is_archived_log_file_name(file_name: &str, file_stem: &str) -> bool {
    file_name.starts_with(&format!("{file_stem} "))
        && file_name.ends_with(".log")
        && file_name != format!("{file_stem} Latest.log")
}

pub(super) fn log_file_stem(app_name: &str) -> String {
    // The app name is also user-visible in the log filename. Replace only characters
    // that are unsafe or awkward across the major target filesystems.
    let sanitized = app_name
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect::<String>()
        .trim()
        .to_string();

    if sanitized.is_empty() {
        "Application".to_string()
    } else {
        sanitized
    }
}
