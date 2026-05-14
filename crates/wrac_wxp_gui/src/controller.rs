use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use wrac_clap_adapter::{
    ClapWindow, GuiApi, GuiConfiguration, GuiResizeHints, GuiSize, HostGuiResizeRequester,
    PluginError, PluginGui, PluginResult,
};
use wxp::{WebViewRef, dpi::LogicalSize};

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
    factory: Box<dyn WxpGuiFactory>,
    layout: Arc<HostGuiLayout>,
    scale: Arc<Mutex<f64>>,
    runtime: Mutex<GuiRuntimeState>,
}

struct HostGuiLayout {
    // CLAP layout queries read this without entering the GUI runtime.
    // The accepted size is the controller's host contract, not copied runtime state.
    accepted_size: AtomicGuiSize,
    limits: GuiSizeLimits,
    resize_policy: GuiResizePolicy,
}

struct GuiRuntimeState {
    session: Option<GuiSession>,
}

// CLAP の `create()` は GUI session の開始だが、embedded WebView の native child は
// parent handle がないと作れない。session と runtime を分けることで、`create()` 後の
// size/scale query には答えつつ、parent が来るまで native object 作成を遅延できる。
struct GuiSession {
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
            factory: Box::new(factory),
            layout: resize_handle.layout.clone(),
            scale: resize_handle.scale.clone(),
            runtime: Mutex::new(GuiRuntimeState { session: None }),
        }
    }

    fn destroy_gui_session(&self) {
        let session = { self.runtime.lock().session.take() };
        drop_session(session);
    }

    fn create_runtime(
        &self,
        configuration: GuiConfiguration,
        size: GuiSize,
        parent: StoredParentWindow,
        scale: f64,
    ) -> PluginResult<GuiRuntimeHandle> {
        let parent = parent.to_parent_window_handle()?;
        let handle = create_gui_runtime_handle(|| {
            self.factory.create_gui_runtime(configuration, size, parent)
        })?;
        if let Err(error) = handle.set_scale(scale) {
            handle.destroy();
            return Err(error);
        }
        Ok(handle)
    }
}

