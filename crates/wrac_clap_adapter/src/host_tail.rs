use clap_sys::ext::tail::{CLAP_EXT_TAIL, clap_host_tail};
use clap_sys::host::clap_host;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use crate::HostTail;

pub(crate) struct HostTailFactory {
    host: *const clap_host,
    initialized: Arc<AtomicBool>,
    changed: OnceLock<Option<HostTailChanged>>,
}

impl HostTailFactory {
    pub(crate) fn new(host: *const clap_host, initialized: Arc<AtomicBool>) -> Self {
        Self {
            host,
            initialized,
            changed: OnceLock::new(),
        }
    }

    pub(crate) fn create_handle(&self) -> Option<Box<dyn HostTail>> {
        self.changed()
            .map(|changed| Box::new(HostTailProxy { changed }) as Box<dyn HostTail>)
    }

    fn changed(&self) -> Option<HostTailChanged> {
        if !self.initialized.load(Ordering::Acquire) {
            log::debug!("host_tail: host extension unavailable before plugin.init");
            return None;
        }

        *self.changed.get_or_init(|| host_tail_changed(self.host))
    }
}

struct HostTailProxy {
    changed: HostTailChanged,
}

impl HostTail for HostTailProxy {
    fn changed(&mut self) {
        unsafe {
            (self.changed.changed)(self.changed.host);
        }
    }
}

#[derive(Clone, Copy)]
struct HostTailChanged {
    host: *const clap_host,
    changed: unsafe extern "C" fn(host: *const clap_host),
}

// The handle is moved into the ActiveProcessor and used only from serialized
// audio-thread callbacks. It is Send so the processor can move between host threads.
unsafe impl Send for HostTailChanged {}

// Extension lookup is delayed until after `plugin.init`; this factory is used by the
// adapter during activation before the audio-thread handle is created.
unsafe impl Send for HostTailFactory {}
unsafe impl Sync for HostTailFactory {}

fn host_tail_changed(host: *const clap_host) -> Option<HostTailChanged> {
    if host.is_null() {
        return None;
    }

    unsafe {
        let get_extension = (*host).get_extension?;
        let tail = get_extension(host, CLAP_EXT_TAIL.as_ptr()) as *const clap_host_tail;
        if tail.is_null() {
            return None;
        }
        let changed = (*tail).changed?;
        Some(HostTailChanged { host, changed })
    }
}
