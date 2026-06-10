//! Adapter crate that connects the CLAP ABI to the plugin core.
//!
//! Product crates only need to implement the safe traits in [`api`] and declare
//! the CLAP entry with [`export_clap_entry!`]. `clap-sys`, raw pointers, event
//! conversion, and host callbacks are all encapsulated inside the adapter.
//! See `api.rs` for the trait contracts.

mod abi;
mod api;
mod descriptor;
mod entry;
mod events;
mod factory;
mod host_gui;
mod host_state;
mod params;
mod process_buffer;

pub use api::{
    ActivateContext, AudioPortConfigRequest, AudioPortFlags, AudioPortInfo, AudioPortType,
    DetectedHost, GuiApi, GuiConfig, GuiResizeHints, GuiSize, HostContext, HostFamily,
    HostGuiResizeRequester, HostParamsEditNotifier, HostStateDirtyNotifier, HostVersion,
    HostWindow, NoteDialects, NotePortInfo, ParamFlags, ParamInfo, ParamValueEvent,
    PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension, PluginCore,
    PluginCoreContext, PluginError, PluginFormat, PluginGuiExtension, PluginLatencyExtension,
    PluginNotePortsExtension, PluginParamsExtension, PluginRenderExtension, PluginRenderMode,
    PluginResult, PluginStateExtension, PluginTailExtension, ProcessContext, ProcessStatus,
    Processor, State, SystemContext,
};
pub use descriptor::{
    AaxDescriptor, AaxStemConfig, Auv2Descriptor, PluginDescriptor, PluginFeature, Vst3Descriptor,
};
pub use entry::{EntryContext, PluginEntry, PluginFactory};
pub use events::{
    InputEvent, InputEvents, Midi2Event, MidiEvent, MidiSysexEvent, NoteEvent, NoteExpressionEvent,
    OutputEvent, OutputEvents, ParamGestureEvent, ParamInputEvents, ParamModEvent, ProcessEvents,
    TransportEvent, TransportFlags, UnknownEvent,
};
pub use process_buffer::{
    AudioBufferError, AudioChannelPair, AudioPairedChannels, AudioPortChannels, AudioPortPair,
    AudioPortPairs, AudioProcessBuffer,
};

#[doc(hidden)]
pub mod __private {
    pub use crate::entry::EntryRegistration;

    #[repr(C)]
    pub struct ClapVersion {
        pub major: u32,
        pub minor: u32,
        pub revision: u32,
    }

    #[repr(C)]
    pub struct ClapPluginEntry {
        pub clap_version: ClapVersion,
        pub init: Option<unsafe extern "C" fn(plugin_path: usize) -> bool>,
        pub deinit: Option<unsafe extern "C" fn()>,
        pub get_factory: Option<unsafe extern "C" fn(factory_id: usize) -> usize>,
    }

    pub const CLAP_VERSION: ClapVersion = ClapVersion {
        major: ::clap_sys::version::CLAP_VERSION.major,
        minor: ::clap_sys::version::CLAP_VERSION.minor,
        revision: ::clap_sys::version::CLAP_VERSION.revision,
    };

    pub unsafe fn entry_init(registration: &'static EntryRegistration, plugin_path: usize) -> bool {
        unsafe { crate::abi::entry_init(registration, plugin_path as *const ::std::ffi::c_char) }
    }

    pub unsafe fn entry_deinit(registration: &'static EntryRegistration) {
        unsafe { crate::abi::entry_deinit(registration) }
    }

    pub unsafe fn entry_get_factory(
        registration: &'static EntryRegistration,
        factory_id: usize,
    ) -> usize {
        unsafe {
            crate::abi::entry_get_factory(registration, factory_id as *const ::std::ffi::c_char)
                as usize
        }
    }
}

#[macro_export]
macro_rules! export_clap_entry {
    (entry: $entry:expr $(,)?) => {
        #[allow(non_snake_case)]
        mod __wrac_clap_export {
            // The CLAP entry symbol must appear exactly once per binary, so this macro
            // expands in the product crate rather than in the adapter. The adapter
            // stays reusable while entry and factory storage lifetimes are confined to
            // the binary.
            static WRAC_CLAP_ENTRY_REGISTRATION: $crate::__private::EntryRegistration =
                $crate::__private::EntryRegistration::new($entry);

            unsafe extern "C" fn wrac_clap_entry_init(plugin_path: usize) -> bool {
                $crate::__private::entry_init(&WRAC_CLAP_ENTRY_REGISTRATION, plugin_path)
            }

            unsafe extern "C" fn wrac_clap_entry_deinit() {
                $crate::__private::entry_deinit(&WRAC_CLAP_ENTRY_REGISTRATION)
            }

            unsafe extern "C" fn wrac_clap_entry_get_factory(factory_id: usize) -> usize {
                $crate::__private::entry_get_factory(&WRAC_CLAP_ENTRY_REGISTRATION, factory_id)
            }

            #[allow(unreachable_pub)]
            #[unsafe(no_mangle)]
            pub static clap_entry: $crate::__private::ClapPluginEntry =
                $crate::__private::ClapPluginEntry {
                    clap_version: $crate::__private::CLAP_VERSION,
                    init: Some(wrac_clap_entry_init),
                    deinit: Some(wrac_clap_entry_deinit),
                    get_factory: Some(wrac_clap_entry_get_factory),
                };

            #[allow(unreachable_pub)]
            #[unsafe(no_mangle)]
            pub extern "C" fn get_clap_entry() -> usize {
                (&clap_entry as *const $crate::__private::ClapPluginEntry) as usize
            }
        }
    };
}
