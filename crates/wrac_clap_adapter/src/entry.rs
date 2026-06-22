use std::sync::{Mutex, OnceLock};

use crate::factory::PluginRegistrationStorage;
use crate::{PluginDescriptor, PluginInstance, PluginInstanceContext, PluginResult};
pub use wrac_log::{LogConfig, LogOutput};

pub struct EntryContext<'a> {
    pub plugin_path: Option<&'a str>,
}

pub trait PluginEntry: Send + Sync + 'static {
    /// Provides process-wide logger configuration without opening files or writing to stderr.
    ///
    /// This can run from the host loader context, such as a Windows AAX wrapper
    /// loading the DSO. Return static data only; do not log, inspect the
    /// environment, start threads, or touch the filesystem here.
    ///
    /// The adapter installs a lightweight logger during entry initialization. The
    /// file destination is opened lazily on the first log write or the first plugin
    /// instance, whichever comes first. Return a stable static configuration so
    /// repeated entry initialization cannot change logger identity or destination.
    ///
    /// Plugins managed by this adapter must not call `wrac_log` configure APIs
    /// directly. Standalone apps and other binaries that do not enter through
    /// this adapter are responsible for calling `wrac_log::configure_standalone`
    /// and holding the returned runtime themselves.
    fn log_config(&'static self) -> Option<&'static LogConfig>;

    /// Initializes entry-level state.
    ///
    /// This callback belongs to the DSO entry lifecycle and may run during plugin
    /// scanning without any plugin instance. It may also run from a Windows loader
    /// context through wrapper formats such as AAX. Implementations must not log,
    /// open files, write to stderr, start worker threads, initialize COM or GUI
    /// state, launch external processes, or perform expensive computation here.
    fn init(&self, _context: EntryContext<'_>) -> PluginResult<()> {
        Ok(())
    }

    /// Releases entry-level state.
    ///
    /// This callback can run close to DSO unload. Implementations must not log,
    /// perform file I/O, join worker threads, or release thread-affine GUI/COM
    /// state here; per-instance cleanup must happen when the last plugin instance
    /// is destroyed.
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
        context: PluginInstanceContext,
    ) -> Option<Box<dyn PluginInstance>>;
}

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
