use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr;

use clap_sys::events::{
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, CLAP_EVENT_MIDI_SYSEX, CLAP_EVENT_MIDI2,
    CLAP_EVENT_NOTE_CHOKE, CLAP_EVENT_NOTE_END, CLAP_EVENT_NOTE_EXPRESSION, CLAP_EVENT_NOTE_OFF,
    CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_GESTURE_BEGIN, CLAP_EVENT_PARAM_GESTURE_END,
    CLAP_EVENT_PARAM_MOD, CLAP_EVENT_PARAM_VALUE, CLAP_EVENT_TRANSPORT, clap_event_header,
    clap_event_midi, clap_event_midi_sysex, clap_event_midi2, clap_event_note,
    clap_event_note_expression, clap_event_param_gesture, clap_event_param_mod,
    clap_event_param_value, clap_event_transport, clap_input_events, clap_note_expression,
    clap_output_events,
};

use crate::api::ParamValueEvent;

const CLAP_FIXED_TIME_FACTOR: f64 = (1_i64 << 31) as f64;
const TRANSPORT_HAS_TEMPO: u32 = 1 << 0;
const TRANSPORT_HAS_BEATS_TIMELINE: u32 = 1 << 1;
const TRANSPORT_HAS_SECONDS_TIMELINE: u32 = 1 << 2;
const TRANSPORT_HAS_TIME_SIGNATURE: u32 = 1 << 3;
const TRANSPORT_IS_PLAYING: u32 = 1 << 4;

/// View that confines the CLAP event lists from `process()`/`flush()` to the callback lifetime.
///
/// The underlying data is owned by the host and is invalid after the callback returns.
/// Raw pointers are not exposed to the product; events are converted to typed enums so
/// they can only be handled within the audio callback.
pub struct ProcessEvents<'a> {
    pub input: InputEvents<'a>,
    pub output: OutputEvents<'a>,
}

impl<'a> ProcessEvents<'a> {
    pub(crate) unsafe fn from_raw(
        input: *const clap_input_events,
        output: *const clap_output_events,
    ) -> Self {
        Self {
            input: unsafe { InputEvents::from_raw(input) },
            output: unsafe { OutputEvents::from_raw(output) },
        }
    }
}

#[derive(Clone, Copy)]
pub struct InputEvents<'a> {
    raw: *const clap_input_events,
    _marker: PhantomData<&'a clap_input_events>,
}

impl<'a> InputEvents<'a> {
    pub(crate) unsafe fn from_raw(raw: *const clap_input_events) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    pub fn len(&self) -> u32 {
        if self.raw.is_null() {
            wrac_log::rtdebug!("input_events.len: null input event list");
            return 0;
        }
        let Some(size) = (unsafe { (*self.raw).size }) else {
            wrac_log::rtwarn!("input_events.len: event list has no size callback");
            return 0;
        };
        unsafe { size(self.raw) }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: u32) -> Option<InputEvent> {
        if index >= self.len() || self.raw.is_null() {
            wrac_log::rtwarn!("input_events.get: invalid index={index}");
            return None;
        }
        let Some(get) = (unsafe { (*self.raw).get }) else {
            wrac_log::rtwarn!("input_events.get: event list has no get callback index={index}");
            return None;
        };
        let header = unsafe { get(self.raw, index) };
        if header.is_null() {
            wrac_log::rtwarn!("input_events.get: host returned null event header index={index}");
            return None;
        }
        unsafe { InputEvent::from_header(&*header) }
    }

    pub fn iter(&self) -> InputEventsIter<'a> {
        InputEventsIter {
            events: *self,
            index: 0,
            len: self.len(),
        }
    }

    pub fn parameter_values(&self) -> impl Iterator<Item = ParamValueEvent> + '_ {
        self.iter().filter_map(|event| match event {
            InputEvent::ParamValue(event) => Some(event),
            _ => None,
        })
    }

    pub fn param_events(&self) -> ParamInputEvents<'a> {
        ParamInputEvents { input: *self }
    }
}

