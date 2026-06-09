use std::ffi::{c_char, c_void};
use std::ptr;

use clap_sys::factory::plugin_factory::clap_plugin_factory;
use clap_sys::plugin::clap_plugin;

use crate::descriptor::{AaxStemConfig, ClapDescriptorStorage};
use crate::entry::EntryRegistration;

pub(crate) struct PluginRegistrationStorage {
    pub clap_factory: ClapFactoryState,
    pub main_thread_hook: MainThreadHookState,
    pub auv2_factory: Auv2FactoryState,
    pub vst3_factory: Vst3FactoryState,
    pub aax_factory: AaxFactoryState,
    pub descriptors: Vec<ClapDescriptorStorage>,
}

// Safety: after creation the storage only reads out factory/descriptor pointers.
// Internal pointers point to buffers owned by this same storage, and `OnceLock`
// prevents initialization races.
unsafe impl Sync for PluginRegistrationStorage {}
unsafe impl Send for PluginRegistrationStorage {}

impl PluginRegistrationStorage {
    pub(crate) fn new(registration: &'static EntryRegistration) -> Self {
        let descriptors = registration
            .entry
            .plugin_factory()
            .map(|factory| {
                (0..factory.plugin_count())
                    .filter_map(|index| factory.plugin_descriptor(index))
                    .map(ClapDescriptorStorage::new)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self {
            clap_factory: ClapFactoryState {
                factory: clap_plugin_factory {
                    get_plugin_count: Some(crate::abi::factory_get_plugin_count),
                    get_plugin_descriptor: Some(crate::abi::factory_get_plugin_descriptor),
                    create_plugin: Some(crate::abi::factory_create_plugin),
                },
                registration,
            },
            main_thread_hook: MainThreadHookState {
                hook: WracPluginMainThreadHook {
                    attach_main_thread: Some(crate::abi::main_thread_hook_attach),
                    detach_main_thread: Some(crate::abi::main_thread_hook_detach),
                },
                registration,
            },
            auv2_factory: Auv2FactoryState {
                factory: ClapPluginFactoryAsAuv2 {
                    manufacturer_code: descriptors
                        .iter()
                        .find_map(ClapDescriptorStorage::auv2_manufacturer_code_ptr)
                        .unwrap_or(ptr::null()),
                    manufacturer_name: descriptors
                        .iter()
                        .find_map(ClapDescriptorStorage::auv2_manufacturer_name_ptr)
                        .unwrap_or(ptr::null()),
                    get_auv2_info: Some(crate::abi::auv2_get_info),
                },
                registration,
            },
            vst3_factory: Vst3FactoryState {
                factory: ClapPluginFactoryAsVst3 {
                    vendor: descriptors
                        .first()
                        .map(ClapDescriptorStorage::vendor_ptr)
                        .unwrap_or(ptr::null()),
                    vendor_url: descriptors
                        .first()
                        .map(ClapDescriptorStorage::url_ptr)
                        .unwrap_or(ptr::null()),
                    email_contact: ptr::null(),
                    get_vst3_info: Some(crate::abi::vst3_get_info),
                },
                registration,
            },
            aax_factory: AaxFactoryState {
                factory: ClapPluginFactoryAsAax {
                    // AAX package fields describe the binary package, not an individual
                    // product. clap-wrapper reads them before selecting a plugin index.
                    package_name: descriptors
                        .iter()
                        .find_map(ClapDescriptorStorage::aax_package_name_ptr)
                        .unwrap_or(ptr::null()),
                    package_manufacturer: descriptors
                        .first()
                        .map(ClapDescriptorStorage::vendor_ptr)
                        .unwrap_or(ptr::null()),
                    package_version: descriptors
                        .iter()
                        .find_map(ClapDescriptorStorage::aax_package_version)
                        .unwrap_or(0),
                    get_aax_info: Some(crate::abi::aax_get_info),
                    // The template describes static AAX products only. Exposing a
                    // configuration callback before there is product-side policy for
                    // host-requested layout changes would make clap-wrapper believe
                    // dynamic AAX configurations are supported.
                    can_apply_configuration: None,
                },
                registration,
            },
            descriptors,
        }
    }
}

// CLAP factory callbacks receive only a factory pointer, so the C ABI struct is placed
// as the first field and cast back to the state inside the callback.
#[repr(C)]
pub(crate) struct ClapFactoryState {
    pub factory: clap_plugin_factory,
    pub registration: &'static EntryRegistration,
}

unsafe impl Sync for ClapFactoryState {}
unsafe impl Send for ClapFactoryState {}

#[repr(C)]
pub(crate) struct MainThreadHookState {
    pub hook: WracPluginMainThreadHook,
    pub registration: &'static EntryRegistration,
}

unsafe impl Sync for MainThreadHookState {}
unsafe impl Send for MainThreadHookState {}

#[repr(C)]
pub(crate) struct Auv2FactoryState {
    pub factory: ClapPluginFactoryAsAuv2,
    pub registration: &'static EntryRegistration,
}

unsafe impl Sync for Auv2FactoryState {}
unsafe impl Send for Auv2FactoryState {}

#[repr(C)]
pub(crate) struct Vst3FactoryState {
    pub factory: ClapPluginFactoryAsVst3,
    pub registration: &'static EntryRegistration,
}

unsafe impl Sync for Vst3FactoryState {}
unsafe impl Send for Vst3FactoryState {}

#[repr(C)]
pub(crate) struct AaxFactoryState {
    pub factory: ClapPluginFactoryAsAax,
    pub registration: &'static EntryRegistration,
}

unsafe impl Sync for AaxFactoryState {}
unsafe impl Send for AaxFactoryState {}

#[repr(C)]
pub(crate) struct ClapPluginInfoAsAuv2 {
    pub au_type: [c_char; 5],
    pub au_subt: [c_char; 5],
}

#[repr(C)]
pub(crate) struct ClapPluginFactoryAsAuv2 {
    pub manufacturer_code: *const c_char,
    pub manufacturer_name: *const c_char,
    pub get_auv2_info: Option<
        unsafe extern "C" fn(
            factory: *const ClapPluginFactoryAsAuv2,
            index: u32,
            info: *mut ClapPluginInfoAsAuv2,
        ) -> bool,
    >,
}

unsafe impl Sync for ClapPluginFactoryAsAuv2 {}
unsafe impl Send for ClapPluginFactoryAsAuv2 {}

#[repr(C)]
pub(crate) struct ClapPluginInfoAsVst3 {
    pub vendor: *const c_char,
    pub component_id: *const [u8; 16],
    pub features: *const c_char,
}

unsafe impl Sync for ClapPluginInfoAsVst3 {}
unsafe impl Send for ClapPluginInfoAsVst3 {}

// Mirrors clap-wrapper's VST3 factory-info extension ABI. Keep this local copy
// aligned with free-audio/clap-wrapper `next`; clap-sys intentionally does not
// define wrapper-private factory extensions.
#[repr(C)]
pub(crate) struct ClapPluginFactoryAsVst3 {
    pub vendor: *const c_char,
    pub vendor_url: *const c_char,
    pub email_contact: *const c_char,
    pub get_vst3_info: Option<
        unsafe extern "C" fn(
            factory: *const ClapPluginFactoryAsVst3,
            index: u32,
        ) -> *const ClapPluginInfoAsVst3,
    >,
}

unsafe impl Sync for ClapPluginFactoryAsVst3 {}
unsafe impl Send for ClapPluginFactoryAsVst3 {}

#[repr(C)]
pub(crate) struct ClapPluginInfoAsAax {
    pub aax_features: u32,
    pub id_manufacturer: u32,
    pub id_product: u32,
    pub midi_in_name: *const c_char,
    pub midi_out_name: *const c_char,
    pub midi_in_channel_mask: u32,
    pub midi_out_channel_mask: u32,
    pub get_num_stem_configs: Option<unsafe extern "C" fn() -> u32>,
    pub get_stem_config: Option<unsafe extern "C" fn(index: u32) -> *const AaxStemConfig>,
}

unsafe impl Sync for ClapPluginInfoAsAax {}
unsafe impl Send for ClapPluginInfoAsAax {}

// Mirrors clap-wrapper's `clap_plugin_factory_as_aax` extension ABI. Keep the
// field order and nullability aligned with free-audio/clap-wrapper `next`; unlike
// the CLAP SDK structs, this extension is not provided by clap-sys.
#[repr(C)]
pub(crate) struct ClapPluginFactoryAsAax {
    pub package_name: *const c_char,
    pub package_manufacturer: *const c_char,
    pub package_version: u32,
    pub get_aax_info: Option<
        unsafe extern "C" fn(
            factory: *const ClapPluginFactoryAsAax,
            index: u32,
        ) -> *const ClapPluginInfoAsAax,
    >,
    pub can_apply_configuration: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            requests: *const c_void,
            request_count: u32,
        ) -> bool,
    >,
}

unsafe impl Sync for ClapPluginFactoryAsAax {}
unsafe impl Send for ClapPluginFactoryAsAax {}

#[repr(C)]
pub(crate) struct WracPluginMainThreadHook {
    pub attach_main_thread: Option<unsafe extern "C" fn(hook: *const WracPluginMainThreadHook)>,
    pub detach_main_thread: Option<unsafe extern "C" fn(hook: *const WracPluginMainThreadHook)>,
}

unsafe impl Sync for WracPluginMainThreadHook {}
unsafe impl Send for WracPluginMainThreadHook {}

pub(crate) fn clap_factory_state(
    factory: *const clap_plugin_factory,
) -> Option<&'static ClapFactoryState> {
    if factory.is_null() {
        return None;
    }
    Some(unsafe { &*(factory as *const ClapFactoryState) })
}

