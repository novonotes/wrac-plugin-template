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
use crate::{AudioPortConfigurationRequest, AudioPortType};

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
        // port layout は Processor の buffer view 契約を変えるため、active 中の変更は拒否する。
        //
        // WRAC adapter では `start_processing()` / `stop_processing()` を active 判定の SoT に
        // しない。wrapper によってはそれらを省略・遅延するため、ここでは実際の Processor の
        // 有無と lifecycle callback 中かどうかだけを見る。これにより、plugin 実装側は
        // `activate()` で layout を snapshot して Processor に渡せば、その Processor が生きている
        // 間に layout store が書き換わらない前提を置ける。
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

        // host が `can_apply` を省略しても、Processor が古い port view を使っている間に
        // layout を変えないよう `apply` 側でも同じ条件を確認する。確認と実際の apply は
        // 同じ lifecycle guard の内側で行う。そうしないと、processor 不在確認の直後に
        // `activate()` が古い layout を snapshot し、その後この callback が layout store を
        // 書き換える race が残る。
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
) -> Option<Vec<AudioPortConfigurationRequest>> {
    if request_count == 0 {
        return Some(Vec::new());
    }
    if requests.is_null() && request_count > 0 {
        return None;
    }
    let requests = unsafe { slice::from_raw_parts(requests, request_count as usize) };
    Some(requests.iter().map(convert_request).collect())
}

fn convert_request(
    request: &clap_audio_port_configuration_request,
) -> AudioPortConfigurationRequest {
    AudioPortConfigurationRequest {
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
        // CLAP の port_type 文字列は callback 中だけ有効な借用値です。`AudioPortType::Other`
        // として製品側へ渡すと lifetime を偽るため、未知 type は channel_count だけで判断させる。
        AudioPortType::Unspecified
    }
}
