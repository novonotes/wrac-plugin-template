use std::ffi::{CStr, c_char, c_void};
use std::ptr;

use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::version::clap_version_is_compatible;
use wrac_host_context::{HostContext, PluginFormat};

use super::ffi::{ffi_bool, ffi_ptr, ffi_unit, four_char_code};
use super::{
    CLAP_PLUGIN_FACTORY_INFO_AAX, CLAP_PLUGIN_FACTORY_INFO_AUV2, CLAP_PLUGIN_FACTORY_INFO_VST3,
    PluginInstanceState, WRAC_PLUGIN_MAIN_THREAD_HOOK,
};
use crate::entry::{
    EntryContext, EntryRegistration, decrement_entry_init_count, entry_init_count,
    increment_entry_init_count, reset_entry_init_count, retain_entry_instance,
};
use crate::factory::{
    AaxFactoryState, Auv2FactoryState, ClapPluginFactoryAsAax, ClapPluginFactoryAsAuv2,
    ClapPluginFactoryAsVst3, ClapPluginInfoAsAax, ClapPluginInfoAsAuv2, ClapPluginInfoAsVst3,
    Vst3FactoryState, WracPluginMainThreadHook, aax_factory_ptr, aax_factory_state,
    auv2_factory_ptr, auv2_factory_state, clap_factory_state, factory_ptr, main_thread_hook_ptr,
    main_thread_hook_state, vst3_factory_ptr, vst3_factory_state,
};

unsafe fn clap_host_name(host: *const clap_host) -> Option<String> {
    if host.is_null() {
        return None;
    }
    let name = unsafe { (*host).name };
    if name.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned(),
    )
}
/// # Safety
///
/// `plugin_path` must be a valid CLAP string pointer when provided by the host.
/// The registration must be the static registration generated for this binary.
pub(crate) unsafe extern "C" fn entry_init(
    registration: &'static EntryRegistration,
    plugin_path: *const c_char,
) -> bool {
    ffi_bool(|| {
        let count = increment_entry_init_count(registration);
        if count > 1 {
            return true;
        }

        if let Some(config) = registration.entry.log_config() {
            registration.configure_log_runtime(config);
        }

        let plugin_path = if plugin_path.is_null() {
            None
        } else {
            let plugin_path = unsafe { CStr::from_ptr(plugin_path) };
            match plugin_path.to_str() {
                Ok(plugin_path) => Some(plugin_path),
                Err(error) => {
                    let _ = error;
                    reset_entry_init_count(registration);
                    return false;
                }
            }
        };
        if registration
            .entry
            .init(EntryContext { plugin_path })
            .is_err()
        {
            reset_entry_init_count(registration);
            return false;
        }
        true
    })
}

/// # Safety
///
/// The registration must be the same static registration previously passed to
/// `entry_init` for this binary.
pub(crate) unsafe extern "C" fn entry_deinit(registration: &'static EntryRegistration) {
    ffi_unit(|| {
        if entry_init_count(registration) == 0 {
            return;
        }
        let count = decrement_entry_init_count(registration);
        if count == 0 {
            registration.entry.deinit();
        }
    })
}

/// # Safety
///
/// `factory_id` must be null or point to a valid NUL-terminated CLAP factory id.
/// The returned pointer is owned by the static plugin registration storage.
pub(crate) unsafe extern "C" fn entry_get_factory(
    registration: &'static EntryRegistration,
    factory_id: *const c_char,
) -> *const c_void {
    ffi_ptr(|| {
        if factory_id.is_null() {
            return ptr::null();
        }
        let factory_id = unsafe { CStr::from_ptr(factory_id) };
        let storage = registration.storage();
        if factory_id == CLAP_PLUGIN_FACTORY_ID {
            factory_ptr(storage)
        } else if factory_id == WRAC_PLUGIN_MAIN_THREAD_HOOK {
            main_thread_hook_ptr(storage)
        } else if factory_id == CLAP_PLUGIN_FACTORY_INFO_AUV2
            && storage
                .descriptors
                .iter()
                .any(|descriptor| descriptor.descriptor().auv2.is_some())
        {
            auv2_factory_ptr(storage)
        } else if factory_id == CLAP_PLUGIN_FACTORY_INFO_VST3
            && storage
                .descriptors
                .iter()
                .any(|descriptor| descriptor.descriptor().vst3.is_some())
        {
            vst3_factory_ptr(storage)
        } else if factory_id == CLAP_PLUGIN_FACTORY_INFO_AAX
            && storage
                .descriptors
                .iter()
                .any(|descriptor| descriptor.descriptor().aax.is_some())
        {
            aax_factory_ptr(storage)
        } else {
            ptr::null()
        }
    })
}

pub(crate) unsafe extern "C" fn main_thread_hook_attach(hook: *const WracPluginMainThreadHook) {
    ffi_unit(|| {
        let Some(state) = main_thread_hook_state(hook) else {
            log::warn!("main_thread_hook.attach: invalid hook pointer");
            return;
        };
        state.registration.entry.attach_main_thread();
    })
}

pub(crate) unsafe extern "C" fn main_thread_hook_detach(hook: *const WracPluginMainThreadHook) {
    ffi_unit(|| {
        let Some(state) = main_thread_hook_state(hook) else {
            log::warn!("main_thread_hook.detach: invalid hook pointer");
            return;
        };
        state.registration.entry.detach_main_thread();
    })
}

