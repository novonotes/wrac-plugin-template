use std::ffi::{CString, c_char};
use std::ptr;

pub(crate) use crate::interface::AaxStemConfig;
use crate::interface::PluginDescriptor;
use clap_sys::plugin::clap_plugin_descriptor;
use clap_sys::version::CLAP_VERSION;

use crate::factory::{ClapPluginInfoAsAax, ClapPluginInfoAsVst3};

// `clap_plugin_descriptor` holds only C string pointers, so the owners of the CString
// and feature pointer arrays are placed in the same storage to keep their lifetimes
// aligned with the descriptor pointer.
pub(crate) struct ClapDescriptorStorage {
    descriptor: PluginDescriptor,
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
    // `vst3_info` exposes raw pointers to these owners through clap-wrapper's
    // factory extension; keep them in the same storage as the descriptor.
    _vst3_subcategories: Option<CString>,
    _vst3_component_id: Option<Box<[u8; 16]>>,
    vst3_info: Option<ClapPluginInfoAsVst3>,
    // AAX package info is returned through a factory extension before any plugin
    // instance exists, so the raw pointer owner must live with descriptor storage.
    _aax_package_name: Option<CString>,
    aax_info: Option<ClapPluginInfoAsAax>,
    clap_descriptor: clap_plugin_descriptor,
}

// Safety: the descriptor storage is not mutated after initialization. Raw pointers point
// into CString/Vec fields owned by this struct, not external memory, so sharing them
// causes no data race.
unsafe impl Sync for ClapDescriptorStorage {}
unsafe impl Send for ClapDescriptorStorage {}

impl ClapDescriptorStorage {
    pub(crate) fn new(descriptor: PluginDescriptor) -> Self {
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
        let vst3_subcategories = descriptor.vst3.map(|vst3| cstring(vst3.subcategories));
        let vst3_component_id = descriptor.vst3.map(|vst3| Box::new(vst3.component_id));
        let aax_package_name = descriptor.aax.map(|aax| cstring(aax.package_name));
        let vst3_info = descriptor.vst3.map(|_| ClapPluginInfoAsVst3 {
            vendor: vendor.as_ptr(),
            component_id: vst3_component_id
                .as_deref()
                .map_or(ptr::null(), |value| value as *const [u8; 16]),
            features: vst3_subcategories
                .as_ref()
                .map(|value| value.as_ptr())
                .unwrap_or(ptr::null()),
        });
        let aax_info = descriptor.aax.map(|aax| ClapPluginInfoAsAax {
            aax_features: aax.categories,
            id_manufacturer: aax.manufacturer_id,
            id_product: aax.product_id,
            midi_in_name: ptr::null(),
            midi_out_name: ptr::null(),
            midi_in_channel_mask: 0,
            midi_out_channel_mask: 0,
            get_num_stem_configs: Some(aax.get_num_stem_configs),
            get_stem_config: Some(aax.get_stem_config),
        });

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
            descriptor,
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
            _vst3_subcategories: vst3_subcategories,
            _vst3_component_id: vst3_component_id,
            vst3_info,
            _aax_package_name: aax_package_name,
            aax_info,
            clap_descriptor,
        }
    }

    pub(crate) fn clap_descriptor(&self) -> *const clap_plugin_descriptor {
        &self.clap_descriptor
    }

    pub(crate) fn vendor_ptr(&self) -> *const c_char {
        self.clap_descriptor.vendor
    }

    pub(crate) fn url_ptr(&self) -> *const c_char {
        self.clap_descriptor.url
    }

    pub(crate) fn auv2_manufacturer_code_ptr(&self) -> Option<*const c_char> {
        self.auv2_manufacturer_code
            .as_ref()
            .map(|value| value.as_ptr())
    }

    pub(crate) fn auv2_manufacturer_name_ptr(&self) -> Option<*const c_char> {
        self.auv2_manufacturer_name
            .as_ref()
            .map(|value| value.as_ptr())
    }

    pub(crate) fn vst3_info_ptr(&self) -> Option<*const ClapPluginInfoAsVst3> {
        self.vst3_info
            .as_ref()
            .map(|value| value as *const ClapPluginInfoAsVst3)
    }

    pub(crate) fn aax_info_ptr(&self) -> Option<*const ClapPluginInfoAsAax> {
        self.aax_info
            .as_ref()
            .map(|value| value as *const ClapPluginInfoAsAax)
    }

    pub(crate) fn aax_package_name_ptr(&self) -> Option<*const c_char> {
        self._aax_package_name.as_ref().map(|value| value.as_ptr())
    }

    pub(crate) fn aax_package_version(&self) -> Option<u32> {
        self.descriptor.aax.map(|aax| aax.package_version)
    }

    pub(crate) fn descriptor(&self) -> PluginDescriptor {
        self.descriptor
    }
}

fn cstring(value: &'static str) -> CString {
    CString::new(value).expect("plugin descriptor strings must not contain NUL bytes")
}
