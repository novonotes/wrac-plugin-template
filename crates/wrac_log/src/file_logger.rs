use env_logger::{Builder, Target};
use log::LevelFilter;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once, OnceLock};
use time::{OffsetDateTime, macros::format_description};

const MAX_LOG_FILES: usize = 30;
const DEFAULT_RECENT_LOG_MAX_FILES: usize = 30;
const DEFAULT_RECENT_LOG_MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
const MAX_UNIQUE_ARCHIVED_LOG_FILE_ATTEMPTS: u32 = 1000;

static INIT: Once = Once::new();
static CURRENT_LOG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
static CURRENT_LOG_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Implementation for [`crate::init!`].
///
/// Prefer calling the macro so `manifest_dir` is captured from the plugin crate,
/// not from `wrac_log`.
pub fn init_impl(manifest_dir: Option<&'static str>, app_name: &str) {
    INIT.call_once(|| {
        let dotenv_rust_log = rust_log_from_debug_dotenv(manifest_dir);

        // Explicit environment configuration is useful for host-driven debugging and CI.
        if let Ok(log_dir) = std::env::var("WRAC_LOG_DIR") {
            init_with_dir(&log_dir, app_name, dotenv_rust_log.as_deref());
            return;
        }

        // Debug builds should make local development logs easy to find in the repository.
        #[cfg(debug_assertions)]
        if let Some(manifest_dir) = manifest_dir {
            let log_dir = Path::new(manifest_dir).join("../.log");
            if let Some(log_dir_str) = log_dir.to_str() {
                init_with_dir(log_dir_str, app_name, dotenv_rust_log.as_deref());
                return;
            }
        }

        // Release builds use a user log directory so installed plugins can keep logs
        // without depending on the build tree.
        #[cfg(not(debug_assertions))]
        {
            let _ = manifest_dir;
            if let Some(log_dir) = resolve_release_log_dir(app_name) {
                init_with_dir(log_dir.to_string_lossy().as_ref(), app_name, None);
                return;
            }
        }

        #[cfg(debug_assertions)]
        let _ = app_name;
        init_stderr(dotenv_rust_log.as_deref());
    });
}

/// Returns the directory currently used for file logging.
///
/// Returns `None` before initialization or when logging fell back to `stderr`.
/// Use [`current_log_file`] when the caller needs the exact current session log.
pub fn current_log_dir() -> Option<PathBuf> {
    CURRENT_LOG_DIR.get().cloned().flatten()
}

/// Returns the current session log file.
///
/// Returns `None` before initialization or when logging fell back to `stderr`.
pub fn current_log_file() -> Option<PathBuf> {
    CURRENT_LOG_FILE.get().cloned().flatten()
}

/// Limits used when collecting recent log files for diagnostics.
#[derive(Clone, Debug)]
pub struct RecentLogFilesOptions {
    /// Maximum number of files to include, including the current log.
    pub max_files: usize,
    /// Maximum total byte size to include. The current log is always included.
    pub max_total_bytes: u64,
}

impl Default for RecentLogFilesOptions {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_RECENT_LOG_MAX_FILES,
            max_total_bytes: DEFAULT_RECENT_LOG_MAX_TOTAL_BYTES,
        }
    }
}

/// Returns the current log and recent archived logs, newest first.
///
/// The current log is always included even if it exceeds `max_total_bytes`.
/// Archived logs are then added newest first until the file or byte limit is reached.
pub fn collect_recent_log_files(options: RecentLogFilesOptions) -> std::io::Result<Vec<PathBuf>> {
    let current_log_file = current_log_file()
        .ok_or_else(|| std::io::Error::other("wrac_log is not writing to a log file"))?;
    collect_recent_log_files_from_current(&current_log_file, &options)
}

