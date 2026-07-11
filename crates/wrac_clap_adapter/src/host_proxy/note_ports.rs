use clap_sys::ext::note_ports::{CLAP_EXT_NOTE_PORTS, clap_host_note_ports};
use clap_sys::host::clap_host;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use crate::interface::{HostNotePorts, NoteDialects};

pub(crate) struct HostNotePortsProxy {
    host: *const clap_host,
    initialized: Arc<AtomicBool>,
    host_note_ports: OnceLock<Option<HostNotePortsCallbacks>>,
}

impl HostNotePortsProxy {
    pub(crate) fn new(host: *const clap_host, initialized: Arc<AtomicBool>) -> Self {
        Self {
            host,
            initialized,
            host_note_ports: OnceLock::new(),
        }
    }

    fn callbacks(&self) -> Option<HostNotePortsCallbacks> {
        if !self.initialized.load(Ordering::Acquire) {
            log::debug!("host_note_ports: host extension unavailable before plugin.init");
            return None;
        }

        *self
            .host_note_ports
            .get_or_init(|| host_note_ports(self.host))
    }
}

impl HostNotePorts for HostNotePortsProxy {
    fn supported_dialects(&self) -> NoteDialects {
        let Some(note_ports) = self.callbacks() else {
            log::debug!("host_note_ports.supported_dialects: host extension unavailable");
            return NoteDialects::default();
        };

        let Some(supported_dialects) = note_ports.supported_dialects else {
            log::debug!("host_note_ports.supported_dialects: host callback unavailable");
            return NoteDialects::default();
        };

        NoteDialects::from_bits(unsafe { supported_dialects(note_ports.host) })
    }

    fn rescan(&self, flags: u32) {
        let Some(note_ports) = self.callbacks() else {
            log::debug!("host_note_ports.rescan: host extension unavailable");
            return;
        };

        let Some(rescan) = note_ports.rescan else {
            log::debug!("host_note_ports.rescan: host callback unavailable");
            return;
        };

        unsafe {
            rescan(note_ports.host, flags);
        }
    }
}

#[derive(Clone, Copy)]
struct HostNotePortsCallbacks {
    host: *const clap_host,
    supported_dialects: Option<unsafe extern "C" fn(host: *const clap_host) -> u32>,
    rescan: Option<unsafe extern "C" fn(host: *const clap_host, flags: u32)>,
}

// The CLAP host pointer is owned by the host for the plugin instance lifetime.
unsafe impl Send for HostNotePortsCallbacks {}
unsafe impl Sync for HostNotePortsCallbacks {}

// Extension lookup is delayed until after `plugin.init`; callers only hold the safe
// trait object exposed by the adapter context.
unsafe impl Send for HostNotePortsProxy {}
unsafe impl Sync for HostNotePortsProxy {}

fn host_note_ports(host: *const clap_host) -> Option<HostNotePortsCallbacks> {
    if host.is_null() {
        return None;
    }

    unsafe {
        let get_extension = (*host).get_extension?;
        let note_ports =
            get_extension(host, CLAP_EXT_NOTE_PORTS.as_ptr()) as *const clap_host_note_ports;
        let note_ports = note_ports.as_ref()?;
        Some(HostNotePortsCallbacks {
            host,
            supported_dialects: note_ports.supported_dialects,
            rescan: note_ports.rescan,
        })
    }
}
