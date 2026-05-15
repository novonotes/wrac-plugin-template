use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use novonotes_run_loop::RunLoop;
use parking_lot::Mutex;
use wrac_clap_adapter::{
    ClapWindow, GuiApi, GuiConfiguration, GuiResizeHints, GuiSize, HostGuiResizeRequester,
    PluginError, PluginGui, PluginResult,
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

/// wxp WebView runtime を [`PluginGui`] として公開する Send/Sync controller。
///
/// 実 runtime は UI thread の TLS 上に保持し、この型は CLAP instance から共有される
/// [`PluginGui`] handle として GUI lifecycle callback を受ける。現在は host parent に
/// child view として貼る embedded GUI のみ対応し、floating window は拒否する。
pub struct WxpGuiController {
    factory: Arc<dyn WxpGuiFactory>,
    layout: Arc<HostGuiLayout>,
    scale: Arc<Mutex<f64>>,
    runtime: Arc<Mutex<GuiRuntimeState>>,
}

struct HostGuiLayout {
    // CLAP layout query が GUI runtime に入らず読む host 契約値 (runtime の複製ではない)。
    accepted_size: AtomicGuiSize,
    // 一部 wrapper は `request_resize()` の中から `set_size()` を再入で呼び (戻り値は
    // false でも)。この revision で、runtime lock も戻り値の推測も無しに
    // 「host が確定したサイズ」を request 側が検出できる。
    accepted_size_revision: AtomicU64,
    limits: GuiSizeLimits,
    resize_policy: GuiResizePolicy,
}

struct GuiRuntimeState {
    session: Option<GuiSession>,
    // editor の開閉を連打すると create/set_parent/show/destroy が高速に来る。
    // WebView 作成は GUI run loop に post するので callback が元の CLAP 呼び出し
    // 後に届く。generation で遅れた callback が stale session を検出し、閉じた
    // editor に繋がず作りかけ runtime を畳めるようにする。
    generation: u64,
    last_runtime_destroyed_at: Option<Instant>,
    // Windows host (特に Ableton Live) は前回 teardown 中に editor を再生成し得る。
    // child WebView 作成を single-flight にし、最新要求 generation だけ覚える。
    is_creating_runtime: bool,
    creating_generation: Option<u64>,
    pending_creation_generation: Option<u64>,
    destroy_requested_while_creating: bool,
}

// runtime 破棄後の静止期間。これが無いと editor 連打時、前回 teardown 完了前に
// 次の child WebView を要求してしまう。
const WEBVIEW_RECREATE_QUIET_PERIOD: Duration = Duration::from_millis(500);

// CLAP の `create()` は GUI session の開始だが、embedded WebView の native child は
// parent handle がないと作れない。session と runtime を分けることで、`create()` 後の
// size/scale query には答えつつ、parent が来るまで native object 作成を遅延できる。
struct GuiSession {
    generation: u64,
    configuration: GuiConfiguration,
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
    // CLAP GUI callback とは意図的に非同期にする。inline で WebView を作ると host
    // lifecycle の再入が起きやすい。run loop に post すれば、作成の直列化・保留中の
    // visibility/size 適用・stale generation 破棄を 1 か所に集約できる。
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
    RunLoop::sender().send(move || {
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
    configuration: GuiConfiguration,
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
    configuration: GuiConfiguration,
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

    fn clamp_logical_size(&self, size: LogicalSize<f64>) -> LogicalSize<f64> {
        LogicalSize::new(
            size.width
                .round()
                .clamp(self.limits.min.width as f64, self.limits.max.width as f64),
            size.height
                .round()
                .clamp(self.limits.min.height as f64, self.limits.max.height as f64),
        )
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

    pub fn request_resize(
        &self,
        requested: LogicalSize<f64>,
        web_view: &WebViewDispatch,
        host_gui_resize_requester: &dyn HostGuiResizeRequester,
    ) -> PluginResult<GuiSize> {
        let logical_size = self.layout.clamp_logical_size(requested);
        let gui_size = GuiSize {
            width: logical_size.width as u32,
            height: logical_size.height as u32,
        };

        let previous_revision = self.layout.accepted_size_revision();
        let resize_result = host_gui_resize_requester.request_resize(gui_size);
        let current_revision = self.layout.accepted_size_revision();

        // Logic の AUv2 wrapper は `request_resize()` 中に NSView frame を適用し
        // `set_size()` を再入で呼んだ上で CLAP には false を返す。この再入 `set_size()`
        // を真実とみなす。ここで楽観的に WebView を resize すると host と geometry を
        // 奪い合い、grip ドラッグ中に視覚的な揺れが出る。
        if current_revision != previous_revision {
            return Ok(self.layout.accepted_size());
        }

        match resize_result {
            Ok(()) => {
                // request を受理しても即 `set_size()` を呼ばない host がある。その場合は
                // plugin 側で WebView を更新し、任意 callback を待たず resize する。
                // command には native owner ではなく `WebViewDispatch` を渡す。閉じかけ
                // editor の寿命を延ばさずに command handler から resize できる。
                let scale = *self.scale.lock();
                web_view
                    .post_set_bounds(DpiConverter::new(scale).create_webview_bounds(logical_size))
                    .map_err(|_| PluginError::Message("failed to resize webview"))?;
                self.layout.store_accepted_size(gui_size);
                Ok(gui_size)
            }
            Err(error) => {
                // 本当の拒否は上の AUv2 再入ケースとは別。child WebView を投機的に
                // 動かして後で戻すより、host 確定済みの最後のサイズを保つ。
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

impl PluginGui for WxpGuiController {
    fn is_api_supported(&self, api: GuiApi, is_floating: bool) -> bool {
        !is_floating && api == default_gui_api()
    }

    fn preferred_api(&self) -> Option<GuiConfiguration> {
        Some(default_gui_configuration())
    }

    fn create(&self, configuration: GuiConfiguration) -> PluginResult<()> {
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
                // 一部 wrapper は embedded view を parent に付けた時点で表示扱いにし、`show()`
                // を呼ばない。初回 parent attach は表示状態として扱い、明示的な `hide()` を優先する。
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

        // editor window が落ち着くまで同じ size を再送する host がある。同一 bounds の
        // 再適用は契約を変えないが、child view に冗長な geometry 処理を与え、resize
        // ドラッグが pointer に遅れて感じる。それでも下で size は記録し、再入
        // `request_resize()` 検出が host callback を観測できるようにする。
        if let Some(handle) = handle {
            if size_changed {
                handle.set_size(size)?;
            }
        }
        self.layout.store_accepted_size(size);
        Ok(())
    }

    fn set_parent(&self, window: ClapWindow) -> PluginResult<()> {
        log::debug!("wxp controller: set_parent called");
        let parent = StoredParentWindow::from_clap_window(window);
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
            // parent handle が変わると、既存 child WebView を安全に reparent できる保証が
            // wxp/wry 側にない。古い runtime を先に落として、新しい parent 上に作り直す。
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
        // ここは parent を受理して WebView 作成を予約するだけ。実作成を host
        // lifecycle callback から外すことで create/destroy 再入を避ける。失敗時は
        // log だけ残し runtime 無しの session にして、後の show/set_parent で再予約させる。
        self.schedule_runtime_creation(generation)?;
        log::debug!("wxp controller: set_parent completed");
        Ok(())
    }

    fn set_transient(&self, _window: ClapWindow) -> PluginResult<()> {
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
        // runtime drop が終わってから parent lease を解放する。timer stop や WebView teardown が
        // run loop 上で完了する前に owner thread を解放しないため。
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

fn default_gui_configuration() -> GuiConfiguration {
    GuiConfiguration {
        api: default_gui_api(),
        is_floating: false,
    }
}
