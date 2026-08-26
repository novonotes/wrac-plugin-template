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
