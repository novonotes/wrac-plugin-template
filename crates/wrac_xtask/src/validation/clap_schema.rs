use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::ptr;

use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::params::{CLAP_EXT_PARAMS, clap_param_info, clap_plugin_params};
use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::host::clap_host;
use clap_sys::version::CLAP_VERSION;
use libloading::Library;

use crate::Result;
use crate::context::Context;
use crate::profile::BuildProfile;
use crate::targets::Platform;

#[derive(Debug)]
pub(crate) struct PluginSchema {
    pub(crate) plugin_id: String,
    pub(crate) plugin_name: String,
    pub(crate) params: Vec<ParameterSchema>,
}

#[derive(Debug)]
pub(crate) struct ParameterSchema {
    pub(crate) id: u32,
    pub(crate) name: String,
    pub(crate) flags: u32,
    pub(crate) min_value: f64,
    pub(crate) max_value: f64,
    pub(crate) default_value: f64,
}

pub(crate) unsafe fn read_clap_schemas(
    ctx: &Context,
    profile: BuildProfile,
    clap_bundle: &Path,
) -> Result<Vec<PluginSchema>> {
    // The checks need the host-visible schema, so query the built CLAP through its public
    // entry points instead of trusting source-side metadata that wrappers or adapters may alter.
    let library_path = clap_library_path(ctx, profile);
    let plugin_path = CString::new(clap_bundle.to_string_lossy().as_bytes())?;
    let library = unsafe { Library::new(&library_path) }?;
    let get_entry = unsafe { library.get::<unsafe extern "C" fn() -> usize>(b"get_clap_entry") }?;
    let entry = unsafe { get_entry() as *const clap_plugin_entry };
    if entry.is_null() {
        return Err("CLAP entry returned a null pointer".into());
    }

    let init = unsafe { (*entry).init }.ok_or("CLAP entry has no init callback")?;
    if !unsafe { init(plugin_path.as_ptr()) } {
        return Err("CLAP entry init failed".into());
    }
    let _entry_guard = ClapEntryGuard { entry };

    let get_factory =
        unsafe { (*entry).get_factory }.ok_or("CLAP entry has no get_factory callback")?;
    let factory =
        unsafe { get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()) as *const clap_plugin_factory };
    if factory.is_null() {
        return Err("CLAP plugin factory is not available".into());
    }

    let create_plugin =
        unsafe { (*factory).create_plugin }.ok_or("CLAP factory has no create_plugin callback")?;
    let count = unsafe { plugin_count(factory) }?;
    let mut schemas = Vec::new();
    for index in 0..count {
        schemas.push(unsafe { read_plugin_schema(factory, create_plugin, index) }?);
    }
    Ok(schemas)
}

fn validator_clap_host() -> clap_host {
    clap_host {
        clap_version: CLAP_VERSION,
        host_data: ptr::null_mut(),
        name: c"WRAC xtask checks".as_ptr(),
        vendor: c"WRAC".as_ptr(),
        url: c"https://github.com/novonotes/wrac-plugin-template".as_ptr(),
        version: c"0".as_ptr(),
        get_extension: Some(validator_host_get_extension),
        request_restart: Some(validator_host_request_restart),
        request_process: Some(validator_host_request_process),
        request_callback: Some(validator_host_request_callback),
    }
}

unsafe extern "C" fn validator_host_get_extension(
    _host: *const clap_host,
    _extension_id: *const c_char,
) -> *const c_void {
    // Schema checks only need plugin-provided extensions. Returning no host extensions keeps
    // this mini-host small and avoids accidentally depending on runtime host behavior.
    ptr::null()
}

unsafe extern "C" fn validator_host_request_restart(_host: *const clap_host) {}

unsafe extern "C" fn validator_host_request_process(_host: *const clap_host) {}

unsafe extern "C" fn validator_host_request_callback(_host: *const clap_host) {}

unsafe fn plugin_count(factory: *const clap_plugin_factory) -> Result<u32> {
    let count = unsafe { (*factory).get_plugin_count }
        .ok_or("CLAP factory has no get_plugin_count callback")?;
    let count = unsafe { count(factory) };
    if count == 0 {
        return Err("CLAP factory contains no plugins".into());
    }
    Ok(count)
}

unsafe fn plugin_descriptor(
    factory: *const clap_plugin_factory,
    index: u32,
) -> Result<*const clap_sys::plugin::clap_plugin_descriptor> {
    let get_descriptor = unsafe { (*factory).get_plugin_descriptor }
        .ok_or("CLAP factory has no get_plugin_descriptor callback")?;
    let descriptor = unsafe { get_descriptor(factory, index) };
    if descriptor.is_null() {
        return Err(format!("CLAP factory returned a null descriptor at index {index}").into());
    }
    Ok(descriptor)
}

