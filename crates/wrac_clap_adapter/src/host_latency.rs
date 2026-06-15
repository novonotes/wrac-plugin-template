use clap_sys::ext::latency::{CLAP_EXT_LATENCY, clap_host_latency};
use clap_sys::host::clap_host;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct HostLatencyProxy {
    host: *const clap_host,
    initialized: Arc<AtomicBool>,
    changed: OnceLock<Option<HostLatencyChanged>>,
}

impl HostLatencyProxy {
    pub(crate) fn new(host: *const clap_host, initialized: Arc<AtomicBool>) -> Self {
        Self {
            host,
            initialized,
            changed: OnceLock::new(),
        }
    }

    pub(crate) fn changed(&self) {
        let Some(changed) = self.callbacks() else {
            log::debug!("host_latency.changed: host latency extension unavailable");
            return;
        };

        unsafe {
            (changed.changed)(changed.host);
        }
    }

    fn callbacks(&self) -> Option<HostLatencyChanged> {
        if !self.initialized.load(Ordering::Acquire) {
            log::debug!("host_latency: host extension unavailable before plugin.init");
            return None;
        }

        *self.changed.get_or_init(|| host_latency_changed(self.host))
    }
}

#[derive(Clone, Copy)]
struct HostLatencyChanged {
    host: *const clap_host,
    changed: unsafe extern "C" fn(host: *const clap_host),
}

// `changed` is called only by the adapter during CLAP activate, where CLAP marks
// the plugin as `[being-activated]`.
unsafe impl Send for HostLatencyChanged {}
unsafe impl Sync for HostLatencyChanged {}

// Extension lookup is delayed until after `plugin.init`; this proxy is used by the
// adapter after initialization during activation.
unsafe impl Send for HostLatencyProxy {}
unsafe impl Sync for HostLatencyProxy {}

fn host_latency_changed(host: *const clap_host) -> Option<HostLatencyChanged> {
    if host.is_null() {
        return None;
    }

    unsafe {
        let get_extension = (*host).get_extension?;
        let latency = get_extension(host, CLAP_EXT_LATENCY.as_ptr()) as *const clap_host_latency;
        if latency.is_null() {
            return None;
        }
        let changed = (*latency).changed?;
        Some(HostLatencyChanged { host, changed })
    }
}
