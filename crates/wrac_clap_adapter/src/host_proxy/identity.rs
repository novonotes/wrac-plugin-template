use std::ffi::{CStr, c_char};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use clap_sys::host::clap_host;

use crate::interface::{DetectedHost, HostContext, HostIdentity, PluginFormat};

const AU_HOST_IDENTIFIER: &CStr = c"clap-wrapper.au-host-identifier/1";

#[repr(C)]
struct AuHostIdentifier {
    get: unsafe extern "C" fn(*const clap_host, *mut c_char, u32) -> bool,
}

pub(crate) struct HostIdentityProxy {
    host: *const clap_host,
    initialized: Arc<AtomicBool>,
    initial: HostContext,
    get_identifier:
        OnceLock<Option<unsafe extern "C" fn(*const clap_host, *mut c_char, u32) -> bool>>,
}

// The host and its extension remain valid for the plugin instance lifetime. The
// callback is thread-safe and is only queried after CLAP initialization.
unsafe impl Send for HostIdentityProxy {}
unsafe impl Sync for HostIdentityProxy {}

impl HostIdentityProxy {
    pub(crate) fn new(
        host: *const clap_host,
        initialized: Arc<AtomicBool>,
        initial: HostContext,
    ) -> Self {
        Self {
            host,
            initialized,
            initial,
            get_identifier: OnceLock::new(),
        }
    }

    fn identifier(&self) -> Option<String> {
        if self.initial.plugin_format != PluginFormat::Au
            || !self.initialized.load(Ordering::Acquire)
        {
            return None;
        }
        let callback = self
            .get_identifier
            .get_or_init(|| {
                let host = unsafe { self.host.as_ref() }?;
                let extension =
                    unsafe { (host.get_extension?)(self.host, AU_HOST_IDENTIFIER.as_ptr()) };
                let extension = unsafe { (extension as *const AuHostIdentifier).as_ref() }?;
                Some(extension.get)
            })
            .as_ref()
            .copied()?;
        let mut buffer = [0_u8; 1024];
        if !unsafe { callback(self.host, buffer.as_mut_ptr().cast(), buffer.len() as u32) } {
            return None;
        }
        let value = CStr::from_bytes_until_nul(&buffer).ok()?;
        value.to_str().ok().map(str::to_owned)
    }
}

impl HostIdentity for HostIdentityProxy {
    fn current_host(&self) -> DetectedHost {
        self.initial
            .host_with_au_identifier(self.identifier().as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, c_void};
    use std::ptr;
    use std::sync::atomic::AtomicU8;
    use wrac_host_context::HostFamily;

    unsafe extern "C" fn get(host: *const clap_host, buffer: *mut c_char, capacity: u32) -> bool {
        let state = unsafe { &*((*host).host_data as *const AtomicU8) };
        let name = match state.load(Ordering::Acquire) {
            1 => b"com.apple.garageband\0".as_slice(),
            2 => b"com.apple.logic.pro\0".as_slice(),
            _ => return false,
        };
        if capacity < name.len() as u32 {
            return false;
        }
        unsafe {
            ptr::copy_nonoverlapping(name.as_ptr(), buffer.cast(), name.len());
        }
        true
    }

    static EXTENSION: AuHostIdentifier = AuHostIdentifier { get };

    unsafe extern "C" fn get_extension(_: *const clap_host, id: *const c_char) -> *const c_void {
        if unsafe { CStr::from_ptr(id) } == AU_HOST_IDENTIFIER {
            (&EXTENSION as *const AuHostIdentifier).cast()
        } else {
            ptr::null()
        }
    }

    #[test]
    fn resolves_identifier_received_after_creation_without_notifications() {
        let state = AtomicU8::new(0);
        let host = clap_host {
            clap_version: clap_sys::version::CLAP_VERSION,
            host_data: (&state as *const AtomicU8).cast_mut().cast(),
            name: c"test".as_ptr(),
            vendor: c"test".as_ptr(),
            url: c"".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(get_extension),
            request_restart: None,
            request_process: None,
            request_callback: None,
        };
        let mut context = HostContext::detect_current(None);
        context.plugin_format = PluginFormat::Au;
        context.host = wrac_host_context::detect_host_from_path("/unknown-host");
        let ready = Arc::new(AtomicBool::new(false));
        let proxy = HostIdentityProxy::new(&host, ready.clone(), context);
        state.store(1, Ordering::Release);
        assert_eq!(proxy.current_host().family, HostFamily::Unknown);
        ready.store(true, Ordering::Release);
        assert_eq!(proxy.current_host().family, HostFamily::AppleGarageBand);
        state.store(2, Ordering::Release);
        assert_eq!(proxy.current_host().family, HostFamily::AppleLogic);
    }
}
