use std::sync::Arc;

use crate::{
    HostGuiResizeRequester, HostParamsEditNotifier, HostStateDirtyNotifier,
    PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension, PluginGuiExtension,
    PluginLatencyExtension, PluginNotePortsExtension, PluginParamsExtension, PluginRenderExtension,
    PluginResult, PluginStateExtension, PluginTailExtension, Processor,
};

#[derive(Debug, Clone, Copy)]
pub struct ActivateContext {
    pub sample_rate: f64,
    pub min_frames_count: u32,
    pub max_frames_count: u32,
}

/// Per-instance environment passed from the adapter to the product core.
///
/// Contains only adapter proxies that the product can hold safely, not raw FFI pointers.
#[derive(Clone)]
pub struct PluginCoreContext {
    pub host_parameter_edit_notifier: Arc<dyn HostParamsEditNotifier>,
    pub host_state_dirty_notifier: Arc<dyn HostStateDirtyNotifier>,
    pub host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
}

/// Entry point for a single plugin instance's lifecycle and capabilities.
///
/// Do not concentrate all state here. Placing `&mut self` `activate`/`deactivate` and
/// concurrently-called parameter/state/GUI queries in the same mutable state would make
/// it impossible to answer one while the other is running. Split each capability into
/// its own thread-safe store and return it as `Arc<dyn ...>` from this trait.
pub trait PluginCore: Send + Sync + 'static {
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>>;
    fn deactivate(&mut self, processor: Box<dyn Processor>) -> PluginResult<()>;

    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPortsExtension>> {
        None
    }

    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPortsExtension>> {
        None
    }

    fn note_ports(&self) -> Option<Arc<dyn PluginNotePortsExtension>> {
        None
    }

    fn params(&self) -> Option<Arc<dyn PluginParamsExtension>> {
        None
    }

    fn state(&self) -> Option<Arc<dyn PluginStateExtension>> {
        None
    }

    fn gui(&self) -> Option<Arc<dyn PluginGuiExtension>> {
        None
    }

    fn render(&self) -> Option<Arc<dyn PluginRenderExtension>> {
        None
    }

    fn tail(&self) -> Option<Arc<dyn PluginTailExtension>> {
        None
    }

    fn latency(&self) -> Option<Arc<dyn PluginLatencyExtension>> {
        None
    }
}
