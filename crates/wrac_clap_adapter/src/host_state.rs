use clap_sys::ext::state::{CLAP_EXT_STATE, clap_host_state};
use clap_sys::host::clap_host;

use crate::HostStateDirtyNotifier;

pub(crate) struct HostStateDirtyNotification {
    host_state: Option<HostStateMarkDirty>,
}

impl HostStateDirtyNotification {
    pub(crate) fn new(host: *const clap_host) -> Self {
        Self {
            host_state: host_state_mark_dirty(host),
        }
    }
}

impl HostStateDirtyNotifier for HostStateDirtyNotification {
    fn mark_dirty(&self) {
        let Some(host_state) = self.host_state else {
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
