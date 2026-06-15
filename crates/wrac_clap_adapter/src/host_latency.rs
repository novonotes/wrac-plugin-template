use clap_sys::ext::latency::{CLAP_EXT_LATENCY, clap_host_latency};
use clap_sys::host::clap_host;

pub(crate) struct HostLatencyProxy {
    changed: Option<HostLatencyChanged>,
}

impl HostLatencyProxy {
    pub(crate) fn new(host: *const clap_host) -> Self {
        Self {
            changed: host_latency_changed(host),
        }
    }

    pub(crate) fn changed(&self) {
        let Some(changed) = self.changed else {
            log::debug!("host_latency.changed: host latency extension unavailable");
            return;
        };

        unsafe {
            (changed.changed)(changed.host);
        }
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
