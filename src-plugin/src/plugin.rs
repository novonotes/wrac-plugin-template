//! host から見える plugin の契約をまとめる場所。
//!
//! ここで宣言するもの:
//! 1. plugin の自己紹介 ([`PLUGIN_DESCRIPTOR`])
//! 2. audio / GUI / host で共有する [`SharedState`]
//! 3. activate 時に渡す audio [`Processor`] と、host extension capability の束ね方
//!
//! parameter / audio port / state persistence の実装は `plugin/` 配下に分ける。
//! CLAP / VST3 / AU の format 差分は `wrac_clap_adapter` が吸収するので、ここは
//! 「この plugin がどの capability を持つか」だけに集中できる。

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
    PluginCoreContext, PluginDescriptor, PluginFeature, PluginGui, PluginParameters, PluginResult,
    PluginStateSupport, Processor,
};
use wrac_wxp_gui::WxpGuiController;

use crate::audio::WracGainAudioProcessor;
use crate::gui::create_gui_integration;
use crate::state::{ProjectStateStore, SharedState};

// plugin identity の SoT は src-plugin/Cargo.toml の [package.metadata.wrac]。
// GUI / xtask / wrapper build も同じ metadata を読むので、ここを直書きせず
// env! 経由にすることで rename 時の不整合 (bundle 名や About 表示のズレ) を防ぐ。
pub(crate) const PLUGIN_ID: &str = env!("WRAC_PLUGIN_ID");
pub(crate) const PLUGIN_NAME: &str = env!("WRAC_PLUGIN_NAME");
pub(crate) const COMPANY_NAME: &str = env!("WRAC_COMPANY_NAME");
const AUV2_TYPE: [u8; 4] = four_char_code(env!("WRAC_AUV2_TYPE"));
const AUV2_SUBTYPE: [u8; 4] = four_char_code(env!("WRAC_AUV2_SUBTYPE"));
const AUV2_MANUFACTURER_CODE: [u8; 4] = four_char_code(env!("WRAC_AUV2_MANUFACTURER_CODE"));

// host への自己紹介。adapter がこれを CLAP / AUv2 の descriptor に変換する。
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
    // AUv2 (macOS Audio Unit v2) 用。code 類は 4 文字 ASCII の固有 ID で、
    // 同じ会社の他 plugin と重複させない。
    auv2: Some(Auv2Descriptor {
        manufacturer_code: AUV2_MANUFACTURER_CODE,
        manufacturer_name: COMPANY_NAME,
        plugin_type: AUV2_TYPE,
        plugin_subtype: AUV2_SUBTYPE,
    }),
};

const fn four_char_code(value: &str) -> [u8; 4] {
    let bytes = value.as_bytes();
    if bytes.len() != 4 {
        panic!("AUv2 code must be exactly 4 ASCII bytes");
    }
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// plugin 1 instance。host が plugin を読み込むごとに 1 つ作られる。
///
/// audio 処理本体は [`PluginCore::activate`] で [`Processor`] に切り出すので、
/// この struct 自体は lifecycle と host へ公開する extension の保持だけを担う。
///
/// extension capability を `Arc` で持つのは、host (wrapper) が lifecycle callback の
/// 最中に capability を再入 query してくるため。`PluginCore` の `&mut self` lock を
/// 取らずに到達できる必要がある。
pub(crate) struct WracGainPlugin {
    // audio / GUI / host が共有する parameter state。詳細は [`SharedState`]。
    shared: Arc<SharedState>,
    // host と交渉した audio layout。non-realtime 専用。詳細は [`AudioLayoutStore`]。
    audio_layout: Arc<AudioLayoutStore>,
    audio_ports: Arc<WracGainAudioPorts>,
    configurable_audio_ports: Arc<WracGainConfigurableAudioPorts>,
    parameters: Arc<WracGainParameters>,
    gui: Arc<WxpGuiController>,
    // project state の save/restore。active 中や wrapper 再入中でも committed
    // snapshot を返せるよう、lifecycle lock から独立した専用 capability にしている。
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

/// [`wrac_clap_adapter::export_clap_plugin!`] から呼ばれる factory。
/// host が instance を要求するたびに呼ばれ、[`PluginCore`] を返す。
pub(crate) fn create_plugin_core(context: PluginCoreContext) -> Box<dyn PluginCore> {
    crate::logging::init_debug_logging_once(PLUGIN_DESCRIPTOR.name);

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
// PluginCore: plugin の lifecycle と、提供する extension の宣言
// ---------------------------------------------------------------------------
impl PluginCore for WracGainPlugin {
    /// host が audio 処理を開始する直前に呼ばれる。
    /// ここで返した [`Processor`] が以降 audio thread 上で `process()` される。
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>> {
        // non-RT layout store と RT processor の境界。
        //
        // adapter は active 中の layout apply を拒否するので、ここで snapshot した
        // channel count は deactivate まで不変な契約になる。`Arc<AudioLayoutStore>`
        // ごと渡すと process() 中に lock を読む余地が残るため、必要な値だけ copy して
        // 渡す。これで「audio thread は immutable な設定だけを見る」が構造で保証される。
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

    /// host が audio 処理を停止したときに呼ばれる。`_processor` は `activate` で
    /// 返した実体で、drop されれば後始末は済む。
    fn deactivate(&mut self, _processor: Box<dyn Processor>) -> PluginResult<()> {
        log::debug!("deactivating audio processor");
        Ok(())
    }

    // 各 extension の宣言。Some = 実装あり / None = 未対応。本体は別 module。

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
