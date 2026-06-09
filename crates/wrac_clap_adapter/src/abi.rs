//! Module that binds the CLAP ABI to `PluginCore` instances.
//!
//! The public API is surfaced through re-exports in `lib.rs` and `export_clap_entry!`.
//! This module is responsible only for C ABI callbacks and owning the adapter state.

use std::cell::UnsafeCell;
use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{
    CLAP_PROCESS_CONTINUE, CLAP_PROCESS_CONTINUE_IF_NOT_QUIET, CLAP_PROCESS_ERROR,
    CLAP_PROCESS_SLEEP, CLAP_PROCESS_TAIL, clap_process, clap_process_status,
};
use clap_sys::version::clap_version_is_compatible;
use parking_lot::Mutex;
use wrac_host_context::{HostContext, PluginFormat};

mod audio_buffers;
mod audio_ports;
mod configurable_audio_ports;
mod ffi;
mod gui_extension;
mod latency_extension;
mod note_ports;
mod params_extension;
mod render_extension;
mod state_extension;
mod tail_extension;
mod vst3_extension;

use self::audio_buffers::audio_buffers;
use self::ffi::{ffi_bool, ffi_ptr, ffi_status, ffi_unit, four_char_code};
use crate::entry::{
    EntryContext, EntryRegistration, decrement_entry_init_count, entry_init_count,
    increment_entry_init_count, reset_entry_init_count,
};
use crate::factory::{
    AaxFactoryState, Auv2FactoryState, ClapPluginFactoryAsAax, ClapPluginFactoryAsAuv2,
    ClapPluginFactoryAsVst3, ClapPluginInfoAsAax, ClapPluginInfoAsAuv2, ClapPluginInfoAsVst3,
    Vst3FactoryState, WracPluginMainThreadHook, aax_factory_ptr, aax_factory_state,
    auv2_factory_ptr, auv2_factory_state, clap_factory_state, factory_ptr, main_thread_hook_ptr,
    main_thread_hook_state, vst3_factory_ptr, vst3_factory_state,
};
use crate::host_gui::HostGuiResizeRequest;
use crate::host_state::HostStateDirtyNotification;
use crate::params::ParameterEditQueue;
use crate::{
    ActivateContext, PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension, PluginCore,
    PluginCoreContext, PluginGuiExtension, PluginLatencyExtension, PluginNotePortsExtension,
    PluginParamsExtension, PluginRenderExtension, PluginStateExtension, PluginTailExtension,
    ProcessContext, ProcessStatus, Processor, TransportEvent,
};

// clap-wrapper reads this draft factory when generating AUv2 metadata. Without a
// separate AU manufacturer/subtype, it can collide with the generic wrapper identity
// and cause auval to validate a different, older plugin instead.
const CLAP_PLUGIN_FACTORY_INFO_AUV2: &CStr = c"clap.plugin-factory-info-as-auv2.draft0";
// clap-wrapper can infer VST3 metadata from CLAP descriptors, but commercial products
// need stable VST3 class IDs and explicit host browser categories across wrapper updates.
const CLAP_PLUGIN_FACTORY_INFO_VST3: &CStr = c"clap.plugin-factory-info-as-vst3/0";
const CLAP_PLUGIN_AS_VST3: &CStr = c"clap.plugin-info-as-vst3/0";
// AAX declares manufacturer/product/stem IDs at factory time, so commercial
// products must provide this extension rather than relying on wrapper-generated IDs.
const CLAP_PLUGIN_FACTORY_INFO_AAX: &CStr = c"clap.plugin-factory-info-as-aax/1";
const WRAC_PLUGIN_MAIN_THREAD_HOOK: &CStr = c"com.novonotes.wrac.plugin-main-thread-hook/0";

