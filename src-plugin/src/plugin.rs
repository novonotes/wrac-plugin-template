use std::ffi::CStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use atomic_float::AtomicF32;
use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::gui::PluginGui;
use clack_extensions::params::PluginParams;
use clack_extensions::state::PluginState;
use clack_plugin::factory::plugin::PluginFactoryImpl;
use clack_plugin::host::HostInfo;
use clack_plugin::plugin::PluginInstance;
use clack_plugin::prelude::*;
use novonotes_run_loop::{RunLoop, RunLoopSender};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use wxp::{Channel, WebContext, WebViewRef, WxpCommandHandler, dpi::LogicalSize};
use wxp_clack::dpi::DpiConverter;

use crate::audio::WxpExampleGainAudioProcessor;

// --- CLAP plugin metadata ---
// PLUGIN_ID must be globally unique (reverse-domain format is the convention).
pub(crate) const PLUGIN_ID: &str = "com.novo-notes.wxp-example-gain";
pub(crate) const PLUGIN_NAME: &str = "WXP Example Gain";
/// Unique ID for the parameter. The host uses this ID to identify and persist parameters,
/// so it must never be changed once published.
pub(crate) const PARAM_GAIN_ID: ClapId = ClapId::new(1);
/// Default gain value. 1.0 = 0 dB (unity gain).
pub(crate) const DEFAULT_GAIN: f32 = 1.0;
pub(crate) const MIN_GAIN: f32 = 0.0;
/// Maximum gain 2.0 ≈ +6 dB.
pub(crate) const MAX_GAIN: f32 = 2.0;
pub(crate) const DEFAULT_GUI_SIZE: LogicalSize<f64> = LogicalSize::new(360.0, 360.0);

/// Plugin factory. Used by the host to enumerate plugins and create instances.
pub(crate) struct WxpExampleGainPluginFactory {
    descriptor: PluginDescriptor,
}

/// Type that implements the clack Plugin trait.
/// Associates the audio processor, shared state, and main thread types via associated types.
pub(crate) struct WxpExampleGainPlugin;

// -----------------------------------------------------------------------
// CLAP plugin thread model
// -----------------------------------------------------------------------
// CLAP divides plugin state into three layers:
//
//   1. SharedState     — accessible from all threads (synchronized via Atomic types)
//   2. MainThread      — main thread only; used for GUI and parameter info operations
//   3. AudioProcessor  — audio thread only; real-time processing
//
// Values are passed between threads through SharedState.
// -----------------------------------------------------------------------

/// State shared across all threads. Wrapped in Arc so that both AudioProcessor
/// and MainThread can hold a reference to it.
pub(crate) struct SharedState {
    pub(crate) inner: Arc<SharedStateInner>,
}

pub(crate) struct SharedStateInner {
    /// Current gain value. Accessed from both the audio thread and the main thread,
    /// so AtomicF32 is used. Lock-free and safe to read/write from real-time threads.
    gain: AtomicF32,
    /// Pending flags for notifying the host of parameter changes made from the UI,
    /// to be consumed in the next flush/process call.
    /// Managed in three stages: gesture begin → value change → gesture end.
    /// A gesture is a single user interaction such as dragging a knob.
    /// The host uses gesture begin/end to determine the unit of automation recording.
    pending_ui: PendingUiState,
    /// Channel used to notify the GUI (WebView).
    /// None when the GUI is not open.
    gui_notifier: Mutex<Option<GuiNotifier>>,
}

/// Pending flags for propagating UI parameter changes to the host.
/// Each flag is an AtomicBool consumed (swapped to false) by process()/flush() on the audio thread.
struct PendingUiState {
    gesture_begin: AtomicBool,
    value_dirty: AtomicBool,
    gesture_end: AtomicBool,
}

/// Handle for sending GUI notifications. Dispatches to the main thread via RunLoopSender,
/// then sends a JSON message to WebView JavaScript via Channel.
#[derive(Clone)]
struct GuiNotifier {
    /// RunLoopSender can post closures from any thread to the main thread (RunLoop).
    /// Because WebView operations are only safe on the main thread,
    /// Channel::send() must not be called directly from the audio thread;
    /// use the sender instead.
    sender: RunLoopSender,
    /// wxp Channel — a bidirectional communication channel subscribed from the JavaScript side.
    /// Used for push notifications in the Rust → JS direction.
    channel: Channel,
}

/// Structure for serializing and saving plugin state.
/// Persisted by the host's "save project" feature.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SavedPluginState {
    pub(crate) gain: f32,
}

