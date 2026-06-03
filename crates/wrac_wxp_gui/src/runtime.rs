use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::ThreadId;

use novonotes_run_loop::{RunLoop, RunLoopGuard, RunLoopLocal};
use parking_lot::Mutex;
use wrac_clap_adapter::{GuiConfig, GuiSize, PluginError, PluginResult};

use crate::window::ParentWindowHandle;

thread_local! {
    // Native GUI objects such as WebViews are typically bound to the thread that created them.
    // `WxpGuiController` lives inside a Send/Sync `PluginCore`, so runtimes are confined to TLS.
    static GUI_RUNTIMES: RefCell<HashMap<u64, GuiRuntimeEntry>> = RefCell::new(HashMap::new());
    // Keep the `!Send` run-loop guard on the GUI thread. `GuiThreadLease` is only a
    // cross-thread token; it releases this guard by dispatching back to the owner thread.
    static GUI_RUN_LOOP_GUARD: RefCell<Option<RunLoopGuard>> = const { RefCell::new(None) };
}

static NEXT_GUI_ID: AtomicU64 = AtomicU64::new(1);
// This helper assumes a single UI thread — an acceptable simplification for a template.
// Supporting multiple UI threads would require redesigning runtime storage and run loop
// ownership on a per-thread basis.
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
    // Keep the run loop alive until the runtime is removed from TLS. Releasing the lease
    // manually from the handle side is error-prone when timer teardown or WebView cleanup
    // still needs the run loop during the runtime's drop, so tie the lease lifetime to
    // the entry's.
    _lease: GuiThreadLease,
}

/// RAII token representing a reference to the GUI thread's run loop.
///
/// The token is `Send + Sync` because `WxpGuiController` is shared with host callbacks,
/// but the `!Send` [`RunLoopGuard`] itself stays in GUI-thread TLS. Dropping this token
/// from another host thread blocks until the reference has been released on the owning
/// GUI thread.
pub(crate) struct GuiThreadLease {
    owner: ThreadId,
    is_active: bool,
}

/// The actual WebView runtime owned by the UI thread.
///
/// `Send` / `Sync` are intentionally not required: implementations may hold
/// GUI-thread-owned toolkit state such as native WebViews, `Rc`, or `RefCell`.
/// Host-facing thread defense belongs in [`WxpGuiController`](crate::WxpGuiController).
pub trait WxpGuiRuntime: 'static {
    fn set_scale(&mut self, scale: f64) -> PluginResult<()>;
    fn set_size(&mut self, size: GuiSize) -> PluginResult<()>;
    fn show(&mut self, _run_loop: &RunLoopLocal) -> PluginResult<()> {
        Ok(())
    }
    fn hide(&mut self) -> PluginResult<()> {
        Ok(())
    }
}

/// Factory that creates a product-specific GUI runtime.
///
/// The factory itself is `Send + Sync` because it is held inside `PluginCore`, but the
/// runtime it returns does not need to be `Send` — it lives in the UI thread's TLS.
/// Runtime creation may allocate and touch native GUI APIs; it is not realtime-safe.
pub trait WxpGuiFactory: Send + Sync + 'static {
    fn create_gui_runtime(
        &self,
        run_loop: &RunLoopLocal,
        configuration: GuiConfig,
        initial_size: GuiSize,
        parent: ParentWindowHandle,
    ) -> PluginResult<Box<dyn WxpGuiRuntime>>;
}

impl<F> WxpGuiFactory for F
where
    F: for<'a> Fn(
            &'a RunLoopLocal,
            GuiConfig,
            GuiSize,
            ParentWindowHandle,
        ) -> PluginResult<Box<dyn WxpGuiRuntime>>
        + Send
        + Sync
        + 'static,
{
    fn create_gui_runtime(
        &self,
        run_loop: &RunLoopLocal,
        configuration: GuiConfig,
        initial_size: GuiSize,
        parent: ParentWindowHandle,
    ) -> PluginResult<Box<dyn WxpGuiRuntime>> {
        self(run_loop, configuration, initial_size, parent)
    }
}

#[derive(Clone)]
pub(crate) struct GuiRuntimeHandle {
    id: u64,
}

