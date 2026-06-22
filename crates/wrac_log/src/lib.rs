//! Logging utilities for WRAC plugins.
//!
//! Regular logs are written through the `log` facade. Realtime audio threads must use
//! the `rt*` macros, which write into a fixed-size global buffer drained later from a
//! non-realtime run loop timer.

mod file_logger;
mod rt;

pub use file_logger::{
    LogConfig, LogOutput, PluginLogInstanceGuard, PluginLogRuntime, RecentLogFilesOptions,
    StandaloneLogRuntime, collect_recent_log_files, configure_plugin, configure_standalone,
    current_log_dir, current_log_file, init_test,
};
/// Macro support function used by the realtime log macros to check filtering.
///
/// This remains public because exported macros refer to it through `$crate`.
/// Plugin code should not call or rely on this symbol directly; use the `rt*`
/// logging macros instead.
pub use rt::rt_log_enabled as __rt_log_enabled;
/// Macro support function used by the realtime log macros to write records.
///
/// This remains public because exported macros refer to it through `$crate`.
/// Plugin code should not call or rely on this symbol directly; use the `rt*`
/// logging macros instead.
pub use rt::write_rt_log as __write_rt_log;
pub use rt::{RtDrainingRunLoopGuard, attach_rt_drain, drain_rt_logs_once};

#[macro_export]
macro_rules! rttrace {
    (target: $target:expr, $($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Trace, $target) {
            $crate::__write_rt_log(log::Level::Trace, $target, format_args!($($arg)+));
        }
    }};
    ($($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Trace, module_path!()) {
            $crate::__write_rt_log(log::Level::Trace, module_path!(), format_args!($($arg)+));
        }
    }};
}

#[macro_export]
macro_rules! rtdebug {
    (target: $target:expr, $($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Debug, $target) {
            $crate::__write_rt_log(log::Level::Debug, $target, format_args!($($arg)+));
        }
    }};
    ($($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Debug, module_path!()) {
            $crate::__write_rt_log(log::Level::Debug, module_path!(), format_args!($($arg)+));
        }
    }};
}

#[macro_export]
macro_rules! rtinfo {
    (target: $target:expr, $($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Info, $target) {
            $crate::__write_rt_log(log::Level::Info, $target, format_args!($($arg)+));
        }
    }};
    ($($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Info, module_path!()) {
            $crate::__write_rt_log(log::Level::Info, module_path!(), format_args!($($arg)+));
        }
    }};
}

#[macro_export]
macro_rules! rtwarn {
    (target: $target:expr, $($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Warn, $target) {
            $crate::__write_rt_log(log::Level::Warn, $target, format_args!($($arg)+));
        }
    }};
    ($($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Warn, module_path!()) {
            $crate::__write_rt_log(log::Level::Warn, module_path!(), format_args!($($arg)+));
        }
    }};
}

#[macro_export]
macro_rules! rterror {
    (target: $target:expr, $($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Error, $target) {
            $crate::__write_rt_log(log::Level::Error, $target, format_args!($($arg)+));
        }
    }};
    ($($arg:tt)+) => {{
        if $crate::__rt_log_enabled(log::Level::Error, module_path!()) {
            $crate::__write_rt_log(log::Level::Error, module_path!(), format_args!($($arg)+));
        }
    }};
}
