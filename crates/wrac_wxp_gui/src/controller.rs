use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::runtime::{
    GuiRuntimeHandle, GuiThreadLease, WxpGuiFactory, create_gui_runtime_handle, is_gui_thread,
};
use crate::window::StoredParentWindow;
use novonotes_run_loop::{RunLoop, RunLoopLocal};
use parking_lot::Mutex;
use wrac_clap_adapter::interface::{
    GuiApi, GuiConfig, GuiResizeHints, GuiSize, HostWindow, PluginError,
    PluginGuiApiSupportExtension, PluginGuiExtension, PluginGuiMainThreadExtension,
    PluginGuiQueryExtension, PluginResult,
};
use wrac_host_context::{HostContext, HostFamily, PluginFormat};

mod defaults;
mod resize;
mod scale;

use self::defaults::{default_gui_api, default_gui_configuration};
use self::resize::HostGuiLayout;
pub use self::resize::WxpGuiResizeHandle;
use self::scale::corrected_scale_for_parent;

/// Minimum and maximum editor bounds.
///
/// On Windows, bounds are specified in 100% logical pixels and scaled into host pixels by
/// the controller. On macOS, bounds are Cocoa logical points. On Linux, bounds remain in
/// host units.
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
    runtime: Arc<Mutex<GuiRuntimeState>>,
    host_context: HostContext,
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
    parent: Option<StoredParentWindow>,
    parent_lease: Option<GuiThreadLease>,
    handle: Option<GuiRuntimeHandle>,
    visible: bool,
}

struct RuntimeCreationRequest {
    configuration: GuiConfig,
    size: GuiSize,
    parent: StoredParentWindow,
    scale: f64,
    generation: u64,
}

const _: () = {
    fn assert_send_sync<T: Send + Sync>() {}

    // These handles are intentionally shared with host callbacks and product command handlers.
    // Thread-affine native GUI objects remain behind run-loop dispatch/TLS.
    let _ = assert_send_sync::<WxpGuiController>;
    let _ = assert_send_sync::<WxpGuiResizeHandle>;
};

