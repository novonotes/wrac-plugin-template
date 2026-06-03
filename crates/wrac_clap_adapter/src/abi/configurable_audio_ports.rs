use std::ffi::CStr;
use std::slice;
use std::sync::atomic::Ordering;

use clap_sys::ext::audio_ports::{CLAP_PORT_MONO, CLAP_PORT_STEREO};
use clap_sys::ext::configurable_audio_ports::{
    clap_audio_port_configuration_request, clap_plugin_configurable_audio_ports,
};
use clap_sys::plugin::clap_plugin;

use super::PluginInstance;
use super::ffi::ffi_bool;
use crate::{AudioPortConfigRequest, AudioPortType};

pub(super) static CONFIGURABLE_AUDIO_PORTS: clap_plugin_configurable_audio_ports =
    clap_plugin_configurable_audio_ports {
        can_apply_configuration: Some(configurable_audio_ports_can_apply_configuration),
        apply_configuration: Some(configurable_audio_ports_apply_configuration),
    };

unsafe extern "C" fn configurable_audio_ports_can_apply_configuration(
    plugin: *const clap_plugin,
    requests: *const clap_audio_port_configuration_request,
    request_count: u32,
) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("configurable_audio_ports.can_apply: missing plugin instance");
            return false;
        };
        // Layout changes invalidate the Processor's buffer view contract, so reject while active.
        // Activity is determined solely by whether a Processor exists and whether lifecycle is busy
        // (wrappers may omit or delay start/stop_processing, so they are not the source of truth).
        // This lets the plugin assume the layout is stable for the lifetime of any Processor.
        if instance.has_processor_or_busy() || instance.lifecycle_busy.load(Ordering::Acquire) {
            log::warn!(
                "configurable_audio_ports.can_apply: rejected while processor/lifecycle is busy"
            );
            return false;
        }
        let Some(requests) = convert_requests(requests, request_count) else {
            log::warn!(
                "configurable_audio_ports.can_apply: invalid request pointer count={request_count}"
            );
            return false;
        };

        let Some(configurable_audio_ports) = instance.configurable_audio_ports.as_ref() else {
            log::debug!(
                "configurable_audio_ports.can_apply: plugin has no configurable audio ports"
            );
            return false;
        };
        let can_apply = configurable_audio_ports.can_apply_audio_port_configuration(&requests);
        log::debug!(
            "configurable_audio_ports.can_apply: request_count={request_count} result={can_apply}"
        );
        can_apply
    })
}

unsafe extern "C" fn configurable_audio_ports_apply_configuration(
    plugin: *const clap_plugin,
    requests: *const clap_audio_port_configuration_request,
    request_count: u32,
) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("configurable_audio_ports.apply: missing plugin instance");
            return false;
        };
        let Some(requests) = convert_requests(requests, request_count) else {
            log::warn!(
                "configurable_audio_ports.apply: invalid request pointer count={request_count}"
            );
            return false;
        };

        let Some(configurable_audio_ports) = instance.configurable_audio_ports.as_ref() else {
            log::debug!("configurable_audio_ports.apply: plugin has no configurable audio ports");
            return false;
        };

        // Re-check the same conditions in `apply` for hosts that skip `can_apply`.
        // Performing the check and the apply under the same lifecycle guard closes the race
        // where `activate()` could snapshot a stale layout immediately after the processor-absent check.
        let Some(_guard) = instance.try_enter_lifecycle() else {
            log::warn!("configurable_audio_ports.apply: rejected while lifecycle is busy");
            return false;
        };
        if instance.has_processor_or_busy() {
            log::warn!("configurable_audio_ports.apply: rejected while processor is busy");
            return false;
        }

        match configurable_audio_ports.apply_audio_port_configuration(&requests) {
            Ok(()) => {
                log::debug!(
                    "configurable_audio_ports.apply: applied request_count={request_count}"
                );
                true
            }
            Err(error) => {
                log::warn!("configurable_audio_ports.apply: rejected by plugin: {error}");
                false
            }
        }
    })
}

fn convert_requests(
    requests: *const clap_audio_port_configuration_request,
    request_count: u32,
) -> Option<Vec<AudioPortConfigRequest>> {
    if request_count == 0 {
        return Some(Vec::new());
    }
    if requests.is_null() && request_count > 0 {
        return None;
    }
    let requests = unsafe { slice::from_raw_parts(requests, request_count as usize) };
    Some(requests.iter().map(convert_request).collect())
}

fn convert_request(request: &clap_audio_port_configuration_request) -> AudioPortConfigRequest {
    AudioPortConfigRequest {
        is_input: request.is_input,
        port_index: request.port_index,
        channel_count: request.channel_count,
        port_type: convert_port_type(request.port_type),
    }
}

fn convert_port_type(port_type: *const std::ffi::c_char) -> AudioPortType {
    if port_type.is_null() {
        return AudioPortType::Unspecified;
    }
    let port_type = unsafe { CStr::from_ptr(port_type) };
    if port_type == CLAP_PORT_MONO {
        AudioPortType::Mono
    } else if port_type == CLAP_PORT_STEREO {
        AudioPortType::Stereo
    } else {
        // The port_type string is valid only during the callback. Passing it as `Other` to product
        // code would lie about its lifetime, so unknown types are represented as Unspecified
        // and the product is expected to decide based on channel_count alone.
        AudioPortType::Unspecified
    }
}
