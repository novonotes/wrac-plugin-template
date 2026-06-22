use env_logger::{Builder, Logger, Target};
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock, mpsc};
use std::thread::JoinHandle;
use time::{OffsetDateTime, macros::format_description};

const MAX_LOG_FILES: usize = 30;
const DEFAULT_RECENT_LOG_MAX_FILES: usize = 30;
const DEFAULT_RECENT_LOG_MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
const MAX_UNIQUE_ARCHIVED_LOG_FILE_ATTEMPTS: u32 = 1000;
const ASYNC_LOG_QUEUE_CAPACITY: usize = 4096;
const ASYNC_MODE: u8 = 0;
const BLOCKING_MODE: u8 = 1;

static INIT: Once = Once::new();
static CURRENT_LOG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
static CURRENT_LOG_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
static RT_SAFE_LOGGER: OnceLock<&'static WracLogger> = OnceLock::new();
static FILE_WRITER: OnceLock<LazyFileWriter> = OnceLock::new();
static ACTIVE_ASYNC_FILE_LOGGERS: AtomicU32 = AtomicU32::new(0);

/// WRAC file logging configuration.
#[derive(Clone, Copy, Debug)]
pub struct LogConfig {
    pub app_name: &'static str,
    pub output: LogOutput,
    debug_dotenv_search_dir: Option<&'static str>,
}

