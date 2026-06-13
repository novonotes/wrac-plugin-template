use clap_sys::ext::gui::{CLAP_EXT_GUI, clap_host_gui};
use clap_sys::host::clap_host;

use crate::{GuiSize, HostGuiResizeRequester, PluginError, PluginResult};

pub(crate) struct HostGuiResizeRequest {
    host_gui: Option<HostGuiCallbacks>,
}

impl HostGuiResizeRequest {
    pub(crate) fn new(host: *const clap_host) -> Self {
        Self {
            host_gui: host_gui_request_resize(host),
        }
    }
}

impl HostGuiResizeRequester for HostGuiResizeRequest {
    fn resize_hints_changed(&self) {
        let Some(host_gui) = self.host_gui else {
            log::debug!("host_gui.resize_hints_changed: host GUI extension unavailable");
            return;
        };

        if let Some(resize_hints_changed) = host_gui.resize_hints_changed {
            unsafe {
                resize_hints_changed(host_gui.host);
            }
        } else {
            log::debug!(
                "host_gui.resize_hints_changed: host resize_hints_changed callback unavailable"
            );
        }
    }

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

    fn request_show(&self) -> bool {
        let Some(host_gui) = self.host_gui else {
            log::debug!("host_gui.request_show: host GUI extension unavailable");
            return false;
        };

        let Some(request_show) = host_gui.request_show else {
            log::debug!("host_gui.request_show: host request_show callback unavailable");
            return false;
        };

        unsafe { request_show(host_gui.host) }
    }

    fn request_hide(&self) -> bool {
        let Some(host_gui) = self.host_gui else {
            log::debug!("host_gui.request_hide: host GUI extension unavailable");
            return false;
        };

        let Some(request_hide) = host_gui.request_hide else {
            log::debug!("host_gui.request_hide: host request_hide callback unavailable");
            return false;
        };

        unsafe { request_hide(host_gui.host) }
    }

    fn closed(&self, was_destroyed: bool) {
        let Some(host_gui) = self.host_gui else {
            log::debug!("host_gui.closed: host GUI extension unavailable");
            return;
        };

        if let Some(closed) = host_gui.closed {
            unsafe {
                closed(host_gui.host, was_destroyed);
            }
        } else {
            log::debug!("host_gui.closed: host closed callback unavailable");
        }
    }
}

#[derive(Clone, Copy)]
struct HostGuiCallbacks {
    host: *const clap_host,
    resize_hints_changed: Option<unsafe extern "C" fn(host: *const clap_host)>,
    request_resize: unsafe extern "C" fn(host: *const clap_host, width: u32, height: u32) -> bool,
    request_show: Option<unsafe extern "C" fn(host: *const clap_host) -> bool>,
    request_hide: Option<unsafe extern "C" fn(host: *const clap_host) -> bool>,
    closed: Option<unsafe extern "C" fn(host: *const clap_host, was_destroyed: bool)>,
}

// The instance lifetime of the host pointer is the minimal unavoidable assumption of the
// CLAP ABI. This handle is Send/Sync only so it can live in shared plugin context; callers
// must still use request_resize from the GUI event path, not from audio/background work.
unsafe impl Send for HostGuiCallbacks {}
unsafe impl Sync for HostGuiCallbacks {}

fn host_gui_request_resize(host: *const clap_host) -> Option<HostGuiCallbacks> {
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
        Some(HostGuiCallbacks {
            host,
            resize_hints_changed: (*gui).resize_hints_changed,
            request_resize,
            request_show: (*gui).request_show,
            request_hide: (*gui).request_hide,
            closed: (*gui).closed,
        })
    }
}