pub(crate) fn create_gui_runtime_handle(
    create: impl FnOnce(&RunLoopLocal) -> PluginResult<Box<dyn WxpGuiRuntime>>,
    run_loop: &RunLoopLocal,
) -> PluginResult<GuiRuntimeHandle> {
    log::debug!("wxp runtime: acquiring GUI thread lease");
    let lease = GuiThreadLease::acquire()?;
    // If `create` fails, the lease is dropped here. Drop order guarantees that a failed
    // runtime creation leaves no GUI thread reference behind.
    match create(run_loop) {
        Ok(runtime) => {
            log::debug!("wxp runtime: factory returned runtime");
            Ok(insert_gui_runtime(runtime, lease))
        }
        Err(error) => {
            log::warn!("wxp runtime: factory failed: {error:?}");
            Err(error)
        }
    }
}

impl GuiRuntimeHandle {
    pub(crate) fn destroy(self) {
        let id = self.id;
        log::debug!("wxp runtime {id}: destroy requested");
        let _ = RunLoop::call(move |_| {
            log::debug!("wxp runtime {id}: removing runtime from GUI thread");
            GUI_RUNTIMES.with(|runtimes| {
                runtimes.borrow_mut().remove(&id);
            });
            log::debug!("wxp runtime {id}: removed runtime from GUI thread");
        });
        log::debug!("wxp runtime {id}: destroy completed");
    }

    pub(crate) fn set_scale(&self, scale: f64) -> PluginResult<()> {
        let id = self.id;
        log::debug!("wxp runtime {id}: set_scale requested: scale={scale}");
        RunLoop::call(move |_| {
            GUI_RUNTIMES.with(|runtimes| {
                let mut runtimes = runtimes.borrow_mut();
                let entry = runtimes.get_mut(&id).ok_or(PluginError::InvalidState)?;
                let result = entry.runtime.set_scale(scale);
                log::debug!("wxp runtime {id}: set_scale completed: result={result:?}");
                result
            })
        })
        .map_err(|_| PluginError::InvalidState)?
    }

    pub(crate) fn set_size(&self, size: GuiSize) -> PluginResult<()> {
        let id = self.id;
        log::debug!(
            "wxp runtime {id}: set_size requested: width={}, height={}",
            size.width,
            size.height
        );
        RunLoop::call(move |_| {
            GUI_RUNTIMES.with(|runtimes| {
                let mut runtimes = runtimes.borrow_mut();
                let entry = runtimes.get_mut(&id).ok_or(PluginError::InvalidState)?;
                let result = entry.runtime.set_size(size);
                log::debug!("wxp runtime {id}: set_size completed: result={result:?}");
                result
            })
        })
        .map_err(|_| PluginError::InvalidState)?
    }

    pub(crate) fn post_set_size(&self, size: GuiSize) -> PluginResult<()> {
        let id = self.id;
        log::debug!(
            "wxp runtime {id}: post_set_size requested: width={}, height={}",
            size.width,
            size.height
        );
        // Used when a host mutates the native parent after the synchronous callback.
        // Posting keeps the latest bounds tied to the runtime id while allowing stale
        // editor sessions to disappear without turning a harmless race into an error.
        RunLoop::post(move |_| {
            GUI_RUNTIMES.with(|runtimes| {
                let mut runtimes = runtimes.borrow_mut();
                let Some(entry) = runtimes.get_mut(&id) else {
                    log::debug!("wxp runtime {id}: post_set_size skipped missing runtime");
                    return;
                };
                let result = entry.runtime.set_size(size);
                log::debug!("wxp runtime {id}: post_set_size completed: result={result:?}");
            });
        })
        .map_err(|_| PluginError::InvalidState)
    }

    pub(crate) fn show(&self) -> PluginResult<()> {
        let id = self.id;
        log::debug!("wxp runtime {id}: show requested");
        RunLoop::call(move |run_loop| {
            GUI_RUNTIMES.with(|runtimes| {
                let mut runtimes = runtimes.borrow_mut();
                let entry = runtimes.get_mut(&id).ok_or(PluginError::InvalidState)?;
                let result = entry.runtime.show(run_loop);
                log::debug!("wxp runtime {id}: show completed: result={result:?}");
                result
            })
        })
        .map_err(|_| PluginError::InvalidState)?
    }

