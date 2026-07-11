use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::atomic::Ordering;

use clap_sys::ext::audio_ports::CLAP_EXT_AUDIO_PORTS;
use clap_sys::ext::configurable_audio_ports::{
    CLAP_EXT_CONFIGURABLE_AUDIO_PORTS, CLAP_EXT_CONFIGURABLE_AUDIO_PORTS_COMPAT,
};
use clap_sys::ext::gui::CLAP_EXT_GUI;
use clap_sys::ext::latency::CLAP_EXT_LATENCY;
use clap_sys::ext::note_ports::CLAP_EXT_NOTE_PORTS;
use clap_sys::ext::params::CLAP_EXT_PARAMS;
use clap_sys::ext::render::CLAP_EXT_RENDER;
use clap_sys::ext::state::CLAP_EXT_STATE;
use clap_sys::ext::tail::CLAP_EXT_TAIL;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{
    CLAP_PROCESS_CONTINUE, CLAP_PROCESS_CONTINUE_IF_NOT_QUIET, CLAP_PROCESS_ERROR,
    CLAP_PROCESS_SLEEP, CLAP_PROCESS_TAIL, clap_process, clap_process_status,
};
use wrac_host_context::PluginFormat;

use super::audio_buffers::audio_buffers;
use super::ffi::{ffi_bool, ffi_ptr, ffi_status, ffi_unit};
use super::{
    CLAP_PLUGIN_AS_VST3, PluginCapabilities, PluginInstanceState, PluginRuntime, RtDepthGuard,
    audio_ports, configurable_audio_ports, gui_extension, latency_extension, note_ports,
    params_extension, render_extension, state_extension, tail_extension, vst3_extension,
};
use crate::entry::release_entry_instance;
use crate::interface::{
    ActivateContext, PluginInstanceContext, ProcessContext, ProcessStatus, TransportEvent,
};

pub(super) unsafe extern "C" fn plugin_init(plugin: *const clap_plugin) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            log::warn!("plugin.init: missing plugin instance");
            return false;
        };
        let Some(_guard) = instance.try_enter_lifecycle() else {
            log::warn!("plugin.init: lifecycle is busy");
            return false;
        };
        if instance.runtime.get().is_some() || instance.core.lock().is_some() {
            log::warn!("plugin.init: plugin instance is already initialized");
            return false;
        }
        let context = PluginInstanceContext {
            host_params: instance.host_params.clone(),
            host_state: instance.host_state.clone(),
            host_audio_ports: instance.host_audio_ports.clone(),
            host_note_ports: instance.host_note_ports.clone(),
            host_lifecycle: instance.host_lifecycle.clone(),
            host_gui: instance.host_gui.clone(),
            host_context: instance.host_context.clone(),
        };
        let Some(mut core) = instance
            .registration
            .entry
            .plugin_factory()
            .and_then(|factory| factory.create_plugin(&instance.plugin_id, context))
        else {
            log::warn!("plugin.init: product factory returned no plugin core");
            return false;
        };
        // Instance creation starts plugin logging before CLAP init. Emit immediately
        // after product construction so wrapper/host routing is visible before
        // capability queries or GUI attachment.
        log::info!(
            "plugin.init: host_context host=\"{}\" process=\"{}\" format={} clap_host_name=\"{}\"",
            instance.host_context.host.display_name,
            instance.host_context.host.process_name,
            instance.host_context.plugin_format.as_str(),
            instance.clap_host_name.as_deref().unwrap_or("")
        );

        // Freeze capabilities during CLAP init. Host extension queries are allowed from this
        // point onward, and later get_extension callbacks can answer without taking the core lock.
        let audio_ports = core.audio_ports();
        let configurable_audio_ports = core.configurable_audio_ports();
        let note_ports = core.note_ports();
        let parameters = core.params();
        let inactive_processor = match core.initialize_processor() {
            Ok(processor) => processor,
            Err(error) => {
                log::warn!("plugin.init: inactive processor creation failed: {error}");
                return false;
            }
        };
        let state = core.state();
        let gui = core.gui();
        let render = core.render();
        let tail = core.tail();
        let latency = core.latency();
        debug_assert!(
            latency.is_some(),
            "plugins should provide a latency extension because wrapper builds, especially AAX, expect it during activation; return zero latency when no delay is required"
        );
        if cfg!(not(debug_assertions)) && latency.is_none() {
            log::warn!(
                "plugin.init: plugin has no latency extension; exposing zero-latency wrapper fallback"
            );
        }
        let capabilities = PluginCapabilities {
            audio_ports: audio_ports.is_some(),
            configurable_audio_ports: configurable_audio_ports.is_some(),
            note_ports: note_ports.is_some(),
            state: state.is_some(),
            gui: gui.is_some(),
            render: render.is_some(),
            tail: tail.is_some(),
            latency: latency.is_some(),
        };
        let runtime = PluginRuntime {
            capabilities,
            audio_ports,
            configurable_audio_ports,
            note_ports,
            parameters,
            state,
            gui,
            render,
            tail,
            latency,
        };
        if instance.runtime.set(runtime).is_err() {
            log::warn!("plugin.init: runtime was already initialized");
            return false;
        }
        instance.put_inactive_processor_blocking(inactive_processor);
        *instance.core.lock() = Some(core);
        // Product-held host extension proxies become live only after plugin capabilities are
        // frozen. This keeps legal init-time `get_extension` re-entry from observing a
        // partially initialized runtime.
        instance
            .host_extensions_initialized
            .store(true, Ordering::Release);
        true
    })
}