/// State accessed only on the main thread. GUI and parameter management happen here.
pub(crate) struct WxpExampleGainMainThread<'a> {
    pub(crate) shared: &'a SharedState,
    /// wxp WebViewRef. Some only while the GUI is open.
    pub(crate) web_view: Option<WebViewRef>,
    /// wxp WebContext. Manages the storage location for WebView user data (cache, etc.).
    /// Must outlive the WebView, so it is kept as a field.
    pub(crate) web_context: Option<WebContext>,
    /// wxp command handler. Registers commands (RPCs) callable from JavaScript.
    pub(crate) command_handler: Arc<WxpCommandHandler>,
    pub(crate) gui_size: LogicalSize<f64>,
    /// Utility for converting between the DPI scale factor presented by the host
    /// and logical/physical sizes.
    pub(crate) dpi_converter: DpiConverter,
}

impl WxpExampleGainPluginFactory {
    pub(crate) fn new() -> Self {
        Self {
            // AUDIO_EFFECT: tells the host this is an effect plugin.
            // STEREO: indicates stereo (2-channel) support.
            descriptor: PluginDescriptor::new(PLUGIN_ID, PLUGIN_NAME).with_features([
                clack_plugin::plugin::features::AUDIO_EFFECT,
                clack_plugin::plugin::features::STEREO,
            ]),
        }
    }
}

/// PluginFactoryImpl is the interface the CLAP host uses to enumerate and create plugins.
impl PluginFactoryImpl for WxpExampleGainPluginFactory {
    /// Number of plugins provided by this factory.
    fn plugin_count(&self) -> u32 {
        1
    }

    /// Returns the descriptor (ID, name, features) for the plugin at the given index.
    fn plugin_descriptor(&self, index: u32) -> Option<&PluginDescriptor> {
        (index == 0).then_some(&self.descriptor)
    }

    /// Called when the host actually creates a plugin instance.
    /// Passes the SharedState and MainThread constructors to clack's PluginInstance::new.
    fn create_plugin<'a>(
        &'a self,
        host_info: HostInfo<'a>,
        plugin_id: &CStr,
    ) -> Option<PluginInstance<'a>> {
        if plugin_id.to_string_lossy() != PLUGIN_ID {
            return None;
        }

        Some(PluginInstance::new::<WxpExampleGainPlugin>(
            host_info,
            &self.descriptor,
            |host| WxpExampleGainPlugin::new_shared(host),
            |host, shared| WxpExampleGainPlugin::new_main_thread(host, shared),
        ))
    }
}

impl Plugin for WxpExampleGainPlugin {
    /// Processor type that runs on the audio thread.
    type AudioProcessor<'a> = WxpExampleGainAudioProcessor<'a>;
    /// State type shared across all threads.
    type Shared<'a> = SharedState;
    /// State type for the main thread only.
    type MainThread<'a> = WxpExampleGainMainThread<'a>;

    /// Declares the CLAP extensions supported by this plugin.
    /// The host will only query extensions registered here.
    fn declare_extensions(
        builder: &mut PluginExtensions<Self>,
        _shared: Option<&Self::Shared<'_>>,
    ) {
        builder
            .register::<PluginAudioPorts>() // audio input/output port definitions
            .register::<PluginParams>() // parameter exposure
            .register::<PluginState>() // state save and restore
            .register::<PluginGui>(); // GUI provision
    }
}

impl DefaultPluginFactory for WxpExampleGainPlugin {
    fn get_descriptor() -> PluginDescriptor {
        PluginDescriptor::new(PLUGIN_ID, PLUGIN_NAME).with_features([
            clack_plugin::plugin::features::AUDIO_EFFECT,
            clack_plugin::plugin::features::STEREO,
        ])
    }

    /// Creates SharedState. One instance is created per plugin instance.
    fn new_shared(_host: HostSharedHandle<'_>) -> Result<Self::Shared<'_>, PluginError> {
        Ok(SharedState::new())
    }

    /// Creates the main thread state. Sets up the wxp command handler here and
    /// registers RPC commands callable from JavaScript.
    fn new_main_thread<'a>(
        _host: HostMainThreadHandle<'a>,
        shared: &'a Self::Shared<'a>,
    ) -> Result<Self::MainThread<'a>, PluginError> {
        // WxpCommandHandler is the RPC bridge between JavaScript and Rust.
        // register_commands() maps command names to their handlers.
        let command_handler = Arc::new(WxpCommandHandler::new());
        register_commands(command_handler.clone(), shared.inner.clone());

        Ok(WxpExampleGainMainThread {
            shared,
            web_view: None,
            web_context: None,
            command_handler,
            gui_size: DEFAULT_GUI_SIZE,
            dpi_converter: DpiConverter::new(1.0),
        })
    }
}