/// WRAC file logging destination policy.
#[derive(Clone, Copy, Debug)]
pub enum LogOutput {
    /// Uses `WRAC_LOG_DIR`, then the debug directory in debug builds, otherwise the platform log directory.
    DefaultPluginLogDir {
        debug_log_dir: Option<&'static str>,
    },
    Directory(&'static str),
    File(&'static str),
    Stderr,
}

impl LogConfig {
    /// Creates a standard WRAC file-log configuration.
    pub const fn new(app_name: &'static str, debug_log_dir: Option<&'static str>) -> Self {
        Self {
            app_name,
            output: LogOutput::DefaultPluginLogDir { debug_log_dir },
            debug_dotenv_search_dir: debug_log_dir,
        }
    }
}

/// Configures logging for a plugin managed by a host adapter.
///
/// This installs the lightweight logger without touching the filesystem.
pub fn configure_plugin(config: &'static LogConfig) -> PluginLogRuntime {
    PluginLogRuntime {
        writer: configure_logger(config),
    }
}

/// Configures logging for a standalone app or service.
///
/// Hold the returned runtime while async file logging should remain active.
pub fn configure_standalone(config: &'static LogConfig) -> StandaloneLogRuntime {
    let writer = configure_logger(config);
    StandaloneLogRuntime {
        _guard: writer.map(start_async_file_logger),
    }
}

fn configure_logger(config: &'static LogConfig) -> Option<LazyFileWriter> {
    INIT.call_once(|| {
        let writer = LazyFileWriter::new(*config);
        let writer_for_runtime = writer.clone();
        if let Some(writer) = install_lazy_logger(writer, writer_for_runtime) {
            let _ = FILE_WRITER.set(writer);
        }
        crate::rt::init_rt_buffer();
    });
    FILE_WRITER.get().cloned()
}

/// Returns the directory currently used for file logging.
pub fn current_log_dir() -> Option<PathBuf> {
    CURRENT_LOG_DIR.get().cloned().flatten()
}

/// Returns the current session log file.
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
pub fn collect_recent_log_files(options: RecentLogFilesOptions) -> std::io::Result<Vec<PathBuf>> {
    let current_log_file = current_log_file()
        .ok_or_else(|| std::io::Error::other("wrac_log is not writing to a log file"))?;
    collect_recent_log_files_from_current(&current_log_file, &options)
}

/// Plugin logger runtime returned by [`configure_plugin`].
pub struct PluginLogRuntime {
    writer: Option<LazyFileWriter>,
}

impl PluginLogRuntime {
    /// Keeps async file logging active while the returned instance guard is alive.
    pub fn retain_instance(&self) -> PluginLogInstanceGuard {
        PluginLogInstanceGuard {
            _guard: self.writer.clone().map(start_async_file_logger),
        }
    }
}

/// Keeps plugin async file logging active for one plugin instance.
pub struct PluginLogInstanceGuard {
    _guard: Option<AsyncFileLoggerGuard>,
}

/// Keeps standalone async file logging active.
pub struct StandaloneLogRuntime {
    _guard: Option<AsyncFileLoggerGuard>,
}

/// Keeps the asynchronous file writer running while this guard is alive.
struct AsyncFileLoggerGuard {
    writer: LazyFileWriter,
}

fn start_async_file_logger(writer: LazyFileWriter) -> AsyncFileLoggerGuard {
    if ACTIVE_ASYNC_FILE_LOGGERS.fetch_add(1, Ordering::AcqRel) == 0 {
        writer.ensure_initialized();
        writer.start();
    }
    AsyncFileLoggerGuard { writer }
}

impl Drop for AsyncFileLoggerGuard {
    fn drop(&mut self) {
        if ACTIVE_ASYNC_FILE_LOGGERS.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.writer.shutdown();
        }
    }
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
fn rust_log_from_debug_dotenv(search_dir: Option<&Path>) -> Option<String> {
    if std::env::var("RUST_LOG").is_ok() {
        return None;
    }

    let dotenv_path = debug_dotenv_path(search_dir?)?;
    let Ok(content) = std::fs::read_to_string(&dotenv_path) else {
        return None;
    };
    parse_dotenv_rust_log(&content)
}

#[cfg(not(debug_assertions))]
fn rust_log_from_debug_dotenv(search_dir: Option<&Path>) -> Option<String> {
    let _ = search_dir;
    None
}

#[cfg(debug_assertions)]
fn debug_dotenv_path(search_dir: &Path) -> Option<PathBuf> {
    let mut fallback = None;

    for ancestor in search_dir.ancestors() {
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
    install_logger(builder, None);
    crate::rt::init_rt_buffer();
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
            let writer = LazyFileWriter::from_open_file(file);
            let writer_for_shutdown = writer.clone();
            builder.target(Target::Pipe(Box::new(writer)));
            if let Some(writer) = install_logger(builder, Some(writer_for_shutdown)) {
                let _ = FILE_WRITER.set(writer);
            }
            crate::rt::init_rt_buffer();
        }
        Err(error) => {
            eprintln!("Failed to open log file '{}': {error}", log_file.display());
            init_stderr(dotenv_rust_log);
        }
    }
}

fn initialize_log_destination(config: LogConfig) -> LogDestination {
    match resolve_log_file_path(&config) {
        Some(log_file) => {
            let Some(file_stem) = log_file
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(" Latest.log"))
                .map(str::to_string)
            else {
                return open_log_file_destination(&log_file);
            };
            if let Some(log_dir) = log_file.parent() {
                if !log_dir.exists()
                    && let Err(error) = std::fs::create_dir_all(log_dir)
                {
                    let _ = writeln!(
                        std::io::stderr(),
                        "Failed to create log directory '{}': {error}",
                        log_dir.display()
                    );
                    record_current_log_paths(None);
                    return LogDestination::Stderr;
                }
                if let Err(error) = archive_existing_latest_log(&log_file, &file_stem) {
                    let _ = writeln!(
                        std::io::stderr(),
                        "Failed to archive latest log file '{}': {error}",
                        log_file.display()
                    );
                }
                rotate_logs(log_dir, &file_stem);
            }
            open_log_file_destination(&log_file)
        }
        None => {
            record_current_log_paths(None);
            LogDestination::Stderr
        }
    }
}