    pub(crate) fn hide(&self) -> PluginResult<()> {
        let id = self.id;
        log::debug!("wxp runtime {id}: hide requested");
        RunLoop::call(move |_| {
            GUI_RUNTIMES.with(|runtimes| {
                let mut runtimes = runtimes.borrow_mut();
                let entry = runtimes.get_mut(&id).ok_or(PluginError::InvalidState)?;
                let result = entry.runtime.hide();
                log::debug!("wxp runtime {id}: hide completed: result={result:?}");
                result
            })
        })
        .map_err(|_| PluginError::InvalidState)?
    }
}

fn insert_gui_runtime(runtime: Box<dyn WxpGuiRuntime>, lease: GuiThreadLease) -> GuiRuntimeHandle {
    let id = NEXT_GUI_ID.fetch_add(1, Ordering::Relaxed);
    log::debug!("wxp runtime {id}: inserting runtime on GUI thread");
    GUI_RUNTIMES.with(|runtimes| {
        runtimes.borrow_mut().insert(
            id,
            GuiRuntimeEntry {
                runtime,
                _lease: lease,
            },
        );
    });
    log::debug!("wxp runtime {id}: inserted runtime on GUI thread");
    GuiRuntimeHandle { id }
}

impl GuiThreadLease {
    pub(crate) fn acquire() -> PluginResult<Self> {
        let current_thread = std::thread::current().id();
        log::debug!("wxp GUI thread lease: acquire requested on thread {current_thread:?}");
        let mut gui_thread = GUI_THREAD_STATE.lock();
        match gui_thread.owner {
            Some(owner_thread) if owner_thread != current_thread => {
                log::debug!(
                    "wxp GUI thread lease: rejecting thread {current_thread:?}; owner is {owner_thread:?}"
                );
                return Err(PluginError::UnsupportedHostGuiThreadingModel);
            }
            Some(_) | None => {}
        }

        if gui_thread.ref_count == 0 {
            let guard = RunLoop::init().map_err(|_| {
                log::debug!("wxp GUI thread lease: RunLoop::init failed");
                PluginError::UnsupportedHostGuiThreadingModel
            })?;
            GUI_RUN_LOOP_GUARD.with(|stored_guard| {
                debug_assert!(stored_guard.borrow().is_none());
                *stored_guard.borrow_mut() = Some(guard);
            });
        }

        // Advance the owner only after `RunLoop::init()` succeeds.
        gui_thread.owner = Some(current_thread);
        gui_thread.ref_count += 1;
        log::debug!(
            "wxp GUI thread lease: acquired on thread {current_thread:?}; ref_count={}",
            gui_thread.ref_count
        );
        Ok(Self {
            owner: current_thread,
            is_active: true,
        })
    }
}

impl Drop for GuiThreadLease {
    fn drop(&mut self) {
        let current_thread = std::thread::current().id();
        log::debug!("wxp GUI thread lease: dropping on thread {current_thread:?}");
        if !self.is_active {
            return;
        }
        self.is_active = false;
        if current_thread == self.owner {
            release_gui_thread_lease();
        } else {
            log::debug!(
                "wxp GUI thread lease: dispatching release from thread {current_thread:?} to owner {:?}",
                self.owner
            );
            if RunLoop::call(move |_| release_gui_thread_lease()).is_err() {
                log::error!("wxp GUI thread lease: failed to release on owner thread");
            }
        }
    }
}

fn release_gui_thread_lease() {
    let current_thread = std::thread::current().id();
    let should_drop_guard = {
        let mut gui_thread = GUI_THREAD_STATE.lock();
        debug_assert!(gui_thread.ref_count > 0);
        gui_thread.ref_count = gui_thread.ref_count.saturating_sub(1);
        log::debug!(
            "wxp GUI thread lease: released on thread {current_thread:?}; ref_count={}",
            gui_thread.ref_count
        );
        if gui_thread.ref_count == 0 {
            // Once both the last runtime and the thread affinity acquired via `set_parent()`
            // are released, allow the next GUI session to arrive from a different host window.
            gui_thread.owner = None;
            log::debug!("wxp GUI thread lease: owner cleared");
            true
        } else {
            false
        }
    };
    if should_drop_guard {
        GUI_RUN_LOOP_GUARD.with(|stored_guard| {
            let guard = stored_guard.borrow_mut().take();
            debug_assert!(guard.is_some());
            drop(guard);
        });
    }
}

pub(crate) fn is_gui_thread() -> bool {
    RunLoop::is_run_loop_thread()
}
