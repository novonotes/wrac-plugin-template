use std::any::Any;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use clap_sys::ext::audio_ports::{
    CLAP_AUDIO_PORTS_RESCAN_NAMES, CLAP_EXT_AUDIO_PORTS, clap_host_audio_ports,
};
use clap_sys::ext::latency::{CLAP_EXT_LATENCY, clap_host_latency, clap_plugin_latency};
use clap_sys::ext::note_ports::{
    CLAP_EXT_NOTE_PORTS, CLAP_NOTE_DIALECT_CLAP, CLAP_NOTE_PORTS_RESCAN_NAMES, clap_host_note_ports,
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
    HostAudioPorts, HostLifecycle, HostNotePorts, InactiveProcessor, LogConfig, NoteDialects,
    ParamFlushContext, PluginDescriptor, PluginEntry, PluginFactory, PluginInstance,
    PluginInstanceContext, PluginLatencyExtension, PluginParamsQuery, PluginResult, ProcessContext,
    ProcessStatus,
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
static ZERO_LATENCY_REGISTRATION: EntryRegistration = EntryRegistration::new(&ZERO_LATENCY_ENTRY);

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
static DEFER_CREATE_REGISTRATION: EntryRegistration = EntryRegistration::new(&DEFER_CREATE_ENTRY);

static LATENCY_CHANGED_COUNT: AtomicU32 = AtomicU32::new(0);
static REQUEST_RESTART_COUNT: AtomicU32 = AtomicU32::new(0);
static REQUEST_PROCESS_COUNT: AtomicU32 = AtomicU32::new(0);
static REQUEST_CALLBACK_COUNT: AtomicU32 = AtomicU32::new(0);
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
    let host_counts = HostCallbackCounts::default();
    let host = test_host_with_callback_counts(&host_counts);
    let instance = test_instance(&REQUEST_HOST_PORTS_REGISTRATION, &host);
    assert_eq!(host_counts.get_extension.load(Ordering::Relaxed), 0);

    assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });
    assert_eq!(host_counts.get_extension.load(Ordering::Relaxed), 0);
    let activated =
        unsafe { plugin_activate(&instance.plugin as *const clap_plugin, 48_000.0, 1, 512) };

    assert!(activated);
    assert_eq!(host_counts.get_extension.load(Ordering::Relaxed), 2);
    assert_eq!(
        host_counts
            .audio_ports_is_rescan_flag_supported
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(host_counts.audio_ports_rescan.load(Ordering::Relaxed), 1);
    assert_eq!(
        host_counts
            .note_ports_supported_dialects
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(host_counts.note_ports_rescan.load(Ordering::Relaxed), 1);
}

#[test]
fn product_construction_keeps_host_extension_proxies_inert() {
    let host_counts = HostCallbackCounts::default();
    let host = test_host_with_callback_counts(&host_counts);
    let instance = test_instance(&REQUEST_HOST_PORTS_DURING_CREATE_REGISTRATION, &host);

    assert!(unsafe { plugin_init(&instance.plugin as *const clap_plugin) });

    assert_eq!(host_counts.request_restart.load(Ordering::Relaxed), 0);
    assert_eq!(host_counts.request_process.load(Ordering::Relaxed), 0);
    assert_eq!(host_counts.request_callback.load(Ordering::Relaxed), 0);
    assert_eq!(host_counts.get_extension.load(Ordering::Relaxed), 0);
    assert_eq!(
        host_counts
            .audio_ports_is_rescan_flag_supported
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(host_counts.audio_ports_rescan.load(Ordering::Relaxed), 0);
    assert_eq!(
        host_counts
            .note_ports_supported_dialects
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(host_counts.note_ports_rescan.load(Ordering::Relaxed), 0);
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
    audio_ports_is_rescan_flag_supported: AtomicU32,
    audio_ports_rescan: AtomicU32,
    note_ports_supported_dialects: AtomicU32,
    note_ports_rescan: AtomicU32,
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
    if extension_id.is_null() {
        return ptr::null();
    }

    let id = unsafe { std::ffi::CStr::from_ptr(extension_id) };
    if id == CLAP_EXT_LATENCY {
        (&TEST_HOST_LATENCY as *const clap_host_latency).cast()
    } else if id == CLAP_EXT_AUDIO_PORTS {
        (&COUNTED_TEST_HOST_AUDIO_PORTS as *const clap_host_audio_ports).cast()
    } else if id == CLAP_EXT_NOTE_PORTS {
        (&COUNTED_TEST_HOST_NOTE_PORTS as *const clap_host_note_ports).cast()
    } else {
        ptr::null()
    }
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
    flag == CLAP_AUDIO_PORTS_RESCAN_NAMES
}

unsafe extern "C" fn test_host_audio_ports_rescan(_host: *const clap_host, _flags: u32) {}

static TEST_HOST_AUDIO_PORTS: clap_host_audio_ports = clap_host_audio_ports {
    is_rescan_flag_supported: Some(test_host_audio_ports_is_rescan_flag_supported),
    rescan: Some(test_host_audio_ports_rescan),
};

unsafe extern "C" fn counted_test_host_audio_ports_is_rescan_flag_supported(
    host: *const clap_host,
    flag: u32,
) -> bool {
    if let Some(counts) = unsafe {
        host.as_ref()
            .and_then(|host| host.host_data.cast::<HostCallbackCounts>().as_ref())
    } {
        counts
            .audio_ports_is_rescan_flag_supported
            .fetch_add(1, Ordering::Relaxed);
    }
    flag == CLAP_AUDIO_PORTS_RESCAN_NAMES
}

unsafe extern "C" fn counted_test_host_audio_ports_rescan(host: *const clap_host, _flags: u32) {
    if let Some(counts) = unsafe {
        host.as_ref()
            .and_then(|host| host.host_data.cast::<HostCallbackCounts>().as_ref())
    } {
        counts.audio_ports_rescan.fetch_add(1, Ordering::Relaxed);
    }
}

static COUNTED_TEST_HOST_AUDIO_PORTS: clap_host_audio_ports = clap_host_audio_ports {
    is_rescan_flag_supported: Some(counted_test_host_audio_ports_is_rescan_flag_supported),
    rescan: Some(counted_test_host_audio_ports_rescan),
};

unsafe extern "C" fn test_host_note_ports_supported_dialects(_host: *const clap_host) -> u32 {
    CLAP_NOTE_DIALECT_CLAP
}

unsafe extern "C" fn test_host_note_ports_rescan(_host: *const clap_host, _flags: u32) {}

static TEST_HOST_NOTE_PORTS: clap_host_note_ports = clap_host_note_ports {
    supported_dialects: Some(test_host_note_ports_supported_dialects),
    rescan: Some(test_host_note_ports_rescan),
};

unsafe extern "C" fn counted_test_host_note_ports_supported_dialects(
    host: *const clap_host,
) -> u32 {
    if let Some(counts) = unsafe {
        host.as_ref()
            .and_then(|host| host.host_data.cast::<HostCallbackCounts>().as_ref())
    } {
        counts
            .note_ports_supported_dialects
            .fetch_add(1, Ordering::Relaxed);
    }
    CLAP_NOTE_DIALECT_CLAP
}

unsafe extern "C" fn counted_test_host_note_ports_rescan(host: *const clap_host, _flags: u32) {
    if let Some(counts) = unsafe {
        host.as_ref()
            .and_then(|host| host.host_data.cast::<HostCallbackCounts>().as_ref())
    } {
        counts.note_ports_rescan.fetch_add(1, Ordering::Relaxed);
    }
}

static COUNTED_TEST_HOST_NOTE_PORTS: clap_host_note_ports = clap_host_note_ports {
    supported_dialects: Some(counted_test_host_note_ports_supported_dialects),
    rescan: Some(counted_test_host_note_ports_rescan),
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
    fn log_config(&'static self) -> Option<&'static LogConfig> {
        None
    }

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