pub(super) unsafe extern "C" fn plugin_destroy(plugin: *const clap_plugin) {
    ffi_unit(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            log::warn!("plugin.destroy: missing plugin instance");
            return;
        };
        let detach_in_adapter = instance.host_context.plugin_format == PluginFormat::Unknown;
        let registration = instance.registration;
        let guard = instance.enter_lifecycle_blocking();
        instance
            .host_extensions_initialized
            .store(false, Ordering::Release);

        if let Some(gui) = instance
            .runtime
            .get()
            .and_then(|runtime| runtime.gui.clone())
        {
            if let Some(_gui_callback) = instance.gui_callback_busy.try_lock() {
                if instance.enter_gui_lifecycle_thread("destroy") {
                    gui.main_thread().destroy();
                    instance.clear_gui_lifecycle_thread();
                }
            } else {
                log::error!(
                    "skipping GUI destroy during plugin destruction because another GUI callback is active"
                );
            }
        }

        if let Some(processor) = instance.take_processor_blocking() {
            let mut core = instance.core.lock();
            if let Some(core) = core.as_mut() {
                match core.deactivate(processor) {
                    Ok(inactive) => drop(inactive),
                    Err(error) => log::warn!("plugin.destroy: plugin deactivate failed: {error}"),
                }
            } else {
                log::warn!("plugin.destroy: plugin core is not initialized");
                drop(processor);
            }
        } else {
            let inactive = if instance.runtime.get().is_some() {
                instance.take_inactive_processor_blocking()
            } else {
                instance.try_take_inactive_processor().flatten()
            };
            drop(inactive);
        }

        if let Some(core) = instance.core.lock().as_mut() {
            core.destroy();
        }

        drop(guard);
        let data = unsafe { (*plugin).plugin_data } as *mut PluginInstanceState;
        unsafe {
            drop(Box::from_raw(data));
        }
        if detach_in_adapter {
            registration.entry.detach_main_thread();
        }
        release_entry_instance(registration);
    });
}

pub(super) unsafe extern "C" fn plugin_activate(
    plugin: *const clap_plugin,
    sample_rate: f64,
    min_frames_count: u32,
    max_frames_count: u32,
) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            log::warn!("plugin.activate: missing plugin instance");
            return false;
        };
        let Some(_guard) = instance.try_enter_lifecycle() else {
            log::warn!("plugin.activate: lifecycle is busy");
            return false;
        };
        if instance.has_processor_or_busy() {
            log::warn!("plugin.activate: processor already exists or audio callback is busy");
            return false;
        }
        let Some(runtime) = instance.runtime.get() else {
            log::warn!("plugin.activate: plugin instance is not initialized");
            return false;
        };

        let Some(inactive_processor) = instance.take_inactive_processor_blocking() else {
            log::warn!("plugin.activate: inactive processor is unavailable");
            return false;
        };

        let mut core = instance.core.lock();
        let Some(core) = core.as_mut() else {
            log::warn!("plugin.activate: plugin core is not initialized");
            drop(inactive_processor);
            return false;
        };
        let processor = match core.activate(
            ActivateContext {
                sample_rate,
                min_frames_count,
                max_frames_count,
                host_tail: runtime
                    .capabilities
                    .tail
                    .then(|| instance.host_tail.create_handle())
                    .flatten(),
            },
            inactive_processor,
        ) {
            Ok(result) => {
                if result.notifications.latency_changed {
                    if runtime.capabilities.latency {
                        instance.host_latency.changed();
                    } else {
                        log::warn!(
                            "plugin.activate: latency_changed requested without latency extension"
                        );
                    }
                }
                result.processor
            }
            Err(error) => {
                log::warn!("plugin.activate: plugin activate failed: {error}");
                match core.initialize_processor() {
                    Ok(inactive) => instance.put_inactive_processor_blocking(inactive),
                    Err(error) => log::warn!(
                        "plugin.activate: inactive processor recreation failed after activation error: {error}"
                    ),
                }
                return false;
            }
        };

        instance.put_processor_blocking(processor);
        true
    })
}

