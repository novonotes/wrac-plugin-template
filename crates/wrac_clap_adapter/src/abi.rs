//! Module that binds the CLAP ABI to `PluginInstance` instances.
//!
//! The public API is surfaced through re-exports in `lib.rs` and `export_clap_entry!`.
//! This module is responsible only for C ABI callbacks and owning the adapter state.

use std::cell::UnsafeCell;
use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::ThreadId;

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
use crate::host_audio_ports::HostAudioPortsProxy;
use crate::host_gui::HostGuiProxy;
use crate::host_latency::HostLatencyProxy;
use crate::host_lifecycle::HostLifecycleProxy;
use crate::host_note_ports::HostNotePortsProxy;
use crate::host_state::HostStateProxy;
use crate::host_tail::HostTailFactory;
use crate::params::HostParamsProxy;
use crate::{
    ActivateContext, ActiveProcessor, InactiveProcessor, PluginAudioPortsExtension,
    PluginConfigurableAudioPortsExtension, PluginGuiExtension, PluginInstance,
    PluginInstanceContext, PluginLatencyExtension, PluginNotePortsExtension, PluginParamsQuery,
    PluginRenderExtension, PluginStateExtension, PluginTailExtension, ProcessContext,
    ProcessStatus, TransportEvent,
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

pub(crate) struct RtDepthGuard<'a>(&'a AtomicU32);

impl<'a> RtDepthGuard<'a> {
    pub(crate) fn enter(depth: &'a AtomicU32) -> Self {
        depth.fetch_add(1, Ordering::Relaxed);
        Self(depth)
    }
}

impl Drop for RtDepthGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Synchronization boundary between a CLAP instance and the Rust core.
///
/// Key design: separate the "lifecycle lock" from "capabilities read directly by
/// host-facing callbacks". The `core` lock is used only by `activate`/`deactivate`,
/// which move processor ownership. Parameter/state/port queries read `Arc`s frozen at
/// plugin initialization. Without this separation, a wrapper that re-enters a query during
/// `activate()` would fail to acquire the core lock and return "no parameters" or "state
/// save failed" to the host — no crash, but project data and routing can be corrupted.
///
/// Lifecycle callbacks that create runtime state fail fast on re-entry instead of waiting.
/// Waiting from pre-init or init-time host callbacks can deadlock with wrapper formats whose
/// native object is still being constructed. Teardown is the exception: `deactivate` and
/// `destroy` wait so they can finish reclaiming processors before the host releases the object.
pub(crate) struct PluginInstanceState {
    plugin: clap_plugin,
    registration: &'static EntryRegistration,
    plugin_id: String,
    clap_host_name: Option<String>,
    // Owner of the product instance lifecycle. It is intentionally empty until
    // `plugin.init`, because CLAP forbids host extension access before that callback.
    core: Mutex<Option<Box<dyn PluginInstance>>>,
    // Capability presence is frozen during `plugin.init`, before host callbacks may query
    // extensions. Coupling it to later runtime state would make extensions appear transient.
    runtime: OnceLock<PluginRuntime>,
    host_latency: HostLatencyProxy,
    host_tail: HostTailFactory,
    // Host extension proxies are passed to product code before the product instance exists.
    // Keep extension lookups inert until capability freeze completes. CLAP permits
    // `get_extension` during `plugin.init`; allowing product constructors to trigger host
    // extension callbacks earlier could make a re-entrant host cache half-initialized plugin
    // capabilities.
    host_extensions_initialized: Arc<AtomicBool>,
    host_context: HostContext,
    // Re-entry guard for GUI mutation callbacks. Fails immediately on re-entry to avoid
    // deadlock (GUI query callbacks do not go through this guard).
    gui_callback_busy: Mutex<()>,
    // Defensive owner check for CLAP GUI [main-thread] callbacks. The adapter cannot
    // identify the OS UI thread portably here, but it can reject lifecycle callbacks
    // that move between host threads within one GUI session.
    gui_lifecycle_thread: Mutex<Option<ThreadId>>,
    host_params: Arc<HostParamsProxy>,
    host_state: Arc<HostStateProxy>,
    host_audio_ports: Arc<HostAudioPortsProxy>,
    host_note_ports: Arc<HostNotePortsProxy>,
    host_lifecycle: Arc<HostLifecycleProxy>,
    host_gui: Arc<HostGuiProxy>,
    // To preserve soundness even when a wrapper violates thread/lifecycle annotations,
    // the RT path never takes a lock — only a callback that wins the atomic guard
    // constructs a `&mut` to the active or inactive processor.
    inactive_processor: UnsafeCell<Option<Box<dyn InactiveProcessor>>>,
    processor: UnsafeCell<Option<Box<dyn ActiveProcessor>>>,
    processor_busy: AtomicBool,
    processor_active: AtomicBool,
    lifecycle_busy: AtomicBool,
    lifecycle_thread: Mutex<Option<ThreadId>>,
    rt_process_depth: AtomicU32,
    rt_flush_depth: AtomicU32,
    rt_processor_contention: AtomicBool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PluginCapabilities {
    audio_ports: bool,
    configurable_audio_ports: bool,
    note_ports: bool,
    state: bool,
    gui: bool,
    render: bool,
    tail: bool,
    latency: bool,
}

struct PluginRuntime {
    capabilities: PluginCapabilities,
    audio_ports: Option<Arc<dyn PluginAudioPortsExtension>>,
    configurable_audio_ports: Option<Arc<dyn PluginConfigurableAudioPortsExtension>>,
    note_ports: Option<Arc<dyn PluginNotePortsExtension>>,
    parameters: Arc<dyn PluginParamsQuery>,
    state: Option<Arc<dyn PluginStateExtension>>,
    gui: Option<Arc<dyn PluginGuiExtension>>,
    render: Option<Arc<dyn PluginRenderExtension>>,
    tail: Option<Arc<dyn PluginTailExtension>>,
    latency: Option<Arc<dyn PluginLatencyExtension>>,
}

// Safety: CLAP shares the same opaque plugin pointer across callbacks. Adapter state is
// shared via locks and atomics, so Rust aliasing rules are never violated even when the
// host's thread annotations or callback ordering breaks down.
unsafe impl Send for PluginInstanceState {}
unsafe impl Sync for PluginInstanceState {}

impl PluginInstanceState {
    fn new(
        registration: &'static EntryRegistration,
        descriptor_index: usize,
        plugin_id: &str,
        host: *const clap_host,
        clap_host_name: Option<String>,
        host_context: HostContext,
    ) -> Option<Box<Self>> {
        let host_extensions_initialized = Arc::new(AtomicBool::new(false));
        let host_params = Arc::new(HostParamsProxy::new(
            host,
            host_extensions_initialized.clone(),
        ));
        let host_state = Arc::new(HostStateProxy::new(
            host,
            host_extensions_initialized.clone(),
        ));
        let host_audio_ports = Arc::new(HostAudioPortsProxy::new(
            host,
            host_extensions_initialized.clone(),
        ));
        let host_note_ports = Arc::new(HostNotePortsProxy::new(
            host,
            host_extensions_initialized.clone(),
        ));
        let host_lifecycle = Arc::new(HostLifecycleProxy::new(
            host,
            host_extensions_initialized.clone(),
        ));
        let host_gui = Arc::new(HostGuiProxy::new(host, host_extensions_initialized.clone()));
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
            plugin_id: plugin_id.to_string(),
            clap_host_name,
            core: Mutex::new(None),
            runtime: OnceLock::new(),
            host_latency: HostLatencyProxy::new(host, host_extensions_initialized.clone()),
            host_tail: HostTailFactory::new(host, host_extensions_initialized.clone()),
            host_extensions_initialized,
            host_context,
            gui_callback_busy: Mutex::new(()),
            gui_lifecycle_thread: Mutex::new(None),
            host_params,
            host_state,
            host_audio_ports,
            host_note_ports,
            host_lifecycle,
            host_gui,
            inactive_processor: UnsafeCell::new(None),
            processor: UnsafeCell::new(None),
            processor_busy: AtomicBool::new(false),
            processor_active: AtomicBool::new(false),
            lifecycle_busy: AtomicBool::new(false),
            lifecycle_thread: Mutex::new(None),
            rt_process_depth: AtomicU32::new(0),
            rt_flush_depth: AtomicU32::new(0),
            rt_processor_contention: AtomicBool::new(false),
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
        f: impl FnOnce(Option<&mut Box<dyn ActiveProcessor>>) -> R,
    ) -> Option<R> {
        if self
            .processor_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.rt_processor_contention.store(true, Ordering::Release);
            return None;
        }

