use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::process::clap_process;

use crate::interface::{AudioBufferError, AudioProcessBuffer};

// CLAP audio buffer arrays are valid only for the duration of the callback. Here they are
// converted to slices bound to the callback lifetime without interpreting port or channel layout.
// Alias detection and sample type checks are deferred to `AudioProcessBuffer` as needed.
pub(super) unsafe fn audio_buffers(
    process: &clap_process,
) -> Result<AudioProcessBuffer<'_>, AudioBufferError> {
    let inputs =
        unsafe { slice_from_external_parts(process.audio_inputs, process.audio_inputs_count)? };
    let outputs = unsafe {
        slice_from_external_parts_mut(process.audio_outputs, process.audio_outputs_count)?
    };
    unsafe { AudioProcessBuffer::from_raw_buffers(inputs, outputs, process.frames_count) }
}

unsafe fn slice_from_external_parts<'a, T>(
    data: *const T,
    len: u32,
) -> Result<&'a [T], AudioBufferError> {
    let len = len as usize;
    if len == 0 {
        Ok(&[])
    } else if data.is_null() {
        Err(AudioBufferError::InvalidPortBuffer)
    } else {
        Ok(unsafe { std::slice::from_raw_parts(data, len) })
    }
}

unsafe fn slice_from_external_parts_mut<'a>(
    data: *mut clap_audio_buffer,
    len: u32,
) -> Result<&'a mut [clap_audio_buffer], AudioBufferError> {
    let len = len as usize;
    if len == 0 {
        Ok(&mut [])
    } else if data.is_null() {
        Err(AudioBufferError::InvalidPortBuffer)
    } else {
        Ok(unsafe { std::slice::from_raw_parts_mut(data, len) })
    }
}
