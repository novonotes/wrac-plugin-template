//! Adapter crate that connects the CLAP ABI to the plugin core.
//!
//! Product crates implement the contracts from `wrac_interface` and use this crate only
//! to export the CLAP entry. ABI callbacks, registration storage, and host proxy
//! implementations stay confined to this adapter.

mod abi;
mod descriptor;
mod entry;
mod factory;
mod host_audio_ports;
mod host_gui;
mod host_latency;
mod host_lifecycle;
mod host_note_ports;
mod host_state;
mod host_tail;
mod params;
// Adapter modules share the complete contract vocabulary internally, while none of these names
// are re-exported from the adapter crate.
#[allow(unused_imports)]
use wrac_interface::*;
#[cfg(feature = "raw-clap-forwarding")]
use wrac_interface::{RawParamFlushContext, RawProcessContext};

/// Macro support items used by [`export_clap_entry!`].
///
/// These items stay public because exported macros must refer to them through
/// `$crate`. They are not part of the plugin authoring API: plugin code should
/// not import, call, or rely on them directly.
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

    /// Initializes the registered CLAP entry.
    ///
    /// # Safety
    ///
    /// `plugin_path` must be a valid CLAP plugin path pointer provided by the host for
    /// the duration of this call.
    pub unsafe fn entry_init(registration: &'static EntryRegistration, plugin_path: usize) -> bool {
        unsafe { crate::abi::entry_init(registration, plugin_path as *const ::std::ffi::c_char) }
    }

    /// Deinitializes the registered CLAP entry.
    ///
    /// # Safety
    ///
    /// The caller must ensure this is called according to the CLAP entry lifecycle,
    /// after initialization and while no plugin factory call is active.
    pub unsafe fn entry_deinit(registration: &'static EntryRegistration) {
        unsafe { crate::abi::entry_deinit(registration) }
    }

    /// Returns a factory pointer from the registered CLAP entry.
    ///
    /// # Safety
    ///
    /// `factory_id` must be a valid CLAP factory identifier pointer provided by the
    /// host for the duration of this call.
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