impl HostGuiLayout {
    fn new(size: GuiSize, limits: GuiSizeLimits, resize_policy: GuiResizePolicy) -> Self {
        let size = clamp_size_with_limits(size, limits);
        Self {
            accepted_size: AtomicGuiSize::new(size),
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
        web_view: &WebViewRef,
        host_gui_resize_requester: &dyn HostGuiResizeRequester,
    ) -> PluginResult<GuiSize> {
        let logical_size = self.layout.clamp_logical_size(requested);
        let gui_size = GuiSize {
            width: logical_size.width as u32,
            height: logical_size.height as u32,
        };
        host_gui_resize_requester.request_resize(gui_size)?;

        self.layout.store_accepted_size(gui_size);
        let scale = *self.scale.lock();
        web_view
            .set_bounds(DpiConverter::new(scale).create_webview_bounds(logical_size))
            .map_err(|_| PluginError::Message("failed to resize webview"))?;
        Ok(gui_size)
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
        if !self.is_api_supported(configuration.api, configuration.is_floating) {
            return Err(PluginError::Message("unsupported GUI configuration"));
        }
        self.destroy_gui_session();
        let scale = *self.scale.lock();
        self.runtime.lock().session = Some(GuiSession {
            configuration,
            scale,
            parent: None,
            parent_lease: None,
            handle: None,
            // 一部 wrapper は embedded view を parent に付けた時点で表示扱いにし、`show()`
            // を呼ばない。初回 parent attach は表示状態として扱い、明示的な `hide()` を優先する。
            visible: true,
        });
        Ok(())
    }

    fn destroy(&self) {
        self.destroy_gui_session();
    }

    fn set_scale(&self, scale: f64) -> PluginResult<()> {
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
        Ok(())
    }

    fn get_size(&self) -> PluginResult<GuiSize> {
        Ok(self.layout.accepted_size())
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
        let handle = {
            self.runtime
                .lock()
                .session
                .as_ref()
                .and_then(|session| session.handle.clone())
        };
        if let Some(handle) = handle {
            handle.set_size(size)?;
        }
        self.layout.store_accepted_size(size);
        Ok(())
    }

    fn set_parent(&self, window: ClapWindow) -> PluginResult<()> {
        let parent = StoredParentWindow::from_clap_window(window);
        let needs_parent_lease = {
            let state = self.runtime.lock();
            let session = state.session.as_ref().ok_or(PluginError::InvalidState)?;
            if session.parent.is_some() {
                if !is_gui_thread() {
                    return Err(PluginError::UnsupportedHostGuiThreadingModel);
                }
                false
            } else {
                true
            }
        };

        let parent_lease = needs_parent_lease
            .then(GuiThreadLease::acquire)
            .transpose()?;

        let old_handle = {
            let mut state = self.runtime.lock();
            let session = state.session.as_mut().ok_or(PluginError::InvalidState)?;
            // parent handle が変わると、既存 child WebView を安全に reparent できる保証が
            // wxp/wry 側にない。古い runtime を先に落として、新しい parent 上に作り直す。
            session.handle.take()
        };
        if let Some(handle) = old_handle {
            handle.destroy();
        }

        let (configuration, size, scale, visible) = {
            let state = self.runtime.lock();
            let session = state.session.as_ref().ok_or(PluginError::InvalidState)?;
            (
                session.configuration,
                self.layout.accepted_size(),
                session.scale,
                session.visible,
            )
        };
        // CLAP の GUI spec 上は `show()` が表示開始の合図だが、clap-wrapper の AUv2
        // 経由では Logic Pro が embedded view を parent に付けた時点で表示扱いにし、
        // `show()` を呼ばない経路がある。WebView 作成は parent が必要なのでここで行う。
        let handle = match self.create_runtime(configuration, size, parent, scale) {
            Ok(handle) => handle,
            Err(error) => {
                if needs_parent_lease {
                    let mut state = self.runtime.lock();
                    if let Some(session) = &mut state.session {
                        session.parent = None;
                        session.parent_lease = None;
                    }
                }
                // parent attach 失敗後に parent lease だけ残ると、次回 GUI session が別 thread
                // から来た時に不要に拒否される。失敗した attach は状態を進めない。
                drop(parent_lease);
                return Err(error);
            }
        };
        if !visible {
            if let Err(error) = handle.hide() {
                // hidden 初期化に失敗した runtime は、session に入れず破棄する。半端な handle を
                // 残すと以降の `show()`/`destroy()` が失敗済み WebView を操作してしまう。
                handle.destroy();
                drop(parent_lease);
                return Err(error);
            }
        }
        let mut state = self.runtime.lock();
        let session = state.session.as_mut().ok_or(PluginError::InvalidState)?;
        session.parent = Some(parent);
        if let Some(parent_lease) = parent_lease {
            session.parent_lease = Some(parent_lease);
        }
        session.handle = Some(handle);
        Ok(())
    }

    fn set_transient(&self, _window: ClapWindow) -> PluginResult<()> {
        Err(PluginError::Message("floating GUI is unsupported"))
    }

    fn suggest_title(&self, _title: &str) {}

    fn show(&self) -> PluginResult<()> {
        let action = {
            let state = self.runtime.lock();
            let session = state.session.as_ref().ok_or(PluginError::InvalidState)?;
            if let Some(handle) = session.handle.clone() {
                ShowAction::ShowExisting(handle)
            } else {
                let parent = session.parent.ok_or(PluginError::InvalidState)?;
                ShowAction::Create {
                    configuration: session.configuration,
                    size: self.layout.accepted_size(),
                    parent,
                    scale: session.scale,
                }
            }
        };

        match action {
            ShowAction::ShowExisting(handle) => {
                handle.show()?;
                if let Some(session) = &mut self.runtime.lock().session {
                    session.visible = true;
                }
                Ok(())
            }
            ShowAction::Create {
                configuration,
                size,
                parent,
                scale,
            } => {
                let handle = self.create_runtime(configuration, size, parent, scale)?;
                handle.show()?;
                let mut state = self.runtime.lock();
                let session = state.session.as_mut().ok_or(PluginError::InvalidState)?;
                session.handle = Some(handle);
                session.visible = true;
                Ok(())
            }
        }
    }

    fn hide(&self) -> PluginResult<()> {
        let handle = {
            let state = self.runtime.lock();
            let session = state.session.as_ref().ok_or(PluginError::InvalidState)?;
            session.handle.clone()
        };
        if let Some(handle) = handle {
            handle.hide()?;
        }
        if let Some(session) = &mut self.runtime.lock().session {
            session.visible = false;
        }
        Ok(())
    }
}

fn drop_session(session: Option<GuiSession>) {
    if let Some(mut session) = session {
        if let Some(handle) = session.handle.take() {
            handle.destroy();
        }
        // runtime drop が終わってから parent lease を解放する。timer stop や WebView teardown が
        // run loop 上で完了する前に owner thread を解放しないため。
        drop(session.parent_lease.take());
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
    ShowExisting(GuiRuntimeHandle),
    Create {
        configuration: GuiConfiguration,
        size: GuiSize,
        parent: StoredParentWindow,
        scale: f64,
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