impl PluginShared<'_> for SharedState {}

impl<'a> PluginMainThread<'a, SharedState> for WxpExampleGainMainThread<'a> {}

/// Audio port definition. One input port and one output port (both stereo).
impl PluginAudioPortsImpl for WxpExampleGainMainThread<'_> {
    fn count(&mut self, _is_input: bool) -> u32 {
        1
    }

    fn get(&mut self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index != 0 {
            return;
        }

        writer.set(&AudioPortInfo {
            // Assign different IDs to the input and output ports.
            id: ClapId::new(if is_input { 1 } else { 2 }),
            name: if is_input { b"Main In" } else { b"Main Out" },
            // Stereo = 2 channels (L, R).
            channel_count: 2,
            // IS_MAIN: indicates this is the main port the host routes by default.
            flags: AudioPortFlags::IS_MAIN,
            port_type: Some(AudioPortType::STEREO),
            // Specifying in_place_pair enables "in-place processing" where
            // the input and output share the same buffer. None here (let the host decide).
            in_place_pair: None,
        });
    }
}

impl SharedState {
    fn new() -> Self {
        Self {
            inner: Arc::new(SharedStateInner::new()),
        }
    }
}

impl SharedStateInner {
    fn new() -> Self {
        Self {
            gain: AtomicF32::new(DEFAULT_GAIN),
            pending_ui: PendingUiState {
                gesture_begin: AtomicBool::new(false),
                value_dirty: AtomicBool::new(false),
                gesture_end: AtomicBool::new(false),
            },
            gui_notifier: Mutex::new(None),
        }
    }

    /// Returns the current gain value. Also called from the audio thread.
    /// Acquire ordering ensures the most recent store is visible.
    pub(crate) fn gain(&self) -> f32 {
        self.gain.load(Ordering::Acquire)
    }

    /// Called when the gain is changed by the host (e.g., DAW automation).
    /// Stores the value and notifies the GUI if it is open.
    pub(crate) fn set_gain_from_host(&self, gain: f64) -> f32 {
        let gain = clamp_gain(gain as f32);
        self.gain.store(gain, Ordering::Release);
        self.notify_gui();
        gain
    }

    // --- Parameter change notification from UI to host ---
    // In CLAP, when the UI changes a parameter, it must notify the host in these steps:
    //   1. begin_gesture  — user starts interacting with a knob or similar control
    //   2. set_value       — change the value (may be called multiple times during a drag)
    //   3. end_gesture    — interaction complete
    // These flags are consumed in the next process()/flush() call and forwarded
    // to the host as output events.

    pub(crate) fn begin_gesture_from_ui(&self) {
        self.pending_ui.gesture_begin.store(true, Ordering::Release);
    }

    /// Changes gain from the UI. Also sets the value_dirty flag for host notification.
    pub(crate) fn set_gain_from_ui(&self, gain: f64) -> f32 {
        let gain = self.set_gain_from_host(gain);
        self.pending_ui.value_dirty.store(true, Ordering::Release);
        gain
    }

    pub(crate) fn end_gesture_from_ui(&self) {
        self.pending_ui.gesture_end.store(true, Ordering::Release);
    }

    // take_* methods: use swap(false) to atomically read and clear a flag.
    // Called from process()/flush() to emit output events to the host.

    pub(crate) fn take_ui_gesture_begin(&self) -> bool {
        self.pending_ui.gesture_begin.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_ui_value_dirty(&self) -> bool {
        self.pending_ui.value_dirty.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_ui_gesture_end(&self) -> bool {
        self.pending_ui.gesture_end.swap(false, Ordering::AcqRel)
    }

    /// Registers a RunLoopSender and Channel when the GUI is opened.
    /// This enables push notifications of host parameter changes to the WebView.
    pub(crate) fn set_gui_channel(&self, sender: RunLoopSender, channel: Channel) {
        *self.gui_notifier.lock() = Some(GuiNotifier { sender, channel });
    }

    /// Called when the GUI is closed. Clears the notification target.
    pub(crate) fn clear_gui_channel(&self) {
        *self.gui_notifier.lock() = None;
    }

    /// Notifies the GUI when the gain value changes.
    /// By dispatching to the main thread via RunLoopSender, WebView messages can be
    /// sent safely from any thread, including the audio thread.
    fn notify_gui(&self) {
        let Some(notifier) = self.gui_notifier.lock().clone() else {
            return;
        };

        let payload = gain_payload(self.gain());
        // RunLoopSender::send() is asynchronous. The closure runs on the main thread.
        // Channel::send() sends the JSON payload to the JavaScript side.
        notifier.sender.send(move || {
            let _ = notifier.channel.send(payload);
        });
    }
}

impl WxpExampleGainMainThread<'_> {
    /// Returns the GUI API appropriate for the current platform.
    /// is_floating: false means the GUI is embedded in the host's window.
    pub(crate) fn preferred_api(&self) -> Option<clack_extensions::gui::GuiConfiguration<'static>> {
        Some(clack_extensions::gui::GuiConfiguration {
            api_type: clack_extensions::gui::GuiApiType::default_for_current_platform()?,
            is_floating: false,
        })
    }

