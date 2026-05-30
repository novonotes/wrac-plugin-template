use env_logger::{Builder, Target};
use log::{Level, LevelFilter};
use std::array;
use std::fmt::{self, Write as _};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock, Weak};
use std::thread;
use std::time::Duration;
use time::{OffsetDateTime, macros::format_description};

static INIT: Once = Once::new();
static CURRENT_LOG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
static CURRENT_LOG_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
const MAX_LOG_FILES: usize = 30;
const DEFAULT_RECENT_LOG_MAX_FILES: usize = 30;
const DEFAULT_RECENT_LOG_MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
const MAX_UNIQUE_ARCHIVED_LOG_FILE_ATTEMPTS: u32 = 1000;
const RT_LOG_CAPACITY: usize = 4096;
const RT_MESSAGE_CAPACITY: usize = 256;
const RT_TARGET_CAPACITY: usize = 96;

static RT_REGISTRY: OnceLock<RtRegistry> = OnceLock::new();
static RT_DRAIN_WORKER: OnceLock<()> = OnceLock::new();
#[macro_export]
macro_rules! init {
    ($app_name:expr) => {
        $crate::init_impl(option_env!("CARGO_MANIFEST_DIR"), $app_name)
    };
}

pub struct FileLoggerConfig {
    app_name: String,
    log_file: PathBuf,
    level: LevelFilter,
    stderr_prefix: String,
}

impl FileLoggerConfig {
    pub fn new(app_name: impl Into<String>, log_file: impl Into<PathBuf>) -> Self {
        Self {
            app_name: app_name.into(),
            log_file: log_file.into(),
            level: std::env::var("RUST_LOG")
                .ok()
                .and_then(|value| parse_level_filter(&value))
                .unwrap_or(default_level_filter()),
            stderr_prefix: "wrac".to_string(),
        }
    }

    pub fn with_stderr_prefix(mut self, stderr_prefix: impl Into<String>) -> Self {
        self.stderr_prefix = stderr_prefix.into();
        self
    }

    pub fn with_level(mut self, level: LevelFilter) -> Self {
        self.level = level;
        self
    }
}

pub struct RtDrainConfig {
    interval: Duration,
}

impl Default for RtDrainConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(100),
        }
    }
}

impl RtDrainConfig {
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }
}

pub fn init_file_logger_once(config: FileLoggerConfig) {
    INIT.call_once(|| {
        announce_log_output_with_prefix(&config.stderr_prefix, &config.log_file.to_string_lossy());
        init_with_file_and_filter(&config.log_file, None, Some(config.level));
        log::debug!("{} logging initialized", config.app_name);
        start_rt_log_drain_if_enabled();
    });
}

pub fn init_rt_log_drain_once(config: RtDrainConfig) {
    RT_DRAIN_WORKER.get_or_init(|| {
        let interval = config.interval;
        let _ = thread::Builder::new()
            .name("wrac-rt-log-drain".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(interval);
                    drain_rt_logs_once();
                }
            });
    });
}

