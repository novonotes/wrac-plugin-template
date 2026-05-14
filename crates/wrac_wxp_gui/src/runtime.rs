use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::ThreadId;

use novonotes_run_loop::{RunLoop, RunLoopSender};
use parking_lot::Mutex;
use wrac_clap_adapter::{GuiConfiguration, GuiSize, PluginError, PluginResult};

use crate::window::ParentWindowHandle;

thread_local! {
    // WebView などの native GUI object は、生成 thread 以外へ移動できない実装が多い。
    // `WxpGuiController` は Send/Sync な `PluginCore` の中に置かれるため、実体は TLS に閉じ込める。
    static GUI_RUNTIMES: RefCell<HashMap<u64, GuiRuntimeEntry>> = RefCell::new(HashMap::new());
}

static NEXT_GUI_ID: AtomicU64 = AtomicU64::new(1);
// この helper は template 用の割り切りとして単一 UI thread を前提にする。複数 UI thread
// まで受け入れるには runtime storage と run loop ownership を per-thread に再設計する必要がある。
static GUI_THREAD_STATE: Mutex<GuiThreadState> = Mutex::new(GuiThreadState {
    owner: None,
    ref_count: 0,
});

struct GuiThreadState {
    owner: Option<ThreadId>,
    ref_count: usize,
}

struct GuiRuntimeEntry {
    runtime: Box<dyn WxpGuiRuntime>,
    // runtime が TLS から remove されるまで run loop を生かす。handle 側で手動 release
    // すると、runtime drop 中に timer や WebView teardown が run loop を必要とする場合に
    // 順序を間違えやすいので、entry の lifetime に lease を結び付ける。
    _lease: GuiThreadLease,
}

/// GUI thread の run loop 参照を表す RAII token。
///
/// TODO: `novonotes_run_loop` 側に transactional な guard API が入ったら、この型は
/// その thin wrapper に寄せる。現状の `RunLoop::init()` は失敗時 rollback が API 契約に
/// なっていないため、ここではローカル state を進めない範囲で防御する。
pub(crate) struct GuiThreadLease;

/// UI thread が所有する実際の WebView runtime。
///
/// `Send` / `Sync` を要求しないのは意図的です。native GUI object は作成 thread に縛られる
/// ため、`GuiRuntimeHandle` から run loop に戻して操作する。
pub trait WxpGuiRuntime: 'static {
    fn set_scale(&mut self, scale: f64) -> PluginResult<()>;
    fn set_size(&mut self, size: GuiSize) -> PluginResult<()>;
    fn show(&mut self) -> PluginResult<()> {
        Ok(())
    }
    fn hide(&mut self) -> PluginResult<()> {
        Ok(())
    }
}

/// 製品固有 runtime を作る factory。
///
/// factory 自体は `PluginCore` 内に保持されるため `Send + Sync` を要求するが、返す
/// runtime は UI thread の TLS に置くので `Send` を要求しない。
pub trait WxpGuiFactory: Send + Sync + 'static {
    fn create_gui_runtime(
        &self,
        configuration: GuiConfiguration,
        initial_size: GuiSize,
        parent: ParentWindowHandle,
    ) -> PluginResult<Box<dyn WxpGuiRuntime>>;
}

impl<F> WxpGuiFactory for F
where
    F: Fn(GuiConfiguration, GuiSize, ParentWindowHandle) -> PluginResult<Box<dyn WxpGuiRuntime>>
        + Send
        + Sync
        + 'static,
{
    fn create_gui_runtime(
        &self,
        configuration: GuiConfiguration,
        initial_size: GuiSize,
        parent: ParentWindowHandle,
    ) -> PluginResult<Box<dyn WxpGuiRuntime>> {
        self(configuration, initial_size, parent)
    }
}

#[derive(Clone)]
pub(crate) struct GuiRuntimeHandle {
    id: u64,
    sender: RunLoopSender,
}

pub(crate) fn create_gui_runtime_handle(
    create: impl FnOnce() -> PluginResult<Box<dyn WxpGuiRuntime>>,
) -> PluginResult<GuiRuntimeHandle> {
    let lease = GuiThreadLease::acquire()?;
    // `create` が失敗した場合、lease はここで drop される。失敗した runtime 作成が
    // GUI thread ref を残さないことを型の drop 順で保証する。
    match create() {
        Ok(runtime) => Ok(insert_gui_runtime(runtime, lease)),
        Err(error) => Err(error),
    }
}