pub(super) unsafe extern "C" fn plugin_deactivate(plugin: *const clap_plugin) {
    ffi_unit(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            log::warn!("plugin.deactivate: missing plugin instance");
            return;
        };
        // deactivate is a cleanup callback that must reclaim the ActiveProcessor before
        // returning completion to the host. Even if a wrapper runs lifecycle callbacks
        // concurrently, wait here to avoid missing the teardown.
        let _guard = instance.enter_lifecycle_blocking();
        if let Some(processor) = instance.take_processor_blocking() {
            let mut core = instance.core.lock();
            let Some(core) = core.as_mut() else {
                log::warn!("plugin.deactivate: plugin core is not initialized");
                drop(processor);
                return;
            };
            match core.deactivate(processor) {
                Ok(inactive) => instance.put_inactive_processor_blocking(inactive),
                Err(error) => {
                    log::warn!("plugin.deactivate: plugin deactivate failed: {error}");
                }
            }
        }
    });
}

pub(super) unsafe extern "C" fn plugin_start_processing(plugin: *const clap_plugin) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            wrac_log::rtwarn!("plugin.start_processing: missing plugin instance");
            return false;
        };
        // In wrapper formats, `start_processing` / `stop_processing` may not be
        // synchronized with the VST3/AU activate. A dedicated flag would become a
        // failure point that stops audio at the host's discretion, so whether processing
        // is possible is determined solely by the presence of an ActiveProcessor.
        let can_process = instance.has_processor_or_busy();
        if !can_process {
            wrac_log::rtwarn!("plugin.start_processing: no processor is available");
        }
        can_process
    })
}

pub(super) unsafe extern "C" fn plugin_stop_processing(_plugin: *const clap_plugin) {
    ffi_unit(|| {});
}

pub(super) unsafe extern "C" fn plugin_reset(plugin: *const clap_plugin) {
    ffi_unit(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            wrac_log::rtwarn!("plugin.reset: missing plugin instance");
            return;
        };
        let Some(()) = instance.with_processor_mut(|processor| {
            if let Some(processor) = processor {
                processor.reset();
            } else {
                wrac_log::rtdebug!("plugin.reset: no processor is available");
            }
        }) else {
            wrac_log::rtwarn!("plugin.reset: processor is busy");
            return;
        };
    });
}

pub(super) unsafe extern "C" fn plugin_process(
    plugin: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    ffi_status(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            wrac_log::rterror!("plugin.process: missing plugin instance");
            return CLAP_PROCESS_ERROR;
        };

        if process.is_null() {
            wrac_log::rtwarn!("plugin.process: null process pointer");
            return CLAP_PROCESS_SLEEP;
        }
        let _process_depth_guard = RtDepthGuard::enter(&instance.rt_process_depth);
        let process = unsafe { &*process };
        let events = unsafe {
            crate::interface::EventLists::from_raw(process.in_events, process.out_events)
        };
        let audio = match unsafe { audio_buffers(process) } {
            Ok(audio) => audio,
            Err(error) => {
                wrac_log::rterror!("plugin.process: invalid audio buffers: {error}");
                return CLAP_PROCESS_ERROR;
            }
        };

        // The audio callback never takes the `PluginInstance` lock. Whether processing is
        // possible is determined by the actual presence of a `ActiveProcessor`, not a separate
        // flag. If a wrapper violates lifecycle ordering, the RT path falls through to
        // sleep/error without waiting.
        let Some(result) = instance.with_processor_mut(|processor| {
            let Some(processor) = processor else {
                wrac_log::rtdebug!("plugin.process: no processor is available");
                return CLAP_PROCESS_SLEEP;
            };

            match processor.process(ProcessContext {
                frames_count: process.frames_count,
                audio,
                events,
                transport: unsafe { process.transport.as_ref() }.map(TransportEvent::from_raw),
                #[cfg(feature = "raw-clap-forwarding")]
                raw: unsafe { crate::interface::RawProcessContext::from_raw(process) },
            }) {
                Ok(ProcessStatus::Continue) => CLAP_PROCESS_CONTINUE,
                Ok(ProcessStatus::ContinueIfNotQuiet) => CLAP_PROCESS_CONTINUE_IF_NOT_QUIET,
                Ok(ProcessStatus::Tail) => CLAP_PROCESS_TAIL,
                Ok(ProcessStatus::Sleep) => CLAP_PROCESS_SLEEP,
                Err(error) => {
                    wrac_log::rterror!("plugin.process: processor failed: {error}");
                    CLAP_PROCESS_ERROR
                }
            }
        }) else {
            let flush_depth = instance.rt_flush_depth.load(Ordering::Relaxed);
            wrac_log::rtdebug!("plugin.process busy fd={}", flush_depth);
            wrac_log::rtwarn!("plugin.process: processor is busy");
            return CLAP_PROCESS_SLEEP;
        };
        result
    })
}

