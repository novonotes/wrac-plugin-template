use clap_sys::host::clap_host;

use crate::HostLifecycle;

pub(crate) struct HostLifecycleProxy {
    callbacks: HostLifecycleCallbacks,
}

impl HostLifecycleProxy {
    pub(crate) fn new(host: *const clap_host) -> Self {
        Self {
            callbacks: HostLifecycleCallbacks::new(host),
        }
    }
}

impl HostLifecycle for HostLifecycleProxy {
    fn request_restart(&self) {
        let Some(request_restart) = self.callbacks.request_restart else {
            log::debug!("host.request_restart: callback unavailable");
            return;
        };

        unsafe {
            request_restart(self.callbacks.host);
        }
    }

    fn request_process(&self) {
        let Some(request_process) = self.callbacks.request_process else {
            log::debug!("host.request_process: callback unavailable");
            return;
        };

        unsafe {
            request_process(self.callbacks.host);
        }
    }

    fn request_callback(&self) {
        let Some(request_callback) = self.callbacks.request_callback else {
            log::debug!("host.request_callback: callback unavailable");
            return;
        };

        unsafe {
            request_callback(self.callbacks.host);
        }
    }
}

#[derive(Clone, Copy)]
struct HostLifecycleCallbacks {
    host: *const clap_host,
    request_restart: Option<unsafe extern "C" fn(host: *const clap_host)>,
    request_process: Option<unsafe extern "C" fn(host: *const clap_host)>,
    request_callback: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

// The CLAP host pointer is owned by the host for the plugin instance lifetime. The public trait
// mirrors CLAP's host lifecycle callbacks and does not add scheduling or thread correction.
unsafe impl Send for HostLifecycleCallbacks {}
unsafe impl Sync for HostLifecycleCallbacks {}

impl HostLifecycleCallbacks {
    fn new(host: *const clap_host) -> Self {
        let Some(host_ref) = (unsafe { host.as_ref() }) else {
            return Self {
                host,
                request_restart: None,
                request_process: None,
                request_callback: None,
            };
        };

        Self {
            host,
            request_restart: host_ref.request_restart,
            request_process: host_ref.request_process,
            request_callback: host_ref.request_callback,
        }
    }
}
