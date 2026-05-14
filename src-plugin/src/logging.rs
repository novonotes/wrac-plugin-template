//! Template-local debug logger.
//!
//! Product code should decide its own logging backend. This helper exists so the
//! template shows logs immediately while developing a plugin in a DAW, where
//! stderr is often not visible from an attached debugger.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use log::{LevelFilter, Log, Metadata, Record};
use time::{OffsetDateTime, macros::format_description};

static INIT: Once = Once::new();
static LOGGER: DebugFileLogger = DebugFileLogger {
    file: Mutex::new(None),
    level: Mutex::new(LevelFilter::Debug),
};

pub(crate) fn init_debug_logging_once(app_name: &str) {
    INIT.call_once(|| {
        let level = std::env::var("RUST_LOG")
            .ok()
            .and_then(|value| parse_level_filter(&value))
            .unwrap_or(LevelFilter::Debug);
        let log_file = default_log_file(app_name);
        let file = open_log_file(&log_file);

        if let Ok(mut logger_file) = LOGGER.file.lock() {
            *logger_file = file;
        }
        if let Ok(mut logger_level) = LOGGER.level.lock() {
            *logger_level = level;
        }

        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(level);
            eprintln!("[wrac_gain_plugin] debug log: {}", log_file.display());
            LOGGER.write_session_header(app_name);
        }
    });
}

struct DebugFileLogger {
    file: Mutex<Option<File>>,
    level: Mutex<LevelFilter>,
}

impl Log for DebugFileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let line = format!(
            "{} [{}] {} - {}\n",
            local_timestamp_millis(),
            record.level(),
            record.target(),
            record.args()
        );
        let _ = std::io::stderr().write_all(line.as_bytes());

        if let Ok(mut file) = self.file.lock() {
            if let Some(file) = file.as_mut() {
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
        }
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
        if let Ok(mut file) = self.file.lock() {
            if let Some(file) = file.as_mut() {
                let _ = file.flush();
            }
        }
    }
}

impl DebugFileLogger {
    fn level(&self) -> LevelFilter {
        self.level
            .lock()
            .map(|level| *level)
            .unwrap_or(LevelFilter::Off)
    }

    fn write_session_header(&self, app_name: &str) {
        let line = format!(
            "\n================ {} session started at {} ================\n",
            app_name,
            local_timestamp_millis()
        );
        let _ = std::io::stderr().write_all(line.as_bytes());

        if let Ok(mut file) = self.file.lock() {
            if let Some(file) = file.as_mut() {
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
        }
    }
}

fn default_log_file(app_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.log")
        .join(format!("{} Latest.log", sanitize_file_stem(app_name)))
}

fn open_log_file(path: &Path) -> Option<File> {
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!(
                "[wrac_gain_plugin] failed to create log directory '{}': {error}",
                parent.display()
            );
            return None;
        }
    }

    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!(
                "[wrac_gain_plugin] failed to open log file '{}': {error}",
                path.display()
            );
            None
        }
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

fn sanitize_file_stem(value: &str) -> String {
    let sanitized = value
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
        "Plugin".to_string()
    } else {
        sanitized
    }
}

fn local_timestamp_millis() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let format =
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]");
    now.format(format)
        .unwrap_or_else(|_| format!("{}.{:03}", now.unix_timestamp(), now.millisecond()))
}
