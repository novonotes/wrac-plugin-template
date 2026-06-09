use std::any::Any;
use std::ffi::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use clap_sys::process::{CLAP_PROCESS_ERROR, clap_process_status};
use clap_sys::stream::{clap_istream, clap_ostream};

pub(super) unsafe fn write_stream(stream: *const clap_ostream, bytes: &[u8]) -> bool {
    let Some(write) = (unsafe { (*stream).write }) else {
        log::warn!(
            "ffi.write_stream: stream has no write callback byte_len={}",
            bytes.len()
        );
        return false;
    };
    let mut offset = 0;
    while offset < bytes.len() {
        let written = unsafe {
            write(
                stream,
                bytes[offset..].as_ptr().cast(),
                (bytes.len() - offset) as u64,
            )
        };
        if written <= 0 {
            log::warn!(
                "ffi.write_stream: write failed written={written} offset={offset} byte_len={}",
                bytes.len()
            );
            return false;
        }
        offset += written as usize;
    }
    true
}

pub(super) unsafe fn read_stream_to_end(
    stream: *const clap_istream,
    max_len: usize,
) -> Option<Vec<u8>> {
    let Some(read) = (unsafe { (*stream).read }) else {
        log::warn!("ffi.read_stream_to_end: stream has no read callback max_len={max_len}");
        return None;
    };
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read_count = unsafe { read(stream, chunk.as_mut_ptr().cast(), chunk.len() as u64) };
        if read_count < 0 {
            log::warn!("ffi.read_stream_to_end: read failed read_count={read_count}");
            return None;
        }
        if read_count == 0 {
            return Some(bytes);
        }
        let read_count = read_count as usize;
        if bytes.len().saturating_add(read_count) > max_len {
            log::warn!("ffi.read_stream_to_end: state too large max_len={max_len}");
            return None;
        }
        bytes.extend_from_slice(&chunk[..read_count]);
    }
}

pub(super) fn fill_c_char_array<const N: usize>(target: &mut [c_char; N], text: &str) {
    target.fill(0);
    for (dst, src) in target
        .iter_mut()
        .take(N.saturating_sub(1))
        .zip(text.bytes())
    {
        *dst = src as c_char;
    }
}

pub(super) fn write_c_str_buffer(out_buffer: *mut c_char, capacity: u32, text: &str) -> bool {
    if out_buffer.is_null() || capacity == 0 {
        log::warn!(
            "ffi.write_c_str_buffer: invalid output buffer capacity={capacity} text_len={}",
            text.len()
        );
        return false;
    }

    let max_len = capacity as usize - 1;
    let bytes = text.as_bytes();
    let len = bytes.len().min(max_len);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), out_buffer.cast::<u8>(), len);
        *out_buffer.add(len) = 0;
    }
    true
}

pub(super) fn four_char_code(bytes: [u8; 4]) -> [c_char; 5] {
    [
        bytes[0] as c_char,
        bytes[1] as c_char,
        bytes[2] as c_char,
        bytes[3] as c_char,
        0,
    ]
}

// Panics must not escape the C ABI boundary. Each callback converts Rust failures into
// conservative CLAP return values so the host receives a rejection without unwinding foreign frames.
pub(super) fn ffi_bool(f: impl FnOnce() -> bool) -> bool {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            log_panic(payload.as_ref());
            false
        }
    }
}

pub(super) fn ffi_u32(f: impl FnOnce() -> u32) -> u32 {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            log_panic(payload.as_ref());
            0
        }
    }
}

pub(super) fn ffi_status(f: impl FnOnce() -> clap_process_status) -> clap_process_status {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            log_panic(payload.as_ref());
            CLAP_PROCESS_ERROR
        }
    }
}

pub(super) fn ffi_ptr<T>(f: impl FnOnce() -> *const T) -> *const T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            log_panic(payload.as_ref());
            ptr::null()
        }
    }
}

pub(super) fn ffi_unit(f: impl FnOnce()) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(f)) {
        log_panic(payload.as_ref());
    }
}

fn log_panic(payload: &(dyn Any + Send)) {
    if let Some(message) = payload.downcast_ref::<&str>() {
        wrac_log::rterror!("panic in CLAP callback: {message}");
    } else if let Some(message) = payload.downcast_ref::<String>() {
        wrac_log::rterror!("panic in CLAP callback: {message}");
    } else {
        wrac_log::rterror!("panic in CLAP callback");
    }
}
