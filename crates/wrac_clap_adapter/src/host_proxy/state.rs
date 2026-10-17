use clap_sys::ext::state::{CLAP_EXT_STATE, clap_host_state};
use clap_sys::host::clap_host;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use crate::interface::HostState;

pub(crate) struct HostStateProxy {
    host: *const clap_host,
    initialized: Arc<AtomicBool>,
    host_state: OnceLock<Option<HostStateMarkDirty>>,
}

impl HostStateProxy {
    pub(crate) fn new(host: *const clap_host, initialized: Arc<AtomicBool>) -> Self {
        Self {
            host,
            initialized,
            host_state: OnceLock::new(),
        }
    }

    fn callbacks(&self) -> Option<HostStateMarkDirty> {
        if !self.initialized.load(Ordering::Acquire) {
            log::debug!("host_state: host extension unavailable before plugin.init");
            return None;
        }

        *self
            .host_state
            .get_or_init(|| host_state_mark_dirty(self.host))
    }
}

impl HostState for HostStateProxy {
    fn mark_dirty(&self) {
        let Some(host_state) = self.callbacks() else {
            log::debug!("host_state.mark_dirty: host state extension unavailable");
            return;
        };

        unsafe {
            (host_state.mark_dirty)(host_state.host);
        }
    }
}

#[derive(Clone, Copy)]
struct HostStateMarkDirty {
    host: *const clap_host,
    mark_dirty: unsafe extern "C" fn(host: *const clap_host),
}

// The instance lifetime of the host pointer is the minimal unavoidable assumption of the
// CLAP ABI. The public trait contract, not this proxy, carries the `mark_dirty`
// main-thread constraint, so products never receive the raw pointer.
unsafe impl Send for HostStateMarkDirty {}
unsafe impl Sync for HostStateMarkDirty {}

// Extension lookup is delayed until after `plugin.init`; callers only hold the safe
// trait object exposed by the adapter context.
unsafe impl Send for HostStateProxy {}
unsafe impl Sync for HostStateProxy {}

fn host_state_mark_dirty(host: *const clap_host) -> Option<HostStateMarkDirty> {
    if host.is_null() {
        return None;
    }

    unsafe {
        let get_extension = (*host).get_extension?;
        let state = get_extension(host, CLAP_EXT_STATE.as_ptr()) as *const clap_host_state;
        if state.is_null() {
            return None;
        }
        let mark_dirty = (*state).mark_dirty?;
        Some(HostStateMarkDirty { host, mark_dirty })
    }
}