pub(crate) fn main_thread_hook_state(
    hook: *const WracPluginMainThreadHook,
) -> Option<&'static MainThreadHookState> {
    if hook.is_null() {
        return None;
    }
    Some(unsafe { &*(hook as *const MainThreadHookState) })
}

pub(crate) fn auv2_factory_state(
    factory: *const ClapPluginFactoryAsAuv2,
) -> Option<&'static Auv2FactoryState> {
    if factory.is_null() {
        return None;
    }
    Some(unsafe { &*(factory as *const Auv2FactoryState) })
}

pub(crate) fn vst3_factory_state(
    factory: *const ClapPluginFactoryAsVst3,
) -> Option<&'static Vst3FactoryState> {
    if factory.is_null() {
        return None;
    }
    Some(unsafe { &*(factory as *const Vst3FactoryState) })
}

pub(crate) fn aax_factory_state(
    factory: *const ClapPluginFactoryAsAax,
) -> Option<&'static AaxFactoryState> {
    if factory.is_null() {
        return None;
    }
    Some(unsafe { &*(factory as *const AaxFactoryState) })
}

pub(crate) fn factory_ptr(storage: &'static PluginRegistrationStorage) -> *const c_void {
    &storage.clap_factory.factory as *const clap_plugin_factory as *const c_void
}

pub(crate) fn main_thread_hook_ptr(storage: &'static PluginRegistrationStorage) -> *const c_void {
    &storage.main_thread_hook.hook as *const WracPluginMainThreadHook as *const c_void
}

pub(crate) fn auv2_factory_ptr(storage: &'static PluginRegistrationStorage) -> *const c_void {
    &storage.auv2_factory.factory as *const ClapPluginFactoryAsAuv2 as *const c_void
}

pub(crate) fn vst3_factory_ptr(storage: &'static PluginRegistrationStorage) -> *const c_void {
    &storage.vst3_factory.factory as *const ClapPluginFactoryAsVst3 as *const c_void
}

pub(crate) fn aax_factory_ptr(storage: &'static PluginRegistrationStorage) -> *const c_void {
    &storage.aax_factory.factory as *const ClapPluginFactoryAsAax as *const c_void
}
