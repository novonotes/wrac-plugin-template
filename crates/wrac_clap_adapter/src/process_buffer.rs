use std::error::Error;
use std::fmt::{Display, Formatter};
use std::slice::{Iter, IterMut};

use clap_sys::audio_buffer::clap_audio_buffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioBufferError {
    InvalidPortBuffer,
    InvalidChannelBuffer,
    MismatchedSampleType,
    AliasedOutputBuffer,
}

impl Display for AudioBufferError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPortBuffer => f.write_str("invalid audio port buffer"),
            Self::InvalidChannelBuffer => f.write_str("invalid audio channel buffer"),
            Self::MismatchedSampleType => {
                f.write_str("input and output audio buffers use different sample types")
            }
            Self::AliasedOutputBuffer => f.write_str("output audio buffers alias each other"),
        }
    }
}

impl Error for AudioBufferError {}

/// 1 回の `process()` だけで借りる audio buffer。
///
/// CLAP audio は port → channel → samples の非 interleaved 構造。raw pointer を
/// callback lifetime に縛ったまま保持し、channel へ降りる時点で in-place alias を
/// 判定して safe slice へ変換する。
pub struct AudioProcessBuffer<'a> {
    inputs: &'a [clap_audio_buffer],
    outputs: &'a mut [clap_audio_buffer],
    frames_count: u32,
}

impl<'a> AudioProcessBuffer<'a> {
    pub(crate) unsafe fn from_raw_buffers(
        inputs: &'a [clap_audio_buffer],
        outputs: &'a mut [clap_audio_buffer],
        frames_count: u32,
    ) -> Result<Self, AudioBufferError> {
        validate_buffers(inputs, outputs, frames_count)?;
        Ok(Self {
            inputs,
            outputs,
            frames_count,
        })
    }

    pub fn frames_count(&self) -> u32 {
        self.frames_count
    }

    pub fn input_port_count(&self) -> usize {
        self.inputs.len()
    }

    pub fn output_port_count(&self) -> usize {
        self.outputs.len()
    }

    pub fn port_pair_count(&self) -> usize {
        self.input_port_count().max(self.output_port_count())
    }

    pub fn port_pair(&mut self, index: usize) -> Option<AudioPortPair<'_>> {
        AudioPortPair::new(
            self.inputs.get(index),
            self.outputs.get_mut(index),
            self.frames_count,
        )
    }

    pub fn port_pairs(&mut self) -> AudioPortPairs<'_> {
        AudioPortPairs {
            inputs: self.inputs.iter(),
            outputs: self.outputs.iter_mut(),
            frames_count: self.frames_count,
        }
    }
}

impl<'a> IntoIterator for &'a mut AudioProcessBuffer<'_> {
    type Item = AudioPortPair<'a>;
    type IntoIter = AudioPortPairs<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.port_pairs()
    }
}

pub struct AudioPortPairs<'a> {
    inputs: Iter<'a, clap_audio_buffer>,
    outputs: IterMut<'a, clap_audio_buffer>,
    frames_count: u32,
}

impl<'a> Iterator for AudioPortPairs<'a> {
    type Item = AudioPortPair<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        AudioPortPair::new(self.inputs.next(), self.outputs.next(), self.frames_count)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len(), Some(self.len()))
    }
}

impl ExactSizeIterator for AudioPortPairs<'_> {
    fn len(&self) -> usize {
        self.inputs.len().max(self.outputs.len())
    }
}

pub struct AudioPortPair<'a> {
    input: Option<&'a clap_audio_buffer>,
    output: Option<&'a mut clap_audio_buffer>,
    frames_count: u32,
}

impl<'a> AudioPortPair<'a> {
    fn new(
        input: Option<&'a clap_audio_buffer>,
        output: Option<&'a mut clap_audio_buffer>,
        frames_count: u32,
    ) -> Option<Self> {
        match (input, output) {
            (None, None) => None,
            (input, output) => Some(Self {
                input,
                output,
                frames_count,
            }),
        }
    }

