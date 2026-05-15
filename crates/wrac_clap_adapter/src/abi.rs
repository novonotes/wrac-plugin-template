//! CLAP ABI と `PluginCore` instance を結びつける module。
//!
//! public API は `lib.rs` の re-export と `export_clap_plugin!` に集約し、この module
//! は C ABI callback と adapter state の所有だけを扱う。

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
use clap_sys::ext::note_ports::CLAP_EXT_NOTE_PORTS;
use clap_sys::ext::params::CLAP_EXT_PARAMS;
use clap_sys::ext::state::CLAP_EXT_STATE;
use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{
    CLAP_PROCESS_CONTINUE, CLAP_PROCESS_CONTINUE_IF_NOT_QUIET, CLAP_PROCESS_ERROR,
    CLAP_PROCESS_SLEEP, CLAP_PROCESS_TAIL, clap_process, clap_process_status,
};
use clap_sys::version::clap_version_is_compatible;
use parking_lot::{Mutex, RwLock};

mod audio_buffers;
mod audio_ports;
mod configurable_audio_ports;
mod ffi;
mod gui_extension;
mod note_ports;
mod params_extension;
mod state_extension;

use self::audio_buffers::audio_buffers;
use self::ffi::{ffi_bool, ffi_ptr, ffi_status, ffi_unit, four_char_code};
use crate::descriptor::{
    Auv2FactoryState, ClapPluginFactoryAsAuv2, ClapPluginInfoAsAuv2, PluginRegistration,
    auv2_factory_ptr, auv2_factory_state, clap_factory_state, factory_ptr,
};
use crate::host_gui::HostGuiResizeRequest;
use crate::params::ParameterEditQueue;
use crate::{
    ActivateContext, PluginAudioPorts, PluginConfigurableAudioPorts, PluginCore, PluginCoreContext,
    PluginGui, PluginNotePorts, PluginParameters, PluginStateSupport, ProcessContext,
    ProcessStatus, Processor,
};

// clap-wrapper は AUv2 metadata 生成時にこの draft factory を読む。CLAP descriptor とは
// 別に AU manufacturer/subtype を渡さないと、generic wrapper identity と衝突して
// auval が古い別 plugin を検証することがある。
const CLAP_PLUGIN_FACTORY_INFO_AUV2: &CStr = c"clap.plugin-factory-info-as-auv2.draft0";

/// CLAP instance と Rust core の同期境界。
///
/// 設計の要: 「lifecycle lock」と「host-facing callback が直接読む capability」を
/// 分ける。`core` lock は processor 所有権を動かす `activate`/`deactivate` だけで
/// 使い、parameter/state/port query は instance 作成時に固定した `Arc` を読む。
/// 分けないと、wrapper が `activate()` 中に query を再入させたとき core lock を
/// 取れず「parameter なし」「state 保存失敗」を host に返してしまう (crash は
/// しないが project data や routing を壊す)。
pub(crate) struct PluginInstance {
    plugin: clap_plugin,
    // processor lifecycle の所有者。lock を取るのは activate/deactivate だけ。
    core: RwLock<Box<dyn PluginCore>>,
    // capability の有無は instance 作成時に固定する。runtime state と連動させると
    // 「query した瞬間だけ extension が消える」不安定な見え方になるため。
    capabilities: PluginCapabilities,
    audio_ports: Option<Arc<dyn PluginAudioPorts>>,
    configurable_audio_ports: Option<Arc<dyn PluginConfigurableAudioPorts>>,
    note_ports: Option<Arc<dyn PluginNotePorts>>,
    parameters: Option<Arc<dyn PluginParameters>>,
    state: Option<Arc<dyn PluginStateSupport>>,
    gui: Option<Arc<dyn PluginGui>>,
    // GUI mutation callback の再入 guard。再入時は待たず失敗させ deadlock を避ける
    // (GUI query callback はこの guard を通らない)。
    gui_callback_busy: Mutex<()>,
    parameter_edits: Arc<ParameterEditQueue>,
    // wrapper が thread/lifecycle 注釈を崩しても soundness を保つため、RT 経路は
    // lock せず atomic guard が取れた callback だけ `Processor` への `&mut` を作る。
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
}