    /// Cleanup when closing the GUI.
    /// Clears the Channel, then drops the WebView and WebContext.
    pub(crate) fn reset_webview(&mut self) {
        self.shared.inner.clear_gui_channel();
        self.web_view = None;
        self.web_context = None;
    }
}

/// Builds the JSON payload sent to the JavaScript side.
/// The UI receives this message format to update knob and text displays.
pub(crate) fn gain_payload(gain: f32) -> serde_json::Value {
    json!({
        "type": "gain-state",
        "value": gain,
        "dbText": gain_db_text(gain as f64),
    })
}

pub(crate) fn clamp_gain(gain: f32) -> f32 {
    gain.clamp(MIN_GAIN, MAX_GAIN)
}

/// Converts a linear gain value to a dB (decibel) string.
/// dB = 20 * log10(gain) is the standard logarithmic scale conversion in audio.
/// gain 1.0 = 0 dB, gain 0.0 = -∞ dB.
pub(crate) fn gain_db_text(gain: f64) -> String {
    if gain <= 0.0 {
        "-inf dB".to_string()
    } else {
        format!("{:.1} dB", 20.0 * gain.log10())
    }
}

// -----------------------------------------------------------------------
// wxp command handler registration
// -----------------------------------------------------------------------
// WxpCommandHandler is the RPC mechanism between JavaScript and Rust.
// When JavaScript calls `invoke("command_name", { args })`,
// the handler registered here is executed.
//
// register_sync: synchronous command (returns result immediately)
// register_async: asynchronous command (returns a Future)
//
// Handlers read and write parameters through SharedStateInner.

pub(crate) fn register_commands(
    command_handler: Arc<WxpCommandHandler>,
    shared: Arc<SharedStateInner>,
) {
    // Command for retrieving the current gain state. Used for the initial GUI render.
    {
        let shared = shared.clone();
        command_handler.register_sync("get_gain_state", move |_ctx| {
            Ok::<_, String>(gain_payload(shared.gain()))
        });
    }

    // Command for notifying gesture begin.
    // Called from JavaScript when the user starts dragging a knob.
    {
        let shared = shared.clone();
        command_handler.register_sync("begin_parameter_gesture", move |_ctx| {
            shared.begin_gesture_from_ui();
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // Command for setting the gain value. Called repeatedly during a drag.
    // ctx.arg() retrieves arguments passed from JavaScript in a type-safe way.
    {
        let shared = shared.clone();
        command_handler.register_sync("set_gain", move |ctx| {
            let value = ctx.arg::<f64>("value").map_err(|e| e.to_string())?;
            let applied = shared.set_gain_from_ui(value);
            Ok::<_, String>(gain_payload(applied))
        });
    }

    // Command for notifying gesture end.
    // Called from JavaScript when the user finishes dragging a knob.
    {
        let shared = shared.clone();
        command_handler.register_sync("end_parameter_gesture", move |_ctx| {
            shared.end_gesture_from_ui();
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // Command for subscribing to gain value changes.
    // JavaScript passes a Channel, and host-side value changes are pushed to it in real time.
    // This is the canonical pattern for asynchronous Rust → JS notifications.
    {
        let shared = shared.clone();
        command_handler.register_sync("subscribe_gain", move |ctx| {
            // Channel is a bidirectional communication channel provided by wxp.
            // It is created on the JavaScript side and passed to Rust as a command argument.
            let channel = ctx.arg::<Channel>("channel").map_err(|e| e.to_string())?;
            // Send the current value immediately upon registration (initial sync).
            channel
                .send(gain_payload(shared.gain()))
                .map_err(|e| e.to_string())?;

            // Obtain a sender handle to the main thread via RunLoop::sender() and
            // store it together with the Channel. Afterward, Channel sends can be
            // posted to the main thread from the audio thread or elsewhere via RunLoopSender.
            shared.set_gui_channel(RunLoop::sender(), channel);

            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // Unsubscribe command. Called when the GUI closes.
    {
        let shared = shared.clone();
        command_handler.register_sync("unsubscribe_gain", move |_ctx| {
            shared.clear_gui_channel();
            Ok::<_, String>(json!({ "ok": true }))
        });
    }
}
