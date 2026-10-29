use std::ffi::{CStr, c_char, c_void};

use clap_sys::plugin::clap_plugin;

use super::PluginInstanceState;
use super::ffi::ffi_bool;

pub(super) const AU_HOST_IDENTIFIER: &CStr = c"com.novonotes.wrac.au-host-identifier/1";

// The AU wrapper forwards a borrowed CFString, which must be copied before returning.
#[repr(C)]
pub(super) struct AuHostIdentity {
    pub(super) set: unsafe extern "C" fn(*const clap_plugin, *const c_void) -> bool,
}

pub(super) static AU_HOST_IDENTITY: AuHostIdentity = AuHostIdentity {
    set: set_identifier,
};

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringGetLength(string: *const c_void) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(
        string: *const c_void,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> bool;
}

unsafe extern "C" fn set_identifier(plugin: *const clap_plugin, string: *const c_void) -> bool {
    ffi_bool(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            return false;
        };
        let identifier = if string.is_null() {
            None
        } else {
            const UTF8: u32 = 0x0800_0100;
            let length = unsafe { CFStringGetLength(string) };
            let maximum = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8) };
            let Some(capacity) = maximum.checked_add(1).filter(|size| *size > 0) else {
                return false;
            };
            let mut buffer = vec![0_u8; capacity as usize];
            if !unsafe { CFStringGetCString(string, buffer.as_mut_ptr().cast(), capacity, UTF8) } {
                return false;
            }
            let Ok(identifier) = CStr::from_bytes_until_nul(&buffer) else {
                return false;
            };
            let Ok(identifier) = identifier.to_str() else {
                return false;
            };
            Some(identifier.to_owned())
        };
        instance.host_identity.set_au_identifier(identifier);
        true
    })
}
