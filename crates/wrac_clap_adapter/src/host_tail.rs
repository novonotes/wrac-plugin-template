use clap_sys::ext::tail::{CLAP_EXT_TAIL, clap_host_tail};
use clap_sys::host::clap_host;

use crate::HostTail;

#[derive(Clone, Copy)]
pub(crate) struct HostTailFactory {
    changed: Option<HostTailChanged>,
}

impl HostTailFactory {
    pub(crate) fn new(host: *const clap_host) -> Self {
        Self {
            changed: host_tail_changed(host),
        }
    }

    pub(crate) fn create_handle(&self) -> Option<Box<dyn HostTail>> {
        self.changed
            .map(|changed| Box::new(HostTailProxy { changed }) as Box<dyn HostTail>)
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
