//! AAX discovers sidechains before instances exist, including in multi-plugin libraries.
use std::ptr;

use super::entry_get_factory;
use crate::entry::EntryRegistration;
use crate::factory::WracPluginFactoryAaxSidechain;
use crate::interface::{
    AaxDescriptor, AaxSidechainInput, AaxStemConfig, LogConfig, PluginDescriptor, PluginEntry,
    PluginFactory, PluginInstance, PluginInstanceContext,
};

struct Entry {
    sidechain: AaxSidechainInput,
}

impl PluginEntry for Entry {
    fn log_config(&'static self) -> Option<&'static LogConfig> {
        None
    }

    fn plugin_factory(&self) -> Option<&dyn PluginFactory> {
        Some(self)
    }
}

impl PluginFactory for Entry {
    fn plugin_count(&self) -> u32 {
        3
    }

    fn plugin_descriptor(&self, index: u32) -> Option<PluginDescriptor> {
        let id = *["test.no-sidechain", "test.sidechain", "test.no-aax"].get(index as usize)?;
        Some(PluginDescriptor {
            id,
            name: "Factory test",
            vendor: "WRAC",
            url: "https://example.invalid",
            manual_url: "",
            support_url: "",
            version: "0.0.0",
            description: "",
            features: &[],
            auv2: None,
            vst3: None,
            aax: (index != 2).then_some(AaxDescriptor {
                package_name: "Factory test",
                package_version: 0,
                categories: 1,
                manufacturer_id: u32::from_be_bytes(*b"Test"),
                product_id: u32::from_be_bytes(*b"Fact"),
                get_num_stem_configs: stem_count,
                get_stem_config: stem_config,
                midi_input_name: None,
                sidechain_input: if index == 1 {
                    self.sidechain
                } else {
                    AaxSidechainInput::None
                },
            }),
        })
    }

    fn create_plugin(&self, _: &str, _: PluginInstanceContext) -> Option<Box<dyn PluginInstance>> {
        panic!("AAX factory metadata must not instantiate a plugin")
    }
}

// This fixture only queries factory metadata, so it does not advertise loadable stems.
unsafe extern "C" fn stem_count() -> u32 {
    0
}

unsafe extern "C" fn stem_config(_: u32) -> *const AaxStemConfig {
    ptr::null()
}

#[test]
fn sidechain_factory_preserves_plugin_indices_without_creating_instances() {
    static ENTRY: Entry = Entry {
        sidechain: AaxSidechainInput::FirstAuxiliary,
    };
    static REGISTRATION: EntryRegistration = EntryRegistration::new(&ENTRY);
    // Use the wire identifier rather than the Rust constant to exercise discovery.
    let factory = unsafe {
        entry_get_factory(
            &REGISTRATION,
            c"com.novonotes.wrac.aax-sidechain/0".as_ptr(),
        )
        .cast::<WracPluginFactoryAaxSidechain>()
        .as_ref()
    }
    .expect("sidechain metadata must be available before instance creation");
    let port = factory.get_input_port_index.expect("sidechain callback");
    assert_eq!(unsafe { port(factory, 0) }, 0);
    assert_eq!(unsafe { port(factory, 1) }, 1);
    assert_eq!(unsafe { port(factory, 2) }, 0);
}

#[test]
fn sidechain_factory_is_absent_without_an_opt_in() {
    static ENTRY: Entry = Entry {
        sidechain: AaxSidechainInput::None,
    };
    static REGISTRATION: EntryRegistration = EntryRegistration::new(&ENTRY);
    assert!(unsafe {
        entry_get_factory(
            &REGISTRATION,
            c"com.novonotes.wrac.aax-sidechain/0".as_ptr(),
        )
        .is_null()
    });
}
