use std::sync::{Mutex, OnceLock};

use crate::factory::PluginRegistrationStorage;
use crate::{PluginCore, PluginCoreContext, PluginDescriptor, PluginResult};

pub struct EntryContext<'a> {
    pub plugin_path: Option<&'a str>,
}

pub trait PluginEntry: Send + Sync + 'static {
    fn init(&self, _context: EntryContext<'_>) -> PluginResult<()> {
        Ok(())
    }

    fn deinit(&self) {}

    fn attach_main_thread(&self) {}

    fn detach_main_thread(&self) {}

    fn plugin_factory(&self) -> Option<&dyn PluginFactory>;
}

pub trait PluginFactory: Send + Sync + 'static {
    fn plugin_count(&self) -> u32;
    fn plugin_descriptor(&self, index: u32) -> Option<PluginDescriptor>;
    fn create_plugin(
        &self,
        plugin_id: &str,
        context: PluginCoreContext,
    ) -> Option<Box<dyn PluginCore>>;
}

/// Static owner for the safe Rust entry and ABI-facing factory storage.
pub struct EntryRegistration {
    pub(crate) entry: &'static dyn PluginEntry,
    storage: OnceLock<PluginRegistrationStorage>,
    init_state: Mutex<EntryInitState>,
}

#[derive(Debug, Default)]
struct EntryInitState {
    count: u32,
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
            init_state: Mutex::new(EntryInitState { count: 0 }),
        }
    }

    pub(crate) fn storage(&'static self) -> &'static PluginRegistrationStorage {
        self.storage
            .get_or_init(|| PluginRegistrationStorage::new(self))
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