fn resolve_log_file_path(config: &LogConfig) -> Option<PathBuf> {
    match &config.output {
        LogOutput::Stderr => None,
        LogOutput::File(path) => Some(PathBuf::from(path)),
        LogOutput::Directory(path) => Some(latest_log_file_path(
            Path::new(path),
            &log_file_stem(config.app_name),
        )),
        LogOutput::DefaultPluginLogDir { debug_log_dir } => {
            if let Ok(log_dir) = std::env::var("WRAC_LOG_DIR") {
                return Some(latest_log_file_path(
                    Path::new(&log_dir),
                    &log_file_stem(config.app_name),
                ));
            }
            #[cfg(debug_assertions)]
            {
                debug_log_dir.map(|log_dir| {
                    latest_log_file_path(Path::new(log_dir), &log_file_stem(config.app_name))
                })
            }
            #[cfg(not(debug_assertions))]
            {
                let _ = debug_log_dir;
                resolve_release_log_dir(config.app_name)
                    .map(|log_dir| latest_log_file_path(&log_dir, &log_file_stem(config.app_name)))
            }
        }
    }
}

fn open_log_file_destination(log_file: &Path) -> LogDestination {
    if let Some(parent) = log_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match OpenOptions::new().create(true).append(true).open(log_file) {
        Ok(file) => {
            let canonical_log_file = log_file
                .canonicalize()
                .unwrap_or_else(|_| log_file.to_path_buf());
            record_current_log_paths(Some(canonical_log_file));
            LogDestination::File(Arc::new(Mutex::new(file)))
        }
        Err(error) => {
            let _ = writeln!(
                std::io::stderr(),
                "Failed to open log file '{}': {error}",
                log_file.display()
            );
            record_current_log_paths(None);
            LogDestination::Stderr
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

fn build_file_logger(config: LogConfig, writer: LazyFileWriter) -> Logger {
    let dotenv_rust_log = rust_log_from_debug_dotenv(config.debug_dotenv_search_dir.map(Path::new));
    let mut builder = Builder::from_default_env();
    apply_default_filter(&mut builder, dotenv_rust_log.as_deref());
    builder.target(Target::Pipe(Box::new(writer)));
    builder.build()
}

fn install_lazy_logger(
    writer: LazyFileWriter,
    file_writer: LazyFileWriter,
) -> Option<LazyFileWriter> {
    let config = writer.config_for_logger();
    let logger = WracLogger {
        inner: LoggerInner::Lazy {
            logger: OnceLock::new(),
            config,
            writer,
        },
        fallback_max_level: default_level_filter(),
    };
    crate::rt::set_rt_fallback_max_level(logger.fallback_max_level);
    install_wrac_logger(logger, Some(file_writer), LevelFilter::Trace)
}

fn install_logger(
    mut builder: Builder,
    file_writer: Option<LazyFileWriter>,
) -> Option<LazyFileWriter> {
    let inner = builder.build();
    let max_level = inner.filter();
    let logger = WracLogger {
        inner: LoggerInner::Immediate(inner),
        fallback_max_level: max_level,
    };
    crate::rt::set_rt_fallback_max_level(max_level);
    install_wrac_logger(logger, file_writer, max_level)
}

fn install_wrac_logger(
    logger: WracLogger,
    file_writer: Option<LazyFileWriter>,
    max_level: LevelFilter,
) -> Option<LazyFileWriter> {
    let logger = Box::new(logger);
    let logger_ptr = Box::into_raw(logger);
    // `log` stores a `'static` logger reference. On failure, another logger is already
    // installed, so RT logging must avoid calling through that unknown implementation.
    let set_logger_result = unsafe { log::set_logger(&*logger_ptr) };
    match set_logger_result {
        Ok(()) => {
            let logger = unsafe { &*logger_ptr };
            let _ = RT_SAFE_LOGGER.set(logger);
            log::set_max_level(max_level);
            file_writer
        }
        Err(_) => {
            drop(unsafe { Box::from_raw(logger_ptr) });
            if let Some(writer) = file_writer {
                writer.shutdown();
            }
            None
        }
    }
}

pub(crate) fn rt_logger_enabled(level: Level, target: &'static str) -> Option<bool> {
    let logger = RT_SAFE_LOGGER.get()?;
    let metadata = Metadata::builder().level(level).target(target).build();
    Some(logger.rt_enabled(&metadata))
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

#[derive(Clone)]
struct LazyFileWriter {
    shared: Arc<LazyFileWriterShared>,
}

struct LazyFileWriterShared {
    config: Mutex<Option<LogConfig>>,
    destination: Mutex<LogDestination>,
    sender: Mutex<Option<mpsc::SyncSender<Vec<u8>>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    shutdown: Mutex<()>,
    mode: AtomicU8,
    dropped_records: AtomicU64,
    initialized: AtomicBool,
}

impl LazyFileWriter {
    fn new(config: LogConfig) -> Self {
        let shared = Arc::new(LazyFileWriterShared {
            config: Mutex::new(Some(config)),
            destination: Mutex::new(LogDestination::Uninitialized),
            sender: Mutex::new(None),
            worker: Mutex::new(None),
            shutdown: Mutex::new(()),
            mode: AtomicU8::new(BLOCKING_MODE),
            dropped_records: AtomicU64::new(0),
            initialized: AtomicBool::new(false),
        });
        Self { shared }
    }

    fn from_open_file(file: std::fs::File) -> Self {
        let shared = Arc::new(LazyFileWriterShared {
            config: Mutex::new(None),
            destination: Mutex::new(LogDestination::File(Arc::new(Mutex::new(file)))),
            sender: Mutex::new(None),
            worker: Mutex::new(None),
            shutdown: Mutex::new(()),
            mode: AtomicU8::new(BLOCKING_MODE),
            dropped_records: AtomicU64::new(0),
            initialized: AtomicBool::new(true),
        });
        Self { shared }
    }

    fn config_for_logger(&self) -> LogConfig {
        *self
            .shared
            .config
            .lock()
            .unwrap()
            .as_ref()
            .expect("lazy file logger config must exist before initialization")
    }

    fn ensure_initialized(&self) {
        if self.shared.initialized.load(Ordering::Acquire) {
            return;
        }

        let _shutdown = self.shared.shutdown.lock().unwrap();
        if self.shared.initialized.load(Ordering::Acquire) {
            return;
        }

        let config = self.shared.config.lock().unwrap().take();
        let destination = config
            .map(initialize_log_destination)
            .unwrap_or(LogDestination::Stderr);
        *self.shared.destination.lock().unwrap() = destination;
        self.shared.initialized.store(true, Ordering::Release);
    }

    fn start(&self) {
        self.ensure_initialized();
        let _shutdown = self.shared.shutdown.lock().unwrap();
        if self.shared.worker.lock().unwrap().is_some() {
            return;
        }

        let (sender, receiver) = mpsc::sync_channel(ASYNC_LOG_QUEUE_CAPACITY);
        let worker_shared = self.shared.clone();
        match std::thread::Builder::new()
            .name("wrac-log-writer".to_string())
            .spawn(move || async_file_writer_worker(worker_shared, receiver))
        {
            Ok(worker) => {
                *self.shared.sender.lock().unwrap() = Some(sender);
                *self.shared.worker.lock().unwrap() = Some(worker);
                self.shared.mode.store(ASYNC_MODE, Ordering::Release);
            }
            Err(error) => {
                let message =
                    format!("[wrac_log] failed to start async file log worker: {error}\n");
                let _ = self.write_blocking(message.as_bytes());
                self.shared.mode.store(BLOCKING_MODE, Ordering::Release);
            }
        }
    }

    fn shutdown(&self) {
        let _shutdown = self.shared.shutdown.lock().unwrap();
        self.shared.mode.store(BLOCKING_MODE, Ordering::Release);
        drop(self.shared.sender.lock().unwrap().take());
        if let Some(worker) = self.shared.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }

    fn write_blocking(&self, buf: &[u8]) -> std::io::Result<()> {
        write_log_bytes_blocking(&mut self.shared.destination.lock().unwrap(), buf)
    }

    fn flush_blocking(&self) -> std::io::Result<()> {
        flush_log_outputs(&mut self.shared.destination.lock().unwrap())
    }
}

enum LoggerInner {
    Immediate(Logger),
    Lazy {
        logger: OnceLock<Logger>,
        config: LogConfig,
        writer: LazyFileWriter,
    },
}

struct WracLogger {
    inner: LoggerInner,
    fallback_max_level: LevelFilter,
}

impl WracLogger {
    fn logger(&self) -> &Logger {
        match &self.inner {
            LoggerInner::Immediate(logger) => logger,
            LoggerInner::Lazy {
                logger,
                config,
                writer,
            } => {
                let logger = logger.get_or_init(|| build_file_logger(*config, writer.clone()));
                let max_level = logger.filter();
                crate::rt::set_rt_fallback_max_level(max_level);
                log::set_max_level(max_level);
                logger
            }
        }
    }

    fn rt_enabled(&self, metadata: &Metadata<'_>) -> bool {
        match &self.inner {
            LoggerInner::Immediate(logger) => logger.enabled(metadata),
            LoggerInner::Lazy { logger, .. } => logger
                .get()
                .map(|logger| logger.enabled(metadata))
                .unwrap_or_else(|| metadata.level() <= self.fallback_max_level),
        }
    }
}

impl Log for WracLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.logger().enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        self.logger().log(record);
    }

    fn flush(&self) {
        self.logger().flush();
    }
}

impl Write for LazyFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.ensure_initialized();
        if ACTIVE_ASYNC_FILE_LOGGERS.load(Ordering::Acquire) > 0 {
            self.start();
        }

        if self.shared.mode.load(Ordering::Acquire) == ASYNC_MODE {
            let send_result = self
                .shared
                .sender
                .lock()
                .unwrap()
                .as_ref()
                .map(|sender| sender.try_send(buf.to_vec()));
            match send_result {
                Some(Ok(())) => return Ok(buf.len()),
                Some(Err(mpsc::TrySendError::Full(_))) => {
                    self.shared.dropped_records.fetch_add(1, Ordering::Relaxed);
                    return Ok(buf.len());
                }
                Some(Err(mpsc::TrySendError::Disconnected(_))) | None => {
                    self.shared.mode.store(BLOCKING_MODE, Ordering::Release);
                }
            }
        }

        let _shutdown = self.shared.shutdown.lock().unwrap();
        self.write_blocking(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.shared.mode.load(Ordering::Acquire) == ASYNC_MODE {
            return Ok(());
        }
        self.flush_blocking()
    }
}

enum LogDestination {
    Uninitialized,
    File(Arc<Mutex<std::fs::File>>),
    Stderr,
}

fn async_file_writer_worker(shared: Arc<LazyFileWriterShared>, receiver: mpsc::Receiver<Vec<u8>>) {
    while let Ok(buf) = receiver.recv() {
        write_dropped_record_notice_if_needed(&shared);
        let _ = write_log_bytes_blocking(&mut shared.destination.lock().unwrap(), &buf);
    }
    write_dropped_record_notice_if_needed(&shared);
    let _ = flush_log_outputs(&mut shared.destination.lock().unwrap());
}

fn write_dropped_record_notice_if_needed(shared: &LazyFileWriterShared) {
    let dropped = shared.dropped_records.swap(0, Ordering::AcqRel);
    if dropped == 0 {
        return;
    }
    let message = format!("[wrac_log] dropped {dropped} async file log records\n");
    let _ = write_log_bytes_blocking(&mut shared.destination.lock().unwrap(), message.as_bytes());
}

fn write_log_bytes_blocking(destination: &mut LogDestination, buf: &[u8]) -> std::io::Result<()> {
    match destination {
        LogDestination::Uninitialized => Ok(()),
        LogDestination::Stderr => std::io::stderr().write_all(buf),
        LogDestination::File(file) => {
            std::io::stderr().write_all(buf)?;
            let mut file = file.lock().unwrap();
            file.write_all(buf)
        }
    }
}

fn flush_log_outputs(destination: &mut LogDestination) -> std::io::Result<()> {
    match destination {
        LogDestination::Uninitialized => Ok(()),
        LogDestination::Stderr => std::io::stderr().flush(),
        LogDestination::File(file) => {
            std::io::stderr().flush()?;
            let mut file = file.lock().unwrap();
            file.flush()
        }
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
mod tests;
