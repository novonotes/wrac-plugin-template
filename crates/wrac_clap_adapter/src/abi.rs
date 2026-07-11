//! Module that binds the CLAP ABI to `PluginInstance` instances.
//!
//! The public API is surfaced through re-exports in `lib.rs` and `export_clap_entry!`.
//! This module is responsible only for C ABI callbacks and owning the adapter state.

use std::cell::UnsafeCell;
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::ThreadId;

use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use parking_lot::Mutex;
use wrac_host_context::HostContext;

mod audio_buffers;
mod audio_ports;
mod configurable_audio_ports;
mod entry_callbacks;
mod ffi;
mod gui_extension;
mod latency_extension;
mod note_ports;
mod params_extension;
mod plugin_callbacks;
mod render_extension;
mod state_extension;
mod tail_extension;
mod vst3_extension;

pub(crate) use self::entry_callbacks::{
    aax_get_info, auv2_get_info, entry_deinit, entry_get_factory, entry_init,
    factory_create_plugin, factory_get_plugin_count, factory_get_plugin_descriptor,
    main_thread_hook_attach, main_thread_hook_detach, vst3_get_info,
};
use self::plugin_callbacks::{
    plugin_activate, plugin_deactivate, plugin_destroy, plugin_get_extension, plugin_init,
    plugin_on_main_thread, plugin_process, plugin_reset, plugin_start_processing,
    plugin_stop_processing,
};
use crate::entry::EntryRegistration;
use crate::host_audio_ports::HostAudioPortsProxy;
use crate::host_gui::HostGuiProxy;
use crate::host_latency::HostLatencyProxy;
use crate::host_lifecycle::HostLifecycleProxy;
use crate::host_note_ports::HostNotePortsProxy;
use crate::host_state::HostStateProxy;
use crate::host_tail::HostTailFactory;
use crate::interface::{
    ActiveProcessor, InactiveProcessor, PluginAudioPortsExtension,
    PluginConfigurableAudioPortsExtension, PluginGuiExtension, PluginInstance,
    PluginLatencyExtension, PluginNotePortsExtension, PluginParamsQuery, PluginRenderExtension,
    PluginStateExtension, PluginTailExtension,
};
use crate::params::HostParamsProxy;

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

#[cfg(test)]
mod tests;
