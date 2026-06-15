use clap_sys::ext::audio_ports::{CLAP_EXT_AUDIO_PORTS, clap_host_audio_ports};
use clap_sys::host::clap_host;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use crate::HostAudioPorts;

pub(crate) struct HostAudioPortsProxy {
    host: *const clap_host,
    initialized: Arc<AtomicBool>,
    host_audio_ports: OnceLock<Option<HostAudioPortsCallbacks>>,
}

impl HostAudioPortsProxy {
    pub(crate) fn new(host: *const clap_host, initialized: Arc<AtomicBool>) -> Self {
        Self {
            host,
            initialized,
            host_audio_ports: OnceLock::new(),
        }
    }

    fn callbacks(&self) -> Option<HostAudioPortsCallbacks> {
        if !self.initialized.load(Ordering::Acquire) {
            log::debug!("host_audio_ports: host extension unavailable before plugin.init");
            return None;
        }

        *self
            .host_audio_ports
            .get_or_init(|| host_audio_ports(self.host))
    }
}

impl HostAudioPorts for HostAudioPortsProxy {
    fn is_rescan_flag_supported(&self, flag: u32) -> bool {
        let Some(audio_ports) = self.callbacks() else {
            log::debug!("host_audio_ports.is_rescan_flag_supported: host extension unavailable");
            return false;
        };

        let Some(is_rescan_flag_supported) = audio_ports.is_rescan_flag_supported else {
            log::debug!("host_audio_ports.is_rescan_flag_supported: host callback unavailable");
            return false;
        };

        unsafe { is_rescan_flag_supported(audio_ports.host, flag) }
    }

    fn rescan(&self, flags: u32) {
        let Some(audio_ports) = self.callbacks() else {
            log::debug!("host_audio_ports.rescan: host extension unavailable");
            return;
        };

        let Some(rescan) = audio_ports.rescan else {
            log::debug!("host_audio_ports.rescan: host callback unavailable");
            return;
        };

        unsafe {
            rescan(audio_ports.host, flags);
        }
    }
}

#[derive(Clone, Copy)]
struct HostAudioPortsCallbacks {
    host: *const clap_host,
    is_rescan_flag_supported:
        Option<unsafe extern "C" fn(host: *const clap_host, flag: u32) -> bool>,
    rescan: Option<unsafe extern "C" fn(host: *const clap_host, flags: u32)>,
}

// The CLAP host pointer is owned by the host for the plugin instance lifetime.
unsafe impl Send for HostAudioPortsCallbacks {}
unsafe impl Sync for HostAudioPortsCallbacks {}

// Extension lookup is delayed until after `plugin.init`; callers only hold the safe
// trait object exposed by the adapter context.
unsafe impl Send for HostAudioPortsProxy {}
unsafe impl Sync for HostAudioPortsProxy {}

fn host_audio_ports(host: *const clap_host) -> Option<HostAudioPortsCallbacks> {
    if host.is_null() {
        return None;
    }

    unsafe {
        let get_extension = (*host).get_extension?;
        let audio_ports =
            get_extension(host, CLAP_EXT_AUDIO_PORTS.as_ptr()) as *const clap_host_audio_ports;
        let audio_ports = audio_ports.as_ref()?;
        Some(HostAudioPortsCallbacks {
            host,
            is_rescan_flag_supported: audio_ports.is_rescan_flag_supported,
            rescan: audio_ports.rescan,
        })
    }
}