pub(crate) unsafe extern "C" fn aax_get_info(
    factory: *const ClapPluginFactoryAsAax,
    index: u32,
) -> *const ClapPluginInfoAsAax {
    ffi_ptr(|| {
        let Some(AaxFactoryState { registration, .. }) = aax_factory_state(factory) else {
            log::warn!("aax.get_info: invalid factory pointer");
            return ptr::null();
        };
        let Some(descriptor) = registration.storage().descriptors.get(index as usize) else {
            log::warn!("aax.get_info: descriptor not found index={index}");
            return ptr::null();
        };
        descriptor.aax_info_ptr().unwrap_or(ptr::null())
    })
}

pub(crate) unsafe extern "C" fn vst3_get_info(
    factory: *const ClapPluginFactoryAsVst3,
    index: u32,
) -> *const ClapPluginInfoAsVst3 {
    ffi_ptr(|| {
        let Some(Vst3FactoryState { registration, .. }) = vst3_factory_state(factory) else {
            log::warn!("vst3.get_info: invalid factory pointer");
            return ptr::null();
        };
        let Some(descriptor) = registration.storage().descriptors.get(index as usize) else {
            log::warn!("vst3.get_info: descriptor not found index={index}");
            return ptr::null();
        };
        descriptor.vst3_info_ptr().unwrap_or(ptr::null())
    })
}

pub(crate) unsafe extern "C" fn auv2_get_info(
    factory: *const ClapPluginFactoryAsAuv2,
    index: u32,
    info: *mut ClapPluginInfoAsAuv2,
) -> bool {
    ffi_bool(|| {
        if info.is_null() {
            log::warn!(
                "auv2.get_info: invalid arguments index={index} info_is_null={}",
                info.is_null()
            );
            return false;
        }

        let Some(Auv2FactoryState { registration, .. }) = auv2_factory_state(factory) else {
            log::warn!("auv2.get_info: invalid factory pointer");
            return false;
        };
        let Some(descriptor) = registration.storage().descriptors.get(index as usize) else {
            log::warn!("auv2.get_info: descriptor not found index={index}");
            return false;
        };
        let Some(auv2) = descriptor.descriptor().auv2 else {
            log::warn!("auv2.get_info: descriptor has no AUv2 info index={index}");
            return false;
        };

        unsafe {
            (*info).au_type = four_char_code(auv2.plugin_type);
            (*info).au_subt = four_char_code(auv2.plugin_subtype);
        }
        true
    })
}

pub(crate) unsafe extern "C" fn factory_get_plugin_count(
    factory: *const clap_plugin_factory,
) -> u32 {
    let Some(state) = clap_factory_state(factory) else {
        log::warn!("factory.get_plugin_count: invalid factory pointer");
        return 0;
    };
    state.registration.storage().descriptors.len() as u32
}

pub(crate) unsafe extern "C" fn factory_get_plugin_descriptor(
    factory: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    let Some(state) = clap_factory_state(factory) else {
        log::warn!("factory.get_plugin_descriptor: invalid factory pointer");
        return ptr::null();
    };
    let Some(descriptor) = state.registration.storage().descriptors.get(index as usize) else {
        log::warn!("factory.get_plugin_descriptor: invalid index={index}");
        return ptr::null();
    };
    descriptor.clap_descriptor()
}

pub(crate) unsafe extern "C" fn factory_create_plugin(
    factory: *const clap_plugin_factory,
    host: *const clap_host,
    plugin_id: *const c_char,
) -> *const clap_plugin {
    ffi_ptr(|| {
        if host.is_null() || plugin_id.is_null() {
            log::warn!(
                "factory.create_plugin: invalid arguments host_is_null={} plugin_id_is_null={}",
                host.is_null(),
                plugin_id.is_null()
            );
            return ptr::null();
        }
        if !clap_version_is_compatible(unsafe { (*host).clap_version }) {
            log::warn!("factory.create_plugin: incompatible CLAP version");
            return ptr::null();
        }

        let Some(factory_state) = clap_factory_state(factory) else {
            log::warn!("factory.create_plugin: invalid factory pointer");
            return ptr::null();
        };
        let registration = factory_state.registration;
        let plugin_id = match unsafe { CStr::from_ptr(plugin_id) }.to_str() {
            Ok(plugin_id) => plugin_id,
            Err(error) => {
                log::warn!("factory.create_plugin: invalid UTF-8 plugin id: {error}");
                return ptr::null();
            }
        };
        let storage = registration.storage();
        let Some((descriptor_index, _descriptor)) = storage
            .descriptors
            .iter()
            .enumerate()
            .find(|(_, descriptor)| descriptor.descriptor().id == plugin_id)
        else {
            log::warn!("factory.create_plugin: requested unknown plugin id");
            return ptr::null();
        };

        let clap_host_name = unsafe { clap_host_name(host) };
        let host_context = HostContext::detect_current(clap_host_name.as_deref());
        let attach_in_adapter = host_context.plugin_format == PluginFormat::Unknown;
        if attach_in_adapter {
            registration.entry.attach_main_thread();
        }

        let Some(mut instance) = PluginInstanceState::new(
            registration,
            descriptor_index,
            plugin_id,
            host,
            clap_host_name,
            host_context,
        ) else {
            if attach_in_adapter {
                registration.entry.detach_main_thread();
            }
            log::warn!("factory.create_plugin: failed to allocate plugin instance state");
            return ptr::null();
        };
        let instance_ptr = (&mut *instance) as *mut PluginInstanceState;
        instance.plugin.plugin_data = instance_ptr.cast();
        let plugin_ptr = &instance.plugin as *const clap_plugin;
        retain_entry_instance(registration);
        let _ = Box::into_raw(instance);
        plugin_ptr
    })
}
