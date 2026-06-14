//! The plugin contract as seen by the host.
//!
//! What is declared here:
//! 1. Plugin self-descriptions generated from package metadata
//! 2. [`SharedState`] shared by the audio thread, GUI, and host
//! 3. The audio [`ActiveProcessor`] handed over at activation, and how host extension
//!    capabilities are bundled
//!
//! Parameter, audio port, and state-persistence implementations live under `plugin/`.
//! Format differences between CLAP, VST3, and AU are absorbed by `wrac_clap_adapter`,
//! so this module focuses solely on "which capabilities this plugin offers."

use std::sync::Arc;

mod audio_ports;
mod params;
mod state;

pub(crate) use params::{
    DEFAULT_GAIN, PARAM_BYPASS_ID, PARAM_GAIN_ID, WracGainParamOutputQueue,
    apply_param_input_events, clamp_gain, notify_gui_parameters, parameter_default_value,
    parameter_host_input_to_plain, parameter_host_value, parameter_infos, parameter_text_value,
    parameter_value_text,
};

use audio_ports::{AudioLayoutStore, WracGainAudioPorts, WracGainConfigurableAudioPorts};
use params::WracGainParamsExtension;
use state::WracGainStateExtension;
use wrac_clap_adapter::{
    AaxDescriptor, AaxStemConfig, ActivateContext, ActiveProcessor, Auv2Descriptor,
    InactiveProcessor, PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension,
    PluginDescriptor, PluginEntry, PluginFactory, PluginFeature, PluginGuiExtension,
    PluginInstance, PluginInstanceContext, PluginParamsQuery, PluginResult, PluginStateExtension,
    Vst3Descriptor,
};
use wrac_wxp_gui::WxpGuiController;

use crate::audio::WracGainAudioProcessor;
use crate::gui::create_gui_integration;
use crate::state::{ProjectStateStore, SharedState};

// Generated from wrac-plugin.toml. The manifest is
// the single source of truth for product identity across descriptors, GUI metadata,
// wrapper arguments, AUv2 registration, WebView data dirs, and logs.
include!(concat!(env!("OUT_DIR"), "/wrac_plugin_products.rs"));

pub(crate) static PLUGIN_ENTRY: WracGainEntry = WracGainEntry;

pub(crate) struct WracGainEntry;

impl PluginEntry for WracGainEntry {
    fn plugin_factory(&self) -> Option<&dyn PluginFactory> {
        Some(&WRAC_GAIN_FACTORY)
    }
}

static WRAC_GAIN_FACTORY: WracGainFactory = WracGainFactory;

struct WracGainFactory;

impl PluginFactory for WracGainFactory {
    fn plugin_count(&self) -> u32 {
        PLUGIN_DESCRIPTORS.len() as u32
    }

    fn plugin_descriptor(&self, index: u32) -> Option<PluginDescriptor> {
        PLUGIN_DESCRIPTORS.get(index as usize).copied()
    }

    fn create_plugin(
        &self,
        plugin_id: &str,
        context: PluginInstanceContext,
    ) -> Option<Box<dyn PluginInstance>> {
        // The host creates by descriptor id after discovery. Carry the matched descriptor
        // into the instance so logs, WebView identity, and About metadata follow the product
        // actually requested instead of falling back to the first manifest entry.
        PLUGIN_DESCRIPTORS
            .iter()
            .find(|descriptor| descriptor.id == plugin_id)
            .map(|descriptor| create_plugin_core(context, *descriptor))
    }
}

/// One instance of the plugin, created each time the host loads the plugin.
///
/// The audio processing core is split into an [`ActiveProcessor`] by [`PluginInstance::activate`],
/// so this struct is responsible only for lifecycle management and holding the host
/// extension capabilities.
///
/// Capabilities are held behind `Arc` because the host (wrapper) may query them
/// re-entrantly during lifecycle callbacks, requiring them to be reachable without
/// acquiring the `&mut self` lock on `PluginInstance`.
pub(crate) struct WracGainPlugin {
    // The descriptor is instance data, not a global primary descriptor. This matters for
    // multi-product bundles where the same binary can expose more than one plugin id.
    descriptor: PluginDescriptor,
    // Parameter state shared by the audio thread, GUI, and host. See [`SharedState`].
    shared: Arc<SharedState>,
    // Audio layout negotiated with the host. Non-realtime only. See [`AudioLayoutStore`].
    audio_layout: Arc<AudioLayoutStore>,
    audio_ports: Arc<WracGainAudioPorts>,
    configurable_audio_ports: Arc<WracGainConfigurableAudioPorts>,
    params: Arc<WracGainParamsExtension>,
    param_output_queue: Arc<WracGainParamOutputQueue>,
    gui: Arc<WxpGuiController>,
    // Project state save/restore. A dedicated capability independent of the lifecycle
    // lock so that a committed snapshot can be returned even while active or during a
    // wrapper re-entry.
    state_extension: Arc<WracGainStateExtension>,
}

