use std::sync::Arc;

use crate::{
    ActiveProcessor, HostGuiResizeRequester, HostParamsFlushRequester, HostStateDirtyNotifier,
    InactiveProcessor, PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension,
    PluginGuiExtension, PluginLatencyExtension, PluginNotePortsExtension, PluginParamsQuery,
    PluginRenderExtension, PluginResult, PluginStateExtension, PluginTailExtension,
};
use wrac_host_context::HostContext;

#[derive(Debug, Clone, Copy)]
pub struct ActivateContext {
    pub sample_rate: f64,
    pub min_frames_count: u32,
    pub max_frames_count: u32,
}

/// Per-instance environment passed from the adapter to the product instance.
///
/// Contains only adapter proxies that the product can hold safely, not raw FFI pointers.
#[derive(Clone)]
pub struct PluginInstanceContext {
    pub host_params_flush_requester: Arc<dyn HostParamsFlushRequester>,
    pub host_state_dirty_notifier: Arc<dyn HostStateDirtyNotifier>,
    pub host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    pub host_context: HostContext,
}

/// Entry point for a single CLAP plugin instance's lifecycle and capabilities.
///
/// Do not concentrate all state here. Placing `&mut self` `activate`/`deactivate` and
/// concurrently-called parameter/state/GUI queries in the same mutable state would make
/// it impossible to answer one while the other is running. Split each capability into
/// its own thread-safe store and return it as `Arc<dyn ...>` from this trait.
pub trait PluginInstance: Send + 'static {
    /// Creates the inactive processing state held before the first activation.
    /// `[control-thread]`
    fn create_inactive_processor(&mut self) -> PluginResult<Box<dyn InactiveProcessor>>;

    /// Called from the plugin activation callback. `[control-thread]`
    fn activate(
        &mut self,
        context: ActivateContext,
        processor: Box<dyn InactiveProcessor>,
    ) -> PluginResult<Box<dyn ActiveProcessor>>;

    /// Called from the plugin deactivation or destruction callback. `[control-thread]`
    fn deactivate(
        &mut self,
        processor: Box<dyn ActiveProcessor>,
    ) -> PluginResult<Box<dyn InactiveProcessor>>;

    /// Returns the CLAP audio-ports extension during plugin instance creation.
    ///
    /// Called once before CLAP callbacks are exposed to the host.
    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPortsExtension>> {
        None
    }

    /// Returns the CLAP configurable-audio-ports extension during plugin instance creation.
    ///
    /// Called once before CLAP callbacks are exposed to the host.
    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPortsExtension>> {
        None
    }

    /// Returns the CLAP note-ports extension during plugin instance creation.
    ///
    /// Called once before CLAP callbacks are exposed to the host.
    fn note_ports(&self) -> Option<Arc<dyn PluginNotePortsExtension>> {
        None
    }

    /// Returns the parameter query surface during plugin instance creation.
    ///
    /// Called once before CLAP callbacks are exposed to the host. Plugins without
    /// parameters return a query object whose count is zero.
    fn params(&self) -> Arc<dyn PluginParamsQuery>;

    /// Returns the CLAP state extension during plugin instance creation.
    ///
    /// Called once before CLAP callbacks are exposed to the host.
    fn state(&self) -> Option<Arc<dyn PluginStateExtension>> {
        None
    }

    /// Returns the CLAP GUI extension during plugin instance creation.
    ///
    /// Called once before CLAP callbacks are exposed to the host.
    fn gui(&self) -> Option<Arc<dyn PluginGuiExtension>> {
        None
    }

    /// Returns the CLAP render extension during plugin instance creation.
    ///
    /// Called once before CLAP callbacks are exposed to the host.
    fn render(&self) -> Option<Arc<dyn PluginRenderExtension>> {
        None
    }

    /// Returns the CLAP tail extension during plugin instance creation.
    ///
    /// Called once before CLAP callbacks are exposed to the host.
    fn tail(&self) -> Option<Arc<dyn PluginTailExtension>> {
        None
    }

    /// Returns the CLAP latency extension during plugin instance creation.
    ///
    /// Called once before CLAP callbacks are exposed to the host.
    fn latency(&self) -> Option<Arc<dyn PluginLatencyExtension>> {
        None
    }
}
