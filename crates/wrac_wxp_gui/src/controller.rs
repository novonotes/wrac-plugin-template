use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use wrac_clap_adapter::{
    GuiApi, GuiConfig, GuiResizeHints, GuiSize, HostGuiResizeRequester, HostWindow, PluginError,
    PluginGuiExtension, PluginResult,
};
use wxp::{WebViewDispatch, dpi::LogicalSize};

use crate::dpi::DpiConverter;
use crate::runtime::{
    GuiRuntimeHandle, GuiThreadLease, WxpGuiFactory, create_gui_runtime_handle, is_gui_thread,
};
use crate::window::StoredParentWindow;

#[derive(Debug, Clone, Copy)]
pub struct GuiSizeLimits {
    pub min: GuiSize,
    pub max: GuiSize,
}

/// Send/Sync controller that exposes the wxp WebView runtime as a [`PluginGuiExtension`].
///
/// The actual runtime lives in TLS on the UI thread; this type receives GUI lifecycle
/// callbacks as the [`PluginGuiExtension`] handle shared across CLAP instances. Only embedded GUI
/// (attached as a child view to the host parent) is supported; floating windows are rejected.
/// Methods may be entered from host callback threads; GUI runtime work is serialized through the
/// GUI run loop once a parent has established the owning GUI thread.
/// This controller is not realtime-safe; do not call it from the audio callback.
pub struct WxpGuiController {
    factory: Arc<dyn WxpGuiFactory>,
    layout: Arc<HostGuiLayout>,
    scale: Arc<Mutex<f64>>,
    runtime: Arc<Mutex<GuiRuntimeState>>,
}

struct HostGuiLayout {
    // Host-contract size value read by CLAP layout queries without entering the GUI runtime
    // (not a copy of the runtime state).
    accepted_size: AtomicGuiSize,
    // Some wrappers call `set_size()` re-entrantly from within `request_resize()` (even
    // when the return value is false). This revision counter lets the request side detect
    // "the size the host confirmed" without holding the runtime lock or guessing the return value.
    accepted_size_revision: AtomicU64,
    limits: GuiSizeLimits,
    resize_policy: GuiResizePolicy,
}

struct GuiRuntimeState {
    session: Option<GuiSession>,
    // Rapid open/close of the editor sends create/set_parent/show/destroy in quick
    // succession. WebView creation is posted to the GUI run loop, so the callback arrives
    // after the originating CLAP call returns. The generation counter lets a delayed
    // callback detect a stale session and tear down the half-created runtime without
    // attaching it to an already-closed editor.
    generation: u64,
    last_runtime_destroyed_at: Option<Instant>,
    // Some Windows hosts (notably Ableton Live) may recreate the editor while the
    // previous teardown is still in progress. Keep child WebView creation single-flight
    // and remember only the latest requested generation.
    is_creating_runtime: bool,
    creating_generation: Option<u64>,
    pending_creation_generation: Option<u64>,
    destroy_requested_while_creating: bool,
}

// Quiet period after runtime teardown. Without it, rapid editor reopens can request a
// new child WebView before the previous teardown completes.
const WEBVIEW_RECREATE_QUIET_PERIOD: Duration = Duration::from_millis(500);

// CLAP `create()` starts a GUI session, but an embedded WebView's native child cannot
// be created without a parent handle. Separating session from runtime allows size/scale
// queries to be answered after `create()` while deferring native object creation until
// the parent arrives.
struct GuiSession {
    generation: u64,
    configuration: GuiConfig,
    scale: f64,
    parent: Option<StoredParentWindow>,
    parent_lease: Option<GuiThreadLease>,
    handle: Option<GuiRuntimeHandle>,
    visible: bool,
}

#[derive(Clone)]
pub struct WxpGuiResizeHandle {
    layout: Arc<HostGuiLayout>,
    scale: Arc<Mutex<f64>>,
}

const _: () = {
    fn assert_send_sync<T: Send + Sync>() {}

    // These handles are intentionally shared with host callbacks and product command handlers.
    // Thread-affine native GUI objects remain behind run-loop dispatch/TLS.
    let _ = assert_send_sync::<WxpGuiController>;
    let _ = assert_send_sync::<WxpGuiResizeHandle>;
};