    pub fn channel_pair_count(&self) -> usize {
        let inputs = self.input.map(|buffer| buffer.channel_count).unwrap_or(0);
        let outputs = self
            .output
            .as_ref()
            .map(|buffer| buffer.channel_count)
            .unwrap_or(0);
        inputs.max(outputs) as usize
    }

    pub fn frames_count(&self) -> u32 {
        self.frames_count
    }

    pub fn channels(&mut self) -> Result<AudioPortChannels<'_>, AudioBufferError> {
        match common_sample_type(self.input, self.output.as_deref())? {
            AudioSampleFormat::F32 => Ok(AudioPortChannels::F32(AudioPairedChannels {
                input_data: self
                    .input
                    .map(input_data32)
                    .transpose()?
                    .unwrap_or_default(),
                output_data: self
                    .output
                    .as_deref_mut()
                    .map(output_data32)
                    .transpose()?
                    .unwrap_or_default(),
                frames_count: self.frames_count,
            })),
            AudioSampleFormat::F64 => Ok(AudioPortChannels::F64(AudioPairedChannels {
                input_data: self
                    .input
                    .map(input_data64)
                    .transpose()?
                    .unwrap_or_default(),
                output_data: self
                    .output
                    .as_deref_mut()
                    .map(output_data64)
                    .transpose()?
                    .unwrap_or_default(),
                frames_count: self.frames_count,
            })),
        }
    }
}

pub enum AudioPortChannels<'a> {
    F32(AudioPairedChannels<'a, f32>),
    F64(AudioPairedChannels<'a, f64>),
}

pub struct AudioPairedChannels<'a, T> {
    input_data: &'a [*mut T],
    output_data: &'a mut [*mut T],
    frames_count: u32,
}

impl<'a, T> AudioPairedChannels<'a, T> {
    pub fn input_channel_count(&self) -> usize {
        self.input_data.len()
    }

    pub fn output_channel_count(&self) -> usize {
        self.output_data.len()
    }

    pub fn channel_pair_count(&self) -> usize {
        self.input_channel_count().max(self.output_channel_count())
    }

    pub fn frames_count(&self) -> u32 {
        self.frames_count
    }

    pub fn channel_pair(&mut self, index: usize) -> Option<AudioChannelPair<'_, T>> {
        let input = self.input_data.get(index).copied();
        let output = self.output_data.get_mut(index).map(|ptr| *ptr);
        AudioChannelPair::from_raw(input, output, self.frames_count as usize)
    }

    pub fn iter_mut(&mut self) -> AudioPairedChannelsIter<'_, T> {
        AudioPairedChannelsIter {
            input_iter: self.input_data.iter(),
            output_iter: self.output_data.iter_mut(),
            frames_count: self.frames_count,
        }
    }
}

impl<'a, T> IntoIterator for AudioPairedChannels<'a, T> {
    type Item = AudioChannelPair<'a, T>;
    type IntoIter = AudioPairedChannelsIntoIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        AudioPairedChannelsIntoIter {
            input_iter: self.input_data.iter(),
            output_iter: self.output_data.iter_mut(),
            frames_count: self.frames_count,
        }
    }
}

pub struct AudioPairedChannelsIter<'a, T> {
    input_iter: Iter<'a, *mut T>,
    output_iter: IterMut<'a, *mut T>,
    frames_count: u32,
}

impl<'a, T> Iterator for AudioPairedChannelsIter<'a, T> {
    type Item = AudioChannelPair<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        let input = self.input_iter.next().copied();
        let output = self.output_iter.next().map(|ptr| *ptr);
        AudioChannelPair::from_raw(input, output, self.frames_count as usize)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len(), Some(self.len()))
    }
}

impl<T> ExactSizeIterator for AudioPairedChannelsIter<'_, T> {
    fn len(&self) -> usize {
        self.input_iter.len().max(self.output_iter.len())
    }
}