pub fn drain_rt_logs_once() {
    rt_registry().drain_all();
}
#[doc(hidden)]
pub fn init_impl(manifest_dir: Option<&'static str>, app_name: &str) {
    INIT.call_once(|| {
        let dotenv_rust_log = rust_log_from_debug_dotenv(manifest_dir);
        if let Ok(log_dir) = std::env::var("WRAC_LOG_DIR") {
            init_with_dir(&log_dir, app_name, dotenv_rust_log.as_deref());
            return;
        }
        #[cfg(debug_assertions)]
        if let Some(manifest_dir) = manifest_dir {
            let log_dir = Path::new(manifest_dir).join("../.log");
            if let Some(log_dir_str) = log_dir.to_str() {
                init_with_dir(log_dir_str, app_name, dotenv_rust_log.as_deref());
                return;
            }
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = manifest_dir; // Suppress the unused warning.
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
pub fn current_log_dir() -> Option<PathBuf> {
    CURRENT_LOG_DIR.get().cloned().flatten()
}
pub fn current_log_file() -> Option<PathBuf> {
    CURRENT_LOG_FILE.get().cloned().flatten()
}
#[derive(Clone, Debug)]
pub struct RecentLogFilesOptions {
    pub max_files: usize,
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
    let mut selected = Vec::new();
    selected.push(current_log_file.to_path_buf());
    selected.extend(archived_logs.into_iter().map(|(_, path)| path));

    let max_files = options.max_files.max(1);
    selected.truncate(max_files);

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
pub fn init_test() {
    #[cfg(debug_assertions)]
    INIT.call_once(|| {
        if let Ok(log_dir) = std::env::var("WRAC_LOG_DIR") {
            let test_name = get_test_name();
            let timestamp = get_timestamp();
            let log_file = format!("{}/{}_{}.log", log_dir, test_name, timestamp);
            init_with_file(&log_file, None);
        } else {
            init_stderr(None);
        }
    });
}
fn init_with_dir(log_dir: &str, app_name: &str, dotenv_rust_log: Option<&str>) {
    let log_dir_path = Path::new(log_dir);
    if !log_dir_path.exists() {
        if let Err(e) = std::fs::create_dir_all(log_dir_path) {
            eprintln!("Failed to create log directory '{}': {}", log_dir, e);
            init_stderr(dotenv_rust_log);
            return;
        }
    }

    let file_stem = log_file_stem(app_name);
    let latest_log_file = latest_log_file_path(log_dir_path, &file_stem);
    if let Err(error) = archive_existing_latest_log(&latest_log_file, &file_stem) {
        eprintln!(
            "Failed to archive latest log file '{}': {}",
            latest_log_file.display(),
            error
        );
    }
    rotate_logs(log_dir_path, &file_stem);

    init_with_file(&latest_log_file, dotenv_rust_log);
}
fn rotate_logs(log_dir: &Path, file_stem: &str) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    let mut log_files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| is_archived_log_file_name(&e.file_name().to_string_lossy(), file_stem))
        .collect();
    if log_files.len() <= MAX_LOG_FILES {
        return;
    }
    log_files.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
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
    let archived_log_file = unique_archived_log_file_path(log_dir, file_stem)?;
    std::fs::rename(latest_log_file, archived_log_file)
}

fn unique_archived_log_file_path(log_dir: &Path, file_stem: &str) -> std::io::Result<PathBuf> {
    let timestamp = get_timestamp();
    let first = log_dir.join(format!("{file_stem} {timestamp}.log"));
    if !first.exists() {
        return Ok(first);
    }
    for index in 1..MAX_UNIQUE_ARCHIVED_LOG_FILE_ATTEMPTS {
        let candidate = log_dir.join(format!("{file_stem} {timestamp}-{index}.log"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!(
            "failed to find a unique archived log file name for '{file_stem}' after {MAX_UNIQUE_ARCHIVED_LOG_FILE_ATTEMPTS} attempts"
        ),
    ))
}

fn is_archived_log_file_name(file_name: &str, file_stem: &str) -> bool {
    file_name.starts_with(&format!("{file_stem} "))
        && file_name.ends_with(".log")
        && file_name != format!("{file_stem} Latest.log")
}

fn log_file_stem(app_name: &str) -> String {
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
    if value.starts_with('"') {
        if let Some(end) = value[1..].find('"') {
            return value[1..end + 1].to_string();
        }
    } else if value.starts_with('\'') {
        if let Some(end) = value[1..].find('\'') {
            return value[1..end + 1].to_string();
        }
    }

    value
        .split_once(" #")
        .map(|(value, _)| value.trim_end())
        .unwrap_or(value)
        .to_string()
}
/// |---------|---------------------------------------------------------------------|
/// | macOS   | `~/Library/Logs/NovoNotes/{app_name}/`                              |
/// | Windows | `%LOCALAPPDATA%\NovoNotes\Logs\{app_name}\`                         |
#[cfg(not(debug_assertions))]
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
    apply_default_filter(&mut builder, dotenv_rust_log, None);
    builder.target(Target::Stderr);
    let _ = builder.try_init();
    start_rt_log_drain_if_enabled();
}

fn init_with_file(log_file: impl AsRef<Path>, dotenv_rust_log: Option<&str>) {
    let log_file = log_file.as_ref();
    announce_log_output(&log_file.to_string_lossy());
    init_with_file_and_filter(log_file, dotenv_rust_log, None);
}

fn init_with_file_and_filter(
    log_file: impl AsRef<Path>,
    dotenv_rust_log: Option<&str>,
    default_filter: Option<LevelFilter>,
) {
    let log_file = log_file.as_ref();
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
            apply_default_filter(&mut builder, dotenv_rust_log, default_filter);
            builder.target(Target::Pipe(Box::new(FileAndStderr::new(file))));
            let _ = builder.try_init();
            start_rt_log_drain_if_enabled();
        }
        Err(e) => {
            eprintln!("Failed to open log file '{}': {}", log_file.display(), e);
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
    announce_log_output_with_prefix("wrac_log", destination);
}

fn announce_log_output_with_prefix(prefix: &str, destination: &str) {
    eprintln!("[{prefix}] output={destination}");
}
fn apply_default_filter(
    builder: &mut Builder,
    dotenv_rust_log: Option<&str>,
    default_filter: Option<LevelFilter>,
) {
    if std::env::var("RUST_LOG").is_err() {
        #[cfg(debug_assertions)]
        if let Some(rust_log) = dotenv_rust_log.filter(|value| !value.trim().is_empty()) {
            builder.parse_filters(rust_log);
            return;
        }

        builder.filter_level(default_filter.unwrap_or_else(default_level_filter));
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

fn parse_level_filter(value: &str) -> Option<LevelFilter> {
    let value = value
        .rsplit(',')
        .next()
        .and_then(|directive| directive.rsplit('=').next())
        .unwrap_or(value)
        .trim();

    match value.to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

fn start_rt_log_drain_if_enabled() {
    if cfg!(debug_assertions) || std::env::var_os("WRAC_RT_LOG").is_some() {
        init_rt_log_drain_once(RtDrainConfig::default());
    }
}

pub struct RtLog {
    inner: Arc<RtLogInner>,
}

impl RtLog {
    pub fn new_registered(name: &'static str) -> Self {
        let inner = Arc::new(RtLogInner::new(name));
        rt_registry().register(&inner);
        Self { inner }
    }

    pub fn writer(&self) -> RtLogWriter {
        RtLogWriter {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for RtLog {
    fn drop(&mut self) {
        self.inner.drain_to_log();
        rt_registry().unregister(&self.inner);
    }
}

#[derive(Clone)]
pub struct RtLogWriter {
    inner: Arc<RtLogInner>,
}

impl RtLogWriter {
    #[doc(hidden)]
    pub fn write_fmt(
        &self,
        level: Level,
        target: &'static str,
        block_seq: u64,
        sample_time: u32,
        args: fmt::Arguments<'_>,
    ) {
        self.inner
            .write_fmt(level, target, block_seq, sample_time, args);
    }
}

#[macro_export]
macro_rules! rttrace {
    ($writer:expr, $block_seq:expr, $sample_time:expr, $($arg:tt)+) => {{
        (&$writer).write_fmt(
            log::Level::Trace,
            module_path!(),
            $block_seq,
            $sample_time,
            format_args!($($arg)+),
        );
    }};
}

#[macro_export]
macro_rules! rtdebug {
    ($writer:expr, $block_seq:expr, $sample_time:expr, $($arg:tt)+) => {{
        (&$writer).write_fmt(
            log::Level::Debug,
            module_path!(),
            $block_seq,
            $sample_time,
            format_args!($($arg)+),
        );
    }};
}

#[macro_export]
macro_rules! rtinfo {
    ($writer:expr, $block_seq:expr, $sample_time:expr, $($arg:tt)+) => {{
        (&$writer).write_fmt(
            log::Level::Info,
            module_path!(),
            $block_seq,
            $sample_time,
            format_args!($($arg)+),
        );
    }};
}

#[macro_export]
macro_rules! rtwarn {
    ($writer:expr, $block_seq:expr, $sample_time:expr, $($arg:tt)+) => {{
        (&$writer).write_fmt(
            log::Level::Warn,
            module_path!(),
            $block_seq,
            $sample_time,
            format_args!($($arg)+),
        );
    }};
}

#[macro_export]
macro_rules! rterror {
    ($writer:expr, $block_seq:expr, $sample_time:expr, $($arg:tt)+) => {{
        (&$writer).write_fmt(
            log::Level::Error,
            module_path!(),
            $block_seq,
            $sample_time,
            format_args!($($arg)+),
        );
    }};
}

struct RtRegistry {
    logs: Mutex<Vec<Weak<RtLogInner>>>,
}

impl RtRegistry {
    fn register(&self, log: &Arc<RtLogInner>) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.push(Arc::downgrade(log));
        }
    }

    fn unregister(&self, log: &Arc<RtLogInner>) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.retain(|registered| {
                registered
                    .upgrade()
                    .is_some_and(|inner| !Arc::ptr_eq(&inner, log))
            });
        }
    }

    fn drain_all(&self) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.retain(|registered| {
                let Some(log) = registered.upgrade() else {
                    return false;
                };
                log.drain_to_log();
                true
            });
        }
    }
}

fn rt_registry() -> &'static RtRegistry {
    RT_REGISTRY.get_or_init(|| RtRegistry {
        logs: Mutex::new(Vec::new()),
    })
}

struct RtLogInner {
    name: &'static str,
    next_sequence: AtomicU64,
    drain_sequence: AtomicU64,
    dropped: AtomicU64,
    slots: Vec<RtLogSlot>,
}

impl RtLogInner {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            next_sequence: AtomicU64::new(0),
            drain_sequence: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            // Keep fixed-size slots on the heap to avoid large plugin-instance stack frames.
            slots: (0..RT_LOG_CAPACITY).map(|_| RtLogSlot::new()).collect(),
        }
    }

    fn write_fmt(
        &self,
        level: Level,
        target: &'static str,
        block_seq: u64,
        sample_time: u32,
        args: fmt::Arguments<'_>,
    ) {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        let drain_sequence = self.drain_sequence.load(Ordering::Acquire);
        if sequence.saturating_sub(drain_sequence) >= RT_LOG_CAPACITY as u64 {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }

        let slot = &self.slots[sequence as usize % RT_LOG_CAPACITY];
        slot.write(sequence, level, target, block_seq, sample_time, args);
    }

    fn drain_to_log(&self) {
        let total = self.next_sequence.load(Ordering::Acquire);
        let retained_start = total.saturating_sub(RT_LOG_CAPACITY as u64);
        let start = self
            .drain_sequence
            .load(Ordering::Acquire)
            .max(retained_start);

        let previous_drain_sequence = self.drain_sequence.load(Ordering::Acquire);
        let dropped = self.dropped.swap(0, Ordering::AcqRel);
        if dropped > 0 || start > previous_drain_sequence {
            log::warn!(
                target: "wrac_log::rt",
                "[rt] name={} dropped={} skipped={}",
                self.name,
                dropped,
                start.saturating_sub(previous_drain_sequence),
            );
        }

        let mut drained_until = start;
        for sequence in start..total {
            if let Some(record) = self.slots[sequence as usize % RT_LOG_CAPACITY].read(sequence) {
                log::log!(
                    target: record.target.as_str(),
                    record.level,
                    "[rt] name={} seq={} block={} sample={} {}",
                    self.name,
                    record.sequence,
                    record.block_seq,
                    record.sample_time,
                    record.message.as_str(),
                );
                drained_until = sequence + 1;
            } else {
                // The writer reserves the sequence before publishing the slot. Stop at the first
                // gap so a record published immediately after this drain is not skipped forever.
                break;
            }
        }
        self.drain_sequence.store(drained_until, Ordering::Release);
    }
}

