use crate::interface::{PluginFactory, PluginResult};
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
    /// `[non-realtime]`
    fn log_config(&'static self) -> Option<&'static LogConfig>;

    /// Initializes entry-level state.
    ///
    /// This callback belongs to the DSO entry lifecycle and may run during plugin
    /// scanning without any plugin instance. The clap-wrapper Windows AAX
    /// implementation can call `clap_entry.init` while the Windows loader lock is
    /// held, so this callback must remain loader-lock-safe. Implementations must
    /// not log, open files, write to stderr, start worker threads, initialize COM
    /// or GUI state, launch external processes, or perform expensive computation
    /// here.
    /// `[non-realtime]`
    fn init(&self, _context: EntryContext<'_>) -> PluginResult<()> {
        Ok(())
    }

    /// Releases entry-level state.
    ///
    /// This callback can run close to DSO unload. Implementations must not log,
    /// perform file I/O, join worker threads, or release thread-affine GUI/COM
    /// state here; per-instance cleanup must happen when the last plugin instance
    /// is destroyed.
    /// `[non-realtime]`
    fn deinit(&self) {}

    /// Begins one host/wrapper-designated plugin-main-thread lifetime.
    /// `[non-realtime & thread-safe]`
    fn attach_main_thread(&self) {}

    /// Ends one host/wrapper-designated plugin-main-thread lifetime.
    ///
    /// Implementations must not infer native CLAP thread affinity.
    /// `[non-realtime & thread-safe]`
    fn detach_main_thread(&self) {}

    /// Returns the static factory used for descriptor discovery and product instance creation.
    /// `[non-realtime & thread-safe]`
    fn plugin_factory(&self) -> Option<&dyn PluginFactory>;
}
