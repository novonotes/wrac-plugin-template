use clap_sys::host::clap_host;

use crate::HostLifecycle;

pub(crate) struct HostLifecycleProxy {
    restart: Option<HostRequestRestart>,
}

impl HostLifecycleProxy {
    pub(crate) fn new(host: *const clap_host) -> Self {
        Self {
            restart: host_request_restart(host),
        }
    }
}

impl HostLifecycle for HostLifecycleProxy {
    fn request_restart(&self) {
        let Some(restart) = self.restart else {
            log::debug!("host.request_restart: callback unavailable");
            return;
        };

        unsafe {
            (restart.request_restart)(restart.host);
        }
    }
}

#[derive(Clone, Copy)]
struct HostRequestRestart {
    host: *const clap_host,
    request_restart: unsafe extern "C" fn(host: *const clap_host),
}

// The CLAP host pointer is owned by the host for the plugin instance lifetime. The
// public trait restricts product-facing use to control-thread contexts.
unsafe impl Send for HostRequestRestart {}
unsafe impl Sync for HostRequestRestart {}

fn host_request_restart(host: *const clap_host) -> Option<HostRequestRestart> {
    if host.is_null() {
        return None;
    }

    unsafe {
        let request_restart = (*host).request_restart?;
        Some(HostRequestRestart {
            host,
            request_restart,
        })
    }
}