        struct ProcessorBusyGuard<'a> {
            busy: &'a AtomicBool,
            contention: &'a AtomicBool,
            process_depth: &'a AtomicU32,
            flush_depth: &'a AtomicU32,
        }
        impl Drop for ProcessorBusyGuard<'_> {
            fn drop(&mut self) {
                self.busy.store(false, Ordering::Release);
                if self.contention.swap(false, Ordering::AcqRel) {
                    let process_depth = self.process_depth.load(Ordering::Relaxed);
                    let flush_depth = self.flush_depth.load(Ordering::Relaxed);
                    wrac_log::rtdebug!(
                        "processor.busy clear pd={} fd={}",
                        process_depth,
                        flush_depth
                    );
                }
            }
        }

        let _guard = ProcessorBusyGuard {
            busy: &self.processor_busy,
            contention: &self.rt_processor_contention,
            process_depth: &self.rt_process_depth,
            flush_depth: &self.rt_flush_depth,
        };
        Some(f(unsafe { &mut *self.processor.get() }.as_mut()))
    }

    fn try_take_processor(&self) -> Option<Option<Box<dyn ActiveProcessor>>> {
        self.with_processor_mut(|_| {
            let processor = unsafe { &mut *self.processor.get() }.take();
            if processor.is_some() {
                self.processor_active.store(false, Ordering::Release);
            }
            processor
        })
    }

    fn try_take_inactive_processor(&self) -> Option<Option<Box<dyn InactiveProcessor>>> {
        self.with_processor_mut(|_| unsafe { &mut *self.inactive_processor.get() }.take())
    }

    fn with_inactive_processor_mut<R>(
        &self,
        f: impl FnOnce(Option<&mut Box<dyn InactiveProcessor>>) -> R,
    ) -> Option<R> {
        if self
            .processor_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.rt_processor_contention.store(true, Ordering::Release);
            return None;
        }

        struct ProcessorBusyGuard<'a> {
            busy: &'a AtomicBool,
            contention: &'a AtomicBool,
            process_depth: &'a AtomicU32,
            flush_depth: &'a AtomicU32,
        }
        impl Drop for ProcessorBusyGuard<'_> {
            fn drop(&mut self) {
                self.busy.store(false, Ordering::Release);
                if self.contention.swap(false, Ordering::AcqRel) {
                    let process_depth = self.process_depth.load(Ordering::Relaxed);
                    let flush_depth = self.flush_depth.load(Ordering::Relaxed);
                    wrac_log::rtdebug!(
                        "processor.busy clear pd={} fd={}",
                        process_depth,
                        flush_depth
                    );
                }
            }
        }

        let _guard = ProcessorBusyGuard {
            busy: &self.processor_busy,
            contention: &self.rt_processor_contention,
            process_depth: &self.rt_process_depth,
            flush_depth: &self.rt_flush_depth,
        };
        Some(f(unsafe { &mut *self.inactive_processor.get() }.as_mut()))
    }

    fn put_processor_blocking(&self, processor: Box<dyn ActiveProcessor>) {
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
                self.processor_active.store(true, Ordering::Release);
                drop(old);
                return;
            }
            // activate is not realtime. Rather than duplicating processor presence as
            // separate state, wait until the borrow guard is free, then store.
            std::thread::yield_now();
        }
    }

    fn put_inactive_processor_blocking(&self, processor: Box<dyn InactiveProcessor>) {
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
                let storage = unsafe { &mut *self.inactive_processor.get() };
                let old = storage.replace(processor.take().expect("stored once"));
                drop(old);
                return;
            }
            std::thread::yield_now();
        }
    }

    pub(crate) fn is_processor_active(&self) -> bool {
        self.processor_active.load(Ordering::Acquire)
    }

    pub(crate) fn is_in_realtime_callback(&self) -> bool {
        self.rt_process_depth.load(Ordering::Acquire) > 0
            || self.rt_flush_depth.load(Ordering::Acquire) > 0
    }

    fn take_processor_blocking(&self) -> Option<Box<dyn ActiveProcessor>> {
        loop {
            if let Some(processor) = self.try_take_processor() {
                return processor;
            }
            // deactivate/destroy are non-realtime lifecycle callbacks. Waiting here
            // ensures that even a wrapper which races lifecycle against audio never
            // frees the instance while process() holds a temporary ActiveProcessor borrow.
            std::thread::yield_now();
        }
    }

    fn take_inactive_processor_blocking(&self) -> Option<Box<dyn InactiveProcessor>> {
        loop {
            if let Some(processor) = self.try_take_inactive_processor() {
                return processor;
            }
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
            .map(|_| {
                *self.lifecycle_thread.lock() = Some(std::thread::current().id());
                LifecycleGuard {
                    busy: &self.lifecycle_busy,
                    thread: &self.lifecycle_thread,
                }
            })
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

    pub(crate) fn enter_lifecycle_blocking_or_reject_reentry(&self) -> Option<LifecycleGuard<'_>> {
        let current = std::thread::current().id();
        loop {
            if let Some(guard) = self.try_enter_lifecycle() {
                return Some(guard);
            }
            if matches!(*self.lifecycle_thread.lock(), Some(owner) if owner == current) {
                return None;
            }
            std::thread::yield_now();
        }
    }

    pub(crate) fn enter_gui_lifecycle_thread(&self, callback_name: &'static str) -> bool {
        let current = std::thread::current().id();
        let mut owner = self.gui_lifecycle_thread.lock();
        match *owner {
            Some(expected) if expected != current => {
                log::error!(
                    "rejecting CLAP GUI main-thread callback from a different thread: callback={callback_name} expected={expected:?} current={current:?}"
                );
                false
            }
            Some(_) => true,
            None => {
                *owner = Some(current);
                true
            }
        }
    }

    pub(crate) fn clear_gui_lifecycle_thread(&self) {
        *self.gui_lifecycle_thread.lock() = None;
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

pub(crate) struct LifecycleGuard<'a> {
    busy: &'a AtomicBool,
    thread: &'a Mutex<Option<ThreadId>>,
}

impl Drop for LifecycleGuard<'_> {
    fn drop(&mut self) {
        *self.thread.lock() = None;
        self.busy.store(false, Ordering::Release);
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
        let _ = Box::into_raw(instance);
        plugin_ptr
    })
}

