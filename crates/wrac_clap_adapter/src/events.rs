use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr;

use clap_sys::events::{
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_CHOKE, CLAP_EVENT_NOTE_END,
    CLAP_EVENT_NOTE_EXPRESSION, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON,
    CLAP_EVENT_PARAM_GESTURE_BEGIN, CLAP_EVENT_PARAM_GESTURE_END, CLAP_EVENT_PARAM_MOD,
    CLAP_EVENT_PARAM_VALUE, CLAP_EVENT_TRANSPORT, clap_event_header, clap_event_note,
    clap_event_note_expression, clap_event_param_gesture, clap_event_param_mod,
    clap_event_param_value, clap_event_transport, clap_input_events, clap_note_expression,
    clap_output_events, clap_transport_flags,
};

use crate::api::ParameterValueEvent;

/// `process()` / `flush()` の CLAP event list を callback lifetime に閉じ込める view。
///
/// event list の実体は host が所有し、callback が終わると無効になる。製品コードへ raw pointer
/// を渡さず typed enum へ寄せることで、sample accurate automation や note event を扱う
/// 場所を audio callback の範囲内に限定する。
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
            log::debug!("input_events.len: null input event list");
            return 0;
        }
        let Some(size) = (unsafe { (*self.raw).size }) else {
            log::warn!("input_events.len: event list has no size callback");
            return 0;
        };
        unsafe { size(self.raw) }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: u32) -> Option<InputEvent> {
        if index >= self.len() || self.raw.is_null() {
            log::warn!("input_events.get: invalid index={index}");
            return None;
        }
        let Some(get) = (unsafe { (*self.raw).get }) else {
            log::warn!("input_events.get: event list has no get callback index={index}");
            return None;
        };
        let header = unsafe { get(self.raw, index) };
        if header.is_null() {
            log::warn!("input_events.get: host returned null event header index={index}");
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

    pub fn parameter_values(&self) -> impl Iterator<Item = ParameterValueEvent> + '_ {
        self.iter().filter_map(|event| match event {
            InputEvent::ParamValue(event) => Some(event),
            _ => None,
        })
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
            log::warn!("output_events.try_push: output event queue is unavailable");
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
            log::warn!("output_events.try_push: host rejected event");
        }
        pushed
    }

    fn try_push_raw(
        &self,
    ) -> Option<unsafe extern "C" fn(*const clap_output_events, *const clap_event_header) -> bool>
    {
        if self.raw.is_null() {
            log::debug!("output_events.try_push_raw: null output event list");
            return None;
        }
        let try_push = unsafe { (*self.raw).try_push };
        if try_push.is_none() {
            log::warn!("output_events.try_push_raw: event list has no try_push callback");
        }
        try_push
    }
}