impl WxpGuiController {
    pub fn new(
        factory: impl WxpGuiFactory,
        resize_handle: WxpGuiResizeHandle,
        host_context: HostContext,
    ) -> Self {
        Self {
            factory: Arc::new(factory),
            layout: resize_handle.layout.clone(),
            runtime: Arc::new(Mutex::new(GuiRuntimeState {
                session: None,
                generation: 0,
                last_runtime_destroyed_at: None,
                is_creating_runtime: false,
                creating_generation: None,
                pending_creation_generation: None,
                destroy_requested_while_creating: false,
            })),
            host_context,
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

    fn should_async_resync_bounds_after_set_size(&self) -> bool {
        is_cubase_vst3(&self.host_context)
    }

    fn correct_host_scale(&self, scale: f64, parent: Option<StoredParentWindow>) -> f64 {
        if !is_cubase_vst3(&self.host_context) {
            return scale;
        }
        // Cubase 10 on Windows has been observed to report integer VST3 scale factors
        // even when the editor is hosted on a fractional-DPI monitor. The host parent
        // HWND is the closest source of truth for the actual size conversion.
        let corrected = corrected_scale_for_parent(parent).unwrap_or(scale);
        if (corrected - scale).abs() > f64::EPSILON {
            log::info!(
                "wxp controller: corrected Cubase VST3 host scale from {scale} to {corrected}"
            );
        }
        corrected
    }

    /// Asks the host to enlarge an attached frame that would clip at the active scale.
    ///
    /// Some hosts report the scale only after `get_size` and `set_parent` (Ableton Live
    /// with Auto-Scale off sends `set_scale` last), so the frame keeps the size negotiated
    /// at the previous scale and falls below the logical minimum. See
    /// `HostGuiLayout::host_size_to_unclip_at_active_scale` for why frames that already fit
    /// are left alone.
    ///
    /// The request is posted to the run loop instead of being sent from the GUI callback:
    /// the CLAP adapter guards `set_scale`, `set_parent` and `set_size` with one mutation
    /// guard, so a host answering `request_resize` with a synchronous `set_size` would be
    /// rejected while the callback is still running. The VST3 wrapper also applies the
    /// `getSize` rectangle in `attached()` after `set_parent`, which would overwrite a
    /// request made before `attached()` returns.
    fn post_resize_if_clipped_at_active_scale(&self, generation: u64) {
        // Checked here as well as in the posted task so hosts and platforms whose frame
        // already fits never queue run-loop work.
        if self.layout.host_size_to_unclip_at_active_scale().is_none() {
            return;
        }
        let layout = self.layout.clone();
        let runtime = self.runtime.clone();
        let post_result = RunLoop::post(move |_| {
            if matches!(
                attached_session(&runtime, generation),
                AttachedSession::Gone
            ) {
                return;
            }
            // The host may have resized the frame between the callback and this task.
            let Some(size) = layout.host_size_to_unclip_at_active_scale() else {
                return;
            };
            log::debug!(
                "wxp controller: requesting host resize for attached scale: width={}, height={}",
                size.width,
                size.height
            );
            let result = layout.request_host_resize(size, |size| {
                // Look the session up again: the host may destroy or recreate the editor while
                // `request_resize()` runs, and a stale session must not record its size.
                match attached_session(&runtime, generation) {
                    AttachedSession::Gone => Err(PluginError::InvalidState),
                    AttachedSession::Runtime(handle) => handle.set_size(size),
                    // The runtime reads the accepted size when its pending creation runs.
                    AttachedSession::RuntimePending => Ok(()),
                }
            });
            if let Err(error) = result {
                log::debug!("wxp controller: attached scale resize was not applied: {error:?}");
            }
        });
        if post_result.is_err() {
            log::warn!("wxp controller: attached scale resize could not be posted");
        }
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

fn is_cubase_vst3(host_context: &HostContext) -> bool {
    host_context.host().family == HostFamily::SteinbergCubase
        && host_context.plugin_format == PluginFormat::Vst3
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
    let (configuration, parent) = {
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
        session
            .parent_lease
            .as_ref()
            .ok_or(PluginError::InvalidState)?;
        let configuration = session.configuration;
        state.is_creating_runtime = true;
        state.creating_generation = Some(generation);
        state.pending_creation_generation = None;
        state.destroy_requested_while_creating = false;
        (configuration, parent)
    };

    log::debug!("wxp controller: posting runtime creation: generation={generation}");
    let factory_for_callback = factory.clone();
    let runtime_for_callback = runtime.clone();
    let layout_for_callback = layout.clone();
    let post_result = RunLoop::post(move |run_loop| {
        log::debug!("wxp controller: posted runtime creation started: generation={generation}");
        let result = create_runtime_on_gui_thread(
            run_loop,
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
            log::warn!("wxp controller: posted runtime creation latest set_size failed: {error:?}");
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
    if post_result.is_err() {
        log::warn!("wxp controller: runtime creation could not be posted: generation={generation}");
        clear_runtime_creation_after_post_failure(runtime.as_ref(), generation);
        return Err(PluginError::InvalidState);
    }
    Ok(())
}

/// State of the attached session a deferred task was scheduled for.
enum AttachedSession {
    /// The session was destroyed, replaced by a newer generation, or never got a parent.
    Gone,
    /// The parent is attached but the runtime is still being created.
    RuntimePending,
    Runtime(GuiRuntimeHandle),
}

fn attached_session(runtime: &Mutex<GuiRuntimeState>, generation: u64) -> AttachedSession {
    let state = runtime.lock();
    let Some(session) = state
        .session
        .as_ref()
        .filter(|session| session.generation == generation && session.parent.is_some())
    else {
        return AttachedSession::Gone;
    };
    match session.handle.clone() {
        Some(handle) => AttachedSession::Runtime(handle),
        None => AttachedSession::RuntimePending,
    }
}

fn clear_runtime_creation_after_post_failure(runtime: &Mutex<GuiRuntimeState>, generation: u64) {
    let mut state = runtime.lock();
    if state.creating_generation != Some(generation) {
        return;
    }
    state.is_creating_runtime = false;
    state.creating_generation = None;
    if state.pending_creation_generation == Some(generation) {
        state.pending_creation_generation = None;
    }
    state.destroy_requested_while_creating = false;
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
    run_loop: &RunLoopLocal,
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
            RuntimeCreationRequest {
                configuration,
                size,
                parent,
                scale,
                generation,
            },
            run_loop,
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
        RuntimeCreationRequest {
            configuration,
            size,
            parent,
            scale,
            generation,
        },
        run_loop,
    )
}

fn create_runtime_after_wait(
    factory: &dyn WxpGuiFactory,
    runtime: &Mutex<GuiRuntimeState>,
    request: RuntimeCreationRequest,
    run_loop: &RunLoopLocal,
) -> PluginResult<GuiRuntimeHandle> {
    let parent = request.parent.to_parent_window_handle()?;
    log::debug!("wxp controller: parent handle converted");
    let handle = match create_gui_runtime_handle(
        |run_loop| {
            factory.create_gui_runtime(run_loop, request.configuration, request.size, parent)
        },
        run_loop,
    ) {
        Ok(handle) => handle,
        Err(error) => {
            let mut state = runtime.lock();
            if state.creating_generation == Some(request.generation) {
                state.is_creating_runtime = false;
                state.creating_generation = None;
                state.pending_creation_generation = None;
                state.destroy_requested_while_creating = false;
            }
            return Err(error);
        }
    };
    log::debug!("wxp controller: runtime handle created");
    if finish_runtime_creation_requested_destroy(runtime, request.generation) {
        log::debug!(
            "wxp controller: destroying newly created runtime after stale/deferred destroy"
        );
        handle.destroy();
        runtime.lock().last_runtime_destroyed_at = Some(Instant::now());
        return Err(PluginError::InvalidState);
    }
    if let Err(error) = handle.set_scale(request.scale) {
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
    Some((layout.accepted_size(), layout.scale()))
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
    Some((session.visible, layout.accepted_size(), layout.scale()))
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

impl PluginGuiApiSupportExtension for WxpGuiController {
    fn is_api_supported(&self, api: GuiApi, is_floating: bool) -> bool {
        !is_floating && api == default_gui_api()
    }
}

impl PluginGuiQueryExtension for WxpGuiController {
    fn preferred_api(&self) -> Option<GuiConfig> {
        Some(default_gui_configuration())
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
        Ok(self.layout.clamp_host_size(size))
    }
}

impl PluginGuiMainThreadExtension for WxpGuiController {
    fn create(&self, configuration: GuiConfig) -> PluginResult<()> {
        log::debug!("wxp controller: create called: configuration={configuration:?}");
        if !PluginGuiApiSupportExtension::is_api_supported(
            self,
            configuration.api,
            configuration.is_floating,
        ) {
            log::debug!("wxp controller: create rejected unsupported configuration");
            return Err(PluginError::Message("unsupported GUI configuration".into()));
        }
        self.destroy_gui_session();
        // Re-derive the suggested host size from the logical baseline so reopening an editor
        // after an attached scale change starts at the expected size even if the host skips
        // calling `set_scale` again before `get_size`.
        self.layout.prepare_for_new_session();
        let generation = {
            let mut state = self.runtime.lock();
            state.generation = state.generation.wrapping_add(1);
            let generation = state.generation;
            state.session = Some(GuiSession {
                generation,
                configuration,
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
        if !scale.is_finite() || scale <= 0.0 {
            return Err(PluginError::Message("invalid GUI scale".into()));
        }
        let (handle, scale, attached_generation) = {
            let state = self.runtime.lock();
            let session = state.session.as_ref();
            let parent = session.and_then(|session| session.parent);
            let scale = self.correct_host_scale(scale, parent);
            // The host can query get_size before it creates the parent. Scale the
            // suggested size then. An attached host's frame stays governed by set_size;
            // only a frame that would clip at the new scale gets a resize request below.
            if parent.is_none() {
                self.layout.set_scale_before_attach(scale);
            } else {
                self.layout.set_scale_while_attached(scale);
            }
            let handle = session.and_then(|session| session.handle.clone());
            let attached_generation = session
                .filter(|session| session.parent.is_some())
                .map(|session| session.generation);
            (handle, scale, attached_generation)
        };
        if let Some(handle) = handle {
            handle.set_scale(scale)?;
        }
        if let Some(generation) = attached_generation {
            self.post_resize_if_clipped_at_active_scale(generation);
        }
        log::debug!("wxp controller: set_scale completed");
        Ok(())
    }

    fn set_size(&self, requested_size: GuiSize) -> PluginResult<()> {
        let size = self.layout.clamp_host_size(requested_size);
        let previous_size = self.layout.accepted_size();
        let size_changed = previous_size.width != size.width || previous_size.height != size.height;
        let handle = self
            .runtime
            .lock()
            .session
            .as_ref()
            .and_then(|session| session.handle.clone());

        // Some hosts repeatedly send the same size until the editor window settles.
        // Re-applying identical bounds does not violate the contract but adds redundant
        // geometry processing to the child view, making resize drags feel laggy. Size is
        // still recorded below so re-entrant `request_resize()` detection can observe
        // host callbacks.
        if let Some(handle) = handle {
            if size_changed {
                handle.set_size(size)?;
            }
            if self.should_async_resync_bounds_after_set_size() {
                // Cubase 10 on macOS can resize the host-owned editor window after
                // delivering `set_size`, leaving the embedded child view one geometry
                // step behind. Re-posting the latest accepted size lets the host finish
                // its adjustment before wxp reapplies child bounds.
                log::debug!(
                    "wxp controller: scheduling Cubase VST3 async bounds resync: width={}, height={}",
                    size.width,
                    size.height
                );
                handle.post_set_size(size)?;
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
        // If `set_scale` arrived before `set_parent`, Cubase VST3 scale correction must
        // wait until the native parent window is known. Update the shared layout scale
        // without rescaling the already-created host frame.
        let corrected_scale = self.correct_host_scale(self.layout.scale(), Some(parent));
        self.layout.set_scale_while_attached(corrected_scale);
        if let Some(parent_lease) = parent_lease {
            session.parent_lease = Some(parent_lease);
        }
        drop(state);
        // Only accept the parent and schedule WebView creation here. Deferring actual
        // creation outside the host lifecycle callback avoids create/destroy re-entry.
        // On failure, leave the session without a runtime and let a subsequent
        // show/set_parent reschedule it.
        self.schedule_runtime_creation(generation)?;
        // The Cubase scale correction above can leave the frame negotiated by get_size
        // below the minimum at the corrected scale.
        self.post_resize_if_clipped_at_active_scale(generation);
        log::debug!("wxp controller: set_parent completed");
        Ok(())
    }

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

impl PluginGuiExtension for WxpGuiController {
    fn api_support(&self) -> &(dyn PluginGuiApiSupportExtension + Send + Sync) {
        self
    }

    fn query(&self) -> &(dyn PluginGuiQueryExtension + Send + Sync) {
        self
    }

    fn main_thread(&self) -> &dyn PluginGuiMainThreadExtension {
        self
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
        // Hosts and validators call this on every teardown path, so logging the no-op case only
        // floods debug output without telling anything about a real session.
        false
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

#[cfg(test)]
mod tests;