impl WxpGuiController {
    pub fn new_with_resize_handle(
        factory: impl WxpGuiFactory,
        resize_handle: WxpGuiResizeHandle,
    ) -> Self {
        Self {
            factory: Arc::new(factory),
            layout: resize_handle.layout.clone(),
            scale: resize_handle.scale.clone(),
            runtime: Arc::new(Mutex::new(GuiRuntimeState {
                session: None,
                generation: 0,
                last_runtime_destroyed_at: None,
                is_creating_runtime: false,
                creating_generation: None,
                pending_creation_generation: None,
                destroy_requested_while_creating: false,
            })),
        }
    }

    fn destroy_gui_session(&self) {
        log::debug!("wxp controller: destroy_gui_session requested");
        {
            let mut state = self.runtime.lock();
            if state.is_creating_runtime {
                log::debug!("wxp controller: destroy_gui_session deferred during runtime creation");
                let session = state.session.take();
                state.generation = state.generation.wrapping_add(1);
                state.destroy_requested_while_creating = true;
                drop(state);
                if drop_session(session) {
                    self.note_runtime_destroyed();
                }
                return;
            }
        }
        let session = { self.runtime.lock().session.take() };
        if drop_session(session) {
            self.note_runtime_destroyed();
        }
        log::debug!("wxp controller: destroy_gui_session completed");
    }

    fn note_runtime_destroyed(&self) {
        self.runtime.lock().last_runtime_destroyed_at = Some(Instant::now());
    }

    fn schedule_runtime_creation(&self, generation: u64) -> PluginResult<()> {
        schedule_runtime_creation(
            self.factory.clone(),
            self.runtime.clone(),
            self.layout.clone(),
            generation,
        )
    }
}