// 安全性: CLAP は callback 間で同じ opaque plugin pointer を共有する。adapter state は
// lock/atomic 経由で共有し、host の thread annotation や callback 順序が崩れても Rust
// aliasing だけは破らない。
unsafe impl Send for PluginInstance {}
unsafe impl Sync for PluginInstance {}

impl PluginInstance {
    fn new(registration: &'static PluginRegistration, host: *const clap_host) -> Box<Self> {
        let parameter_edits = Arc::new(ParameterEditQueue::new(host));
        // notifier は safe proxy として渡す。製品 GUI は host pointer や CLAP event
        // lifetime を知らずにこれを保持できる。
        let context = PluginCoreContext {
            host_parameter_edit_notifier: parameter_edits.clone(),
            host_gui_resize_requester: Arc::new(HostGuiResizeRequest::new(host)),
        };
        let core = (registration.create)(context);
        // capability は callback 開始前のここで固定する (get_extension 中に core
        // lock を待つと host の再入順に依存するため)。得た Arc は入口にすぎず、
        // 値の SoT は plugin 実装側の store に残る。
        let audio_ports = core.audio_ports();
        let configurable_audio_ports = core.configurable_audio_ports();
        let note_ports = core.note_ports();
        let parameters = core.parameters();
        let state = core.state();
        let gui = core.gui();
        let capabilities = PluginCapabilities {
            audio_ports: audio_ports.is_some(),
            configurable_audio_ports: configurable_audio_ports.is_some(),
            note_ports: note_ports.is_some(),
            parameters: parameters.is_some(),
            state: state.is_some(),
            gui: gui.is_some(),
        };
        let storage = registration.storage();

        Box::new(Self {
            plugin: clap_plugin {
                desc: storage.descriptor.clap_descriptor(),
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
            core: RwLock::new(core),
            capabilities,
            audio_ports,
            configurable_audio_ports,
            note_ports,
            parameters,
            state,
            gui,
            gui_callback_busy: Mutex::new(()),
            parameter_edits,
            processor: UnsafeCell::new(None),
            processor_busy: AtomicBool::new(false),
            lifecycle_busy: AtomicBool::new(false),
        })
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
            // activate は非 RT。Processor の有無を別状態へ複製せず、実体の borrow guard が
            // 空くまで待ってから格納する。
            std::thread::yield_now();
        }
    }

    fn take_processor_blocking(&self) -> Option<Box<dyn Processor>> {
        loop {
            if let Some(processor) = self.try_take_processor() {
                return processor;
            }
            // deactivate/destroy は非 RT の lifecycle callback です。ここで待つことで、
            // lifecycle と audio を競合させる wrapper でも `process()` が一時的な
            // Processor borrow を持ったまま instance を解放しないようにする。
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
            // `destroy()` は待てる callback です。待たずに解放すると、順序の崩れた
            // wrapper lifecycle callback が adapter state を保持したままになる。
            std::thread::yield_now();
        }
    }
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
pub unsafe extern "C" fn entry_init(
    _registration: &'static PluginRegistration,
    _plugin_path: *const c_char,
) -> bool {
    true
}

/// # Safety
///
/// The registration must be the same static registration previously passed to
/// `entry_init` for this binary.
pub unsafe extern "C" fn entry_deinit(_registration: &'static PluginRegistration) {}

/// # Safety
///
/// `factory_id` must be null or point to a valid NUL-terminated CLAP factory id.
/// The returned pointer is owned by the static plugin registration storage.
pub unsafe extern "C" fn entry_get_factory(
    registration: &'static PluginRegistration,
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
        } else if factory_id == CLAP_PLUGIN_FACTORY_INFO_AUV2
            && registration.descriptor.auv2.is_some()
        {
            auv2_factory_ptr(storage)
        } else {
            ptr::null()
        }
    })
}

