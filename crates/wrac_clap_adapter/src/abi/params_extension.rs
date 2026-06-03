use std::ffi::{CStr, c_char};
use std::ptr;

use clap_sys::events::{clap_input_events, clap_output_events};
use clap_sys::ext::params::{
    CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_AUTOMATABLE_PER_CHANNEL,
    CLAP_PARAM_IS_AUTOMATABLE_PER_KEY, CLAP_PARAM_IS_AUTOMATABLE_PER_NOTE_ID,
    CLAP_PARAM_IS_AUTOMATABLE_PER_PORT, CLAP_PARAM_IS_BYPASS, CLAP_PARAM_IS_ENUM,
    CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_MODULATABLE, CLAP_PARAM_IS_MODULATABLE_PER_CHANNEL,
    CLAP_PARAM_IS_MODULATABLE_PER_KEY, CLAP_PARAM_IS_MODULATABLE_PER_NOTE_ID,
    CLAP_PARAM_IS_MODULATABLE_PER_PORT, CLAP_PARAM_IS_PERIODIC, CLAP_PARAM_IS_READONLY,
    CLAP_PARAM_IS_STEPPED, CLAP_PARAM_REQUIRES_PROCESS, clap_param_info, clap_plugin_params,
};
use clap_sys::plugin::clap_plugin;

use super::PluginInstance;
use super::ffi::{ffi_bool, ffi_u32, ffi_unit, fill_c_char_array, write_c_str_buffer};
use crate::ParamFlags;
use wrac_host_context::PluginFormat;

const CLAP_INVALID_PARAM_ID: u32 = u32::MAX;
const VST3_MAX_PUBLIC_PARAM_ID: u32 = 0x7fff_ffff;

pub(super) static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_get_info),
    get_value: Some(params_get_value),
    value_to_text: Some(params_value_to_text),
    text_to_value: Some(params_text_to_value),
    flush: Some(params_flush),
};

// VST3/AU/AAX wrappers may invoke parameter queries outside the CLAP `[main-thread]` assumption.
// The parameters capability reads the Arc fixed at instance creation and does not touch
// GUI/runtime ownership or lifecycle mutation.
unsafe extern "C" fn params_count(plugin: *const clap_plugin) -> u32 {
    ffi_u32(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("params.count: missing plugin instance");
            return 0;
        };
        let Some(parameters) = instance.parameters.as_ref() else {
            log::warn!("params.count: plugin has no parameters");
            return 0;
        };
        let count = parameters.param_count();
        log::debug!(
            "params.count: count={count} thread={:?}",
            std::thread::current().id()
        );
        count
    })
}

unsafe extern "C" fn params_get_info(
    plugin: *const clap_plugin,
    param_index: u32,
    param_info: *mut clap_param_info,
) -> bool {
    ffi_bool(|| {
        if param_info.is_null() {
            log::warn!("params.get_info: null output pointer index={param_index}");
            return false;
        }
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("params.get_info: missing plugin instance index={param_index}");
            return false;
        };
        let Some(parameters) = instance.parameters.as_ref() else {
            log::warn!("params.get_info: plugin has no parameters index={param_index}");
            return false;
        };
        let Some(info) = parameters.param_info(param_index) else {
            log::warn!("params.get_info: invalid index={param_index}");
            return false;
        };
        // `UINT32_MAX` is invalid in CLAP. Through VST3, the high-bit range is also
        // reserved and clap-wrapper masks it when translating IDs, which can collide
        // otherwise distinct parameters. Rejecting at discovery keeps the bad mapping
        // out of the host instead of letting automation lanes corrupt later.
        if !is_param_id_exposable(info.id, instance.host_context.plugin_format) {
            log::error!(
                "params.get_info: rejecting unsupported parameter id index={param_index} id={} format={}",
                info.id,
                instance.host_context.plugin_format.as_str()
            );
            return false;
        }
        log::debug!(
            "params.get_info: index={param_index} id={} name={} flags={} thread={:?}",
            info.id,
            info.name,
            parameter_flags(info.flags),
            std::thread::current().id()
        );

        unsafe {
            (*param_info).id = info.id;
            (*param_info).flags = parameter_flags(info.flags);
            (*param_info).cookie = ptr::null_mut();
            fill_c_char_array(&mut (*param_info).name, info.name);
            fill_c_char_array(&mut (*param_info).module, info.module);
            (*param_info).min_value = info.min_value;
            (*param_info).max_value = info.max_value;
            (*param_info).default_value = info.default_value;
        }
        true
    })
}

fn is_param_id_exposable(param_id: u32, plugin_format: PluginFormat) -> bool {
    if param_id == CLAP_INVALID_PARAM_ID {
        return false;
    }
    plugin_format != PluginFormat::Vst3 || param_id <= VST3_MAX_PUBLIC_PARAM_ID
}

unsafe extern "C" fn params_get_value(
    plugin: *const clap_plugin,
    param_id: u32,
    out_value: *mut f64,
) -> bool {
    ffi_bool(|| {
        if out_value.is_null() {
            log::warn!("params.get_value: null output pointer param_id={param_id}");
            return false;
        }
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("params.get_value: missing plugin instance param_id={param_id}");
            return false;
        };
        let Some(parameters) = instance.parameters.as_ref() else {
            log::warn!("params.get_value: plugin has no parameters param_id={param_id}");
            return false;
        };
        let Ok(value) = parameters.param_value(param_id) else {
            log::warn!("params.get_value: invalid param_id={param_id}");
            return false;
        };
        log::debug!(
            "params.get_value: param_id={param_id} value={value} thread={:?}",
            std::thread::current().id()
        );
        unsafe {
            *out_value = value;
        }
        true
    })
}