pub struct AudioPairedChannelsIntoIter<'a, T> {
    input_iter: Iter<'a, *mut T>,
    output_iter: IterMut<'a, *mut T>,
    frames_count: u32,
}

impl<'a, T> Iterator for AudioPairedChannelsIntoIter<'a, T> {
    type Item = AudioChannelPair<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        let input = self.input_iter.next().copied();
        let output = self.output_iter.next().map(|ptr| *ptr);
        AudioChannelPair::from_raw(input, output, self.frames_count as usize)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len(), Some(self.len()))
    }
}

impl<T> ExactSizeIterator for AudioPairedChannelsIntoIter<'_, T> {
    fn len(&self) -> usize {
        self.input_iter.len().max(self.output_iter.len())
    }
}

pub enum AudioChannelPair<'a, T> {
    InputOnly(&'a [T]),
    OutputOnly(&'a mut [T]),
    InputOutput(&'a [T], &'a mut [T]),
    InPlace(&'a mut [T]),
}

impl<'a, T> AudioChannelPair<'a, T> {
    fn from_raw(input: Option<*mut T>, output: Option<*mut T>, len: usize) -> Option<Self> {
        match (input, output) {
            (None, None) => None,
            (Some(input), None) => Some(Self::InputOnly(unsafe {
                slice_from_external_parts(input.cast_const(), len)
            })),
            (None, Some(output)) => Some(Self::OutputOnly(unsafe {
                slice_from_external_parts_mut(output, len)
            })),
            (Some(input), Some(output)) if input == output => Some(Self::InPlace(unsafe {
                slice_from_external_parts_mut(output, len)
            })),
            (Some(input), Some(output)) => Some(Self::InputOutput(
                unsafe { slice_from_external_parts(input.cast_const(), len) },
                unsafe { slice_from_external_parts_mut(output, len) },
            )),
        }
    }

    pub fn input(&self) -> Option<&[T]> {
        match self {
            Self::InputOnly(input) | Self::InputOutput(input, _) => Some(input),
            Self::OutputOnly(_) => None,
            Self::InPlace(buffer) => Some(buffer),
        }
    }

    pub fn output_mut(&mut self) -> Option<&mut [T]> {
        match self {
            Self::OutputOnly(output) | Self::InputOutput(_, output) | Self::InPlace(output) => {
                Some(output)
            }
            Self::InputOnly(_) => None,
        }
    }

    pub fn map_samples(&mut self, mut f: impl FnMut(T) -> T)
    where
        T: Copy + Default,
    {
        match self {
            Self::InputOnly(_) => {}
            Self::OutputOnly(output) => output.fill(T::default()),
            Self::InputOutput(input, output) => {
                let copy_len = input.len().min(output.len());
                for index in 0..copy_len {
                    output[index] = f(input[index]);
                }
                output[copy_len..].fill(T::default());
            }
            Self::InPlace(buffer) => {
                for sample in buffer.iter_mut() {
                    *sample = f(*sample);
                }
            }
        }
    }

    pub fn map_samples_range(&mut self, start: usize, len: usize, mut f: impl FnMut(T) -> T)
    where
        T: Copy + Default,
    {
        let end = start.saturating_add(len);
        match self {
            Self::InputOnly(_) => {}
            Self::OutputOnly(output) => {
                let start = start.min(output.len());
                let end = end.min(output.len());
                output[start..end].fill(T::default());
            }
            Self::InputOutput(input, output) => {
                let start = start.min(input.len()).min(output.len());
                let end = end.min(input.len()).min(output.len());
                for index in start..end {
                    output[index] = f(input[index]);
                }
            }
            Self::InPlace(buffer) => {
                let start = start.min(buffer.len());
                let end = end.min(buffer.len());
                for sample in &mut buffer[start..end] {
                    *sample = f(*sample);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioSampleFormat {
    F32,
    F64,
}

fn common_sample_type(
    input: Option<&clap_audio_buffer>,
    output: Option<&clap_audio_buffer>,
) -> Result<AudioSampleFormat, AudioBufferError> {
    let input = input.map(sample_mask).transpose()?;
    let output = output.map(sample_mask).transpose()?;
    let common = match (input, output) {
        (Some(input), Some(output)) => input & output,
        (Some(input), None) => input,
        (None, Some(output)) => output,
        (None, None) => 0,
    };

    if common & SAMPLE_F64 != 0 {
        Ok(AudioSampleFormat::F64)
    } else if common & SAMPLE_F32 != 0 {
        Ok(AudioSampleFormat::F32)
    } else {
        Err(AudioBufferError::MismatchedSampleType)
    }
}

const SAMPLE_F32: u8 = 0b01;
const SAMPLE_F64: u8 = 0b10;

fn sample_mask(buffer: &clap_audio_buffer) -> Result<u8, AudioBufferError> {
    let mut mask = 0;
    if !buffer.data32.is_null() {
        mask |= SAMPLE_F32;
    }
    if !buffer.data64.is_null() {
        mask |= SAMPLE_F64;
    }
    if mask == 0 && buffer.channel_count > 0 {
        Err(AudioBufferError::InvalidChannelBuffer)
    } else if mask == 0 {
        Ok(SAMPLE_F32 | SAMPLE_F64)
    } else {
        Ok(mask)
    }
}

fn input_data32(buffer: &clap_audio_buffer) -> Result<&[*mut f32], AudioBufferError> {
    if buffer.data32.is_null() && buffer.channel_count > 0 {
        return Err(AudioBufferError::InvalidChannelBuffer);
    }
    Ok(unsafe { slice_from_external_parts(buffer.data32, buffer.channel_count as usize) })
}

fn input_data64(buffer: &clap_audio_buffer) -> Result<&[*mut f64], AudioBufferError> {
    if buffer.data64.is_null() && buffer.channel_count > 0 {
        return Err(AudioBufferError::InvalidChannelBuffer);
    }
    Ok(unsafe { slice_from_external_parts(buffer.data64, buffer.channel_count as usize) })
}

fn output_data32(buffer: &mut clap_audio_buffer) -> Result<&mut [*mut f32], AudioBufferError> {
    if buffer.data32.is_null() && buffer.channel_count > 0 {
        return Err(AudioBufferError::InvalidChannelBuffer);
    }
    Ok(unsafe { slice_from_external_parts_mut(buffer.data32, buffer.channel_count as usize) })
}

fn output_data64(buffer: &mut clap_audio_buffer) -> Result<&mut [*mut f64], AudioBufferError> {
    if buffer.data64.is_null() && buffer.channel_count > 0 {
        return Err(AudioBufferError::InvalidChannelBuffer);
    }
    Ok(unsafe { slice_from_external_parts_mut(buffer.data64, buffer.channel_count as usize) })
}

fn validate_buffers(
    inputs: &[clap_audio_buffer],
    outputs: &[clap_audio_buffer],
    frames_count: u32,
) -> Result<(), AudioBufferError> {
    for buffer in inputs {
        validate_buffer_channels(buffer, frames_count)?;
    }
    for buffer in outputs {
        validate_buffer_channels(buffer, frames_count)?;
    }
    validate_unique_output_pointers(outputs, frames_count)
}

fn validate_buffer_channels(
    buffer: &clap_audio_buffer,
    frames_count: u32,
) -> Result<(), AudioBufferError> {
    let _ = sample_mask(buffer)?;
    if frames_count == 0 {
        return Ok(());
    }

    if !buffer.data32.is_null() {
        validate_channel_pointer_array(buffer.data32.cast_const(), buffer.channel_count)?;
    }
    if !buffer.data64.is_null() {
        validate_channel_pointer_array(buffer.data64.cast_const(), buffer.channel_count)?;
    }
    Ok(())
}

fn validate_channel_pointer_array<T>(
    data: *const *mut T,
    channel_count: u32,
) -> Result<(), AudioBufferError> {
    if data.is_null() && channel_count > 0 {
        return Err(AudioBufferError::InvalidChannelBuffer);
    }
    for channel_index in 0..channel_count as usize {
        if unsafe { *data.add(channel_index) }.is_null() {
            return Err(AudioBufferError::InvalidChannelBuffer);
        }
    }
    Ok(())
}

fn validate_unique_output_pointers(
    outputs: &[clap_audio_buffer],
    frames_count: u32,
) -> Result<(), AudioBufferError> {
    if frames_count == 0 {
        return Ok(());
    }
    for port_index in 0..outputs.len() {
        let output = &outputs[port_index];
        if !output.data32.is_null() {
            validate_unique_output_pointer_type(outputs, port_index, |buffer| buffer.data32)?;
        }
        if !output.data64.is_null() {
            validate_unique_output_pointer_type(outputs, port_index, |buffer| buffer.data64)?;
        }
    }
    Ok(())
}

fn validate_unique_output_pointer_type<T>(
    outputs: &[clap_audio_buffer],
    current_port_index: usize,
    data: impl Fn(&clap_audio_buffer) -> *mut *mut T,
) -> Result<(), AudioBufferError> {
    let current = &outputs[current_port_index];
    let current_data = data(current);
    for current_channel in 0..current.channel_count as usize {
        let current_ptr = unsafe { *current_data.add(current_channel) };
        for (previous_port_index, previous) in
            outputs.iter().enumerate().take(current_port_index + 1)
        {
            let previous_data = data(previous);
            if previous_data.is_null() {
                continue;
            }
            let previous_channel_limit = if previous_port_index == current_port_index {
                current_channel
            } else {
                previous.channel_count as usize
            };
            for previous_channel in 0..previous_channel_limit {
                let previous_ptr = unsafe { *previous_data.add(previous_channel) };
                if current_ptr == previous_ptr {
                    return Err(AudioBufferError::AliasedOutputBuffer);
                }
            }
        }
    }
    Ok(())
}

unsafe fn slice_from_external_parts<'a, T>(data: *const T, len: usize) -> &'a [T] {
    if len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(data, len) }
    }
}

unsafe fn slice_from_external_parts_mut<'a, T>(data: *mut T, len: usize) -> &'a mut [T] {
    if len == 0 {
        &mut []
    } else {
        unsafe { std::slice::from_raw_parts_mut(data, len) }
    }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use clap_sys::audio_buffer::clap_audio_buffer;

    use super::{AudioBufferError, AudioChannelPair, AudioPortChannels, AudioProcessBuffer};

    fn buffer32(channels: &mut [*mut f32]) -> clap_audio_buffer {
        clap_audio_buffer {
            data32: channels.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: channels.len() as u32,
            latency: 0,
            constant_mask: 0,
        }
    }

    #[test]
    fn separate_buffers_support_more_than_two_channels() {
        let mut input_l = [1.0_f32, 2.0];
        let mut input_r = [3.0_f32, 4.0];
        let mut input_c = [5.0_f32, 6.0];
        let mut input_channels = [
            input_l.as_mut_ptr(),
            input_r.as_mut_ptr(),
            input_c.as_mut_ptr(),
        ];
        let inputs = [buffer32(&mut input_channels)];

        let mut output_l = [0.0_f32; 2];
        let mut output_r = [0.0_f32; 2];
        let mut output_c = [0.0_f32; 2];
        let mut output_channels = [
            output_l.as_mut_ptr(),
            output_r.as_mut_ptr(),
            output_c.as_mut_ptr(),
        ];
        let mut outputs = [buffer32(&mut output_channels)];

        {
            let mut audio =
                unsafe { AudioProcessBuffer::from_raw_buffers(&inputs, &mut outputs, 2) }.unwrap();
            assert_eq!(audio.port_pair_count(), 1);
            let mut port = audio.port_pair(0).unwrap();
            let AudioPortChannels::F32(channels) = port.channels().unwrap() else {
                panic!("expected f32 channels");
            };
            assert_eq!(channels.channel_pair_count(), 3);

            for mut channel in channels {
                assert!(matches!(channel, AudioChannelPair::InputOutput(_, _)));
                channel.map_samples(|sample| sample * 2.0);
            }
        }

        assert_eq!(output_l, [2.0, 4.0]);
        assert_eq!(output_r, [6.0, 8.0]);
        assert_eq!(output_c, [10.0, 12.0]);
    }

    #[test]
    fn in_place_alias_is_exposed_without_separate_input_slice() {
        let mut left = [1.0_f32, 2.0];
        let mut right = [3.0_f32, 4.0];
        let mut input_channels = [left.as_mut_ptr(), right.as_mut_ptr()];
        let inputs = [buffer32(&mut input_channels)];
        let mut output_channels = [left.as_mut_ptr(), right.as_mut_ptr()];
        let mut outputs = [buffer32(&mut output_channels)];

        {
            let mut audio =
                unsafe { AudioProcessBuffer::from_raw_buffers(&inputs, &mut outputs, 2) }.unwrap();
            let mut port = audio.port_pair(0).unwrap();
            let AudioPortChannels::F32(channels) = port.channels().unwrap() else {
                panic!("expected f32 channels");
            };

            for mut channel in channels {
                assert!(matches!(channel, AudioChannelPair::InPlace(_)));
                channel.map_samples(|sample| sample * 3.0);
            }
        }

        assert_eq!(left, [3.0, 6.0]);
        assert_eq!(right, [9.0, 12.0]);
    }

    #[test]
    fn output_only_channels_are_silenced_by_map_samples() {
        let inputs = [];
        let mut output = [1.0_f32, 2.0, 3.0];
        let mut output_channels = [output.as_mut_ptr()];
        let mut outputs = [buffer32(&mut output_channels)];

        {
            let mut audio =
                unsafe { AudioProcessBuffer::from_raw_buffers(&inputs, &mut outputs, 3) }.unwrap();
            let mut port = audio.port_pair(0).unwrap();
            let AudioPortChannels::F32(mut channels) = port.channels().unwrap() else {
                panic!("expected f32 channels");
            };
            let mut channel = channels.channel_pair(0).unwrap();
            assert!(matches!(channel, AudioChannelPair::OutputOnly(_)));
            channel.map_samples(|sample| sample * 10.0);
        }

        assert_eq!(output, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn duplicate_output_channel_pointer_is_rejected() {
        let inputs = [];
        let mut output = [0.0_f32; 2];
        let mut output_channels = [output.as_mut_ptr(), output.as_mut_ptr()];
        let mut outputs = [buffer32(&mut output_channels)];

        let result = unsafe { AudioProcessBuffer::from_raw_buffers(&inputs, &mut outputs, 2) };
        assert_eq!(result.err(), Some(AudioBufferError::AliasedOutputBuffer));
    }

    #[test]
    fn asymmetric_port_count_is_visible_as_port_pairs() {
        let mut input = [1.0_f32, 2.0];
        let mut input_channels = [input.as_mut_ptr()];
        let inputs = [buffer32(&mut input_channels)];

        let mut output_a = [0.0_f32; 2];
        let mut output_b = [0.0_f32; 2];
        let mut output_channels_a = [output_a.as_mut_ptr()];
        let mut output_channels_b = [output_b.as_mut_ptr()];
        let mut outputs = [
            buffer32(&mut output_channels_a),
            buffer32(&mut output_channels_b),
        ];

        let mut audio =
            unsafe { AudioProcessBuffer::from_raw_buffers(&inputs, &mut outputs, 2) }.unwrap();
        assert_eq!(audio.port_pair_count(), 2);

        let mut pairs = audio.port_pairs();
        let first = pairs.next().unwrap();
        assert_eq!(first.channel_pair_count(), 1);
        let second = pairs.next().unwrap();
        assert_eq!(second.channel_pair_count(), 1);
        assert!(pairs.next().is_none());
    }
}