fn schedule_runtime_creation(
    factory: Arc<dyn WxpGuiFactory>,
    runtime: Arc<Mutex<GuiRuntimeState>>,
    layout: Arc<HostGuiLayout>,
    generation: u64,
) -> PluginResult<()> {
    // Intentionally asynchronous with CLAP GUI callbacks. Creating a WebView inline
    // makes host lifecycle re-entry more likely. Posting to the run loop centralizes
    // creation serialization, pending visibility/size application, and stale-generation
    // teardown in one place.
    let (configuration, parent, sender) = {
        let mut state = runtime.lock();
        if state.is_creating_runtime {
            log::debug!(
                "wxp controller: runtime creation pending while another creation is in progress: generation={generation}"
            );
            state.pending_creation_generation = Some(generation);
            return Ok(());
        }
        let session = state.session.as_ref().ok_or(PluginError::InvalidState)?;
        if session.generation != generation {
            return Err(PluginError::InvalidState);
        }
        if session.handle.is_some() {
            log::debug!(
                "wxp controller: runtime creation skipped; runtime already exists: generation={generation}"
            );
            return Ok(());
        }
        let parent = session.parent.ok_or(PluginError::InvalidState)?;
        let sender = session
            .parent_lease
            .as_ref()
            .ok_or(PluginError::InvalidState)?
            .sender();
        let configuration = session.configuration;
        state.is_creating_runtime = true;
        state.creating_generation = Some(generation);
        state.pending_creation_generation = None;
        state.destroy_requested_while_creating = false;
        (configuration, parent, sender)
    };

    log::debug!("wxp controller: posting runtime creation: generation={generation}");
    let factory_for_callback = factory.clone();
    let runtime_for_callback = runtime.clone();
    let layout_for_callback = layout.clone();
    sender.send(move || {
            log::debug!("wxp controller: posted runtime creation started: generation={generation}");
            let result = create_runtime_on_gui_thread(
                factory_for_callback.as_ref(),
                runtime_for_callback.as_ref(),
                layout_for_callback.as_ref(),
                configuration,
                parent,
                generation,
            );

            let handle = match result {
                Ok(handle) => handle,
                Err(error) => {
                    log::warn!(
                        "wxp controller: posted runtime creation failed: generation={generation}, error={error:?}"
                    );
                    schedule_pending_runtime_creation(
                        factory_for_callback,
                        runtime_for_callback,
                        layout_for_callback,
                    );
                    return;
                }
            };

            let Some((visible, size, scale)) = latest_runtime_state(
                runtime_for_callback.as_ref(),
                layout_for_callback.as_ref(),
                generation,
            ) else {
                log::debug!(
                    "wxp controller: posted runtime creation produced stale runtime: generation={generation}"
                );
                handle.destroy();
                runtime_for_callback.lock().last_runtime_destroyed_at = Some(Instant::now());
                schedule_pending_runtime_creation(
                    factory_for_callback,
                    runtime_for_callback,
                    layout_for_callback,
                );
                return;
            };

            if let Err(error) = handle.set_size(size) {
                log::warn!(
                    "wxp controller: posted runtime creation latest set_size failed: {error:?}"
                );
                handle.destroy();
                runtime_for_callback.lock().last_runtime_destroyed_at = Some(Instant::now());
                schedule_pending_runtime_creation(
                    factory_for_callback,
                    runtime_for_callback,
                    layout_for_callback,
                );
                return;
            }
            if let Err(error) = handle.set_scale(scale) {
                log::warn!(
                    "wxp controller: posted runtime creation latest set_scale failed: {error:?}"
                );
                handle.destroy();
                runtime_for_callback.lock().last_runtime_destroyed_at = Some(Instant::now());
                schedule_pending_runtime_creation(
                    factory_for_callback,
                    runtime_for_callback,
                    layout_for_callback,
                );
                return;
            }

            if !visible {
                log::debug!("wxp controller: posted runtime creation hiding initially hidden runtime");
                if let Err(error) = handle.hide() {
                    log::warn!(
                        "wxp controller: posted runtime creation initial hide failed: {error:?}"
                    );
                    handle.destroy();
                    runtime_for_callback.lock().last_runtime_destroyed_at = Some(Instant::now());
                    schedule_pending_runtime_creation(
                        factory_for_callback,
                        runtime_for_callback,
                        layout_for_callback,
                    );
                    return;
                }
            }

            let mut state = runtime_for_callback.lock();
            let Some(session) = state.session.as_mut() else {
                drop(state);
                handle.destroy();
                runtime_for_callback.lock().last_runtime_destroyed_at = Some(Instant::now());
                schedule_pending_runtime_creation(
                    factory_for_callback,
                    runtime_for_callback,
                    layout_for_callback,
                );
                return;
            };
            if session.generation != generation {
                drop(state);
                handle.destroy();
                runtime_for_callback.lock().last_runtime_destroyed_at = Some(Instant::now());
                schedule_pending_runtime_creation(
                    factory_for_callback,
                    runtime_for_callback,
                    layout_for_callback,
                );
                return;
            }
            if let Some(old_handle) = session.handle.replace(handle) {
                log::debug!(
                    "wxp controller: destroying previous runtime before replacing handle: generation={generation}"
                );
                drop(state);
                old_handle.destroy();
                runtime_for_callback.lock().last_runtime_destroyed_at = Some(Instant::now());
                schedule_pending_runtime_creation(
                    factory_for_callback,
                    runtime_for_callback,
                    layout_for_callback,
                );
                return;
            }
            if state.pending_creation_generation == Some(generation) {
                log::debug!(
                    "wxp controller: dropping redundant pending runtime creation: generation={generation}"
                );
                state.pending_creation_generation = None;
            }
            log::debug!("wxp controller: posted runtime creation completed: generation={generation}");
            drop(state);
            schedule_pending_runtime_creation(
                factory_for_callback,
                runtime_for_callback,
                layout_for_callback,
            );
        });
    Ok(())
}

fn schedule_pending_runtime_creation(
    factory: Arc<dyn WxpGuiFactory>,
    runtime: Arc<Mutex<GuiRuntimeState>>,
    layout: Arc<HostGuiLayout>,
) {
    let pending_generation = {
        let mut state = runtime.lock();
        let pending = state.pending_creation_generation.take();
        if let Some(generation) = pending
            && state
                .session
                .as_ref()
                .is_some_and(|session| session.generation == generation && session.handle.is_some())
        {
            log::debug!(
                "wxp controller: pending runtime creation skipped; runtime already exists: generation={generation}"
            );
            None
        } else {
            pending
        }
    };
    let Some(generation) = pending_generation else {
        return;
    };
    log::debug!("wxp controller: scheduling pending runtime creation: generation={generation}");
    if let Err(error) = schedule_runtime_creation(factory, runtime, layout, generation) {
        log::warn!("wxp controller: pending runtime creation was dropped: {error:?}");
    }
}