/// View over parameter input events delivered by `params.flush()`.
///
/// This preserves the CLAP callback's event-list boundary while exposing only
/// parameter value events to product code. The underlying host-owned event list is
/// confined to the callback lifetime.
#[derive(Clone, Copy)]
pub struct ParamInputEvents<'a> {
    input: InputEvents<'a>,
}

impl<'a> ParamInputEvents<'a> {
    pub fn values(&self) -> impl Iterator<Item = ParamValueEvent> + '_ {
        self.input.parameter_values()
    }

    pub fn is_empty(&self) -> bool {
        self.values().next().is_none()
    }
}

pub struct InputEventsIter<'a> {
    events: InputEvents<'a>,
    index: u32,
    len: u32,
}

impl Iterator for InputEventsIter<'_> {
    type Item = InputEvent;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.len {
            let event = self.events.get(self.index);
            self.index += 1;
            if event.is_some() {
                return event;
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len.saturating_sub(self.index) as usize;
        (0, Some(remaining))
    }
}

pub struct OutputEvents<'a> {
    raw: *const clap_output_events,
    _marker: PhantomData<&'a mut clap_output_events>,
}

impl<'a> OutputEvents<'a> {
    pub(crate) unsafe fn from_raw(raw: *const clap_output_events) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    pub fn try_push(&mut self, event: OutputEvent) -> bool {
        let Some(try_push) = self.try_push_raw() else {
            wrac_log::rtwarn!("output_events.try_push: output event queue is unavailable");
            return false;
        };

        let pushed = match event {
            OutputEvent::NoteOn(event) => {
                let raw = event.to_raw(CLAP_EVENT_NOTE_ON);
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::NoteOff(event) => {
                let raw = event.to_raw(CLAP_EVENT_NOTE_OFF);
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::NoteChoke(event) => {
                let raw = event.to_raw(CLAP_EVENT_NOTE_CHOKE);
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::NoteEnd(event) => {
                let raw = event.to_raw(CLAP_EVENT_NOTE_END);
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::NoteExpression(event) => {
                let raw = event.to_raw();
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::Midi(event) => {
                let raw = event.to_raw();
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::MidiSysex(event) => {
                let raw = event.to_raw();
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::Midi2(event) => {
                let raw = event.to_raw();
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::ParamValue(event) => {
                let raw = param_value_to_raw(event);
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::ParamMod(event) => {
                let raw = event.to_raw();
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::ParamGestureBegin(event) => {
                let raw = event.to_raw(CLAP_EVENT_PARAM_GESTURE_BEGIN);
                unsafe { try_push(self.raw, &raw.header) }
            }
            OutputEvent::ParamGestureEnd(event) => {
                let raw = event.to_raw(CLAP_EVENT_PARAM_GESTURE_END);
                unsafe { try_push(self.raw, &raw.header) }
            }
        };
        if !pushed {
            wrac_log::rtwarn!("output_events.try_push: host rejected event");
        }
        pushed
    }

    fn try_push_raw(
        &self,
    ) -> Option<unsafe extern "C" fn(*const clap_output_events, *const clap_event_header) -> bool>
    {
        if self.raw.is_null() {
            wrac_log::rtdebug!("output_events.try_push_raw: null output event list");
            return None;
        }
        let try_push = unsafe { (*self.raw).try_push };
        if try_push.is_none() {
            wrac_log::rtwarn!("output_events.try_push_raw: event list has no try_push callback");
        }
        try_push
    }
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    NoteOn(NoteEvent),
    NoteOff(NoteEvent),
    NoteChoke(NoteEvent),
    NoteEnd(NoteEvent),
    NoteExpression(NoteExpressionEvent),
    Midi(MidiEvent),
    MidiSysex(MidiSysexEvent),
    Midi2(Midi2Event),
    ParamValue(ParamValueEvent),
    ParamMod(ParamModEvent),
    ParamGestureBegin(ParamGestureEvent),
    ParamGestureEnd(ParamGestureEvent),
    Transport(TransportEvent),
    Unknown(UnknownEvent),
}

impl InputEvent {
    unsafe fn from_header(header: &clap_event_header) -> Option<Self> {
        if header.space_id != CLAP_CORE_EVENT_SPACE_ID {
            return Some(Self::Unknown(UnknownEvent::from_header(header)));
        }

        match header.type_ {
            CLAP_EVENT_NOTE_ON if has_size::<clap_event_note>(header) => {
                Some(Self::NoteOn(NoteEvent::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_NOTE_OFF if has_size::<clap_event_note>(header) => {
                Some(Self::NoteOff(NoteEvent::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_NOTE_CHOKE if has_size::<clap_event_note>(header) => {
                Some(Self::NoteChoke(NoteEvent::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_NOTE_END if has_size::<clap_event_note>(header) => {
                Some(Self::NoteEnd(NoteEvent::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_NOTE_EXPRESSION if has_size::<clap_event_note_expression>(header) => Some(
                Self::NoteExpression(NoteExpressionEvent::from_raw(unsafe { cast_event(header) })),
            ),
            CLAP_EVENT_MIDI if has_size::<clap_event_midi>(header) => {
                Some(midi_event_from_raw(unsafe { cast_event(header) }))
            }
            CLAP_EVENT_MIDI_SYSEX if has_size::<clap_event_midi_sysex>(header) => {
                Some(Self::MidiSysex(MidiSysexEvent::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_MIDI2 if has_size::<clap_event_midi2>(header) => {
                Some(Self::Midi2(Midi2Event::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_PARAM_VALUE if has_size::<clap_event_param_value>(header) => {
                Some(Self::ParamValue(parameter_value_from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_PARAM_MOD if has_size::<clap_event_param_mod>(header) => {
                Some(Self::ParamMod(ParamModEvent::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_PARAM_GESTURE_BEGIN if has_size::<clap_event_param_gesture>(header) => Some(
                Self::ParamGestureBegin(ParamGestureEvent::from_raw(unsafe { cast_event(header) })),
            ),
            CLAP_EVENT_PARAM_GESTURE_END if has_size::<clap_event_param_gesture>(header) => {
                Some(Self::ParamGestureEnd(ParamGestureEvent::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_TRANSPORT if has_size::<clap_event_transport>(header) => {
                Some(Self::Transport(TransportEvent::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            _ => Some(Self::Unknown(UnknownEvent::from_header(header))),
        }
    }

    pub fn time(&self) -> u32 {
        match self {
            Self::NoteOn(event)
            | Self::NoteOff(event)
            | Self::NoteChoke(event)
            | Self::NoteEnd(event) => event.time,
            Self::NoteExpression(event) => event.time,
            Self::Midi(event) => event.time,
            Self::MidiSysex(event) => event.time,
            Self::Midi2(event) => event.time,
            Self::ParamValue(event) => event.time,
            Self::ParamMod(event) => event.time,
            Self::ParamGestureBegin(event) | Self::ParamGestureEnd(event) => event.time,
            Self::Transport(event) => event.time,
            Self::Unknown(event) => event.time,
        }
    }
}

fn midi_event_from_raw(raw: &clap_event_midi) -> InputEvent {
    // CLAP hosts must not send the same MIDI note twice as both raw MIDI and CLAP note
    // events. Normalize channel note messages at the adapter boundary so processors do
    // not need their own duplicate-note suppression policy.
    let status = raw.data[0] & 0xF0;
    let channel = raw.data[0] & 0x0F;
    let key = raw.data[1];
    let velocity = raw.data[2];
    let note = NoteEvent {
        time: raw.header.time,
        note_id: -1,
        port_index: raw.port_index as i16,
        channel: channel as i16,
        key: key as i16,
        velocity: f64::from(velocity) / 127.0,
    };

    match status {
        0x80 => InputEvent::NoteOff(note),
        0x90 if velocity == 0 => InputEvent::NoteOff(note),
        0x90 => InputEvent::NoteOn(note),
        _ => InputEvent::Midi(MidiEvent::from_raw(raw)),
    }
}

#[derive(Debug, Clone)]
pub enum OutputEvent {
    NoteOn(NoteEvent),
    NoteOff(NoteEvent),
    NoteChoke(NoteEvent),
    NoteEnd(NoteEvent),
    NoteExpression(NoteExpressionEvent),
    Midi(MidiEvent),
    MidiSysex(MidiSysexEvent),
    Midi2(Midi2Event),
    ParamValue(ParamValueEvent),
    ParamMod(ParamModEvent),
    ParamGestureBegin(ParamGestureEvent),
    ParamGestureEnd(ParamGestureEvent),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEvent {
    pub time: u32,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
    pub velocity: f64,
}

impl NoteEvent {
    fn from_raw(raw: &clap_event_note) -> Self {
        Self {
            time: raw.header.time,
            note_id: raw.note_id,
            port_index: raw.port_index,
            channel: raw.channel,
            key: raw.key,
            velocity: raw.velocity,
        }
    }

    fn to_raw(self, event_type: u16) -> clap_event_note {
        clap_event_note {
            header: event_header::<clap_event_note>(self.time, event_type),
            note_id: self.note_id,
            port_index: self.port_index,
            channel: self.channel,
            key: self.key,
            velocity: self.velocity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiEvent {
    pub time: u32,
    pub port_index: u16,
    pub data: [u8; 3],
}

impl MidiEvent {
    fn from_raw(raw: &clap_event_midi) -> Self {
        Self {
            time: raw.header.time,
            port_index: raw.port_index,
            data: raw.data,
        }
    }

    fn to_raw(self) -> clap_event_midi {
        clap_event_midi {
            header: event_header::<clap_event_midi>(self.time, CLAP_EVENT_MIDI),
            port_index: self.port_index,
            data: self.data,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiSysexEvent {
    pub time: u32,
    pub port_index: u16,
    pub data: Vec<u8>,
}

impl MidiSysexEvent {
    fn from_raw(raw: &clap_event_midi_sysex) -> Self {
        let data = if raw.buffer.is_null() || raw.size == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(raw.buffer, raw.size as usize).to_vec() }
        };
        Self {
            time: raw.header.time,
            port_index: raw.port_index,
            data,
        }
    }

    fn to_raw(&self) -> clap_event_midi_sysex {
        clap_event_midi_sysex {
            header: event_header::<clap_event_midi_sysex>(self.time, CLAP_EVENT_MIDI_SYSEX),
            port_index: self.port_index,
            buffer: self.data.as_ptr(),
            size: self.data.len() as u32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Midi2Event {
    pub time: u32,
    pub port_index: u16,
    pub data: [u32; 4],
}

impl Midi2Event {
    fn from_raw(raw: &clap_event_midi2) -> Self {
        Self {
            time: raw.header.time,
            port_index: raw.port_index,
            data: raw.data,
        }
    }

    fn to_raw(self) -> clap_event_midi2 {
        clap_event_midi2 {
            header: event_header::<clap_event_midi2>(self.time, CLAP_EVENT_MIDI2),
            port_index: self.port_index,
            data: self.data,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteExpressionEvent {
    pub time: u32,
    pub expression_id: clap_note_expression,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
    pub value: f64,
}

impl NoteExpressionEvent {
    fn from_raw(raw: &clap_event_note_expression) -> Self {
        Self {
            time: raw.header.time,
            expression_id: raw.expression_id,
            note_id: raw.note_id,
            port_index: raw.port_index,
            channel: raw.channel,
            key: raw.key,
            value: raw.value,
        }
    }

    fn to_raw(self) -> clap_event_note_expression {
        clap_event_note_expression {
            header: event_header::<clap_event_note_expression>(
                self.time,
                CLAP_EVENT_NOTE_EXPRESSION,
            ),
            expression_id: self.expression_id,
            note_id: self.note_id,
            port_index: self.port_index,
            channel: self.channel,
            key: self.key,
            value: self.value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamModEvent {
    pub time: u32,
    pub param_id: u32,
    pub amount: f64,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
}

impl ParamModEvent {
    fn from_raw(raw: &clap_event_param_mod) -> Self {
        Self {
            time: raw.header.time,
            param_id: raw.param_id,
            amount: raw.amount,
            note_id: raw.note_id,
            port_index: raw.port_index,
            channel: raw.channel,
            key: raw.key,
        }
    }

    fn to_raw(self) -> clap_event_param_mod {
        clap_event_param_mod {
            header: event_header::<clap_event_param_mod>(self.time, CLAP_EVENT_PARAM_MOD),
            param_id: self.param_id,
            cookie: ptr::null_mut(),
            note_id: self.note_id,
            port_index: self.port_index,
            channel: self.channel,
            key: self.key,
            amount: self.amount,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamGestureEvent {
    pub time: u32,
    pub param_id: u32,
}

impl ParamGestureEvent {
    fn from_raw(raw: &clap_event_param_gesture) -> Self {
        Self {
            time: raw.header.time,
            param_id: raw.param_id,
        }
    }

    fn to_raw(self, event_type: u16) -> clap_event_param_gesture {
        clap_event_param_gesture {
            header: event_header::<clap_event_param_gesture>(self.time, event_type),
            param_id: self.param_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransportFlags(u32);

impl TransportFlags {
    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn has_tempo(self) -> bool {
        self.0 & TRANSPORT_HAS_TEMPO != 0
    }

    pub const fn has_beats_timeline(self) -> bool {
        self.0 & TRANSPORT_HAS_BEATS_TIMELINE != 0
    }

    pub const fn has_seconds_timeline(self) -> bool {
        self.0 & TRANSPORT_HAS_SECONDS_TIMELINE != 0
    }

    pub const fn has_time_signature(self) -> bool {
        self.0 & TRANSPORT_HAS_TIME_SIGNATURE != 0
    }

    pub const fn is_playing(self) -> bool {
        self.0 & TRANSPORT_IS_PLAYING != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransportEvent {
    pub time: u32,
    pub flags: TransportFlags,
    pub tempo: f64,
    pub tempo_inc: f64,
    pub song_position_beats: f64,
    pub song_position_seconds: f64,
    pub loop_start_beats: f64,
    pub loop_end_beats: f64,
    pub loop_start_seconds: f64,
    pub loop_end_seconds: f64,
    pub bar_start_beats: f64,
    pub bar_number: i32,
    pub tsig_num: u16,
    pub tsig_denom: u16,
}

impl TransportEvent {
    pub fn song_position_beats(self) -> f64 {
        self.song_position_beats
    }

    pub fn song_position_seconds(self) -> f64 {
        self.song_position_seconds
    }

    pub(crate) fn from_raw(raw: &clap_event_transport) -> Self {
        Self {
            time: raw.header.time,
            flags: TransportFlags(raw.flags),
            tempo: raw.tempo,
            tempo_inc: raw.tempo_inc,
            song_position_beats: fixed_time_to_float(raw.song_pos_beats),
            song_position_seconds: fixed_time_to_float(raw.song_pos_seconds),
            loop_start_beats: fixed_time_to_float(raw.loop_start_beats),
            loop_end_beats: fixed_time_to_float(raw.loop_end_beats),
            loop_start_seconds: fixed_time_to_float(raw.loop_start_seconds),
            loop_end_seconds: fixed_time_to_float(raw.loop_end_seconds),
            bar_start_beats: fixed_time_to_float(raw.bar_start),
            bar_number: raw.bar_number,
            tsig_num: raw.tsig_num,
            tsig_denom: raw.tsig_denom,
        }
    }
}

fn fixed_time_to_float(value: i64) -> f64 {
    value as f64 / CLAP_FIXED_TIME_FACTOR
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownEvent {
    pub time: u32,
    pub space_id: u16,
    pub event_type: u16,
}

impl UnknownEvent {
    fn from_header(header: &clap_event_header) -> Self {
        Self {
            time: header.time,
            space_id: header.space_id,
            event_type: header.type_,
        }
    }
}

fn parameter_value_from_raw(raw: &clap_event_param_value) -> ParamValueEvent {
    ParamValueEvent {
        time: raw.header.time,
        param_id: raw.param_id,
        value: raw.value,
        note_id: raw.note_id,
        port_index: raw.port_index,
        channel: raw.channel,
        key: raw.key,
    }
}

fn param_value_to_raw(event: ParamValueEvent) -> clap_event_param_value {
    clap_event_param_value {
        header: event_header::<clap_event_param_value>(event.time, CLAP_EVENT_PARAM_VALUE),
        param_id: event.param_id,
        cookie: ptr::null_mut(),
        note_id: event.note_id,
        port_index: event.port_index,
        channel: event.channel,
        key: event.key,
        value: event.value,
    }
}

fn event_header<T>(time: u32, event_type: u16) -> clap_event_header {
    clap_event_header {
        size: size_of::<T>() as u32,
        time,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_: event_type,
        flags: 0,
    }
}

fn has_size<T>(header: &clap_event_header) -> bool {
    header.size as usize >= size_of::<T>()
}

unsafe fn cast_event<T>(header: &clap_event_header) -> &T {
    unsafe { &*(header as *const clap_event_header as *const T) }
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;

    use clap_sys::events::{
        CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, CLAP_EVENT_MIDI_SYSEX, CLAP_EVENT_MIDI2,
        CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_VALUE, clap_event_header, clap_event_midi,
        clap_event_midi_sysex, clap_event_midi2, clap_event_note, clap_event_param_value,
        clap_input_events,
    };

    use super::{InputEvent, InputEvents};

    struct EventList {
        events: Vec<*const clap_event_header>,
    }

    unsafe extern "C" fn event_count(list: *const clap_input_events) -> u32 {
        let list = unsafe { &*((*list).ctx as *const EventList) };
        list.events.len() as u32
    }

    unsafe extern "C" fn event_get(
        list: *const clap_input_events,
        index: u32,
    ) -> *const clap_event_header {
        let list = unsafe { &*((*list).ctx as *const EventList) };
        list.events[index as usize]
    }

    #[test]
    fn input_events_parse_param_and_note_events() {
        let param = clap_event_param_value {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_param_value>() as u32,
                time: 12,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id: 7,
            cookie: std::ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: 0.75,
        };
        let note = clap_event_note {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_note>() as u32,
                time: 18,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_NOTE_ON,
                flags: 0,
            },
            note_id: 3,
            port_index: 1,
            channel: 2,
            key: 60,
            velocity: 0.5,
        };
        let mut list_data = EventList {
            events: vec![&param.header, &note.header],
        };
        let raw = clap_input_events {
            ctx: (&mut list_data as *mut EventList).cast::<c_void>(),
            size: Some(event_count),
            get: Some(event_get),
        };
        let events = unsafe { InputEvents::from_raw(&raw) };

        assert_eq!(events.len(), 2);
        match events.get(0).unwrap() {
            InputEvent::ParamValue(event) => {
                assert_eq!(event.time, 12);
                assert_eq!(event.param_id, 7);
                assert_eq!(event.value, 0.75);
            }
            _ => panic!("expected param value"),
        }
        match events.get(1).unwrap() {
            InputEvent::NoteOn(event) => {
                assert_eq!(event.time, 18);
                assert_eq!(event.note_id, 3);
                assert_eq!(event.key, 60);
                assert_eq!(event.velocity, 0.5);
            }
            _ => panic!("expected note on"),
        }
    }

    #[test]
    fn input_events_convert_midi_note_messages() {
        let note_on = clap_event_midi {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_midi>() as u32,
                time: 10,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_MIDI,
                flags: 0,
            },
            port_index: 0,
            data: [0x91, 64, 100],
        };
        let note_off = clap_event_midi {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_midi>() as u32,
                time: 20,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_MIDI,
                flags: 0,
            },
            port_index: 0,
            data: [0x80, 64, 0],
        };
        let zero_velocity_note_on = clap_event_midi {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_midi>() as u32,
                time: 30,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_MIDI,
                flags: 0,
            },
            port_index: 0,
            data: [0x91, 67, 0],
        };
        let mut list_data = EventList {
            events: vec![
                &note_on.header,
                &note_off.header,
                &zero_velocity_note_on.header,
            ],
        };
        let raw = clap_input_events {
            ctx: (&mut list_data as *mut EventList).cast::<c_void>(),
            size: Some(event_count),
            get: Some(event_get),
        };
        let events = unsafe { InputEvents::from_raw(&raw) };

        match events.get(0).unwrap() {
            InputEvent::NoteOn(event) => {
                assert_eq!(event.time, 10);
                assert_eq!(event.channel, 1);
                assert_eq!(event.key, 64);
                assert_eq!(event.velocity, 100.0 / 127.0);
            }
            _ => panic!("expected midi note on"),
        }
        match events.get(1).unwrap() {
            InputEvent::NoteOff(event) => {
                assert_eq!(event.time, 20);
                assert_eq!(event.key, 64);
            }
            _ => panic!("expected midi note off"),
        }
        match events.get(2).unwrap() {
            InputEvent::NoteOff(event) => {
                assert_eq!(event.time, 30);
                assert_eq!(event.key, 67);
            }
            _ => panic!("expected zero-velocity note on as note off"),
        }
    }

    #[test]
    fn input_events_keep_non_note_midi_messages_raw() {
        let cc = clap_event_midi {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_midi>() as u32,
                time: 40,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_MIDI,
                flags: 0,
            },
            port_index: 2,
            data: [0xB1, 74, 100],
        };
        let mut list_data = EventList {
            events: vec![&cc.header],
        };
        let raw = clap_input_events {
            ctx: (&mut list_data as *mut EventList).cast::<c_void>(),
            size: Some(event_count),
            get: Some(event_get),
        };
        let events = unsafe { InputEvents::from_raw(&raw) };

        match events.get(0).unwrap() {
            InputEvent::Midi(event) => {
                assert_eq!(event.time, 40);
                assert_eq!(event.port_index, 2);
                assert_eq!(event.data, [0xB1, 74, 100]);
            }
            _ => panic!("expected raw MIDI CC"),
        }
    }

    #[test]
    fn input_events_copy_sysex_and_midi2_messages() {
        let sysex_data = [0xF0, 0x7D, 0x01, 0xF7];
        let sysex = clap_event_midi_sysex {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_midi_sysex>() as u32,
                time: 50,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_MIDI_SYSEX,
                flags: 0,
            },
            port_index: 1,
            buffer: sysex_data.as_ptr(),
            size: sysex_data.len() as u32,
        };
        let midi2 = clap_event_midi2 {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_midi2>() as u32,
                time: 60,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_MIDI2,
                flags: 0,
            },
            port_index: 3,
            data: [1, 2, 3, 4],
        };
        let mut list_data = EventList {
            events: vec![&sysex.header, &midi2.header],
        };
        let raw = clap_input_events {
            ctx: (&mut list_data as *mut EventList).cast::<c_void>(),
            size: Some(event_count),
            get: Some(event_get),
        };
        let events = unsafe { InputEvents::from_raw(&raw) };

        match events.get(0).unwrap() {
            InputEvent::MidiSysex(event) => {
                assert_eq!(event.time, 50);
                assert_eq!(event.port_index, 1);
                assert_eq!(event.data, sysex_data);
            }
            _ => panic!("expected MIDI sysex"),
        }
        match events.get(1).unwrap() {
            InputEvent::Midi2(event) => {
                assert_eq!(event.time, 60);
                assert_eq!(event.port_index, 3);
                assert_eq!(event.data, [1, 2, 3, 4]);
            }
            _ => panic!("expected MIDI2"),
        }
    }

    #[test]
    fn input_events_iter_skips_null_slots() {
        let param = clap_event_param_value {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_param_value>() as u32,
                time: 4,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id: 9,
            cookie: std::ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: 0.25,
        };
        let mut list_data = EventList {
            events: vec![std::ptr::null(), &param.header],
        };
        let raw = clap_input_events {
            ctx: (&mut list_data as *mut EventList).cast::<c_void>(),
            size: Some(event_count),
            get: Some(event_get),
        };
        let events = unsafe { InputEvents::from_raw(&raw) };
        let parsed: Vec<_> = events.iter().collect();

        assert_eq!(parsed.len(), 1);
        match &parsed[0] {
            InputEvent::ParamValue(event) => assert_eq!(event.param_id, 9),
            _ => panic!("expected param value"),
        }
    }
}