impl WracGainPlugin {
    pub(crate) fn new(context: PluginInstanceContext, descriptor: PluginDescriptor) -> Self {
        let shared = Arc::new(SharedState::new());
        let audio_layout = Arc::new(AudioLayoutStore::new(2));
        let audio_ports = Arc::new(WracGainAudioPorts::new(audio_layout.clone()));
        let configurable_audio_ports =
            Arc::new(WracGainConfigurableAudioPorts::new(audio_layout.clone()));
        let params = Arc::new(WracGainParamsExtension::new(shared.clone()));
        let param_output_queue =
            Arc::new(WracGainParamOutputQueue::new(context.host_params.clone()));
        let project_state = Arc::new(ProjectStateStore::new());
        let gui = create_gui_integration(
            descriptor,
            project_state.clone(),
            shared.clone(),
            param_output_queue.clone(),
            context.host_gui,
            context.host_context,
        );
        let state_extension = Arc::new(WracGainStateExtension::new(
            project_state,
            shared.clone(),
            gui.notifier.clone(),
        ));

        Self {
            descriptor,
            shared,
            audio_layout,
            audio_ports,
            configurable_audio_ports,
            params,
            param_output_queue,
            gui: gui.controller,
            state_extension,
        }
    }
}

/// Called from this product's [`PluginFactory`] implementation.
/// Called each time the host requests a new instance; returns a [`PluginInstance`].
pub(crate) fn create_plugin_core(
    context: PluginInstanceContext,
    descriptor: PluginDescriptor,
) -> Box<dyn PluginInstance> {
    wrac_log::init!(descriptor.name);

    log::debug!(
        "creating plugin core: id={}, name={}",
        descriptor.id,
        descriptor.name
    );
    for parameter in parameter_infos() {
        log::debug!(
            "host parameter schema: id={}, name={}, min={}, max={}, default={}, automatable={}, stepped={}, enum={}, bypass={}",
            parameter.id,
            parameter.name,
            parameter.min_value,
            parameter.max_value,
            parameter.default_value,
            parameter.flags.is_automatable,
            parameter.flags.is_stepped,
            parameter.flags.is_enum,
            parameter.flags.is_bypass
        );
    }
    Box::new(WracGainPlugin::new(context, descriptor))
}

// ---------------------------------------------------------------------------
// PluginInstance: plugin lifecycle and the extension capabilities offered
// ---------------------------------------------------------------------------
impl PluginInstance for WracGainPlugin {
    fn initialize_processor(&mut self) -> PluginResult<Box<dyn InactiveProcessor>> {
        Ok(Box::new(crate::audio::WracGainInactiveProcessor::new(
            self.shared.clone(),
            self.param_output_queue.clone(),
        )))
    }

    /// Called just before the host starts audio processing.
    /// The returned [`ActiveProcessor`] is subsequently `process()`-ed on the audio thread.
    fn activate(
        &mut self,
        context: ActivateContext,
        _processor: Box<dyn InactiveProcessor>,
    ) -> PluginResult<Box<dyn ActiveProcessor>> {
        // Boundary between the non-RT layout store and the RT processor.
        //
        // The adapter rejects layout changes while active, so the channel count
        // snapshotted here is contractually immutable until deactivate. Passing the
        // full `Arc<AudioLayoutStore>` would leave room for process() to acquire the
        // lock; copying only the needed value instead structurally enforces "the audio
        // thread sees only immutable configuration."
        let audio_channel_count = self.audio_layout.channel_count();
        log::debug!(
            "activating audio processor: plugin_id={}, sample_rate={}, min_frames_count={}, max_frames_count={}, audio_channel_count={}",
            self.descriptor.id,
            context.sample_rate,
            context.min_frames_count,
            context.max_frames_count,
            audio_channel_count
        );
        Ok(Box::new(WracGainAudioProcessor::new(
            self.shared.clone(),
            self.param_output_queue.clone(),
            audio_channel_count,
        )))
    }

    /// Called when the host stops audio processing.
    fn deactivate(
        &mut self,
        _processor: Box<dyn ActiveProcessor>,
    ) -> PluginResult<Box<dyn InactiveProcessor>> {
        log::debug!("deactivating audio processor");
        self.initialize_processor()
    }

    // Extension declarations. Some = implemented, None = unsupported. Implementations live in separate modules.

    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPortsExtension>> {
        Some(self.audio_ports.clone())
    }

    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPortsExtension>> {
        Some(self.configurable_audio_ports.clone())
    }

    fn params(&self) -> Arc<dyn PluginParamsQuery> {
        self.params.clone()
    }

    fn state(&self) -> Option<Arc<dyn PluginStateExtension>> {
        Some(self.state_extension.clone())
    }

    fn gui(&self) -> Option<Arc<dyn PluginGuiExtension>> {
        Some(self.gui.clone())
    }
}
