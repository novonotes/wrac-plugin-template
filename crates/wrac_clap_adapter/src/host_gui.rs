use clap_sys::ext::gui::{CLAP_EXT_GUI, clap_host_gui};
use clap_sys::host::clap_host;

use crate::{GuiSize, HostGuiResizeRequester, PluginError, PluginResult};

pub(crate) struct HostGuiResizeRequest {
    host_gui: Option<HostGuiRequestResize>,
}

impl HostGuiResizeRequest {
    pub(crate) fn new(host: *const clap_host) -> Self {
        Self {
            host_gui: host_gui_request_resize(host),
        }
    }
}

impl HostGuiResizeRequester for HostGuiResizeRequest {
    fn request_resize(&self, size: GuiSize) -> PluginResult<()> {
        let Some(host_gui) = self.host_gui else {
            return Err(PluginError::Message(
                "host does not expose CLAP GUI extension",
            ));
        };

        let accepted = unsafe { (host_gui.request_resize)(host_gui.host, size.width, size.height) };
        if accepted {
            Ok(())
        } else {
            Err(PluginError::Message("host rejected GUI resize request"))
        }
    }
}

#[derive(Clone, Copy)]
struct HostGuiRequestResize {
    host: *const clap_host,
    request_resize: unsafe extern "C" fn(host: *const clap_host, width: u32, height: u32) -> bool,
}

// The instance lifetime of the host pointer is the minimal unavoidable assumption of the
// CLAP ABI. This handle is Send/Sync only so it can live in shared plugin context; callers
// must still use request_resize from the GUI event path, not from audio/background work.
unsafe impl Send for HostGuiRequestResize {}
unsafe impl Sync for HostGuiRequestResize {}

fn host_gui_request_resize(host: *const clap_host) -> Option<HostGuiRequestResize> {
    if host.is_null() {
        return None;
    }

    unsafe {
        let get_extension = (*host).get_extension?;
        let gui = get_extension(host, CLAP_EXT_GUI.as_ptr()) as *const clap_host_gui;
        if gui.is_null() {
            return None;
        }
        let request_resize = (*gui).request_resize?;
        Some(HostGuiRequestResize {
            host,
            request_resize,
        })
    }
}
