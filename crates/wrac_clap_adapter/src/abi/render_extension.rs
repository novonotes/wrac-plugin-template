use clap_sys::ext::render::{
    CLAP_RENDER_OFFLINE, CLAP_RENDER_REALTIME, clap_plugin_render, clap_plugin_render_mode,
};
use clap_sys::plugin::clap_plugin;

use super::PluginInstance;
use super::ffi::ffi_bool;
use crate::PluginRenderMode;

pub(super) static RENDER: clap_plugin_render = clap_plugin_render {
    has_hard_realtime_requirement: Some(render_has_hard_realtime_requirement),
    set: Some(render_set),
};

unsafe extern "C" fn render_has_hard_realtime_requirement(plugin: *const clap_plugin) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("render.has_hard_realtime_requirement: missing plugin instance");
            return false;
        };
        let Some(render) = instance.render.as_ref() else {
            log::warn!("render.has_hard_realtime_requirement: plugin has no render support");
            return false;
        };
        render.has_hard_realtime_requirement()
    })
}

unsafe extern "C" fn render_set(plugin: *const clap_plugin, mode: clap_plugin_render_mode) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("render.set: missing plugin instance mode={mode}");
            return false;
        };
        let Some(render) = instance.render.as_ref() else {
            log::warn!("render.set: plugin has no render support mode={mode}");
            return false;
        };
        let Some(mode) = convert_render_mode(mode) else {
            log::warn!("render.set: unsupported render mode={mode}");
            return false;
        };
        match render.set_render_mode(mode) {
            Ok(()) => true,
            Err(error) => {
                log::warn!("render.set: plugin rejected render mode {mode:?}: {error}");
                false
            }
        }
    })
}

fn convert_render_mode(mode: clap_plugin_render_mode) -> Option<PluginRenderMode> {
    match mode {
        CLAP_RENDER_REALTIME => Some(PluginRenderMode::Realtime),
        CLAP_RENDER_OFFLINE => Some(PluginRenderMode::Offline),
        _ => None,
    }
}
