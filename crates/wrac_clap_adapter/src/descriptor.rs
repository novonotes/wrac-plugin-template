use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::OnceLock;

use clap_sys::factory::plugin_factory::clap_plugin_factory;
use clap_sys::plugin::clap_plugin_descriptor;
use clap_sys::plugin_features::{
    CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_DISTORTION,
    CLAP_PLUGIN_FEATURE_INSTRUMENT, CLAP_PLUGIN_FEATURE_LIMITER, CLAP_PLUGIN_FEATURE_MONO,
    CLAP_PLUGIN_FEATURE_STEREO, CLAP_PLUGIN_FEATURE_UTILITY,
};
use clap_sys::version::CLAP_VERSION;

use crate::{PluginCore, PluginCoreContext};

pub(crate) type CreatePluginCore = fn(PluginCoreContext) -> Box<dyn PluginCore>;

#[derive(Debug, Clone, Copy)]
pub struct PluginDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor: &'static str,
    pub url: &'static str,
    pub manual_url: &'static str,
    pub support_url: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub features: &'static [PluginFeature],
    pub auv2: Option<Auv2Descriptor>,
}

#[derive(Debug, Clone, Copy)]
pub enum PluginFeature {
    AudioEffect,
    Instrument,
    Distortion,
    Limiter,
    Mono,
    Stereo,
    Utility,
}

