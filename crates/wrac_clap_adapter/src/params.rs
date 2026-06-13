use clap_sys::ext::params::{
    CLAP_EXT_PARAMS, CLAP_PARAM_RESCAN_VALUES, clap_host_params, clap_param_clear_flags,
    clap_param_rescan_flags,
};
use clap_sys::host::clap_host;

use crate::HostParamsFlushRequester;

/// Thin proxy for the CLAP host params extension.
pub(crate) struct HostParamsProxy {
    host_params: Option<HostParams>,
}

impl HostParamsProxy {
    pub(crate) fn new(host: *const clap_host) -> Self {
        Self {
            host_params: host_params(host),
        }
    }

    pub(crate) fn rescan_values(&self) {
        let Some(params) = self.host_params else {
            log::debug!("host_params.rescan_values: host params extension unavailable");
            return;
        };

        if let Some(rescan) = params.rescan {
            unsafe {
                rescan(params.host, CLAP_PARAM_RESCAN_VALUES);
            }
        } else {
            log::debug!("host_params.rescan_values: host rescan callback unavailable");
        }
    }
}

impl HostParamsFlushRequester for HostParamsProxy {
    fn rescan(&self, flags: u32) {
        let Some(params) = self.host_params else {
            log::debug!("host_params.rescan: host params extension unavailable");
            return;
        };

        if let Some(rescan) = params.rescan {
            unsafe {
                rescan(params.host, flags);
            }
        } else {
            log::debug!("host_params.rescan: host rescan callback unavailable");
        }
    }

    fn clear(&self, param_id: u32, flags: u32) {
        let Some(params) = self.host_params else {
            log::debug!("host_params.clear: host params extension unavailable");
            return;
        };

        if let Some(clear) = params.clear {
            unsafe {
                clear(params.host, param_id, flags);
            }
        } else {
            log::debug!("host_params.clear: host clear callback unavailable");
        }
    }

    fn request_flush(&self) {
        let Some(params) = self.host_params else {
            log::debug!("host_params.request_flush: host params extension unavailable");
            return;
        };

        if let Some(request_flush) = params.request_flush {
            unsafe {
                request_flush(params.host);
            }
        } else {
            log::debug!("host_params.request_flush: host request_flush callback unavailable");
        }
    }
}

#[derive(Clone, Copy)]
struct HostParams {
    host: *const clap_host,
    rescan: Option<unsafe extern "C" fn(host: *const clap_host, flags: clap_param_rescan_flags)>,
    clear: Option<
        unsafe extern "C" fn(host: *const clap_host, param_id: u32, flags: clap_param_clear_flags),
    >,
    request_flush: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

// The instance lifetime of the host pointer is the minimal unavoidable assumption of the
// CLAP ABI. Product-facing usage is limited to `request_flush()`; adapter-internal
// `rescan_values()` is called only after state load, where CLAP gives the callback a
// main-thread contract.
unsafe impl Send for HostParams {}
unsafe impl Sync for HostParams {}

fn host_params(host: *const clap_host) -> Option<HostParams> {
    if host.is_null() {
        return None;
    }

    unsafe {
        let get_extension = (*host).get_extension?;
        let params = get_extension(host, CLAP_EXT_PARAMS.as_ptr()) as *const clap_host_params;
        if params.is_null() {
            return None;
        }
        Some(HostParams {
            host,
            rescan: (*params).rescan,
            clear: (*params).clear,
            request_flush: (*params).request_flush,
        })
    }
}
