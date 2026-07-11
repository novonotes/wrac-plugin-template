use std::sync::{Mutex, OnceLock};

use crate::factory::PluginRegistrationStorage;
use wrac_interface::{LogConfig, PluginEntry};

/// Static owner for the safe Rust entry and ABI-facing factory storage.
pub struct EntryRegistration {
    pub(crate) entry: &'static dyn PluginEntry,
    storage: OnceLock<PluginRegistrationStorage>,
    log_runtime: OnceLock<wrac_log::PluginLogRuntime>,
    init_state: Mutex<EntryInitState>,
    instance_state: Mutex<EntryInstanceState>,
}

#[derive(Debug, Default)]
struct EntryInitState {
    count: u32,
}

#[derive(Default)]
struct EntryInstanceState {
    count: u32,
    async_file_logger_guard: Option<wrac_log::PluginLogInstanceGuard>,
}

// Safety: `entry` is immutable and all mutable state is synchronized. Factory queries
// return shared references to storage owned by this registration.
unsafe impl Sync for EntryRegistration {}
unsafe impl Send for EntryRegistration {}

impl EntryRegistration {
    pub const fn new(entry: &'static dyn PluginEntry) -> Self {
        Self {
            entry,
            storage: OnceLock::new(),
            log_runtime: OnceLock::new(),
            init_state: Mutex::new(EntryInitState { count: 0 }),
            instance_state: Mutex::new(EntryInstanceState {
                count: 0,
                async_file_logger_guard: None,
            }),
        }
    }

    pub(crate) fn storage(&'static self) -> &'static PluginRegistrationStorage {
        self.storage
            .get_or_init(|| PluginRegistrationStorage::new(self))
    }

    pub(crate) fn configure_log_runtime(&'static self, config: &'static LogConfig) {
        let _ = self
            .log_runtime
            .get_or_init(|| wrac_log::configure_plugin(config));
    }

    fn log_runtime(&self) -> Option<&wrac_log::PluginLogRuntime> {
        self.log_runtime.get()
    }
}

pub(crate) fn entry_init_count(registration: &'static EntryRegistration) -> u32 {
    registration
        .init_state
        .lock()
        .map(|state| state.count)
        .unwrap_or(0)
}

pub(crate) fn increment_entry_init_count(registration: &'static EntryRegistration) -> u32 {
    let mut state = registration
        .init_state
        .lock()
        .expect("entry init state mutex poisoned");
    state.count = state.count.saturating_add(1);
    state.count
}

pub(crate) fn decrement_entry_init_count(registration: &'static EntryRegistration) -> u32 {
    let mut state = registration
        .init_state
        .lock()
        .expect("entry init state mutex poisoned");
    state.count = state.count.saturating_sub(1);
    state.count
}

pub(crate) fn reset_entry_init_count(registration: &'static EntryRegistration) {
    let mut state = registration
        .init_state
        .lock()
        .expect("entry init state mutex poisoned");
    state.count = 0;
}

pub(crate) fn retain_entry_instance(registration: &'static EntryRegistration) {
    let mut state = registration
        .instance_state
        .lock()
        .expect("entry instance state mutex poisoned");
    state.count = state.count.saturating_add(1);
    if state.count == 1 {
        let Some(log_runtime) = registration.log_runtime() else {
            return;
        };
        state.async_file_logger_guard = Some(log_runtime.retain_instance());
    }
}

pub(crate) fn release_entry_instance(registration: &'static EntryRegistration) {
    let mut state = registration
        .instance_state
        .lock()
        .expect("entry instance state mutex poisoned");
    state.count = state.count.saturating_sub(1);
    if state.count == 0 {
        state.async_file_logger_guard = None;
    }
}
