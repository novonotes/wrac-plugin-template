use std::sync::Arc;

use crate::interface::{
    ActiveProcessor, HostAudioPorts, HostGui, HostLifecycle, HostNotePorts, HostParams, HostState,
    HostTail, InactiveProcessor, PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension,
    PluginGuiExtension, PluginLatencyExtension, PluginNotePortsExtension, PluginParamsQuery,
    PluginRenderExtension, PluginResult, PluginStateExtension, PluginTailExtension,
};
use wrac_host_context::HostContext;

pub struct ActivateContext {
    pub sample_rate: f64,
    pub min_frames_count: u32,
    pub max_frames_count: u32,
    pub host_tail: Option<Box<dyn HostTail>>,
}

pub struct ActivateResult {
    pub processor: Box<dyn ActiveProcessor>,
    pub notifications: ActivateNotifications,
}

impl ActivateResult {
    pub fn new(processor: Box<dyn ActiveProcessor>) -> Self {
        Self {
            processor,
            notifications: ActivateNotifications::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ActivateNotifications {
    /// Requests `clap_host_latency.changed` during the current CLAP activate call.
    ///
    /// The adapter converts this flag to the host callback only while CLAP marks the
    /// plugin as `[being-activated]`.
    pub latency_changed: bool,
}

/// Per-instance environment passed from the adapter to the product instance.
///
/// Contains only adapter proxies that the product can hold safely, not raw FFI pointers.
#[derive(Clone)]
pub struct PluginInstanceContext {
    pub host_params: Arc<dyn HostParams>,
    pub host_state: Arc<dyn HostState>,
    pub host_audio_ports: Arc<dyn HostAudioPorts>,
    pub host_note_ports: Arc<dyn HostNotePorts>,
    pub host_lifecycle: Arc<dyn HostLifecycle>,
    pub host_gui: Arc<dyn HostGui>,
    pub host_context: HostContext,
}

/// Entry point for a single CLAP plugin instance's lifecycle and capabilities.
///
/// Do not concentrate all state here. Placing `&mut self` `activate`/`deactivate` and
/// concurrently-called parameter/state/GUI queries in the same mutable state would make
/// it impossible to answer one while the other is running. Split each capability into
/// its own thread-safe store and return it as `Arc<dyn ...>` from this trait.
pub trait PluginInstance: Send + 'static {
    /// Initializes the processor lifecycle in its inactive state.
    ///
    /// The adapter calls this once during plugin initialization. It may call
    /// it again after `activate` returns an error, because `activate` consumes
    /// the previous inactive processor before attempting to create an active one.
    /// Implementations must therefore return a fresh inactive processor each time.
    ///
    /// The returned processor may receive `params.flush` while the plugin is
    /// inactive, and is later consumed by `activate`.
    /// `[non-realtime]`
    fn initialize_processor(&mut self) -> PluginResult<Box<dyn InactiveProcessor>>;

    /// Called from the plugin activation callback. `[non-realtime]`
    fn activate(
        &mut self,
        context: ActivateContext,
        processor: Box<dyn InactiveProcessor>,
    ) -> PluginResult<ActivateResult>;

    /// Called from the plugin deactivation or destruction callback. `[non-realtime]`
    fn deactivate(
        &mut self,
        processor: Box<dyn ActiveProcessor>,
    ) -> PluginResult<Box<dyn InactiveProcessor>>;

    /// Called only from the host-requested plugin destruction callback, after processor teardown.
    /// The plugin is inactive when this method is called, and the adapter drops the instance
    /// immediately after this method returns.
    ///
    /// This hook and [`Drop`] are intentionally not interchangeable. `Drop` may also run while
    /// plugin initialization is being rolled back or unwinding after a panic, on whichever thread
    /// releases the owner. Use this hook for ordered lifecycle work that must run only after a
    /// successfully initialized instance reaches its normal host-requested teardown boundary,
    /// especially work that waits for another thread or closes thread-affine resources. Use
    /// [`Drop`] only as an unconditional, failure-path-safe fallback for releasing owned resources.
    /// `[non-realtime]`
    fn destroy(&mut self) {}

    /// Called from CLAP `plugin.on_main_thread`, usually after `HostLifecycle::request_callback`.
    /// `[main-thread]`
    fn on_main_thread(&mut self) {}

    /// Returns the CLAP audio-ports extension during plugin initialization.
    ///
    /// Called once from `plugin.init` before CLAP extension callbacks are exposed to the host.
    /// `[non-realtime]`
    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPortsExtension>> {
        None
    }

    /// Returns the CLAP configurable-audio-ports extension during plugin initialization.
    ///
    /// Called once from `plugin.init` before CLAP extension callbacks are exposed to the host.
    /// `[non-realtime]`
    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPortsExtension>> {
        None
    }

    /// Returns the CLAP note-ports extension during plugin initialization.
    ///
    /// Called once from `plugin.init` before CLAP extension callbacks are exposed to the host.
    /// `[non-realtime]`
    fn note_ports(&self) -> Option<Arc<dyn PluginNotePortsExtension>> {
        None
    }

    /// Returns the parameter query surface during plugin initialization.
    ///
    /// Called once from `plugin.init` before CLAP callbacks are exposed to the host. Plugins without
    /// parameters return a query object whose count is zero.
    /// `[non-realtime]`
    fn params(&self) -> Arc<dyn PluginParamsQuery>;

    /// Returns the CLAP state extension during plugin initialization.
    ///
    /// Called once from `plugin.init` before CLAP extension callbacks are exposed to the host.
    /// `[non-realtime]`
    fn state(&self) -> Option<Arc<dyn PluginStateExtension>> {
        None
    }

    /// Returns the CLAP GUI extension during plugin initialization.
    ///
    /// Called once from `plugin.init` before CLAP extension callbacks are exposed to the host.
    /// `[non-realtime]`
    fn gui(&self) -> Option<Arc<dyn PluginGuiExtension>> {
        None
    }

    /// Returns the CLAP render extension during plugin initialization.
    ///
    /// Called once from `plugin.init` before CLAP extension callbacks are exposed to the host.
    /// `[non-realtime]`
    fn render(&self) -> Option<Arc<dyn PluginRenderExtension>> {
        None
    }

    /// Returns the CLAP tail extension during plugin initialization.
    ///
    /// Called once from `plugin.init` before CLAP extension callbacks are exposed to the host.
    /// `[non-realtime]`
    fn tail(&self) -> Option<Arc<dyn PluginTailExtension>> {
        None
    }

    /// Returns the CLAP latency extension during plugin initialization.
    ///
    /// Called once from `plugin.init` before CLAP extension callbacks are exposed to the host.
    /// `[non-realtime]`
    fn latency(&self) -> Option<Arc<dyn PluginLatencyExtension>> {
        None
    }
}
