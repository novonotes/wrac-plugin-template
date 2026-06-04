use std::num::{NonZeroIsize, NonZeroU64, NonZeroUsize};

use clap_sys::ext::note_ports::{
    CLAP_NOTE_DIALECT_CLAP, CLAP_NOTE_DIALECT_MIDI, CLAP_NOTE_DIALECT_MIDI_MPE,
    CLAP_NOTE_DIALECT_MIDI2,
};

#[derive(Debug, Clone, Copy)]
pub struct AudioPortInfo {
    pub id: u32,
    pub name: &'static str,
    pub flags: AudioPortFlags,
    pub channel_count: u32,
    pub port_type: AudioPortType,
    pub in_place_pair: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct AudioPortConfigRequest {
    pub is_input: bool,
    pub port_index: u32,
    pub channel_count: u32,
    pub port_type: AudioPortType,
}

#[derive(Debug, Clone, Copy)]
pub struct NotePortInfo {
    pub id: u32,
    pub supported_dialects: NoteDialects,
    pub preferred_dialect: NoteDialects,
    pub name: &'static str,
}

/// Thin Rust representation of the CLAP note dialect bitset.
/// Used in the note-ports extension to negotiate which note dialects can be sent and received.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoteDialects(u32);

impl NoteDialects {
    pub const CLAP: Self = Self(CLAP_NOTE_DIALECT_CLAP);
    pub const MIDI: Self = Self(CLAP_NOTE_DIALECT_MIDI);
    pub const MIDI_MPE: Self = Self(CLAP_NOTE_DIALECT_MIDI_MPE);
    pub const MIDI2: Self = Self(CLAP_NOTE_DIALECT_MIDI2);

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AudioPortFlags {
    pub is_main: bool,
    pub supports_64bits: bool,
    pub prefers_64bits: bool,
    pub requires_common_sample_size: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum AudioPortType {
    #[default]
    Unspecified,
    Mono,
    Stereo,
}

#[derive(Debug, Clone, Copy)]
pub struct ParamInfo {
    pub id: u32,
    pub name: &'static str,
    pub module: &'static str,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub flags: ParamFlags,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ParamFlags {
    pub is_stepped: bool,
    pub is_periodic: bool,
    pub is_hidden: bool,
    pub is_readonly: bool,
    pub is_bypass: bool,
    pub is_automatable: bool,
    pub is_automatable_per_note_id: bool,
    pub is_automatable_per_key: bool,
    pub is_automatable_per_channel: bool,
    pub is_automatable_per_port: bool,
    pub is_modulatable: bool,
    pub is_modulatable_per_note_id: bool,
    pub is_modulatable_per_key: bool,
    pub is_modulatable_per_channel: bool,
    pub is_modulatable_per_port: bool,
    pub requires_process: bool,
    pub is_enum: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ParamValueEvent {
    pub time: u32,
    pub param_id: u32,
    pub value: f64,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
}

#[derive(Debug, Clone)]
pub struct State {
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct GuiConfig {
    pub api: GuiApi,
    pub is_floating: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiApi {
    Cocoa,
    Win32,
    X11,
}

#[derive(Debug, Clone, Copy)]
pub struct GuiSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct GuiResizeHints {
    pub can_resize_horizontally: bool,
    pub can_resize_vertically: bool,
    pub preserve_aspect_ratio: bool,
    pub aspect_ratio_width: u32,
    pub aspect_ratio_height: u32,
}

/// Toolkit-neutral host window handle passed to GUI backends.
#[derive(Debug, Clone, Copy)]
pub enum HostWindow {
    Cocoa { ns_view: NonZeroUsize },
    Win32 { hwnd: NonZeroIsize },
    X11 { window: NonZeroU64 },
}

impl HostWindow {
    pub(crate) fn cocoa(ns_view: *mut std::ffi::c_void) -> Option<Self> {
        Some(Self::Cocoa {
            ns_view: NonZeroUsize::new(ns_view as usize)?,
        })
    }

    pub(crate) fn win32(hwnd: *mut std::ffi::c_void) -> Option<Self> {
        Some(Self::Win32 {
            hwnd: NonZeroIsize::new(hwnd as isize)?,
        })
    }

    pub(crate) fn x11(window: u64) -> Option<Self> {
        Some(Self::X11 {
            window: NonZeroU64::new(window)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginRenderMode {
    Realtime,
    Offline,
}