unsafe extern "C" fn params_value_to_text(
    plugin: *const clap_plugin,
    param_id: u32,
    value: f64,
    out_buffer: *mut c_char,
    out_buffer_capacity: u32,
) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("params.value_to_text: missing plugin instance param_id={param_id}");
            return false;
        };
        let Some(parameters) = instance.parameters.as_ref() else {
            log::warn!("params.value_to_text: plugin has no parameters param_id={param_id}");
            return false;
        };
        let Ok(text) = parameters.value_to_text(param_id, value) else {
            log::warn!("params.value_to_text: invalid param_id={param_id} value={value}");
            return false;
        };
        log::debug!(
            "params.value_to_text: param_id={param_id} value={value} text={text} thread={:?}",
            std::thread::current().id()
        );
        write_c_str_buffer(out_buffer, out_buffer_capacity, &text)
    })
}

unsafe extern "C" fn params_text_to_value(
    plugin: *const clap_plugin,
    param_id: u32,
    param_value_text: *const c_char,
    out_value: *mut f64,
) -> bool {
    ffi_bool(|| {
        if param_value_text.is_null() || out_value.is_null() {
            log::warn!("params.text_to_value: null pointer param_id={param_id}");
            return false;
        }
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("params.text_to_value: missing plugin instance param_id={param_id}");
            return false;
        };
        let Ok(text) = unsafe { CStr::from_ptr(param_value_text) }.to_str() else {
            log::warn!("params.text_to_value: invalid utf8 param_id={param_id}");
            return false;
        };
        let Some(parameters) = instance.parameters.as_ref() else {
            log::warn!("params.text_to_value: plugin has no parameters param_id={param_id}");
            return false;
        };
        let Ok(value) = parameters.text_to_value(param_id, text) else {
            log::warn!("params.text_to_value: invalid param_id={param_id} text={text}");
            return false;
        };
        log::debug!(
            "params.text_to_value: param_id={param_id} text={text} value={value} thread={:?}",
            std::thread::current().id()
        );
        unsafe {
            *out_value = value;
        }
        true
    })
}

unsafe extern "C" fn params_flush(
    plugin: *const clap_plugin,
    in_events: *const clap_input_events,
    out_events: *const clap_output_events,
) {
    ffi_unit(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            return;
        };
        unsafe {
            let mut events = crate::ProcessEvents::from_raw(in_events, out_events);
            if let Some(parameters) = instance.parameters.as_ref() {
                instance
                    .parameter_edits
                    .apply_input_parameter_events(parameters.as_ref(), &events.input);
            } else {
                wrac_log::rtwarn!("params.flush: plugin has no parameters");
            }
            instance
                .parameter_edits
                .drain_output_parameter_events(&mut events.output);
        }
    });
}

fn parameter_flags(flags: ParamFlags) -> u32 {
    let mut raw = 0;
    if flags.is_stepped {
        raw |= CLAP_PARAM_IS_STEPPED;
    }
    if flags.is_periodic {
        raw |= CLAP_PARAM_IS_PERIODIC;
    }
    if flags.is_hidden {
        raw |= CLAP_PARAM_IS_HIDDEN;
    }
    if flags.is_readonly {
        raw |= CLAP_PARAM_IS_READONLY;
    }
    if flags.is_bypass {
        raw |= CLAP_PARAM_IS_BYPASS;
    }
    if flags.is_automatable {
        raw |= CLAP_PARAM_IS_AUTOMATABLE;
    }
    if flags.is_automatable_per_note_id {
        raw |= CLAP_PARAM_IS_AUTOMATABLE_PER_NOTE_ID;
    }
    if flags.is_automatable_per_key {
        raw |= CLAP_PARAM_IS_AUTOMATABLE_PER_KEY;
    }
    if flags.is_automatable_per_channel {
        raw |= CLAP_PARAM_IS_AUTOMATABLE_PER_CHANNEL;
    }
    if flags.is_automatable_per_port {
        raw |= CLAP_PARAM_IS_AUTOMATABLE_PER_PORT;
    }
    if flags.is_modulatable {
        raw |= CLAP_PARAM_IS_MODULATABLE;
    }
    if flags.is_modulatable_per_note_id {
        raw |= CLAP_PARAM_IS_MODULATABLE_PER_NOTE_ID;
    }
    if flags.is_modulatable_per_key {
        raw |= CLAP_PARAM_IS_MODULATABLE_PER_KEY;
    }
    if flags.is_modulatable_per_channel {
        raw |= CLAP_PARAM_IS_MODULATABLE_PER_CHANNEL;
    }
    if flags.is_modulatable_per_port {
        raw |= CLAP_PARAM_IS_MODULATABLE_PER_PORT;
    }
    if flags.requires_process {
        raw |= CLAP_PARAM_REQUIRES_PROCESS;
    }
    if flags.is_enum {
        raw |= CLAP_PARAM_IS_ENUM;
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_clap_invalid_param_id_for_all_formats() {
        assert!(!is_param_id_exposable(
            CLAP_INVALID_PARAM_ID,
            PluginFormat::Unknown
        ));
        assert!(!is_param_id_exposable(
            CLAP_INVALID_PARAM_ID,
            PluginFormat::Vst3
        ));
    }

    #[test]
    fn rejects_vst3_reserved_param_id_range_only_for_vst3() {
        assert!(is_param_id_exposable(0x8000_0000, PluginFormat::Unknown));
        assert!(!is_param_id_exposable(0x8000_0000, PluginFormat::Vst3));
        assert!(is_param_id_exposable(
            VST3_MAX_PUBLIC_PARAM_ID,
            PluginFormat::Vst3
        ));
    }
}
