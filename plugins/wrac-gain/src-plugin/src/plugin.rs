//! The plugin contract as seen by the host.
//!
//! What is declared here:
//! 1. The plugin's self-description ([`PLUGIN_DESCRIPTOR`])
//! 2. [`SharedState`] shared by the audio thread, GUI, and host
//! 3. The audio [`Processor`] handed over at activation, and how host extension
//!    capabilities are bundled
//!
//! Parameter, audio port, and state-persistence implementations live under `plugin/`.
//! Format differences between CLAP, VST3, and AU are absorbed by `wrac_clap_adapter`,
//! so this module focuses solely on "which capabilities this plugin offers."

use std::sync::Arc;

mod audio_ports;
mod parameters;
mod state_support;

pub(crate) use parameters::{
    DEFAULT_GAIN, PARAM_BYPASS_ID, PARAM_GAIN_ID, clamp_gain, gain_parameter_info,
    host_value_to_gain, parameter_default_value, parameter_host_value, parameter_text_value,
    parameter_value_text,
};

use audio_ports::{AudioLayoutStore, WracGainAudioPorts, WracGainConfigurableAudioPorts};
use parameters::{WracGainParameters, bypass_parameter_info};
use state_support::WracGainStateSupport;
use wrac_clap_adapter::{
    ActivateContext, Auv2Descriptor, PluginAudioPorts, PluginConfigurableAudioPorts, PluginCore,
    PluginCoreContext, PluginDescriptor, PluginEntry, PluginFactory, PluginFeature, PluginGui,
    PluginParameters, PluginResult, PluginStateSupport, Processor,
};
use wrac_wxp_gui::WxpGuiController;

use crate::audio::WracGainAudioProcessor;
use crate::gui::create_gui_integration;
use crate::state::{ProjectStateStore, SharedState};

// The single source of truth for plugin identity is [package.metadata.wrac] in
// src-plugin/Cargo.toml. The GUI, xtask, and wrapper build all read the same metadata,
// so env! macros are used here instead of hard-coded strings to prevent mismatches
// (mismatched bundle names or About-dialog text) when renaming the template.
pub(crate) const PLUGIN_ID: &str = env!("WRAC_PLUGIN_0_ID");
pub(crate) const PLUGIN_NAME: &str = env!("WRAC_PLUGIN_0_NAME");
pub(crate) const COMPANY_NAME: &str = env!("WRAC_COMPANY_NAME");
const AUV2_TYPE: [u8; 4] = four_char_code(env!("WRAC_PLUGIN_0_AUV2_TYPE"));
const AUV2_SUBTYPE: [u8; 4] = four_char_code(env!("WRAC_PLUGIN_0_AUV2_SUBTYPE"));
const AUV2_MANUFACTURER_CODE: [u8; 4] = four_char_code(env!("WRAC_AUV2_MANUFACTURER_CODE"));

// Plugin self-description sent to the host. The adapter converts this into CLAP / AUv2 descriptors.
pub(crate) const PLUGIN_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    id: PLUGIN_ID,
    name: PLUGIN_NAME,
    vendor: COMPANY_NAME,
    url: "",
    manual_url: "",
    support_url: "",
    version: env!("CARGO_PKG_VERSION"),
    description: "Simple gain plugin",
    features: &[
        PluginFeature::AudioEffect,
        PluginFeature::Utility,
        PluginFeature::Stereo,
    ],
    // For AUv2 (macOS Audio Unit v2). The codes are 4-character ASCII identifiers
    // that must be unique within a company's plugin catalogue.
    auv2: Some(Auv2Descriptor {
        manufacturer_code: AUV2_MANUFACTURER_CODE,
        manufacturer_name: COMPANY_NAME,
        plugin_type: AUV2_TYPE,
        plugin_subtype: AUV2_SUBTYPE,
    }),
};

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
        1
    }

    fn plugin_descriptor(&self, index: u32) -> Option<PluginDescriptor> {
        (index == 0).then_some(PLUGIN_DESCRIPTOR)
    }

    fn create_plugin(
        &self,
        plugin_id: &str,
        context: PluginCoreContext,
    ) -> Option<Box<dyn PluginCore>> {
        (plugin_id == PLUGIN_ID).then(|| create_plugin_core(context))
    }
}

