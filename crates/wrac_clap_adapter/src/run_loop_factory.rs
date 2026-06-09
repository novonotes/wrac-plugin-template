use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use novonotes_run_loop::{RunLoop, RunLoopGuard};

pub(crate) const WRAC_PLUGIN_FACTORY_RUN_LOOP: &[u8] =
    b"com.novonotes.wrac.plugin-factory-run-loop/0\0";

thread_local! {
    static RUN_LOOP_GUARDS: RefCell<Vec<RunLoopGuard>> = const { RefCell::new(Vec::new()) };
}

#[repr(C)]
pub(crate) struct WracPluginFactoryRunLoop {
    pub bind_current_thread:
        Option<unsafe extern "C" fn(factory: *const WracPluginFactoryRunLoop) -> bool>,
    pub unbind_current_thread:
        Option<unsafe extern "C" fn(factory: *const WracPluginFactoryRunLoop)>,
}

// Safety: the factory is a static table of function pointers. The non-Send
// RunLoopGuard values created by those callbacks are stored only in thread-local
// storage on the bound host/UI thread.
unsafe impl Sync for WracPluginFactoryRunLoop {}
unsafe impl Send for WracPluginFactoryRunLoop {}

pub(crate) static WRAC_RUN_LOOP_FACTORY: WracPluginFactoryRunLoop = WracPluginFactoryRunLoop {
    bind_current_thread: Some(bind_current_thread),
    unbind_current_thread: Some(unbind_current_thread),
};

unsafe extern "C" fn bind_current_thread(_factory: *const WracPluginFactoryRunLoop) -> bool {
    ffi_bool(|| match RunLoop::init() {
        Ok(guard) => {
            RUN_LOOP_GUARDS.with(|guards| guards.borrow_mut().push(guard));
            true
        }
        Err(error) => {
            log::warn!("WRAC run loop bind failed: {error}");
            false
        }
    })
}

unsafe extern "C" fn unbind_current_thread(_factory: *const WracPluginFactoryRunLoop) {
    ffi_unit(|| {
        RUN_LOOP_GUARDS.with(|guards| {
            let guard = guards.borrow_mut().pop();
            if guard.is_none() {
                log::warn!("WRAC run loop unbind called without a matching bind");
            }
        });
    })
}

pub(crate) fn factory_ptr() -> *const std::ffi::c_void {
    &WRAC_RUN_LOOP_FACTORY as *const WracPluginFactoryRunLoop as *const std::ffi::c_void
}

fn ffi_bool(f: impl FnOnce() -> bool) -> bool {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(_) => {
            log::error!("panic in WRAC run loop factory callback");
            false
        }
    }
}

fn ffi_unit(f: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        log::error!("panic in WRAC run loop factory callback");
    }
}