impl PluginFeature {
    fn as_cstr(self) -> &'static CStr {
        match self {
            Self::AudioEffect => CLAP_PLUGIN_FEATURE_AUDIO_EFFECT,
            Self::Instrument => CLAP_PLUGIN_FEATURE_INSTRUMENT,
            Self::Distortion => CLAP_PLUGIN_FEATURE_DISTORTION,
            Self::Limiter => CLAP_PLUGIN_FEATURE_LIMITER,
            Self::Mono => CLAP_PLUGIN_FEATURE_MONO,
            Self::Stereo => CLAP_PLUGIN_FEATURE_STEREO,
            Self::Utility => CLAP_PLUGIN_FEATURE_UTILITY,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Auv2Descriptor {
    pub manufacturer_code: [u8; 4],
    pub manufacturer_name: &'static str,
    pub plugin_type: [u8; 4],
    pub plugin_subtype: [u8; 4],
}

/// plugin binary に固定される descriptor と factory 関数。
///
/// factory callback は任意タイミングで呼ばれるので static に置く。不変にすることで
/// global mutable registry や pointer リークに頼らず C ABI に渡せる。
pub struct PluginRegistration {
    pub(crate) descriptor: PluginDescriptor,
    pub(crate) create: CreatePluginCore,
    storage: OnceLock<PluginRegistrationStorage>,
}

// 安全性: `descriptor` と `create` は static registration の不変データで、mutable state は
// `OnceLock` が同期する。C ABI から複数 thread で factory query されても共有参照だけを返す。
unsafe impl Sync for PluginRegistration {}
unsafe impl Send for PluginRegistration {}

impl PluginRegistration {
    pub const fn new(descriptor: PluginDescriptor, create: CreatePluginCore) -> Self {
        Self {
            descriptor,
            create,
            storage: OnceLock::new(),
        }
    }

    pub(crate) fn storage(&'static self) -> &'static PluginRegistrationStorage {
        self.storage
            .get_or_init(|| PluginRegistrationStorage::new(self))
    }
}

pub(crate) struct PluginRegistrationStorage {
    pub clap_factory: ClapFactoryState,
    pub auv2_factory: Auv2FactoryState,
    pub descriptor: ClapDescriptorStorage,
}

// 安全性: storage 作成後は factory/descriptor の pointer を読み出すだけにしている。
// 内部 pointer は同じ storage が所有する buffer を指し、`OnceLock` で初期化競合も防ぐ。
unsafe impl Sync for PluginRegistrationStorage {}
unsafe impl Send for PluginRegistrationStorage {}

impl PluginRegistrationStorage {
    fn new(registration: &'static PluginRegistration) -> Self {
        let descriptor = ClapDescriptorStorage::new(registration.descriptor);
        Self {
            clap_factory: ClapFactoryState {
                factory: clap_plugin_factory {
                    get_plugin_count: Some(crate::abi::factory_get_plugin_count),
                    get_plugin_descriptor: Some(crate::abi::factory_get_plugin_descriptor),
                    create_plugin: Some(crate::abi::factory_create_plugin),
                },
                registration,
            },
            auv2_factory: Auv2FactoryState {
                factory: ClapPluginFactoryAsAuv2 {
                    manufacturer_code: descriptor.auv2_manufacturer_code_ptr(),
                    manufacturer_name: descriptor.auv2_manufacturer_name_ptr(),
                    get_auv2_info: Some(crate::abi::auv2_get_info),
                },
                registration,
            },
            descriptor,
        }
    }
}

// CLAP factory callback は factory pointer だけを受け取るため、先頭 field に C ABI
// struct を置き、callback 内で state へ戻す。
#[repr(C)]
pub(crate) struct ClapFactoryState {
    pub factory: clap_plugin_factory,
    pub registration: &'static PluginRegistration,
}

unsafe impl Sync for ClapFactoryState {}
unsafe impl Send for ClapFactoryState {}

#[repr(C)]
pub(crate) struct Auv2FactoryState {
    pub factory: ClapPluginFactoryAsAuv2,
    pub registration: &'static PluginRegistration,
}

unsafe impl Sync for Auv2FactoryState {}
unsafe impl Send for Auv2FactoryState {}

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

// `clap_plugin_descriptor` は C string pointer だけを保持するため、CString/feature
// pointer の owner を同じ storage に置いて descriptor pointer の有効期間を揃える。
pub(crate) struct ClapDescriptorStorage {
    _id: CString,
    _name: CString,
    _vendor: CString,
    _url: CString,
    _manual_url: CString,
    _support_url: CString,
    _version: CString,
    _description: CString,
    _feature_ptrs: Vec<*const c_char>,
    auv2_manufacturer_code: Option<CString>,
    auv2_manufacturer_name: Option<CString>,
    clap_descriptor: clap_plugin_descriptor,
}

// 安全性: descriptor storage は初期化後に変更しない。raw pointer は外部所有物ではなく、
// この struct 内の CString/Vec へ向いているため、共有しても data race にはならない。
unsafe impl Sync for ClapDescriptorStorage {}
unsafe impl Send for ClapDescriptorStorage {}

impl ClapDescriptorStorage {
    fn new(descriptor: PluginDescriptor) -> Self {
        let id = cstring(descriptor.id);
        let name = cstring(descriptor.name);
        let vendor = cstring(descriptor.vendor);
        let url = cstring(descriptor.url);
        let manual_url = cstring(descriptor.manual_url);
        let support_url = cstring(descriptor.support_url);
        let version = cstring(descriptor.version);
        let description = cstring(descriptor.description);

        let mut feature_ptrs = descriptor
            .features
            .iter()
            .map(|feature| feature.as_cstr().as_ptr())
            .collect::<Vec<_>>();
        feature_ptrs.push(ptr::null());

        let auv2_manufacturer_code = descriptor
            .auv2
            .map(|auv2| CString::new(auv2.manufacturer_code).expect("four char code"));
        let auv2_manufacturer_name = descriptor.auv2.map(|auv2| cstring(auv2.manufacturer_name));

        let clap_descriptor = clap_plugin_descriptor {
            clap_version: CLAP_VERSION,
            id: id.as_ptr(),
            name: name.as_ptr(),
            vendor: vendor.as_ptr(),
            url: url.as_ptr(),
            manual_url: manual_url.as_ptr(),
            support_url: support_url.as_ptr(),
            version: version.as_ptr(),
            description: description.as_ptr(),
            features: feature_ptrs.as_ptr(),
        };

        Self {
            _id: id,
            _name: name,
            _vendor: vendor,
            _url: url,
            _manual_url: manual_url,
            _support_url: support_url,
            _version: version,
            _description: description,
            _feature_ptrs: feature_ptrs,
            auv2_manufacturer_code,
            auv2_manufacturer_name,
            clap_descriptor,
        }
    }

    pub(crate) fn clap_descriptor(&self) -> *const clap_plugin_descriptor {
        &self.clap_descriptor
    }

    fn auv2_manufacturer_code_ptr(&self) -> *const c_char {
        self.auv2_manufacturer_code
            .as_ref()
            .map_or(ptr::null(), |value| value.as_ptr())
    }

    fn auv2_manufacturer_name_ptr(&self) -> *const c_char {
        self.auv2_manufacturer_name
            .as_ref()
            .map_or(ptr::null(), |value| value.as_ptr())
    }
}

fn cstring(value: &'static str) -> CString {
    CString::new(value).expect("plugin descriptor strings must not contain NUL bytes")
}

pub(crate) fn clap_factory_state(
    factory: *const clap_plugin_factory,
) -> Option<&'static ClapFactoryState> {
    if factory.is_null() {
        return None;
    }
    Some(unsafe { &*(factory as *const ClapFactoryState) })
}

pub(crate) fn auv2_factory_state(
    factory: *const ClapPluginFactoryAsAuv2,
) -> Option<&'static Auv2FactoryState> {
    if factory.is_null() {
        return None;
    }
    Some(unsafe { &*(factory as *const Auv2FactoryState) })
}

pub(crate) fn factory_ptr(storage: &'static PluginRegistrationStorage) -> *const c_void {
    &storage.clap_factory.factory as *const clap_plugin_factory as *const c_void
}

pub(crate) fn auv2_factory_ptr(storage: &'static PluginRegistrationStorage) -> *const c_void {
    &storage.auv2_factory.factory as *const ClapPluginFactoryAsAuv2 as *const c_void
}