/// Synchronization boundary between a CLAP instance and the Rust core.
///
/// Key design: separate the "lifecycle lock" from "capabilities read directly by
/// host-facing callbacks". The `core` lock is used only by `activate`/`deactivate`,
/// which move processor ownership. Parameter/state/port queries read `Arc`s frozen at
/// instance creation. Without this separation, a wrapper that re-enters a query during
/// `activate()` would fail to acquire the core lock and return "no parameters" or "state
/// save failed" to the host — no crash, but project data and routing can be corrupted.
pub(crate) struct PluginInstance {
    plugin: clap_plugin,
    registration: &'static EntryRegistration,
    // Owner of the processor lifecycle; only activate/deactivate take this lock.
    core: Mutex<Box<dyn PluginCore>>,
    // Capability presence is frozen at instance creation. Coupling it to runtime state
    // would make extensions appear to disappear transiently during queries.
    capabilities: PluginCapabilities,
    audio_ports: Option<Arc<dyn PluginAudioPortsExtension>>,
    configurable_audio_ports: Option<Arc<dyn PluginConfigurableAudioPortsExtension>>,
    note_ports: Option<Arc<dyn PluginNotePortsExtension>>,
    parameters: Option<Arc<dyn PluginParamsExtension>>,
    state: Option<Arc<dyn PluginStateExtension>>,
    gui: Option<Arc<dyn PluginGuiExtension>>,
    render: Option<Arc<dyn PluginRenderExtension>>,
    tail: Option<Arc<dyn PluginTailExtension>>,
    latency: Option<Arc<dyn PluginLatencyExtension>>,
    host_context: HostContext,
    // Re-entry guard for GUI mutation callbacks. Fails immediately on re-entry to avoid
    // deadlock (GUI query callbacks do not go through this guard).
    gui_callback_busy: Mutex<()>,
    parameter_edits: Arc<ParameterEditQueue>,
    // To preserve soundness even when a wrapper violates thread/lifecycle annotations,
    // the RT path never takes a lock — only a callback that wins the atomic guard
    // constructs a `&mut` to `Processor`.
    processor: UnsafeCell<Option<Box<dyn Processor>>>,
    processor_busy: AtomicBool,
    lifecycle_busy: AtomicBool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PluginCapabilities {
    audio_ports: bool,
    configurable_audio_ports: bool,
    note_ports: bool,
    parameters: bool,
    state: bool,
    gui: bool,
    render: bool,
    tail: bool,
}

// Safety: CLAP shares the same opaque plugin pointer across callbacks. Adapter state is
// shared via locks and atomics, so Rust aliasing rules are never violated even when the
// host's thread annotations or callback ordering breaks down.
unsafe impl Send for PluginInstance {}
unsafe impl Sync for PluginInstance {}

impl PluginInstance {
    fn new(
        registration: &'static EntryRegistration,
        descriptor_index: usize,
        plugin_id: &str,
        host: *const clap_host,
        clap_host_name: Option<String>,
        host_context: HostContext,
    ) -> Option<Box<Self>> {
        let parameter_edits = Arc::new(ParameterEditQueue::new(host));
        // Pass as a safe proxy so product GUI code can hold it without knowing about
        // host pointers or CLAP event lifetimes.
        let context = PluginCoreContext {
            host_parameter_edit_notifier: parameter_edits.clone(),
            host_state_dirty_notifier: Arc::new(HostStateDirtyNotification::new(host)),
            host_gui_resize_requester: Arc::new(HostGuiResizeRequest::new(host)),
            host_context: host_context.clone(),
        };
        let core = registration
            .entry
            .plugin_factory()?
            .create_plugin(plugin_id, context)?;
        // Product construction initializes logging. Emit immediately afterward so
        // wrapper/host routing is visible before capability queries or GUI attachment.
        log::info!(
            "factory.create_plugin: host_context host=\"{}\" process=\"{}\" format={} clap_host_name=\"{}\"",
            host_context.host.display_name,
            host_context.host.process_name,
            host_context.plugin_format.as_str(),
            clap_host_name.as_deref().unwrap_or("")
        );
        // Freeze capabilities here, before callbacks begin. Waiting on the core lock
        // inside get_extension would make us dependent on host re-entry order. The Arc
        // is just an entry point; the source of truth remains in the plugin's store.
        let audio_ports = core.audio_ports();
        let configurable_audio_ports = core.configurable_audio_ports();
        let note_ports = core.note_ports();
        let parameters = core.params();
        let state = core.state();
        let gui = core.gui();
        let render = core.render();
        let tail = core.tail();
        let latency = core.latency();
        let capabilities = PluginCapabilities {
            audio_ports: audio_ports.is_some(),
            configurable_audio_ports: configurable_audio_ports.is_some(),
            note_ports: note_ports.is_some(),
            parameters: parameters.is_some(),
            state: state.is_some(),
            gui: gui.is_some(),
            render: render.is_some(),
            tail: tail.is_some(),
        };
        let storage = registration.storage();

        Some(Box::new(Self {
            plugin: clap_plugin {
                desc: storage.descriptors.get(descriptor_index)?.clap_descriptor(),
                plugin_data: ptr::null_mut(),
                init: Some(plugin_init),
                destroy: Some(plugin_destroy),
                activate: Some(plugin_activate),
                deactivate: Some(plugin_deactivate),
                start_processing: Some(plugin_start_processing),
                stop_processing: Some(plugin_stop_processing),
                reset: Some(plugin_reset),
                process: Some(plugin_process),
                get_extension: Some(plugin_get_extension),
                on_main_thread: Some(plugin_on_main_thread),
            },
            registration,
            core: Mutex::new(core),
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
            host_context,
            gui_callback_busy: Mutex::new(()),
            parameter_edits,
            processor: UnsafeCell::new(None),
            processor_busy: AtomicBool::new(false),
            lifecycle_busy: AtomicBool::new(false),
        }))
    }