fn create_runtime_on_gui_thread(
    factory: &dyn WxpGuiFactory,
    runtime: &Mutex<GuiRuntimeState>,
    layout: &HostGuiLayout,
    configuration: GuiConfig,
    parent: StoredParentWindow,
    generation: u64,
) -> PluginResult<GuiRuntimeHandle> {
    let (size, scale) = latest_runtime_creation_inputs(runtime, layout, generation)
        .ok_or(PluginError::InvalidState)?;
    log::debug!(
        "wxp controller: create_runtime start: generation={}, width={}, height={}, scale={}, configuration={configuration:?}",
        generation,
        size.width,
        size.height,
        scale
    );
    let Some(wait_duration) = runtime
        .lock()
        .last_runtime_destroyed_at
        .and_then(|at| WEBVIEW_RECREATE_QUIET_PERIOD.checked_sub(at.elapsed()))
    else {
        return create_runtime_after_wait(
            factory,
            runtime,
            configuration,
            size,
            parent,
            scale,
            generation,
        );
    };
    log::debug!(
        "wxp controller: waiting before WebView recreate: {}ms",
        wait_duration.as_millis()
    );
    std::thread::sleep(wait_duration);
    log::debug!("wxp controller: WebView recreate wait completed");
    let (size, scale) = latest_runtime_creation_inputs(runtime, layout, generation)
        .ok_or(PluginError::InvalidState)?;
    create_runtime_after_wait(
        factory,
        runtime,
        configuration,
        size,
        parent,
        scale,
        generation,
    )
}

fn create_runtime_after_wait(
    factory: &dyn WxpGuiFactory,
    runtime: &Mutex<GuiRuntimeState>,
    configuration: GuiConfig,
    size: GuiSize,
    parent: StoredParentWindow,
    scale: f64,
    generation: u64,
) -> PluginResult<GuiRuntimeHandle> {
    let parent = parent.to_parent_window_handle()?;
    log::debug!("wxp controller: parent handle converted");
    let handle =
        match create_gui_runtime_handle(|| factory.create_gui_runtime(configuration, size, parent))
        {
            Ok(handle) => handle,
            Err(error) => {
                let mut state = runtime.lock();
                if state.creating_generation == Some(generation) {
                    state.is_creating_runtime = false;
                    state.creating_generation = None;
                    state.pending_creation_generation = None;
                    state.destroy_requested_while_creating = false;
                }
                return Err(error);
            }
        };
    log::debug!("wxp controller: runtime handle created");
    if finish_runtime_creation_requested_destroy(runtime, generation) {
        log::debug!(
            "wxp controller: destroying newly created runtime after stale/deferred destroy"
        );
        handle.destroy();
        runtime.lock().last_runtime_destroyed_at = Some(Instant::now());
        return Err(PluginError::InvalidState);
    }
    if let Err(error) = handle.set_scale(scale) {
        log::warn!("wxp controller: initial set_scale failed: {error:?}");
        handle.destroy();
        return Err(error);
    }
    log::debug!("wxp controller: create_runtime completed");
    Ok(handle)
}

fn latest_runtime_creation_inputs(
    runtime: &Mutex<GuiRuntimeState>,
    layout: &HostGuiLayout,
    generation: u64,
) -> Option<(GuiSize, f64)> {
    let state = runtime.lock();
    let session = state.session.as_ref()?;
    if session.generation != generation {
        return None;
    }
    Some((layout.accepted_size(), session.scale))
}

fn latest_runtime_state(
    runtime: &Mutex<GuiRuntimeState>,
    layout: &HostGuiLayout,
    generation: u64,
) -> Option<(bool, GuiSize, f64)> {
    let state = runtime.lock();
    let session = state.session.as_ref()?;
    if session.generation != generation {
        return None;
    }
    Some((session.visible, layout.accepted_size(), session.scale))
}