struct RtLogSlot {
    sequence: AtomicU64,
    level: AtomicU8,
    block_seq: AtomicU64,
    sample_time: AtomicU32,
    target_len: AtomicUsize,
    target: [AtomicU8; RT_TARGET_CAPACITY],
    message_len: AtomicUsize,
    message: [AtomicU8; RT_MESSAGE_CAPACITY],
}

impl RtLogSlot {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            level: AtomicU8::new(level_to_u8(Level::Debug)),
            block_seq: AtomicU64::new(0),
            sample_time: AtomicU32::new(0),
            target_len: AtomicUsize::new(0),
            target: array::from_fn(|_| AtomicU8::new(0)),
            message_len: AtomicUsize::new(0),
            message: array::from_fn(|_| AtomicU8::new(0)),
        }
    }

    fn write(
        &self,
        sequence: u64,
        level: Level,
        target: &str,
        block_seq: u64,
        sample_time: u32,
        args: fmt::Arguments<'_>,
    ) {
        self.sequence.store(0, Ordering::Release);
        self.level.store(level_to_u8(level), Ordering::Relaxed);
        self.block_seq.store(block_seq, Ordering::Relaxed);
        self.sample_time.store(sample_time, Ordering::Relaxed);
        write_atomic_bytes(&self.target, &self.target_len, target.as_bytes());

        let mut message = FixedMessage::new();
        let _ = message.write_fmt(args);
        write_atomic_bytes(&self.message, &self.message_len, message.as_bytes());
        self.sequence.store(sequence + 1, Ordering::Release);
    }

    fn read(&self, sequence: u64) -> Option<RtLogRecord> {
        if self.sequence.load(Ordering::Acquire) != sequence + 1 {
            return None;
        }

        let record = RtLogRecord {
            sequence,
            level: u8_to_level(self.level.load(Ordering::Relaxed)),
            block_seq: self.block_seq.load(Ordering::Relaxed),
            sample_time: self.sample_time.load(Ordering::Relaxed),
            target: read_atomic_string::<RT_TARGET_CAPACITY>(&self.target, &self.target_len),
            message: read_atomic_string::<RT_MESSAGE_CAPACITY>(&self.message, &self.message_len),
        };

        if self.sequence.load(Ordering::Acquire) == sequence + 1 {
            Some(record)
        } else {
            None
        }
    }
}

