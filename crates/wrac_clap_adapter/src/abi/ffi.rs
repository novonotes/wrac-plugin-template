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
    let written = unsafe { write(stream, bytes.as_ptr().cast(), bytes.len() as u64) };
    let expected = bytes.len() as i64;
    if written != expected {
        log::warn!("ffi.write_stream: short write written={written} expected={expected}");
        return false;
    }
    true
}

pub(super) unsafe fn read_stream_exact(stream: *const clap_istream, len: usize) -> Option<Vec<u8>> {
    let Some(read) = (unsafe { (*stream).read }) else {
        log::warn!("ffi.read_stream_exact: stream has no read callback byte_len={len}");
        return None;
    };
    let mut bytes = vec![0_u8; len];
    let mut offset = 0;
    while offset < len {
        let read_count = unsafe {
            read(
                stream,
                bytes[offset..].as_mut_ptr().cast(),
                (len - offset) as u64,
            )
        };
        if read_count <= 0 {
            log::warn!(
                "ffi.read_stream_exact: read failed read_count={read_count} offset={offset} byte_len={len}"
            );
            return None;
        }
        offset += read_count as usize;
    }
    Some(bytes)
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

// panic を C ABI の外へ出してはいけない。各 callback は Rust 側の失敗を返り値ごとの
// conservative な CLAP value に変換し、foreign frame を unwind せず host に拒否させる。
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
        log::error!("panic in CLAP callback: {message}");
    } else if let Some(message) = payload.downcast_ref::<String>() {
        log::error!("panic in CLAP callback: {message}");
    } else {
        log::error!("panic in CLAP callback");
    }
}