fn finish_runtime_creation_requested_destroy(
    runtime: &Mutex<GuiRuntimeState>,
    generation: u64,
) -> bool {
    let mut state = runtime.lock();
    let session_is_stale = match state.session.as_ref() {
        Some(session) => session.generation != generation,
        None => true,
    };
    let should_destroy = state.destroy_requested_while_creating || session_is_stale;
    if state.creating_generation == Some(generation) {
        state.is_creating_runtime = false;
        state.creating_generation = None;
        if should_destroy {
            state.pending_creation_generation =
                state.session.as_ref().map(|session| session.generation);
        }
        state.destroy_requested_while_creating = false;
    }
    should_destroy
}

impl HostGuiLayout {
    fn new(size: GuiSize, limits: GuiSizeLimits, resize_policy: GuiResizePolicy) -> Self {
        let size = clamp_size_with_limits(size, limits);
        Self {
            accepted_size: AtomicGuiSize::new(size),
            accepted_size_revision: AtomicU64::new(0),
            limits,
            resize_policy,
        }
    }

    fn accepted_size(&self) -> GuiSize {
        self.accepted_size.load()
    }

    fn clamp_size(&self, size: GuiSize) -> GuiSize {
        clamp_size_with_limits(size, self.limits)
    }

    fn clamp_logical_size(&self, size: LogicalSize<f64>, scale: f64) -> LogicalSize<f64> {
        let dpi = DpiConverter::new(scale);
        // Resize commands receive frontend logical pixels, while the host's min/max
        // contract is physical pixels. Convert before clamping so limits mean the
        // same thing at every DPI scale.
        let physical = dpi.logical_size_to_gui(size);
        let clamped = clamp_size_with_limits(physical, self.limits);
        dpi.gui_size_to_logical(clamped)
    }

    fn store_accepted_size(&self, size: GuiSize) {
        self.accepted_size.store(size);
        self.accepted_size_revision.fetch_add(1, Ordering::Relaxed);
    }

    fn accepted_size_revision(&self) -> u64 {
        self.accepted_size_revision.load(Ordering::Relaxed)
    }

    fn can_resize(&self) -> bool {
        self.resize_policy.can_resize()
    }

    fn resize_hints(&self) -> GuiResizeHints {
        self.resize_policy.resize_hints()
    }
}

impl WxpGuiResizeHandle {
    pub fn new(initial_size: GuiSize, limits: GuiSizeLimits) -> Self {
        Self {
            layout: Arc::new(HostGuiLayout::new(
                initial_size,
                limits,
                GuiResizePolicy::RESIZABLE,
            )),
            scale: Arc::new(Mutex::new(1.0)),
        }
    }

    /// Requests a host-approved resize from the GUI event path and mirrors accepted bounds to wxp.
    ///
    /// `WxpGuiResizeHandle` is `Send + Sync` so command registration can share it, but this method
    /// enters the host GUI resize extension and must only be called from GUI commands/events.
    pub fn request_resize(
        &self,
        requested: LogicalSize<f64>,
        web_view: &WebViewDispatch,
        host_gui_resize_requester: &dyn HostGuiResizeRequester,
    ) -> PluginResult<LogicalSize<f64>> {
        // `HostGuiResizeRequester` can be shared from Send/Sync product state, but the target
        // API is a host GUI extension. Keep the "GUI command only" threading contract at the
        // command registration boundary rather than making this a generic background-thread API.
        let scale = *self.scale.lock();
        let logical_size = self.layout.clamp_logical_size(requested, scale);
        let gui_size = DpiConverter::new(scale).logical_size_to_gui(logical_size);

        let previous_revision = self.layout.accepted_size_revision();
        let resize_result = host_gui_resize_requester.request_resize(gui_size);
        let current_revision = self.layout.accepted_size_revision();

        // Logic's AUv2 wrapper applies the NSView frame inside `request_resize()`, calls
        // `set_size()` re-entrantly, and then returns false to CLAP. Treat that re-entrant
        // `set_size()` as the ground truth. Optimistically resizing the WebView here would
        // race geometry with the host and cause visual jitter during grip dragging.
        let dpi = DpiConverter::new(scale);
        if current_revision != previous_revision {
            return Ok(dpi.gui_size_to_logical(self.layout.accepted_size()));
        }

        match resize_result {
            Ok(()) => {
                // Some hosts accept the request but never call `set_size()`. In that case,
                // update the WebView directly without waiting for an async callback.
                // Pass `WebViewDispatch` rather than the native owner so the command handler
                // can resize without extending the lifetime of a closing editor.
                web_view
                    .post_set_bounds(dpi.create_webview_bounds(logical_size))
                    .map_err(|_| PluginError::Message("failed to resize webview"))?;
                self.layout.store_accepted_size(gui_size);
                Ok(logical_size)
            }
            Err(error) => {
                // A genuine rejection is distinct from the AUv2 re-entry case above. Rather
                // than speculatively moving the child WebView and rolling it back, keep the
                // last host-confirmed size.
                Err(error)
            }
        }
    }
}

