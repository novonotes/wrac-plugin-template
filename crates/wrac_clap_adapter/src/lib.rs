//! CLAP ABI と plugin core を繋ぐ adapter crate。
//!
//! 製品 crate は [`api`] の safe trait を実装し、[`export_clap_plugin!`] で CLAP
//! entry を宣言するだけでよい。`clap-sys`・raw pointer・event 変換・host callback
//! は adapter 内部に閉じる。trait の契約は `api.rs` を参照。

mod abi;
mod api;
mod descriptor;
mod events;
mod host_gui;
mod params;
mod process_buffer;

pub use api::{
    ActivateContext, AudioPortConfigurationRequest, AudioPortFlags, AudioPortInfo, AudioPortType,
    ClapWindow, GuiApi, GuiConfiguration, GuiResizeHints, GuiSize, HostGuiResizeRequester,
    HostParameterEditNotifier, NoteDialects, NotePortInfo, ParameterFlags, ParameterInfo,
    ParameterValueEvent, PluginAudioPorts, PluginConfigurableAudioPorts, PluginCore,
    PluginCoreContext, PluginError, PluginGui, PluginNotePorts, PluginParameters, PluginResult,
    PluginState, PluginStateSupport, ProcessContext, ProcessStatus, Processor,
};
pub use descriptor::{Auv2Descriptor, PluginDescriptor, PluginFeature};
pub use events::{
    InputEvent, InputEvents, NoteEvent, NoteExpressionEvent, OutputEvent, OutputEvents,
    ParameterGestureEvent, ParameterModEvent, ProcessEvents, TransportEvent, UnknownEvent,
};
pub use process_buffer::{
    AudioBufferError, AudioChannelPair, AudioPairedChannels, AudioPortChannels, AudioPortPair,
    AudioPortPairs, AudioProcessBuffer,
};

#[doc(hidden)]
pub mod __private {
    pub use clap_sys::entry::clap_plugin_entry;
    pub use clap_sys::version::CLAP_VERSION;
    pub use std::ffi::{c_char, c_void};

    pub use crate::abi::{entry_deinit, entry_get_factory, entry_init};
    pub use crate::descriptor::PluginRegistration;
}

#[macro_export]
macro_rules! export_clap_plugin {
    (descriptor: $descriptor:expr, create: $create:path $(,)?) => {
        #[allow(non_snake_case)]
        mod __wrac_clap_export {
            // CLAP entry symbol は binary ごとに 1 つ必要なので、adapter ではなく
            // 製品 crate 側で展開する。adapter は再利用可能なまま、descriptor と
            // factory の static lifetime を binary に閉じ込められる。
            static WRAC_CLAP_PLUGIN_REGISTRATION: $crate::__private::PluginRegistration =
                $crate::__private::PluginRegistration::new($descriptor, $create);

            unsafe extern "C" fn wrac_clap_entry_init(
                plugin_path: *const $crate::__private::c_char,
            ) -> bool {
                $crate::__private::entry_init(&WRAC_CLAP_PLUGIN_REGISTRATION, plugin_path)
            }

            unsafe extern "C" fn wrac_clap_entry_deinit() {
                $crate::__private::entry_deinit(&WRAC_CLAP_PLUGIN_REGISTRATION)
            }

            unsafe extern "C" fn wrac_clap_entry_get_factory(
                factory_id: *const $crate::__private::c_char,
            ) -> *const $crate::__private::c_void {
                $crate::__private::entry_get_factory(&WRAC_CLAP_PLUGIN_REGISTRATION, factory_id)
            }

            #[allow(unreachable_pub)]
            #[unsafe(no_mangle)]
            pub static clap_entry: $crate::__private::clap_plugin_entry =
                $crate::__private::clap_plugin_entry {
                    clap_version: $crate::__private::CLAP_VERSION,
                    init: Some(wrac_clap_entry_init),
                    deinit: Some(wrac_clap_entry_deinit),
                    get_factory: Some(wrac_clap_entry_get_factory),
                };

            #[allow(unreachable_pub)]
            #[unsafe(no_mangle)]
            pub extern "C" fn get_clap_entry() -> *const $crate::__private::clap_plugin_entry {
                &clap_entry
            }
        }
    };
}