unsafe extern "C" fn plugin_init(plugin: *const clap_plugin) -> bool {
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
        // Product construction initializes logging. Emit immediately afterward so
        // wrapper/host routing is visible before capability queries or GUI attachment.
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

unsafe extern "C" fn plugin_destroy(plugin: *const clap_plugin) {
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
    });
}

unsafe extern "C" fn plugin_activate(
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

unsafe extern "C" fn plugin_deactivate(plugin: *const clap_plugin) {
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

unsafe extern "C" fn plugin_start_processing(plugin: *const clap_plugin) -> bool {
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

unsafe extern "C" fn plugin_stop_processing(_plugin: *const clap_plugin) {
    ffi_unit(|| {});
}

unsafe extern "C" fn plugin_reset(plugin: *const clap_plugin) {
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

unsafe extern "C" fn plugin_process(
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
        let events = unsafe { crate::EventLists::from_raw(process.in_events, process.out_events) };
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
                raw: unsafe { crate::RawProcessContext::from_raw(process) },
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

unsafe extern "C" fn plugin_on_main_thread(plugin: *const clap_plugin) {
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

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::ptr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use clap_sys::ext::audio_ports::{
        CLAP_AUDIO_PORTS_RESCAN_NAMES, CLAP_EXT_AUDIO_PORTS, clap_host_audio_ports,
    };
    use clap_sys::ext::latency::{CLAP_EXT_LATENCY, clap_host_latency, clap_plugin_latency};
    use clap_sys::ext::note_ports::{
        CLAP_EXT_NOTE_PORTS, CLAP_NOTE_DIALECT_CLAP, CLAP_NOTE_PORTS_RESCAN_NAMES,
        clap_host_note_ports,
    };
    use clap_sys::host::clap_host;
    use clap_sys::plugin::clap_plugin;
    use clap_sys::version::CLAP_VERSION;

    use super::{
        PluginInstanceState, plugin_activate, plugin_destroy, plugin_get_extension, plugin_init,
        plugin_on_main_thread,
    };
    use crate::entry::EntryRegistration;
    use crate::{
        ActivateContext, ActivateNotifications, ActivateResult, ActiveProcessor, EntryContext,
        HostAudioPorts, HostLifecycle, HostNotePorts, InactiveProcessor, NoteDialects,
        ParamFlushContext, PluginDescriptor, PluginEntry, PluginFactory, PluginInstance,
        PluginInstanceContext, PluginLatencyExtension, PluginParamsQuery, PluginResult,
        ProcessContext, ProcessStatus,
    };
    use wrac_host_context::HostContext;

    static ZERO_LATENCY_ENTRY: TestEntry = TestEntry {
        factory: TestFactory {
            activate_latency_changed: false,
            request_host_lifecycle: false,
            request_host_lifecycle_during_create: false,
            request_host_ports: false,
            request_host_ports_during_create: false,
            count_create_plugin: false,
        },
    };
    static ZERO_LATENCY_REGISTRATION: EntryRegistration =
        EntryRegistration::new(&ZERO_LATENCY_ENTRY);

    static ACTIVATE_LATENCY_CHANGED_ENTRY: TestEntry = TestEntry {
        factory: TestFactory {
            activate_latency_changed: true,
            request_host_lifecycle: false,
            request_host_lifecycle_during_create: false,
            request_host_ports: false,
            request_host_ports_during_create: false,
            count_create_plugin: false,
        },
    };
    static ACTIVATE_LATENCY_CHANGED_REGISTRATION: EntryRegistration =
        EntryRegistration::new(&ACTIVATE_LATENCY_CHANGED_ENTRY);

    static REQUEST_HOST_LIFECYCLE_ENTRY: TestEntry = TestEntry {
        factory: TestFactory {
            activate_latency_changed: false,
            request_host_lifecycle: true,
            request_host_lifecycle_during_create: false,
            request_host_ports: false,
            request_host_ports_during_create: false,
            count_create_plugin: false,
        },
    };
    static REQUEST_HOST_PORTS_ENTRY: TestEntry = TestEntry {
        factory: TestFactory {
            activate_latency_changed: false,
            request_host_lifecycle: false,
            request_host_lifecycle_during_create: false,
            request_host_ports: true,
            request_host_ports_during_create: false,
            count_create_plugin: false,
        },
    };
    static REQUEST_HOST_PORTS_DURING_CREATE_ENTRY: TestEntry = TestEntry {
        factory: TestFactory {
            activate_latency_changed: false,
            request_host_lifecycle: false,
            request_host_lifecycle_during_create: true,
            request_host_ports: false,
            request_host_ports_during_create: true,
            count_create_plugin: false,
        },
    };
    static DEFER_CREATE_ENTRY: TestEntry = TestEntry {
        factory: TestFactory {
            activate_latency_changed: false,
            request_host_lifecycle: false,
            request_host_lifecycle_during_create: false,
            request_host_ports: false,
            request_host_ports_during_create: false,
            count_create_plugin: true,
        },
    };
    static REQUEST_HOST_LIFECYCLE_REGISTRATION: EntryRegistration =
        EntryRegistration::new(&REQUEST_HOST_LIFECYCLE_ENTRY);
    static REQUEST_HOST_PORTS_REGISTRATION: EntryRegistration =
        EntryRegistration::new(&REQUEST_HOST_PORTS_ENTRY);
    static REQUEST_HOST_PORTS_DURING_CREATE_REGISTRATION: EntryRegistration =
        EntryRegistration::new(&REQUEST_HOST_PORTS_DURING_CREATE_ENTRY);
    static DEFER_CREATE_REGISTRATION: EntryRegistration =
        EntryRegistration::new(&DEFER_CREATE_ENTRY);

    static LATENCY_CHANGED_COUNT: AtomicU32 = AtomicU32::new(0);
    static REQUEST_RESTART_COUNT: AtomicU32 = AtomicU32::new(0);
    static REQUEST_PROCESS_COUNT: AtomicU32 = AtomicU32::new(0);
    static REQUEST_CALLBACK_COUNT: AtomicU32 = AtomicU32::new(0);
    static AUDIO_PORTS_IS_RESCAN_FLAG_SUPPORTED_COUNT: AtomicU32 = AtomicU32::new(0);
    static AUDIO_PORTS_RESCAN_COUNT: AtomicU32 = AtomicU32::new(0);
    static NOTE_PORTS_SUPPORTED_DIALECTS_COUNT: AtomicU32 = AtomicU32::new(0);
    static NOTE_PORTS_RESCAN_COUNT: AtomicU32 = AtomicU32::new(0);
    static ON_MAIN_THREAD_COUNT: AtomicU32 = AtomicU32::new(0);
    static DESTROY_COUNT: AtomicU32 = AtomicU32::new(0);
    static CREATE_PLUGIN_COUNT: AtomicU32 = AtomicU32::new(0);

    #[test]
    fn zero_latency_exposes_latency_extension() {
        let instance = test_instance(&ZERO_LATENCY_REGISTRATION, ptr::null());
        assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });
        let extension = unsafe {
            plugin_get_extension(
                &instance.plugin as *const clap_plugin,
                CLAP_EXT_LATENCY.as_ptr(),
            )
        };
        assert!(!extension.is_null());

        let latency = unsafe { &*(extension as *const clap_plugin_latency) };
        let get = latency.get.expect("latency.get callback");
        let frames = unsafe { get(&instance.plugin as *const clap_plugin) };
        assert_eq!(frames, 0);
    }

    #[test]
    fn activate_notification_calls_host_latency_changed_during_activate() {
        LATENCY_CHANGED_COUNT.store(0, Ordering::Relaxed);
        let host_get_extension_count = AtomicU32::new(0);
        let host = test_host_with_get_extension_count(&host_get_extension_count);
        let instance = test_instance(&ACTIVATE_LATENCY_CHANGED_REGISTRATION, &host);
        assert_eq!(host_get_extension_count.load(Ordering::Relaxed), 0);

        assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });
        assert_eq!(host_get_extension_count.load(Ordering::Relaxed), 0);
        let activated =
            unsafe { plugin_activate(&instance.plugin as *const clap_plugin, 48_000.0, 1, 512) };

        assert!(activated);
        assert_eq!(host_get_extension_count.load(Ordering::Relaxed), 1);
        assert_eq!(LATENCY_CHANGED_COUNT.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn activate_forwards_host_lifecycle_requests() {
        REQUEST_RESTART_COUNT.store(0, Ordering::Relaxed);
        REQUEST_PROCESS_COUNT.store(0, Ordering::Relaxed);
        REQUEST_CALLBACK_COUNT.store(0, Ordering::Relaxed);
        let host = test_host();
        let instance = test_instance(&REQUEST_HOST_LIFECYCLE_REGISTRATION, &host);

        assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });
        let activated =
            unsafe { plugin_activate(&instance.plugin as *const clap_plugin, 48_000.0, 1, 512) };

        assert!(activated);
        assert_eq!(REQUEST_RESTART_COUNT.load(Ordering::Relaxed), 1);
        assert_eq!(REQUEST_PROCESS_COUNT.load(Ordering::Relaxed), 1);
        assert_eq!(REQUEST_CALLBACK_COUNT.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn plugin_on_main_thread_calls_instance_hook() {
        ON_MAIN_THREAD_COUNT.store(0, Ordering::Relaxed);
        let instance = test_instance(&ZERO_LATENCY_REGISTRATION, ptr::null());
        assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });

        unsafe {
            plugin_on_main_thread(&instance.plugin as *const clap_plugin);
        }

        assert_eq!(ON_MAIN_THREAD_COUNT.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn plugin_destroy_calls_instance_hook() {
        DESTROY_COUNT.store(0, Ordering::Relaxed);
        let instance = test_instance(&ZERO_LATENCY_REGISTRATION, ptr::null());
        assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });
        let plugin = &instance.plugin as *const clap_plugin;
        let _instance = Box::into_raw(instance);

        unsafe {
            plugin_destroy(plugin);
        }

        assert_eq!(DESTROY_COUNT.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn factory_create_plugin_defers_product_construction_until_plugin_init() {
        CREATE_PLUGIN_COUNT.store(0, Ordering::Relaxed);
        let instance = test_instance(&DEFER_CREATE_REGISTRATION, ptr::null());

        assert_eq!(CREATE_PLUGIN_COUNT.load(Ordering::Relaxed), 0);
        assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });
        assert_eq!(CREATE_PLUGIN_COUNT.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn activate_forwards_host_port_requests() {
        AUDIO_PORTS_IS_RESCAN_FLAG_SUPPORTED_COUNT.store(0, Ordering::Relaxed);
        AUDIO_PORTS_RESCAN_COUNT.store(0, Ordering::Relaxed);
        NOTE_PORTS_SUPPORTED_DIALECTS_COUNT.store(0, Ordering::Relaxed);
        NOTE_PORTS_RESCAN_COUNT.store(0, Ordering::Relaxed);
        let host_get_extension_count = AtomicU32::new(0);
        let host = test_host_with_get_extension_count(&host_get_extension_count);
        let instance = test_instance(&REQUEST_HOST_PORTS_REGISTRATION, &host);
        assert_eq!(host_get_extension_count.load(Ordering::Relaxed), 0);

        assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });
        assert_eq!(host_get_extension_count.load(Ordering::Relaxed), 0);
        let activated =
            unsafe { plugin_activate(&instance.plugin as *const clap_plugin, 48_000.0, 1, 512) };

        assert!(activated);
        assert_eq!(host_get_extension_count.load(Ordering::Relaxed), 2);
        assert_eq!(
            AUDIO_PORTS_IS_RESCAN_FLAG_SUPPORTED_COUNT.load(Ordering::Relaxed),
            1
        );
        assert_eq!(AUDIO_PORTS_RESCAN_COUNT.load(Ordering::Relaxed), 1);
        assert_eq!(
            NOTE_PORTS_SUPPORTED_DIALECTS_COUNT.load(Ordering::Relaxed),
            1
        );
        assert_eq!(NOTE_PORTS_RESCAN_COUNT.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn product_construction_keeps_host_extension_proxies_inert() {
        AUDIO_PORTS_IS_RESCAN_FLAG_SUPPORTED_COUNT.store(0, Ordering::Relaxed);
        AUDIO_PORTS_RESCAN_COUNT.store(0, Ordering::Relaxed);
        NOTE_PORTS_SUPPORTED_DIALECTS_COUNT.store(0, Ordering::Relaxed);
        NOTE_PORTS_RESCAN_COUNT.store(0, Ordering::Relaxed);
        let host_counts = HostCallbackCounts::default();
        let host = test_host_with_callback_counts(&host_counts);
        let instance = test_instance(&REQUEST_HOST_PORTS_DURING_CREATE_REGISTRATION, &host);

        assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });

        assert_eq!(host_counts.request_restart.load(Ordering::Relaxed), 0);
        assert_eq!(host_counts.request_process.load(Ordering::Relaxed), 0);
        assert_eq!(host_counts.request_callback.load(Ordering::Relaxed), 0);
        assert_eq!(host_counts.get_extension.load(Ordering::Relaxed), 0);
        assert_eq!(
            AUDIO_PORTS_IS_RESCAN_FLAG_SUPPORTED_COUNT.load(Ordering::Relaxed),
            0
        );
        assert_eq!(AUDIO_PORTS_RESCAN_COUNT.load(Ordering::Relaxed), 0);
        assert_eq!(
            NOTE_PORTS_SUPPORTED_DIALECTS_COUNT.load(Ordering::Relaxed),
            0
        );
        assert_eq!(NOTE_PORTS_RESCAN_COUNT.load(Ordering::Relaxed), 0);
    }

    fn test_instance(
        registration: &'static EntryRegistration,
        host: *const clap_host,
    ) -> Box<PluginInstanceState> {
        let mut instance = PluginInstanceState::new(
            registration,
            0,
            TEST_DESCRIPTOR.id,
            host,
            None,
            HostContext::detect_current(None),
        )
        .expect("test plugin instance");
        let instance_ptr = (&mut *instance) as *mut PluginInstanceState;
        instance.plugin.plugin_data = instance_ptr.cast();
        instance
    }

    fn test_host() -> clap_host {
        test_host_with_data(ptr::null_mut())
    }

    fn test_host_with_get_extension_count(count: &AtomicU32) -> clap_host {
        test_host_with_data((count as *const AtomicU32).cast_mut().cast())
    }

    #[derive(Default)]
    struct HostCallbackCounts {
        get_extension: AtomicU32,
        request_restart: AtomicU32,
        request_process: AtomicU32,
        request_callback: AtomicU32,
    }

    fn test_host_with_callback_counts(counts: &HostCallbackCounts) -> clap_host {
        clap_host {
            clap_version: CLAP_VERSION,
            host_data: (counts as *const HostCallbackCounts).cast_mut().cast(),
            name: c"Test Host".as_ptr(),
            vendor: c"Test Vendor".as_ptr(),
            url: c"https://example.invalid".as_ptr(),
            version: c"0.0.0".as_ptr(),
            get_extension: Some(test_host_get_extension_with_callback_counts),
            request_restart: Some(test_host_request_restart_with_callback_counts),
            request_process: Some(test_host_request_process_with_callback_counts),
            request_callback: Some(test_host_request_callback_with_callback_counts),
        }
    }

    fn test_host_with_data(host_data: *mut std::ffi::c_void) -> clap_host {
        clap_host {
            clap_version: CLAP_VERSION,
            host_data,
            name: c"Test Host".as_ptr(),
            vendor: c"Test Vendor".as_ptr(),
            url: c"https://example.invalid".as_ptr(),
            version: c"0.0.0".as_ptr(),
            get_extension: Some(test_host_get_extension),
            request_restart: Some(test_host_request_restart),
            request_process: Some(test_host_request_process),
            request_callback: Some(test_host_request_callback),
        }
    }

    unsafe extern "C" fn test_host_get_extension(
        host: *const clap_host,
        extension_id: *const std::ffi::c_char,
    ) -> *const std::ffi::c_void {
        if extension_id.is_null() {
            return ptr::null();
        }
        if let Some(count) = unsafe {
            host.as_ref()
                .and_then(|host| host.host_data.cast::<AtomicU32>().as_ref())
        } {
            count.fetch_add(1, Ordering::Relaxed);
        }
        let id = unsafe { std::ffi::CStr::from_ptr(extension_id) };
        if id == CLAP_EXT_LATENCY {
            (&TEST_HOST_LATENCY as *const clap_host_latency).cast()
        } else if id == CLAP_EXT_AUDIO_PORTS {
            (&TEST_HOST_AUDIO_PORTS as *const clap_host_audio_ports).cast()
        } else if id == CLAP_EXT_NOTE_PORTS {
            (&TEST_HOST_NOTE_PORTS as *const clap_host_note_ports).cast()
        } else {
            ptr::null()
        }
    }

    unsafe extern "C" fn test_host_get_extension_with_callback_counts(
        host: *const clap_host,
        extension_id: *const std::ffi::c_char,
    ) -> *const std::ffi::c_void {
        if let Some(counts) = unsafe {
            host.as_ref()
                .and_then(|host| host.host_data.cast::<HostCallbackCounts>().as_ref())
        } {
            counts.get_extension.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { test_host_get_extension(ptr::null(), extension_id) }
    }

    unsafe extern "C" fn test_host_latency_changed(_host: *const clap_host) {
        LATENCY_CHANGED_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    unsafe extern "C" fn test_host_request_restart(_host: *const clap_host) {
        REQUEST_RESTART_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    unsafe extern "C" fn test_host_request_process(_host: *const clap_host) {
        REQUEST_PROCESS_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    unsafe extern "C" fn test_host_request_callback(_host: *const clap_host) {
        REQUEST_CALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    unsafe extern "C" fn test_host_request_restart_with_callback_counts(host: *const clap_host) {
        if let Some(counts) = unsafe {
            host.as_ref()
                .and_then(|host| host.host_data.cast::<HostCallbackCounts>().as_ref())
        } {
            counts.request_restart.fetch_add(1, Ordering::Relaxed);
        }
    }

    unsafe extern "C" fn test_host_request_process_with_callback_counts(host: *const clap_host) {
        if let Some(counts) = unsafe {
            host.as_ref()
                .and_then(|host| host.host_data.cast::<HostCallbackCounts>().as_ref())
        } {
            counts.request_process.fetch_add(1, Ordering::Relaxed);
        }
    }

    unsafe extern "C" fn test_host_request_callback_with_callback_counts(host: *const clap_host) {
        if let Some(counts) = unsafe {
            host.as_ref()
                .and_then(|host| host.host_data.cast::<HostCallbackCounts>().as_ref())
        } {
            counts.request_callback.fetch_add(1, Ordering::Relaxed);
        }
    }

    static TEST_HOST_LATENCY: clap_host_latency = clap_host_latency {
        changed: Some(test_host_latency_changed),
    };

    unsafe extern "C" fn test_host_audio_ports_is_rescan_flag_supported(
        _host: *const clap_host,
        flag: u32,
    ) -> bool {
        AUDIO_PORTS_IS_RESCAN_FLAG_SUPPORTED_COUNT.fetch_add(1, Ordering::Relaxed);
        flag == CLAP_AUDIO_PORTS_RESCAN_NAMES
    }

    unsafe extern "C" fn test_host_audio_ports_rescan(_host: *const clap_host, _flags: u32) {
        AUDIO_PORTS_RESCAN_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    static TEST_HOST_AUDIO_PORTS: clap_host_audio_ports = clap_host_audio_ports {
        is_rescan_flag_supported: Some(test_host_audio_ports_is_rescan_flag_supported),
        rescan: Some(test_host_audio_ports_rescan),
    };

    unsafe extern "C" fn test_host_note_ports_supported_dialects(_host: *const clap_host) -> u32 {
        NOTE_PORTS_SUPPORTED_DIALECTS_COUNT.fetch_add(1, Ordering::Relaxed);
        CLAP_NOTE_DIALECT_CLAP
    }

    unsafe extern "C" fn test_host_note_ports_rescan(_host: *const clap_host, _flags: u32) {
        NOTE_PORTS_RESCAN_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    static TEST_HOST_NOTE_PORTS: clap_host_note_ports = clap_host_note_ports {
        supported_dialects: Some(test_host_note_ports_supported_dialects),
        rescan: Some(test_host_note_ports_rescan),
    };

    static TEST_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
        id: "dev.wrac.test",
        name: "WRAC Test",
        vendor: "WRAC",
        url: "https://example.invalid",
        manual_url: "",
        support_url: "",
        version: "0.0.0",
        description: "",
        features: &[],
        auv2: None,
        vst3: None,
        aax: None,
    };

    struct TestEntry {
        factory: TestFactory,
    }

    impl PluginEntry for TestEntry {
        fn init(&self, _context: EntryContext<'_>) -> PluginResult<()> {
            Ok(())
        }

        fn plugin_factory(&self) -> Option<&dyn PluginFactory> {
            Some(&self.factory)
        }
    }

    #[derive(Clone, Copy)]
    struct TestFactory {
        activate_latency_changed: bool,
        request_host_lifecycle: bool,
        request_host_lifecycle_during_create: bool,
        request_host_ports: bool,
        request_host_ports_during_create: bool,
        count_create_plugin: bool,
    }

    impl PluginFactory for TestFactory {
        fn plugin_count(&self) -> u32 {
            1
        }

        fn plugin_descriptor(&self, index: u32) -> Option<PluginDescriptor> {
            (index == 0).then_some(TEST_DESCRIPTOR)
        }

        fn create_plugin(
            &self,
            plugin_id: &str,
            context: PluginInstanceContext,
        ) -> Option<Box<dyn PluginInstance>> {
            if self.count_create_plugin {
                CREATE_PLUGIN_COUNT.fetch_add(1, Ordering::Relaxed);
            }
            if self.request_host_lifecycle_during_create {
                context.host_lifecycle.request_restart();
                context.host_lifecycle.request_process();
                context.host_lifecycle.request_callback();
            }
            if self.request_host_ports_during_create {
                assert!(
                    !context
                        .host_audio_ports
                        .is_rescan_flag_supported(CLAP_AUDIO_PORTS_RESCAN_NAMES)
                );
                context
                    .host_audio_ports
                    .rescan(CLAP_AUDIO_PORTS_RESCAN_NAMES);
                assert_eq!(
                    context.host_note_ports.supported_dialects(),
                    NoteDialects::default()
                );
                context.host_note_ports.rescan(CLAP_NOTE_PORTS_RESCAN_NAMES);
            }
            (plugin_id == TEST_DESCRIPTOR.id).then(|| {
                Box::new(TestPlugin {
                    activate_latency_changed: self.activate_latency_changed,
                    request_host_lifecycle: self.request_host_lifecycle,
                    request_host_ports: self.request_host_ports,
                    host_lifecycle: context.host_lifecycle,
                    host_audio_ports: context.host_audio_ports,
                    host_note_ports: context.host_note_ports,
                }) as Box<dyn PluginInstance>
            })
        }
    }

    struct TestPlugin {
        activate_latency_changed: bool,
        request_host_lifecycle: bool,
        request_host_ports: bool,
        host_lifecycle: Arc<dyn HostLifecycle>,
        host_audio_ports: Arc<dyn HostAudioPorts>,
        host_note_ports: Arc<dyn HostNotePorts>,
    }

    impl PluginInstance for TestPlugin {
        fn initialize_processor(&mut self) -> PluginResult<Box<dyn InactiveProcessor>> {
            Ok(Box::new(TestInactiveProcessor))
        }

        fn activate(
            &mut self,
            _context: ActivateContext,
            _processor: Box<dyn InactiveProcessor>,
        ) -> PluginResult<ActivateResult> {
            if self.request_host_lifecycle {
                self.host_lifecycle.request_restart();
                self.host_lifecycle.request_process();
                self.host_lifecycle.request_callback();
            }
            if self.request_host_ports {
                assert!(
                    self.host_audio_ports
                        .is_rescan_flag_supported(CLAP_AUDIO_PORTS_RESCAN_NAMES)
                );
                self.host_audio_ports.rescan(CLAP_AUDIO_PORTS_RESCAN_NAMES);
                assert_eq!(
                    self.host_note_ports.supported_dialects(),
                    NoteDialects::CLAP
                );
                self.host_note_ports.rescan(CLAP_NOTE_PORTS_RESCAN_NAMES);
            }
            Ok(ActivateResult {
                processor: Box::new(TestActiveProcessor),
                notifications: ActivateNotifications {
                    latency_changed: self.activate_latency_changed,
                },
            })
        }

        fn deactivate(
            &mut self,
            _processor: Box<dyn ActiveProcessor>,
        ) -> PluginResult<Box<dyn InactiveProcessor>> {
            Ok(Box::new(TestInactiveProcessor))
        }

        fn destroy(&mut self) {
            DESTROY_COUNT.fetch_add(1, Ordering::Relaxed);
        }

        fn on_main_thread(&mut self) {
            ON_MAIN_THREAD_COUNT.fetch_add(1, Ordering::Relaxed);
        }

        fn params(&self) -> Arc<dyn PluginParamsQuery> {
            Arc::new(TestParams)
        }

        fn latency(&self) -> Option<Arc<dyn PluginLatencyExtension>> {
            Some(Arc::new(TestLatency))
        }
    }

    struct TestLatency;

    impl PluginLatencyExtension for TestLatency {
        fn latency_frames(&self) -> u32 {
            0
        }
    }

    struct TestParams;

    impl PluginParamsQuery for TestParams {
        fn count(&self) -> u32 {
            0
        }

        fn get_info(&self, _index: u32) -> Option<crate::ParamInfo> {
            None
        }

        fn get_value(&self, _param_id: u32) -> PluginResult<f64> {
            Err(crate::PluginError::InvalidParameter)
        }

        fn value_to_text(&self, _param_id: u32, _value: f64) -> PluginResult<String> {
            Err(crate::PluginError::InvalidParameter)
        }

        fn text_to_value(&self, _param_id: u32, _text: &str) -> PluginResult<f64> {
            Err(crate::PluginError::InvalidParameter)
        }
    }

    struct TestInactiveProcessor;

    impl InactiveProcessor for TestInactiveProcessor {
        fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
            self
        }

        fn flush_params(&mut self, _context: ParamFlushContext<'_>) -> PluginResult<()> {
            Ok(())
        }
    }

    struct TestActiveProcessor;

    impl ActiveProcessor for TestActiveProcessor {
        fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
            self
        }

        fn process(&mut self, _context: ProcessContext<'_>) -> PluginResult<ProcessStatus> {
            Ok(ProcessStatus::Continue)
        }

        fn flush_params(&mut self, _context: ParamFlushContext<'_>) -> PluginResult<()> {
            Ok(())
        }
    }
}