impl GuiRuntimeHandle {
    pub(crate) fn destroy(self) {
        let id = self.id;
        self.sender.send_and_wait(move || {
            GUI_RUNTIMES.with(|runtimes| {
                runtimes.borrow_mut().remove(&id);
            });
        });
    }

    pub(crate) fn set_scale(&self, scale: f64) -> PluginResult<()> {
        let id = self.id;
        self.sender.send_and_wait(move || {
            GUI_RUNTIMES.with(|runtimes| {
                let mut runtimes = runtimes.borrow_mut();
                let entry = runtimes.get_mut(&id).ok_or(PluginError::InvalidState)?;
                entry.runtime.set_scale(scale)
            })
        })
    }

    pub(crate) fn set_size(&self, size: GuiSize) -> PluginResult<()> {
        let id = self.id;
        self.sender.send_and_wait(move || {
            GUI_RUNTIMES.with(|runtimes| {
                let mut runtimes = runtimes.borrow_mut();
                let entry = runtimes.get_mut(&id).ok_or(PluginError::InvalidState)?;
                entry.runtime.set_size(size)
            })
        })
    }

    pub(crate) fn show(&self) -> PluginResult<()> {
        let id = self.id;
        self.sender.send_and_wait(move || {
            GUI_RUNTIMES.with(|runtimes| {
                let mut runtimes = runtimes.borrow_mut();
                let entry = runtimes.get_mut(&id).ok_or(PluginError::InvalidState)?;
                entry.runtime.show()
            })
        })
    }

    pub(crate) fn hide(&self) -> PluginResult<()> {
        let id = self.id;
        self.sender.send_and_wait(move || {
            GUI_RUNTIMES.with(|runtimes| {
                let mut runtimes = runtimes.borrow_mut();
                let entry = runtimes.get_mut(&id).ok_or(PluginError::InvalidState)?;
                entry.runtime.hide()
            })
        })
    }
}

fn insert_gui_runtime(runtime: Box<dyn WxpGuiRuntime>, lease: GuiThreadLease) -> GuiRuntimeHandle {
    let id = NEXT_GUI_ID.fetch_add(1, Ordering::Relaxed);
    GUI_RUNTIMES.with(|runtimes| {
        runtimes.borrow_mut().insert(
            id,
            GuiRuntimeEntry {
                runtime,
                _lease: lease,
            },
        );
    });
    GuiRuntimeHandle {
        id,
        sender: RunLoop::sender(),
    }
}

impl GuiThreadLease {
    pub(crate) fn acquire() -> PluginResult<Self> {
        let current_thread = std::thread::current().id();
        let mut gui_thread = GUI_THREAD_STATE.lock();
        match gui_thread.owner {
            Some(owner_thread) if owner_thread != current_thread => {
                return Err(PluginError::UnsupportedHostGuiThreadingModel);
            }
            Some(_) | None => {}
        }

        if RunLoop::init().is_err() {
            return Err(PluginError::UnsupportedHostGuiThreadingModel);
        }

        // owner は `RunLoop::init()` 成功後にだけ進める。依存 crate の init が失敗時に
        // 完全 rollback する保証はまだないため、少なくともこちらの SoT は汚さない。
        gui_thread.owner = Some(current_thread);
        gui_thread.ref_count += 1;
        Ok(Self)
    }
}

impl Drop for GuiThreadLease {
    fn drop(&mut self) {
        RunLoop::deinit();
        let mut gui_thread = GUI_THREAD_STATE.lock();
        debug_assert!(gui_thread.ref_count > 0);
        gui_thread.ref_count = gui_thread.ref_count.saturating_sub(1);
        if gui_thread.ref_count == 0 {
            // 最後の runtime だけでなく `set_parent()` 由来の thread 固定も解放された時点で、
            // 次の GUI session が別 host window から来ることを許可する。
            gui_thread.owner = None;
        }
    }
}

pub(crate) fn is_gui_thread() -> bool {
    RunLoop::is_run_loop_thread()
}