    pub(crate) unsafe fn from_plugin<'a>(plugin: *const clap_plugin) -> Option<&'a Self> {
        if plugin.is_null() {
            return None;
        }
        let data = unsafe { (*plugin).plugin_data };
        if data.is_null() {
            return None;
        }
        Some(unsafe { &*(data as *const Self) })
    }

    fn with_processor_mut<R>(
        &self,
        f: impl FnOnce(Option<&mut Box<dyn Processor>>) -> R,
    ) -> Option<R> {
        if self
            .processor_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }

        struct ProcessorBusyGuard<'a>(&'a AtomicBool);
        impl Drop for ProcessorBusyGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }

        let _guard = ProcessorBusyGuard(&self.processor_busy);
        Some(f(unsafe { &mut *self.processor.get() }.as_mut()))
    }

    fn try_take_processor(&self) -> Option<Option<Box<dyn Processor>>> {
        self.with_processor_mut(|_| unsafe { &mut *self.processor.get() }.take())
    }

    fn put_processor_blocking(&self, processor: Box<dyn Processor>) {
        let mut processor = Some(processor);
        loop {
            if self
                .processor_busy
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                struct ProcessorBusyGuard<'a>(&'a AtomicBool);
                impl Drop for ProcessorBusyGuard<'_> {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let _guard = ProcessorBusyGuard(&self.processor_busy);
                let storage = unsafe { &mut *self.processor.get() };
                let old = storage.replace(processor.take().expect("stored once"));
                drop(old);
                return;
            }
            // activate is not realtime. Rather than duplicating processor presence as
            // separate state, wait until the borrow guard is free, then store.
            std::thread::yield_now();
        }
    }

    fn take_processor_blocking(&self) -> Option<Box<dyn Processor>> {
        loop {
            if let Some(processor) = self.try_take_processor() {
                return processor;
            }
            // deactivate/destroy are non-realtime lifecycle callbacks. Waiting here
            // ensures that even a wrapper which races lifecycle against audio never
            // frees the instance while process() holds a temporary Processor borrow.
            std::thread::yield_now();
        }
    }

    pub(crate) fn has_processor_or_busy(&self) -> bool {
        self.with_processor_mut(|processor| processor.is_some())
            .unwrap_or(true)
    }

    fn try_enter_lifecycle(&self) -> Option<LifecycleGuard<'_>> {
        self.lifecycle_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| LifecycleGuard(&self.lifecycle_busy))
    }

    fn enter_lifecycle_blocking(&self) -> LifecycleGuard<'_> {
        loop {
            if let Some(guard) = self.try_enter_lifecycle() {
                return guard;
            }
            // `destroy()` is a callback that can afford to wait. Releasing without
            // waiting would leave out-of-order wrapper lifecycle callbacks holding
            // stale adapter state.
            std::thread::yield_now();
        }
    }
}

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

struct LifecycleGuard<'a>(&'a AtomicBool);