unsafe fn read_plugin_schema(
    factory: *const clap_plugin_factory,
    create_plugin: unsafe extern "C" fn(
        *const clap_plugin_factory,
        *const clap_host,
        *const c_char,
    ) -> *const clap_sys::plugin::clap_plugin,
    index: u32,
) -> Result<PluginSchema> {
    let descriptor = unsafe { plugin_descriptor(factory, index) }?;
    let descriptor = unsafe { &*descriptor };
    if descriptor.id.is_null() {
        return Err(format!("CLAP descriptor has a null plugin id at index {index}").into());
    }
    let plugin_id = unsafe { CStr::from_ptr(descriptor.id) };
    let plugin_name = unsafe { descriptor_string(descriptor.name) };
    let host = validator_clap_host();
    let plugin = unsafe { create_plugin(factory, &host, plugin_id.as_ptr()) };
    if plugin.is_null() {
        return Err(format!(
            "CLAP factory failed to create plugin id={}",
            plugin_id.to_string_lossy()
        )
        .into());
    }
    let _plugin_guard = ClapPluginGuard { plugin };

    if let Some(init_plugin) = unsafe { (*plugin).init } {
        if !unsafe { init_plugin(plugin) } {
            return Err(format!(
                "CLAP plugin init failed for plugin id={}",
                plugin_id.to_string_lossy()
            )
            .into());
        }
    }

    // Read only the host-visible schema used by the current release-policy checks.
    // Broader CLAP capability validation is left to external format validators.
    let params = unsafe { read_params(plugin) }?;
    Ok(PluginSchema {
        plugin_id: plugin_id.to_string_lossy().into_owned(),
        plugin_name,
        params,
    })
}

unsafe fn descriptor_string(value: *const c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    }
}

unsafe fn plugin_extension(
    plugin: *const clap_sys::plugin::clap_plugin,
    extension_id: *const c_char,
) -> Result<*const c_void> {
    let get_extension =
        unsafe { (*plugin).get_extension }.ok_or("CLAP plugin has no get_extension callback")?;
    Ok(unsafe { get_extension(plugin, extension_id) })
}

unsafe fn read_params(
    plugin: *const clap_sys::plugin::clap_plugin,
) -> Result<Vec<ParameterSchema>> {
    let params =
        unsafe { plugin_extension(plugin, CLAP_EXT_PARAMS.as_ptr())? as *const clap_plugin_params };
    if params.is_null() {
        return Ok(Vec::new());
    }
    let count = unsafe { (*params).count }.ok_or("CLAP params extension has no count callback")?;
    let get_info =
        unsafe { (*params).get_info }.ok_or("CLAP params extension has no get_info callback")?;
    let mut result = Vec::new();
    for index in 0..unsafe { count(plugin) } {
        let mut info = clap_param_info {
            id: 0,
            flags: 0,
            cookie: ptr::null_mut(),
            name: [0; clap_sys::string_sizes::CLAP_NAME_SIZE],
            module: [0; clap_sys::string_sizes::CLAP_PATH_SIZE],
            min_value: 0.0,
            max_value: 0.0,
            default_value: 0.0,
        };
        if !unsafe { get_info(plugin, index, &mut info) } {
            return Err(format!("CLAP params.get_info failed for index {index}").into());
        }
        result.push(ParameterSchema {
            id: info.id,
            name: c_char_array_to_string(&info.name),
            flags: info.flags,
            min_value: info.min_value,
            max_value: info.max_value,
            default_value: info.default_value,
        });
    }
    Ok(result)
}

fn c_char_array_to_string(buffer: &[std::ffi::c_char]) -> String {
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn clap_library_path(ctx: &Context, profile: BuildProfile) -> PathBuf {
    match ctx.platform {
        Platform::Macos => ctx
            .clap_bundle(profile)
            .join("Contents")
            .join("MacOS")
            .join(&ctx.metadata.bundle_name),
        Platform::Windows | Platform::Linux => ctx.clap_bundle(profile),
    }
}

struct ClapEntryGuard {
    entry: *const clap_plugin_entry,
}

impl Drop for ClapEntryGuard {
    fn drop(&mut self) {
        if let Some(deinit) = unsafe { (*self.entry).deinit } {
            unsafe { deinit() };
        }
    }
}

struct ClapPluginGuard {
    plugin: *const clap_sys::plugin::clap_plugin,
}

impl Drop for ClapPluginGuard {
    fn drop(&mut self) {
        if let Some(destroy) = unsafe { (*self.plugin).destroy } {
            unsafe { destroy(self.plugin) };
        }
    }
}