pub(crate) unsafe extern "C" fn auv2_get_info(
    factory: *const ClapPluginFactoryAsAuv2,
    index: u32,
    info: *mut ClapPluginInfoAsAuv2,
) -> bool {
    ffi_bool(|| {
        if index != 0 || info.is_null() {
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
        let Some(auv2) = registration.descriptor.auv2 else {
            log::warn!("auv2.get_info: registration has no AUv2 descriptor");
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
    _factory: *const clap_plugin_factory,
) -> u32 {
    1
}

pub(crate) unsafe extern "C" fn factory_get_plugin_descriptor(
    factory: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    if index != 0 {
        log::warn!("factory.get_plugin_descriptor: invalid index={index}");
        return ptr::null();
    }

    let Some(state) = clap_factory_state(factory) else {
        log::warn!("factory.get_plugin_descriptor: invalid factory pointer");
        return ptr::null();
    };
    state.registration.storage().descriptor.clap_descriptor()
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
        if unsafe { CStr::from_ptr(plugin_id) }.to_bytes() != registration.descriptor.id.as_bytes()
        {
            log::warn!("factory.create_plugin: requested unknown plugin id");
            return ptr::null();
        }

        let mut instance = PluginInstance::new(registration, host);
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
            if let Err(error) = instance.core.write().deactivate(processor) {
                log::warn!("plugin.destroy: plugin deactivate failed: {error}");
            }
        }

        drop(guard);
        let data = unsafe { (*plugin).plugin_data } as *mut PluginInstance;
        unsafe {
            drop(Box::from_raw(data));
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

        let processor = match instance.core.write().activate(ActivateContext {
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
        // deactivate は host へ完了を返す前に Processor を必ず回収したい cleanup callback。
        // wrapper が lifecycle callback を並行させても、ここでは待って破棄漏れを避ける。
        let _guard = instance.enter_lifecycle_blocking();
        if let Some(processor) = instance.take_processor_blocking() {
            if let Err(error) = instance.core.write().deactivate(processor) {
                log::warn!("plugin.deactivate: plugin deactivate failed: {error}");
            }
        }
    });
}

unsafe extern "C" fn plugin_start_processing(plugin: *const clap_plugin) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("plugin.start_processing: missing plugin instance");
            return false;
        };
        // `start_processing` / `stop_processing` は wrapper format では VST3/AU 側の
        // activate と同期しないことがある。専用 flag は host 都合で audio を止める
        // 故障点になるため、処理可否は Processor の有無だけで判断する。
        let can_process = instance.has_processor_or_busy();
        if !can_process {
            log::warn!("plugin.start_processing: no processor is available");
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
            log::warn!("plugin.reset: missing plugin instance");
            return;
        };
        let Some(()) = instance.with_processor_mut(|processor| {
            if let Some(processor) = processor {
                processor.reset();
            } else {
                log::debug!("plugin.reset: no processor is available");
            }
        }) else {
            log::warn!("plugin.reset: processor is busy");
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
            log::error!("plugin.process: missing plugin instance");
            return CLAP_PROCESS_ERROR;
        };

        if process.is_null() {
            log::warn!("plugin.process: null process pointer");
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
                log::error!("plugin.process: invalid audio buffers: {error}");
                return CLAP_PROCESS_ERROR;
            }
        };

        // audio callback は `PluginCore` の lock を取らない。処理可能かどうかも別 flag ではなく
        // 実際の `Processor` の有無だけで決める。wrapper が lifecycle 順序を崩した場合でも、
        // RT 経路では待たずに sleep/error へ倒す。
        let Some(result) = instance.with_processor_mut(|processor| {
            let Some(processor) = processor else {
                log::debug!("plugin.process: no processor is available");
                return CLAP_PROCESS_SLEEP;
            };

            match processor.process(ProcessContext {
                frames_count: process.frames_count,
                audio,
                events,
            }) {
                Ok(ProcessStatus::Continue) => CLAP_PROCESS_CONTINUE,
                Ok(ProcessStatus::ContinueIfNotQuiet) => CLAP_PROCESS_CONTINUE_IF_NOT_QUIET,
                Ok(ProcessStatus::Tail) => CLAP_PROCESS_TAIL,
                Ok(ProcessStatus::Sleep) => CLAP_PROCESS_SLEEP,
                Err(error) => {
                    log::error!("plugin.process: processor failed: {error}");
                    CLAP_PROCESS_ERROR
                }
            }
        }) else {
            log::warn!("plugin.process: processor is busy");
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
            log::warn!("plugin.get_extension: null extension id");
            return ptr::null();
        }
        let id = unsafe { CStr::from_ptr(id) };
        let Some(instance) = (unsafe { PluginInstance::from_plugin(_plugin) }) else {
            log::warn!("plugin.get_extension: missing plugin instance");
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
        } else {
            ptr::null()
        }
    })
}

unsafe extern "C" fn plugin_on_main_thread(_plugin: *const clap_plugin) {}