impl Drop for LifecycleGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
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

        let plugin_path = if plugin_path.is_null() {
            None
        } else {
            let plugin_path = unsafe { CStr::from_ptr(plugin_path) };
            match plugin_path.to_str() {
                Ok(plugin_path) => Some(plugin_path),
                Err(error) => {
                    log::warn!("entry.init: invalid UTF-8 plugin_path: {error}");
                    reset_entry_init_count(registration);
                    return false;
                }
            }
        };
        if let Err(error) = registration.entry.init(EntryContext { plugin_path }) {
            log::warn!("entry.init: product init failed: {error}");
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
            log::warn!("entry.deinit: called while entry is not initialized");
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

        let Some(mut instance) = PluginInstance::new(
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
            log::warn!("factory.create_plugin: product factory returned no plugin core");
            return ptr::null();
        };
        let instance_ptr = (&mut *instance) as *mut PluginInstance;
        instance.plugin.plugin_data = instance_ptr.cast();
        let plugin_ptr = &instance.plugin as *const clap_plugin;
        let _ = Box::into_raw(instance);
        plugin_ptr
    })
}

unsafe extern "C" fn plugin_init(plugin: *const clap_plugin) -> bool {
    ffi_bool(|| {
        let initialized = unsafe { PluginInstance::from_plugin(plugin).is_some() };
        if !initialized {
            log::warn!("plugin.init: missing plugin instance");
        }
        initialized
    })
}

unsafe extern "C" fn plugin_destroy(plugin: *const clap_plugin) {
    ffi_unit(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("plugin.destroy: missing plugin instance");
            return;
        };
        let detach_in_adapter = instance.host_context.plugin_format == PluginFormat::Unknown;
        let registration = instance.registration;
        let guard = instance.enter_lifecycle_blocking();

        if let Some(gui) = &instance.gui {
            if let Some(_gui_callback) = instance.gui_callback_busy.try_lock() {
                gui.destroy();
            } else {
                log::error!(
                    "skipping GUI destroy during plugin destruction because another GUI callback is active"
                );
            }
        }

        if let Some(processor) = instance.take_processor_blocking() {
            if let Err(error) = instance.core.lock().deactivate(processor) {
                log::warn!("plugin.destroy: plugin deactivate failed: {error}");
            }
        }

        drop(guard);
        let data = unsafe { (*plugin).plugin_data } as *mut PluginInstance;
        unsafe {
            drop(Box::from_raw(data));
        }
        if detach_in_adapter {
            registration.entry.detach_main_thread();
        }
    });
}

unsafe extern "C" fn plugin_activate(
    plugin: *const clap_plugin,
    sample_rate: f64,
    min_frames_count: u32,
    max_frames_count: u32,
) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
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

        let processor = match instance.core.lock().activate(ActivateContext {
            sample_rate,
            min_frames_count,
            max_frames_count,
        }) {
            Ok(processor) => processor,
            Err(error) => {
                log::warn!("plugin.activate: plugin activate failed: {error}");
                return false;
            }
        };

        instance.put_processor_blocking(processor);
        true
    })
}

unsafe extern "C" fn plugin_deactivate(plugin: *const clap_plugin) {
    ffi_unit(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("plugin.deactivate: missing plugin instance");
            return;
        };
        // deactivate is a cleanup callback that must reclaim the Processor before
        // returning completion to the host. Even if a wrapper runs lifecycle callbacks
        // concurrently, wait here to avoid missing the teardown.
        let _guard = instance.enter_lifecycle_blocking();
        if let Some(processor) = instance.take_processor_blocking() {
            if let Err(error) = instance.core.lock().deactivate(processor) {
                log::warn!("plugin.deactivate: plugin deactivate failed: {error}");
            }
        }
    });
}

unsafe extern "C" fn plugin_start_processing(plugin: *const clap_plugin) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            wrac_log::rtwarn!("plugin.start_processing: missing plugin instance");
            return false;
        };
        // In wrapper formats, `start_processing` / `stop_processing` may not be
        // synchronized with the VST3/AU activate. A dedicated flag would become a
        // failure point that stops audio at the host's discretion, so whether processing
        // is possible is determined solely by the presence of a Processor.
        let can_process = instance.has_processor_or_busy();
        if !can_process {
            wrac_log::rtwarn!("plugin.start_processing: no processor is available");
        }
        can_process
    })
}