struct AtomicGuiSize(AtomicU64);

impl AtomicGuiSize {
    fn new(size: GuiSize) -> Self {
        Self(AtomicU64::new(pack_size(size)))
    }

    fn load(&self) -> GuiSize {
        unpack_size(self.0.load(Ordering::Relaxed))
    }

    fn store(&self, size: GuiSize) {
        self.0.store(pack_size(size), Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy)]
struct GuiResizePolicy {
    can_resize: bool,
}

impl GuiResizePolicy {
    const RESIZABLE: Self = Self { can_resize: true };

    fn can_resize(self) -> bool {
        self.can_resize
    }

    fn resize_hints(self) -> GuiResizeHints {
        GuiResizeHints {
            can_resize_horizontally: self.can_resize,
            can_resize_vertically: self.can_resize,
            preserve_aspect_ratio: false,
            aspect_ratio_width: 0,
            aspect_ratio_height: 0,
        }
    }
}

fn pack_size(size: GuiSize) -> u64 {
    ((size.width as u64) << 32) | size.height as u64
}

fn unpack_size(size: u64) -> GuiSize {
    GuiSize {
        width: (size >> 32) as u32,
        height: size as u32,
    }
}

impl PluginGuiExtension for WxpGuiController {
    fn is_api_supported(&self, api: GuiApi, is_floating: bool) -> bool {
        !is_floating && api == default_gui_api()
    }

    fn preferred_api(&self) -> Option<GuiConfig> {
        Some(default_gui_configuration())
    }

    fn create(&self, configuration: GuiConfig) -> PluginResult<()> {
        log::debug!("wxp controller: create called: configuration={configuration:?}");
        if !self.is_api_supported(configuration.api, configuration.is_floating) {
            log::debug!("wxp controller: create rejected unsupported configuration");
            return Err(PluginError::Message("unsupported GUI configuration"));
        }
        self.destroy_gui_session();
        let scale = *self.scale.lock();
        let generation = {
            let mut state = self.runtime.lock();
            state.generation = state.generation.wrapping_add(1);
            let generation = state.generation;
            state.session = Some(GuiSession {
                generation,
                configuration,
                scale,
                parent: None,
                parent_lease: None,
                handle: None,
                // Some wrappers treat attachment to the parent as an implicit show and never
                // call `show()`. Default to visible so the first parent attach works; an
                // explicit `hide()` overrides this.
                visible: true,
            });
            generation
        };
        log::debug!("wxp controller: create completed: generation={generation}");
        Ok(())
    }

    fn destroy(&self) {
        log::debug!("wxp controller: destroy called");
        self.destroy_gui_session();
        log::debug!("wxp controller: destroy completed");
    }

    fn set_scale(&self, scale: f64) -> PluginResult<()> {
        log::debug!("wxp controller: set_scale called: scale={scale}");
        let handle = {
            let mut state = self.runtime.lock();
            if let Some(session) = &mut state.session {
                session.scale = scale;
                session.handle.clone()
            } else {
                None
            }
        };
        if let Some(handle) = handle {
            handle.set_scale(scale)?;
        }
        *self.scale.lock() = scale;
        log::debug!("wxp controller: set_scale completed");
        Ok(())
    }