struct RtLogRecord {
    sequence: u64,
    level: Level,
    block_seq: u64,
    sample_time: u32,
    target: FixedString<RT_TARGET_CAPACITY>,
    message: FixedString<RT_MESSAGE_CAPACITY>,
}

struct FixedMessage {
    bytes: [u8; RT_MESSAGE_CAPACITY],
    len: usize,
}

impl FixedMessage {
    fn new() -> Self {
        Self {
            bytes: [0; RT_MESSAGE_CAPACITY],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl fmt::Write for FixedMessage {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let remaining = RT_MESSAGE_CAPACITY.saturating_sub(self.len);
        let count = utf8_boundary_len(value, remaining);
        self.bytes[self.len..self.len + count].copy_from_slice(&value.as_bytes()[..count]);
        self.len += count;
        Ok(())
    }
}

fn utf8_boundary_len(value: &str, limit: usize) -> usize {
    if value.len() <= limit {
        return value.len();
    }
    let mut count = limit.min(value.len());
    while count > 0 && !value.is_char_boundary(count) {
        count -= 1;
    }
    count
}

struct FixedString<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> FixedString<N> {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("<invalid utf8>")
    }
}

fn write_atomic_bytes<const N: usize>(target: &[AtomicU8; N], len: &AtomicUsize, bytes: &[u8]) {
    let count = N.min(bytes.len());
    for index in 0..count {
        target[index].store(bytes[index], Ordering::Relaxed);
    }
    len.store(count, Ordering::Relaxed);
}

fn read_atomic_string<const N: usize>(source: &[AtomicU8; N], len: &AtomicUsize) -> FixedString<N> {
    let len = len.load(Ordering::Relaxed).min(N);
    let mut bytes = [0; N];
    for index in 0..len {
        bytes[index] = source[index].load(Ordering::Relaxed);
    }
    FixedString { bytes, len }
}

const fn level_to_u8(level: Level) -> u8 {
    match level {
        Level::Error => 1,
        Level::Warn => 2,
        Level::Info => 3,
        Level::Debug => 4,
        Level::Trace => 5,
    }
}

fn u8_to_level(level: u8) -> Level {
    match level {
        1 => Level::Error,
        2 => Level::Warn,
        3 => Level::Info,
        5 => Level::Trace,
        _ => Level::Debug,
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
        .replace(" ", "_")
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
mod test;
