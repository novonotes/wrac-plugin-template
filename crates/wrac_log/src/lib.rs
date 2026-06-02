//! Logging utilities for WRAC plugins.
//!
//! Regular logs are written through the `log` facade. Realtime audio threads must use
//! the `rt*` macros, which write into a fixed-size global buffer drained later from a
//! non-realtime worker.

mod file_logger;
mod rt;

pub use file_logger::{
    RecentLogFilesOptions, collect_recent_log_files, current_log_dir, current_log_file, init_impl,
    init_test,
};
#[doc(hidden)]
pub use rt::write_rt_log as __write_rt_log;
pub use rt::{RtDrainConfig, drain_rt_logs_once, init_rt_log_drain_once};

/// Initializes logging for a WRAC plugin.
///
/// This macro must be called from the plugin crate so `CARGO_MANIFEST_DIR` points at
/// the caller. In debug builds, the default log directory is resolved as
/// `{plugin_crate}/../.log`; calling the implementation function directly from this
/// crate would resolve that path relative to `wrac_log` instead.
///
/// Initialization is process-wide and idempotent. The first call wins.
///
/// Output destination priority:
/// 1. `WRAC_LOG_DIR`
/// 2. Debug builds: `{plugin_crate}/../.log`
/// 3. Release builds: the platform user log directory under `NovoNotes/{app_name}`
/// 4. `stderr` when no file destination can be resolved
///
/// When writing to a file, the current session is written to
/// `{app_name} Latest.log`; any previous latest log is archived with a timestamp and
/// old archives are rotated.
#[macro_export]
macro_rules! init {
    ($app_name:expr) => {
        $crate::init_impl(option_env!("CARGO_MANIFEST_DIR"), $app_name)
    };
}

#[macro_export]
macro_rules! rttrace {
    (target: $target:expr, $($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Trace, $target, format_args!($($arg)+));
    }};
    ($($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Trace, module_path!(), format_args!($($arg)+));
    }};
}

#[macro_export]
macro_rules! rtdebug {
    (target: $target:expr, $($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Debug, $target, format_args!($($arg)+));
    }};
    ($($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Debug, module_path!(), format_args!($($arg)+));
    }};
}

#[macro_export]
macro_rules! rtinfo {
    (target: $target:expr, $($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Info, $target, format_args!($($arg)+));
    }};
    ($($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Info, module_path!(), format_args!($($arg)+));
    }};
}

#[macro_export]
macro_rules! rtwarn {
    (target: $target:expr, $($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Warn, $target, format_args!($($arg)+));
    }};
    ($($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Warn, module_path!(), format_args!($($arg)+));
    }};
}

#[macro_export]
macro_rules! rterror {
    (target: $target:expr, $($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Error, $target, format_args!($($arg)+));
    }};
    ($($arg:tt)+) => {{
        $crate::__write_rt_log(log::Level::Error, module_path!(), format_args!($($arg)+));
    }};
}