fn collect_recent_log_files_from_current(
    current_log_file: &Path,
    options: &RecentLogFilesOptions,
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

/// Initializes logging for tests.
///
/// In debug builds, `WRAC_LOG_DIR` creates a per-test timestamped log file. Without
/// that environment variable, logs go to `stderr`. Initialization is idempotent.
pub fn init_test() {
    #[cfg(debug_assertions)]
    INIT.call_once(|| {
        if let Ok(log_dir) = std::env::var("WRAC_LOG_DIR") {
            let test_name = get_test_name();
            let timestamp = get_timestamp();
            let log_file = format!("{log_dir}/{test_name}_{timestamp}.log");
            init_with_file(&log_file, None);
        } else {
            init_stderr(None);
        }
    });
}

fn init_with_dir(log_dir: &str, app_name: &str, dotenv_rust_log: Option<&str>) {
    let log_dir_path = Path::new(log_dir);
    if !log_dir_path.exists()
        && let Err(error) = std::fs::create_dir_all(log_dir_path)
    {
        eprintln!("Failed to create log directory '{log_dir}': {error}");
        init_stderr(dotenv_rust_log);
        return;
    }

    let file_stem = log_file_stem(app_name);
    let latest_log_file = latest_log_file_path(log_dir_path, &file_stem);
    // Keep a stable Latest filename for users while preserving the previous session
    // before the new logger appends current-session output.
    if let Err(error) = archive_existing_latest_log(&latest_log_file, &file_stem) {
        eprintln!(
            "Failed to archive latest log file '{}': {error}",
            latest_log_file.display(),
        );
    }
    rotate_logs(log_dir_path, &file_stem);
    init_with_file(&latest_log_file, dotenv_rust_log);
}

fn rotate_logs(log_dir: &Path, file_stem: &str) {
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

fn latest_log_file_path(log_dir: &Path, file_stem: &str) -> PathBuf {
    log_dir.join(format!("{file_stem} Latest.log"))
}

fn archive_existing_latest_log(latest_log_file: &Path, file_stem: &str) -> std::io::Result<()> {
    if !latest_log_file.exists() {
        return Ok(());
    }

    let Some(log_dir) = latest_log_file.parent() else {
        return Ok(());
    };
    std::fs::rename(
        latest_log_file,
        unique_archived_log_file_path(log_dir, file_stem)?,
    )
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

fn is_archived_log_file_name(file_name: &str, file_stem: &str) -> bool {
    file_name.starts_with(&format!("{file_stem} "))
        && file_name.ends_with(".log")
        && file_name != format!("{file_stem} Latest.log")
}

fn log_file_stem(app_name: &str) -> String {
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

#[cfg(debug_assertions)]
fn rust_log_from_debug_dotenv(manifest_dir: Option<&str>) -> Option<String> {
    if std::env::var("RUST_LOG").is_ok() {
        return None;
    }

    let dotenv_path = debug_dotenv_path(manifest_dir?)?;
    let Ok(content) = std::fs::read_to_string(&dotenv_path) else {
        return None;
    };
    parse_dotenv_rust_log(&content)
}

#[cfg(not(debug_assertions))]
fn rust_log_from_debug_dotenv(manifest_dir: Option<&str>) -> Option<String> {
    let _ = manifest_dir;
    None
}

#[cfg(debug_assertions)]
fn debug_dotenv_path(manifest_dir: &str) -> Option<PathBuf> {
    let start = Path::new(manifest_dir);
    let mut fallback = None;

    for ancestor in start.ancestors() {
        let candidate = ancestor.join(".env");
        if ancestor.join(".git").exists() {
            if candidate.is_file() {
                return Some(candidate);
            }
            break;
        }
        if fallback.is_none() && candidate.is_file() {
            fallback = Some(candidate);
        }
    }
    fallback
}

#[cfg(debug_assertions)]
fn parse_dotenv_rust_log(content: &str) -> Option<String> {
    let mut rust_log = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "RUST_LOG" {
            continue;
        }

        let value = parse_dotenv_value(value.trim());
        if !value.is_empty() {
            rust_log = Some(value);
        }
    }
    rust_log
}

#[cfg(debug_assertions)]
fn parse_dotenv_value(value: &str) -> String {
    if let Some(stripped) = value.strip_prefix('"') {
        if let Some(end) = stripped.find('"') {
            return stripped[..end].to_string();
        }
    } else if let Some(stripped) = value.strip_prefix('\'')
        && let Some(end) = stripped.find('\'')
    {
        return stripped[..end].to_string();
    }

    value
        .split_once(" #")
        .map(|(value, _)| value.trim_end())
        .unwrap_or(value)
        .to_string()
}

#[cfg(not(debug_assertions))]
/// Resolves the release-build default log directory for the current platform.
///
/// Each platform stores logs under a `NovoNotes/{app_name}` directory so installed
/// plugins keep separate user-facing logs.
fn resolve_release_log_dir(app_name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        return Some(
            PathBuf::from(home)
                .join("Library")
                .join("Logs")
                .join("NovoNotes")
                .join(app_name),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA")?;
        return Some(
            PathBuf::from(local_app_data)
                .join("NovoNotes")
                .join("Logs")
                .join(app_name),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })?;
        return Some(base.join("NovoNotes").join("logs").join(app_name));
    }

    #[allow(unreachable_code)]
    None
}

fn init_stderr(dotenv_rust_log: Option<&str>) {
    record_current_log_paths(None);
    announce_log_output("stderr");
    let mut builder = Builder::from_default_env();
    apply_default_filter(&mut builder, dotenv_rust_log);
    builder.target(Target::Stderr);
    let _ = builder.try_init();
    crate::rt::start_drain_if_enabled();
}

fn init_with_file(log_file: impl AsRef<Path>, dotenv_rust_log: Option<&str>) {
    let log_file = log_file.as_ref();
    announce_log_output(&log_file.to_string_lossy());

    if let Some(parent) = log_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match OpenOptions::new().create(true).append(true).open(log_file) {
        Ok(file) => {
            let canonical_log_file = log_file
                .canonicalize()
                .unwrap_or_else(|_| log_file.to_path_buf());
            record_current_log_paths(Some(canonical_log_file));
            let mut builder = Builder::from_default_env();
            apply_default_filter(&mut builder, dotenv_rust_log);
            builder.target(Target::Pipe(Box::new(FileAndStderr::new(file))));
            let _ = builder.try_init();
            crate::rt::start_drain_if_enabled();
        }
        Err(error) => {
            eprintln!("Failed to open log file '{}': {error}", log_file.display());
            init_stderr(dotenv_rust_log);
        }
    }
}

fn record_current_log_paths(log_file: Option<PathBuf>) {
    let log_dir = log_file
        .as_ref()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let _ = CURRENT_LOG_FILE.set(log_file);
    let _ = CURRENT_LOG_DIR.set(log_dir);
}

fn announce_log_output(destination: &str) {
    eprintln!("[wrac_log] output={destination}");
}

fn apply_default_filter(builder: &mut Builder, dotenv_rust_log: Option<&str>) {
    if std::env::var("RUST_LOG").is_err() {
        #[cfg(debug_assertions)]
        if let Some(rust_log) = dotenv_rust_log.filter(|value| !value.trim().is_empty()) {
            builder.parse_filters(rust_log);
            return;
        }

        builder.filter_level(default_level_filter());
    }
}

fn default_level_filter() -> LevelFilter {
    #[cfg(debug_assertions)]
    {
        LevelFilter::Debug
    }
    #[cfg(not(debug_assertions))]
    {
        LevelFilter::Info
    }
}

struct FileAndStderr {
    file: Arc<Mutex<std::fs::File>>,
}

impl FileAndStderr {
    fn new(file: std::fs::File) -> Self {
        Self {
            file: Arc::new(Mutex::new(file)),
        }
    }
}

impl Write for FileAndStderr {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::io::stderr().write_all(buf)?;
        let mut file = self.file.lock().unwrap();
        file.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()?;
        let mut file = self.file.lock().unwrap();
        file.flush()
    }
}

fn get_test_name() -> String {
    std::thread::current()
        .name()
        .unwrap_or("unknown_test")
        .replace("::", "_")
        .replace(' ', "_")
}

fn get_timestamp() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let format = format_description!("[year][month][day]_[hour][minute][second]");
    let timestamp = now
        .format(format)
        .unwrap_or_else(|_| now.unix_timestamp().to_string());
    format!("{timestamp}_{:03}", now.millisecond())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn init_test_is_idempotent() {
        init_test();
        init_test();
    }

    #[test]
    fn test_name_and_timestamp_are_available() {
        assert!(get_test_name().contains("test"));

        let timestamp = get_timestamp();
        assert_eq!(timestamp.len(), 19);
        assert_eq!(timestamp.chars().nth(8).unwrap(), '_');
        assert_eq!(timestamp.chars().nth(15).unwrap(), '_');
    }

    #[test]
    fn logging_is_thread_safe_after_initialization() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::thread;

        let counter = Arc::new(AtomicUsize::new(0));
        let handles = (0..10)
            .map(|i| {
                let counter = counter.clone();
                thread::spawn(move || {
                    for j in 0..100 {
                        log::info!("Thread {i} - Message {j}");
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(counter.load(Ordering::SeqCst), 1000);
    }

    #[test]
    fn log_file_stem_replaces_path_unsafe_characters() {
        assert_eq!(log_file_stem("TestApp"), "TestApp");
        assert_eq!(log_file_stem("Bad/Name:Plugin"), "Bad_Name_Plugin");
        assert_eq!(log_file_stem("   "), "Application");
    }

    #[test]
    fn archive_existing_latest_log_moves_latest_to_timestamped_log() {
        let temp_dir = TempDir::new("wrac_log_archive_latest");
        let latest = temp_dir.path().join("TestApp Latest.log");
        std::fs::write(&latest, "previous session").unwrap();

        archive_existing_latest_log(&latest, "TestApp").unwrap();

        assert!(!latest.exists());
        let archived = log_files(temp_dir.path());
        assert_eq!(archived.len(), 1);
        let archived_name = archived[0].file_name().unwrap().to_string_lossy();
        assert!(archived_name.starts_with("TestApp "));
        assert!(archived_name.ends_with(".log"));
        assert_ne!(archived_name, "TestApp Latest.log");
        assert_eq!(
            std::fs::read_to_string(&archived[0]).unwrap(),
            "previous session",
        );
    }

    #[test]
    fn collect_recent_log_files_includes_latest_first_and_respects_limit() {
        let temp_dir = TempDir::new("wrac_log_collect_recent");
        let latest = temp_dir.path().join("TestApp Latest.log");
        let archived1 = temp_dir.path().join("TestApp 20260101_000000_000.log");
        let archived2 = temp_dir.path().join("TestApp 20260102_000000_000.log");
        let other = temp_dir.path().join("Other 20260103_000000_000.log");
        std::fs::write(&latest, "latest").unwrap();
        std::fs::write(&archived1, "archived1").unwrap();
        std::fs::write(&archived2, "archived2").unwrap();
        std::fs::write(&other, "other").unwrap();

        let files = collect_recent_log_files_from_current(
            &latest,
            &RecentLogFilesOptions {
                max_files: 2,
                max_total_bytes: 1024,
            },
        )
        .unwrap();

        assert_eq!(files.len(), 2);
        assert_eq!(files[0], latest);
        assert!(files[1] == archived1 || files[1] == archived2);
        assert!(!files.contains(&other));
    }

    #[test]
    fn rotate_logs_keeps_max_archived_logs() {
        let temp_dir = TempDir::new("wrac_log_rotate");
        for index in 0..(MAX_LOG_FILES + 2) {
            let file = temp_dir
                .path()
                .join(format!("TestApp 20260101_000000_{index:03}.log"));
            std::fs::write(file, format!("log {index}")).unwrap();
        }
        std::fs::write(temp_dir.path().join("TestApp Latest.log"), "latest").unwrap();

        rotate_logs(temp_dir.path(), "TestApp");

        let archived = log_files(temp_dir.path())
            .into_iter()
            .filter(|path| path.file_name().unwrap().to_string_lossy() != "TestApp Latest.log")
            .collect::<Vec<_>>();
        assert_eq!(archived.len(), MAX_LOG_FILES);
        assert!(temp_dir.path().join("TestApp Latest.log").exists());
    }

    #[test]
    fn parse_dotenv_rust_log_reads_last_non_empty_rust_log() {
        let content = r#"
            # wrac_log reads this only in development builds
            OTHER=value
            export RUST_LOG="wrac_gain_plugin=debug,wrac_log=trace"
            RUST_LOG=wrac_gain_plugin=info # the last definition wins
        "#;

        assert_eq!(
            parse_dotenv_rust_log(content).as_deref(),
            Some("wrac_gain_plugin=info"),
        );
    }

    #[test]
    fn parse_dotenv_rust_log_ignores_empty_rust_log() {
        assert_eq!(parse_dotenv_rust_log("RUST_LOG=\n"), None);
    }

    #[test]
    fn debug_dotenv_path_prefers_repository_root() {
        let temp_dir = TempDir::new("wrac_log_dotenv_root");
        std::fs::create_dir(temp_dir.path().join(".git")).unwrap();
        std::fs::write(temp_dir.path().join(".env"), "RUST_LOG=info").unwrap();

        let crate_dir = temp_dir.path().join("plugins").join("gain");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(crate_dir.join(".env"), "RUST_LOG=trace").unwrap();

        let expected = temp_dir.path().join(".env");
        assert_eq!(
            debug_dotenv_path(crate_dir.to_str().unwrap()).as_deref(),
            Some(expected.as_path()),
        );
    }

    #[test]
    fn debug_dotenv_path_falls_back_to_nearest_dotenv_when_repository_root_has_none() {
        let temp_dir = TempDir::new("wrac_log_dotenv_fallback");
        std::fs::create_dir(temp_dir.path().join(".git")).unwrap();

        let crate_dir = temp_dir.path().join("plugins").join("gain");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(crate_dir.join(".env"), "RUST_LOG=trace").unwrap();

        let expected = crate_dir.join(".env");
        assert_eq!(
            debug_dotenv_path(crate_dir.to_str().unwrap()).as_deref(),
            Some(expected.as_path()),
        );
    }

    #[test]
    fn parse_dotenv_rust_log_strips_comment_after_quoted_value() {
        assert_eq!(
            parse_dotenv_rust_log(r#"RUST_LOG="debug" # comment"#).as_deref(),
            Some("debug"),
        );
        assert_eq!(
            parse_dotenv_rust_log("RUST_LOG='wrac_gain_plugin=trace' # comment").as_deref(),
            Some("wrac_gain_plugin=trace"),
        );
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}_{nanos}"));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn log_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
            .collect::<Vec<_>>();
        files.sort();
        files
    }
}