unsafe extern "C" fn plugin_stop_processing(_plugin: *const clap_plugin) {
    ffi_unit(|| {});
}

unsafe extern "C" fn plugin_reset(plugin: *const clap_plugin) {
    ffi_unit(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
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

unsafe extern "C" fn plugin_process(
    plugin: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    ffi_status(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            wrac_log::rterror!("plugin.process: missing plugin instance");
            return CLAP_PROCESS_ERROR;
        };

        if process.is_null() {
            wrac_log::rtwarn!("plugin.process: null process pointer");
            return CLAP_PROCESS_SLEEP;
        }
        let process = unsafe { &*process };
        let mut events =
            unsafe { crate::ProcessEvents::from_raw(process.in_events, process.out_events) };
        instance
            .parameter_edits
            .drain_output_parameter_events(&mut events.output);
        let audio = match unsafe { audio_buffers(process) } {
            Ok(audio) => audio,
            Err(error) => {
                wrac_log::rterror!("plugin.process: invalid audio buffers: {error}");
                return CLAP_PROCESS_ERROR;
            }
        };

        // The audio callback never takes the `PluginCore` lock. Whether processing is
        // possible is determined by the actual presence of a `Processor`, not a separate
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
            wrac_log::rtwarn!("plugin.process: processor is busy");
            return CLAP_PROCESS_SLEEP;
        };
        result
    })
}

unsafe extern "C" fn plugin_get_extension(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    ffi_ptr(|| {
        if id.is_null() {
            wrac_log::rtwarn!("plugin.get_extension: null extension id");
            return ptr::null();
        }
        let id = unsafe { CStr::from_ptr(id) };
        let Some(instance) = (unsafe { PluginInstance::from_plugin(_plugin) }) else {
            wrac_log::rtwarn!("plugin.get_extension: missing plugin instance");
            return ptr::null();
        };
        if id == CLAP_EXT_AUDIO_PORTS && instance.capabilities.audio_ports {
            &audio_ports::AUDIO_PORTS as *const _ as *const c_void
        } else if (id == CLAP_EXT_CONFIGURABLE_AUDIO_PORTS
            || id == CLAP_EXT_CONFIGURABLE_AUDIO_PORTS_COMPAT)
            && instance.capabilities.configurable_audio_ports
        {
            &configurable_audio_ports::CONFIGURABLE_AUDIO_PORTS as *const _ as *const c_void
        } else if id == CLAP_EXT_NOTE_PORTS && instance.capabilities.note_ports {
            &note_ports::NOTE_PORTS as *const _ as *const c_void
        } else if id == CLAP_EXT_PARAMS && instance.capabilities.parameters {
            &params_extension::PARAMS as *const _ as *const c_void
        } else if id == CLAP_EXT_STATE && instance.capabilities.state {
            &state_extension::STATE as *const _ as *const c_void
        } else if id == CLAP_EXT_GUI && instance.capabilities.gui {
            &gui_extension::GUI as *const _ as *const c_void
        } else if id == CLAP_EXT_RENDER && instance.capabilities.render {
            &render_extension::RENDER as *const _ as *const c_void
        } else if id == CLAP_EXT_TAIL && instance.capabilities.tail {
            &tail_extension::TAIL as *const _ as *const c_void
        } else if id == CLAP_EXT_LATENCY {
            // Some wrappers query latency unconditionally during activation. Exposing a
            // zero-latency fallback keeps optional product support from becoming a null
            // extension pointer at the wrapper boundary.
            &latency_extension::LATENCY as *const _ as *const c_void
        } else if id == CLAP_PLUGIN_AS_VST3 {
            &vst3_extension::VST3 as *const _ as *const c_void
        } else {
            ptr::null()
        }
    })
}

unsafe extern "C" fn plugin_on_main_thread(_plugin: *const clap_plugin) {
    // WRAC intentionally does not route product work through CLAP's main-thread callback.
    // GUI and main-thread tasks should use novonotes_run_loop/wxp instead, which keeps
    // behavior consistent across native CLAP and wrapper-backed VST3/AU hosts.
}