pub(super) unsafe extern "C" fn plugin_get_extension(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    ffi_ptr(|| {
        if id.is_null() {
            wrac_log::rtwarn!("plugin.get_extension: null extension id");
            return ptr::null();
        }
        let id = unsafe { CStr::from_ptr(id) };
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(_plugin) }) else {
            wrac_log::rtwarn!("plugin.get_extension: missing plugin instance");
            return ptr::null();
        };
        let Some(runtime) = instance.runtime.get() else {
            // Some wrapper backends may probe from native object construction paths. Returning
            // null keeps those calls non-blocking and avoids exposing half-frozen capabilities.
            wrac_log::rtwarn!("plugin.get_extension: plugin instance is not initialized");
            return ptr::null();
        };
        if id == CLAP_EXT_AUDIO_PORTS && runtime.capabilities.audio_ports {
            &audio_ports::AUDIO_PORTS as *const _ as *const c_void
        } else if (id == CLAP_EXT_CONFIGURABLE_AUDIO_PORTS
            || id == CLAP_EXT_CONFIGURABLE_AUDIO_PORTS_COMPAT)
            && runtime.capabilities.configurable_audio_ports
        {
            &configurable_audio_ports::CONFIGURABLE_AUDIO_PORTS as *const _ as *const c_void
        } else if id == CLAP_EXT_NOTE_PORTS && runtime.capabilities.note_ports {
            &note_ports::NOTE_PORTS as *const _ as *const c_void
        } else if id == CLAP_EXT_PARAMS {
            &params_extension::PARAMS as *const _ as *const c_void
        } else if id == CLAP_EXT_STATE && runtime.capabilities.state {
            &state_extension::STATE as *const _ as *const c_void
        } else if id == CLAP_EXT_GUI && runtime.capabilities.gui {
            &gui_extension::GUI as *const _ as *const c_void
        } else if id == CLAP_EXT_RENDER && runtime.capabilities.render {
            &render_extension::RENDER as *const _ as *const c_void
        } else if id == CLAP_EXT_TAIL && runtime.capabilities.tail {
            &tail_extension::TAIL as *const _ as *const c_void
        } else if id == CLAP_EXT_LATENCY {
            // Keep this pointer non-null even when `PluginInstance::latency()` returned
            // `None`. clap-wrapper's AAX backend calls `_ext._latency->get(...)`
            // unconditionally during activation, so exposing capability absence as a
            // null CLAP extension pointer can crash before WRAC code gets a chance to
            // diagnose the product bug. The product-facing contract is still enforced
            // during plugin initialization by the debug assertion above; this fallback exists
            // only to keep release wrapper builds from failing at the ABI boundary.
            &latency_extension::LATENCY as *const _ as *const c_void
        } else if id == CLAP_PLUGIN_AS_VST3 {
            &vst3_extension::VST3 as *const _ as *const c_void
        } else {
            ptr::null()
        }
    })
}

pub(super) unsafe extern "C" fn plugin_on_main_thread(plugin: *const clap_plugin) {
    ffi_unit(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            log::warn!("plugin.on_main_thread: missing plugin instance");
            return;
        };
        let mut core = instance.core.lock();
        let Some(core) = core.as_mut() else {
            log::warn!("plugin.on_main_thread: plugin core is not initialized");
            return;
        };
        core.on_main_thread();
    });
}