    fn get_size(&self) -> PluginResult<GuiSize> {
        let size = self.layout.accepted_size();
        log::debug!(
            "wxp controller: get_size called: width={}, height={}",
            size.width,
            size.height
        );
        Ok(size)
    }

    fn can_resize(&self) -> bool {
        self.layout.can_resize()
    }

    fn resize_hints(&self) -> Option<GuiResizeHints> {
        Some(self.layout.resize_hints())
    }

    fn adjust_size(&self, size: GuiSize) -> PluginResult<GuiSize> {
        Ok(self.layout.clamp_size(size))
    }

    fn set_size(&self, size: GuiSize) -> PluginResult<()> {
        let size = self.layout.clamp_size(size);
        let previous_size = self.layout.accepted_size();
        let size_changed = previous_size.width != size.width || previous_size.height != size.height;
        let handle = {
            self.runtime
                .lock()
                .session
                .as_ref()
                .and_then(|session| session.handle.clone())
        };

        // Some hosts repeatedly send the same size until the editor window settles.
        // Re-applying identical bounds does not violate the contract but adds redundant
        // geometry processing to the child view, making resize drags feel laggy. Size is
        // still recorded below so re-entrant `request_resize()` detection can observe
        // host callbacks.
        if let Some(handle) = handle {
            if size_changed {
                handle.set_size(size)?;
            }
        }
        self.layout.store_accepted_size(size);
        Ok(())
    }

    fn set_parent(&self, window: HostWindow) -> PluginResult<()> {
        log::debug!("wxp controller: set_parent called");
        let parent = StoredParentWindow::from_host_window(window);
        let (generation, needs_parent_lease) = {
            let state = self.runtime.lock();
            let session = state.session.as_ref().ok_or(PluginError::InvalidState)?;
            let needs_parent_lease = if session.parent.is_some() {
                if !is_gui_thread() {
                    log::debug!("wxp controller: set_parent rejected non-GUI thread reparent");
                    return Err(PluginError::UnsupportedHostGuiThreadingModel);
                }
                false
            } else {
                true
            };
            (session.generation, needs_parent_lease)
        };
        log::debug!(
            "wxp controller: set_parent needs_parent_lease={needs_parent_lease}, generation={generation}"
        );

        let parent_lease = needs_parent_lease
            .then(GuiThreadLease::acquire)
            .transpose()?;
        log::debug!("wxp controller: set_parent parent lease acquired");

        let old_handle = {
            let mut state = self.runtime.lock();
            let session = state.session.as_mut().ok_or(PluginError::InvalidState)?;
            if session.generation != generation {
                drop(parent_lease);
                return Err(PluginError::InvalidState);
            }
            // wxp/wry gives no guarantee that an existing child WebView can be safely
            // reparented. Tear down the old runtime first and recreate it on the new parent.
            session.handle.take()
        };
        if let Some(handle) = old_handle {
            log::debug!("wxp controller: set_parent destroying old runtime before reparent");
            handle.destroy();
            self.note_runtime_destroyed();
            log::debug!("wxp controller: set_parent old runtime destroyed");
        }

        {
            let state = self.runtime.lock();
            let session = state.session.as_ref().ok_or(PluginError::InvalidState)?;
            if session.generation != generation {
                drop(parent_lease);
                return Err(PluginError::InvalidState);
            }
        }
        let mut state = self.runtime.lock();
        let session = state.session.as_mut().ok_or(PluginError::InvalidState)?;
        if session.generation != generation {
            drop(state);
            drop(parent_lease);
            return Err(PluginError::InvalidState);
        }
        session.parent = Some(parent);
        if let Some(parent_lease) = parent_lease {
            session.parent_lease = Some(parent_lease);
        }
        drop(state);
        // Only accept the parent and schedule WebView creation here. Deferring actual
        // creation outside the host lifecycle callback avoids create/destroy re-entry.
        // On failure, leave the session without a runtime and let a subsequent
        // show/set_parent reschedule it.
        self.schedule_runtime_creation(generation)?;
        log::debug!("wxp controller: set_parent completed");
        Ok(())
    }

    fn set_transient(&self, _window: HostWindow) -> PluginResult<()> {
        Err(PluginError::Message("floating GUI is unsupported"))
    }