#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    NoteOn(NoteEvent),
    NoteOff(NoteEvent),
    NoteChoke(NoteEvent),
    NoteEnd(NoteEvent),
    NoteExpression(NoteExpressionEvent),
    ParamValue(ParameterValueEvent),
    ParamMod(ParameterModEvent),
    ParamGestureBegin(ParameterGestureEvent),
    ParamGestureEnd(ParameterGestureEvent),
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
            CLAP_EVENT_PARAM_VALUE if has_size::<clap_event_param_value>(header) => {
                Some(Self::ParamValue(parameter_value_from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_PARAM_MOD if has_size::<clap_event_param_mod>(header) => {
                Some(Self::ParamMod(ParameterModEvent::from_raw(unsafe {
                    cast_event(header)
                })))
            }
            CLAP_EVENT_PARAM_GESTURE_BEGIN if has_size::<clap_event_param_gesture>(header) => {
                Some(Self::ParamGestureBegin(ParameterGestureEvent::from_raw(
                    unsafe { cast_event(header) },
                )))
            }
            CLAP_EVENT_PARAM_GESTURE_END if has_size::<clap_event_param_gesture>(header) => {
                Some(Self::ParamGestureEnd(ParameterGestureEvent::from_raw(
                    unsafe { cast_event(header) },
                )))
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
            Self::ParamValue(event) => event.time,
            Self::ParamMod(event) => event.time,
            Self::ParamGestureBegin(event) | Self::ParamGestureEnd(event) => event.time,
            Self::Transport(event) => event.time,
            Self::Unknown(event) => event.time,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum OutputEvent {
    NoteOn(NoteEvent),
    NoteOff(NoteEvent),
    NoteChoke(NoteEvent),
    NoteEnd(NoteEvent),
    NoteExpression(NoteExpressionEvent),
    ParamValue(ParameterValueEvent),
    ParamMod(ParameterModEvent),
    ParamGestureBegin(ParameterGestureEvent),
    ParamGestureEnd(ParameterGestureEvent),
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
pub struct ParameterModEvent {
    pub time: u32,
    pub parameter_id: u32,
    pub amount: f64,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
}

impl ParameterModEvent {
    fn from_raw(raw: &clap_event_param_mod) -> Self {
        Self {
            time: raw.header.time,
            parameter_id: raw.param_id,
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
            param_id: self.parameter_id,
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
pub struct ParameterGestureEvent {
    pub time: u32,
    pub parameter_id: u32,
}

impl ParameterGestureEvent {
    fn from_raw(raw: &clap_event_param_gesture) -> Self {
        Self {
            time: raw.header.time,
            parameter_id: raw.param_id,
        }
    }

    fn to_raw(self, event_type: u16) -> clap_event_param_gesture {
        clap_event_param_gesture {
            header: event_header::<clap_event_param_gesture>(self.time, event_type),
            param_id: self.parameter_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransportEvent {
    pub time: u32,
    pub flags: clap_transport_flags,
    pub tempo: f64,
    pub tempo_inc: f64,
    pub song_pos_beats: i64,
    pub song_pos_seconds: i64,
    pub loop_start_beats: i64,
    pub loop_end_beats: i64,
    pub loop_start_seconds: i64,
    pub loop_end_seconds: i64,
    pub bar_start: i64,
    pub bar_number: i32,
    pub tsig_num: u16,
    pub tsig_denom: u16,
}

impl TransportEvent {
    fn from_raw(raw: &clap_event_transport) -> Self {
        Self {
            time: raw.header.time,
            flags: raw.flags,
            tempo: raw.tempo,
            tempo_inc: raw.tempo_inc,
            song_pos_beats: raw.song_pos_beats,
            song_pos_seconds: raw.song_pos_seconds,
            loop_start_beats: raw.loop_start_beats,
            loop_end_beats: raw.loop_end_beats,
            loop_start_seconds: raw.loop_start_seconds,
            loop_end_seconds: raw.loop_end_seconds,
            bar_start: raw.bar_start,
            bar_number: raw.bar_number,
            tsig_num: raw.tsig_num,
            tsig_denom: raw.tsig_denom,
        }
    }
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

fn parameter_value_from_raw(raw: &clap_event_param_value) -> ParameterValueEvent {
    ParameterValueEvent {
        time: raw.header.time,
        parameter_id: raw.param_id,
        value: raw.value,
        note_id: raw.note_id,
        port_index: raw.port_index,
        channel: raw.channel,
        key: raw.key,
    }
}

fn param_value_to_raw(event: ParameterValueEvent) -> clap_event_param_value {
    clap_event_param_value {
        header: event_header::<clap_event_param_value>(event.time, CLAP_EVENT_PARAM_VALUE),
        param_id: event.parameter_id,
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
        CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_VALUE, clap_event_header,
        clap_event_note, clap_event_param_value, clap_input_events,
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
                assert_eq!(event.parameter_id, 7);
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
        match parsed[0] {
            InputEvent::ParamValue(event) => assert_eq!(event.parameter_id, 9),
            _ => panic!("expected param value"),
        }
    }
}