const fn four_char_code(value: &str) -> [u8; 4] {
    let bytes = value.as_bytes();
    if bytes.len() != 4 {
        panic!("AUv2 code must be exactly 4 ASCII bytes");
    }
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// One instance of the plugin, created each time the host loads the plugin.
///
/// The audio processing core is split into a [`Processor`] by [`PluginCore::activate`],
/// so this struct is responsible only for lifecycle management and holding the host
/// extension capabilities.
///
/// Capabilities are held behind `Arc` because the host (wrapper) may query them
/// re-entrantly during lifecycle callbacks, requiring them to be reachable without
/// acquiring the `&mut self` lock on `PluginCore`.
pub(crate) struct WracGainPlugin {
    // Parameter state shared by the audio thread, GUI, and host. See [`SharedState`].
    shared: Arc<SharedState>,
    // Audio layout negotiated with the host. Non-realtime only. See [`AudioLayoutStore`].
    audio_layout: Arc<AudioLayoutStore>,
    audio_ports: Arc<WracGainAudioPorts>,
    configurable_audio_ports: Arc<WracGainConfigurableAudioPorts>,
    parameters: Arc<WracGainParameters>,
    gui: Arc<WxpGuiController>,
    // Project state save/restore. A dedicated capability independent of the lifecycle
    // lock so that a committed snapshot can be returned even while active or during a
    // wrapper re-entry.
    state_support: Arc<WracGainStateSupport>,
}

impl WracGainPlugin {
    pub(crate) fn new(context: PluginCoreContext) -> Self {
        let shared = Arc::new(SharedState::new());
        let audio_layout = Arc::new(AudioLayoutStore::new(2));
        let audio_ports = Arc::new(WracGainAudioPorts::new(audio_layout.clone()));
        let configurable_audio_ports =
            Arc::new(WracGainConfigurableAudioPorts::new(audio_layout.clone()));
        let parameters = Arc::new(WracGainParameters::new(shared.clone()));
        let project_state = Arc::new(ProjectStateStore::new());
        let gui = create_gui_integration(
            project_state.clone(),
            shared.clone(),
            context.host_parameter_edit_notifier,
            context.host_gui_resize_requester,
        );
        let state_support = Arc::new(WracGainStateSupport::new(
            project_state,
            shared.clone(),
            gui.notifier.clone(),
        ));

        Self {
            shared,
            audio_layout,
            audio_ports,
            configurable_audio_ports,
            parameters,
            gui: gui.controller,
            state_support,
        }
    }
}

/// Called from this product's [`PluginFactory`] implementation.
/// Called each time the host requests a new instance; returns a [`PluginCore`].
pub(crate) fn create_plugin_core(context: PluginCoreContext) -> Box<dyn PluginCore> {
    wrac_log::init!(PLUGIN_DESCRIPTOR.name);

    log::debug!(
        "creating plugin core: id={}, name={}",
        PLUGIN_DESCRIPTOR.id,
        PLUGIN_DESCRIPTOR.name
    );
    for parameter in [gain_parameter_info(), bypass_parameter_info()] {
        log::info!(
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
    Box::new(WracGainPlugin::new(context))
}

// ---------------------------------------------------------------------------
// PluginCore: plugin lifecycle and the extension capabilities offered
// ---------------------------------------------------------------------------
impl PluginCore for WracGainPlugin {
    /// Called just before the host starts audio processing.
    /// The returned [`Processor`] is subsequently `process()`-ed on the audio thread.
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>> {
        // Boundary between the non-RT layout store and the RT processor.
        //
        // The adapter rejects layout changes while active, so the channel count
        // snapshotted here is contractually immutable until deactivate. Passing the
        // full `Arc<AudioLayoutStore>` would leave room for process() to acquire the
        // lock; copying only the needed value instead structurally enforces "the audio
        // thread sees only immutable configuration."
        let audio_channel_count = self.audio_layout.channel_count();
        log::debug!(
            "activating audio processor: sample_rate={}, min_frames_count={}, max_frames_count={}, audio_channel_count={}",
            context.sample_rate,
            context.min_frames_count,
            context.max_frames_count,
            audio_channel_count
        );
        Ok(Box::new(WracGainAudioProcessor::new(
            self.shared.clone(),
            audio_channel_count,
        )))
    }

    /// Called when the host stops audio processing. `_processor` is the value returned
    /// from `activate`; dropping it is sufficient cleanup.
    fn deactivate(&mut self, _processor: Box<dyn Processor>) -> PluginResult<()> {
        log::debug!("deactivating audio processor");
        Ok(())
    }

    // Extension declarations. Some = implemented, None = unsupported. Implementations live in separate modules.

    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPorts>> {
        Some(self.audio_ports.clone())
    }

    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPorts>> {
        Some(self.configurable_audio_ports.clone())
    }

    fn parameters(&self) -> Option<Arc<dyn PluginParameters>> {
        Some(self.parameters.clone())
    }

    fn state(&self) -> Option<Arc<dyn PluginStateSupport>> {
        Some(self.state_support.clone())
    }

    fn gui(&self) -> Option<Arc<dyn PluginGui>> {
        Some(self.gui.clone())
    }
}