    fn suggest_title(&self, _title: &str) {}

    fn show(&self) -> PluginResult<()> {
        log::debug!("wxp controller: show called");
        let action = {
            let state = self.runtime.lock();
            let session = state.session.as_ref().ok_or(PluginError::InvalidState)?;
            if let Some(handle) = session.handle.clone() {
                ShowAction::ShowExisting {
                    handle,
                    generation: session.generation,
                }
            } else {
                let parent = session.parent.ok_or(PluginError::InvalidState)?;
                let _ = parent;
                ShowAction::Create {
                    generation: session.generation,
                }
            }
        };

        match action {
            ShowAction::ShowExisting { handle, generation } => {
                log::debug!("wxp controller: show existing runtime");
                handle.show()?;
                if let Some(session) = &mut self.runtime.lock().session
                    && session.generation == generation
                {
                    session.visible = true;
                }
                log::debug!("wxp controller: show completed on existing runtime");
                Ok(())
            }
            ShowAction::Create { generation } => {
                log::debug!("wxp controller: show scheduling runtime creation");
                self.schedule_runtime_creation(generation)?;
                if let Some(session) = &mut self.runtime.lock().session
                    && session.generation == generation
                {
                    session.visible = true;
                }
                log::debug!("wxp controller: show completed by scheduled runtime creation");
                Ok(())
            }
        }
    }

    fn hide(&self) -> PluginResult<()> {
        log::debug!("wxp controller: hide called");
        let (generation, handle) = {
            let state = self.runtime.lock();
            let session = state.session.as_ref().ok_or(PluginError::InvalidState)?;
            (session.generation, session.handle.clone())
        };
        if let Some(handle) = handle {
            handle.hide()?;
        }
        if let Some(session) = &mut self.runtime.lock().session
            && session.generation == generation
        {
            session.visible = false;
        }
        log::debug!("wxp controller: hide completed");
        Ok(())
    }
}

fn drop_session(session: Option<GuiSession>) -> bool {
    if let Some(mut session) = session {
        log::debug!("wxp controller: drop_session start");
        let mut destroyed_runtime = false;
        if let Some(handle) = session.handle.take() {
            handle.destroy();
            destroyed_runtime = true;
        }
        // Release the parent lease only after the runtime has been dropped, so the owner
        // thread is not freed before timer stop and WebView teardown complete on the run loop.
        drop(session.parent_lease.take());
        log::debug!("wxp controller: drop_session completed");
        destroyed_runtime
    } else {
        log::debug!("wxp controller: drop_session skipped; no active session");
        false
    }
}

fn clamp_size_with_limits(size: GuiSize, limits: GuiSizeLimits) -> GuiSize {
    GuiSize {
        width: size.width.clamp(limits.min.width, limits.max.width),
        height: size.height.clamp(limits.min.height, limits.max.height),
    }
}

impl Drop for WxpGuiController {
    fn drop(&mut self) {
        self.destroy_gui_session();
    }
}

enum ShowAction {
    ShowExisting {
        handle: GuiRuntimeHandle,
        generation: u64,
    },
    Create {
        generation: u64,
    },
}

fn default_gui_api() -> GuiApi {
    if cfg!(target_os = "macos") {
        GuiApi::Cocoa
    } else if cfg!(target_os = "windows") {
        GuiApi::Win32
    } else {
        GuiApi::X11
    }
}

fn default_gui_configuration() -> GuiConfig {
    GuiConfig {
        api: default_gui_api(),
        is_floating: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_logical_resize_request_in_physical_pixels() {
        let layout = HostGuiLayout::new(
            GuiSize {
                width: 600,
                height: 400,
            },
            GuiSizeLimits {
                min: GuiSize {
                    width: 300,
                    height: 200,
                },
                max: GuiSize {
                    width: 900,
                    height: 600,
                },
            },
            GuiResizePolicy::RESIZABLE,
        );

        let clamped = layout.clamp_logical_size(LogicalSize::new(700.0, 100.0), 1.5);

        assert_eq!(clamped.width, 600.0);
        assert_eq!(clamped.height, 200.0 / 1.5);
    }
}
