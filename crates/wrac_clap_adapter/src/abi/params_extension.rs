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
use crate::ParameterFlags;

pub(super) static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_get_info),
    get_value: Some(params_get_value),
    value_to_text: Some(params_value_to_text),
    text_to_value: Some(params_text_to_value),
    flush: Some(params_flush),
};

// VST3/AU/AAX wrapper では parameter query が CLAP の `[main-thread]` 前提から外れて
// 呼ばれることがある。ここは read lock と `PluginCore` の `&self` API だけに寄せ、
// GUI/runtime 所有権や lifecycle mutation に入らない。state restore などの write 側と
// 競合した場合も待たず、host へ「今は取得不可」を返す。
unsafe extern "C" fn params_count(plugin: *const clap_plugin) -> u32 {
    ffi_u32(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("params.count: missing plugin instance");
            return 0;
        };
        let Some(core) = instance.core.try_read() else {
            log::warn!(
                "params.count: core try_read failed on thread {:?}",
                std::thread::current().id()
            );
            return 0;
        };
        let Some(parameters) = core.parameters() else {
            log::warn!("params.count: plugin has no parameters");
            return 0;
        };
        let count = parameters.parameter_count();
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
        let Some(core) = instance.core.try_read() else {
            log::warn!(
                "params.get_info: core try_read failed index={param_index} thread={:?}",
                std::thread::current().id()
            );
            return false;
        };
        let Some(parameters) = core.parameters() else {
            log::warn!("params.get_info: plugin has no parameters index={param_index}");
            return false;
        };
        let Some(info) = parameters.parameter_info(param_index) else {
            log::warn!("params.get_info: invalid index={param_index}");
            return false;
        };
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
        let Some(core) = instance.core.try_read() else {
            log::warn!(
                "params.get_value: core try_read failed param_id={param_id} thread={:?}",
                std::thread::current().id()
            );
            return false;
        };
        let Some(parameters) = core.parameters() else {
            log::warn!("params.get_value: plugin has no parameters param_id={param_id}");
            return false;
        };
        let Ok(value) = parameters.parameter_value(param_id) else {
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
        let Some(core) = instance.core.try_read() else {
            log::warn!(
                "params.value_to_text: core try_read failed param_id={param_id} thread={:?}",
                std::thread::current().id()
            );
            return false;
        };
        let Some(parameters) = core.parameters() else {
            log::warn!("params.value_to_text: plugin has no parameters param_id={param_id}");
            return false;
        };
        let Ok(text) = parameters.parameter_value_to_text(param_id, value) else {
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
        let Some(core) = instance.core.try_read() else {
            log::warn!(
                "params.text_to_value: core try_read failed param_id={param_id} thread={:?}",
                std::thread::current().id()
            );
            return false;
        };
        let Some(parameters) = core.parameters() else {
            log::warn!("params.text_to_value: plugin has no parameters param_id={param_id}");
            return false;
        };
        let Ok(value) = parameters.parameter_text_to_value(param_id, text) else {
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
            // wrapper 固有の realtime-ish path から `flush()` が来ることがある。ここで
            // lifecycle/state 側の core write lock を待つと adapter が避けたい host 依存の
            // deadlock を再導入するため、input event の取りこぼしを待機より優先する。
            if let Some(core) = instance.core.try_read() {
                if let Some(parameters) = core.parameters() {
                    instance
                        .parameter_edits
                        .apply_input_parameter_events(parameters, &events.input);
                }
                drop(core);
            } else {
                log::warn!(
                    "params.flush: core try_read failed thread={:?}",
                    std::thread::current().id()
                );
            }
            instance
                .parameter_edits
                .drain_output_parameter_events(&mut events.output);
        }
    });
}

fn parameter_flags(flags: ParameterFlags) -> u32 {
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
