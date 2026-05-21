use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::OnceLock;

use clap_sys::factory::plugin_factory::clap_plugin_factory;
use clap_sys::plugin::clap_plugin_descriptor;
use clap_sys::plugin_features::{
    CLAP_PLUGIN_FEATURE_AMBISONIC, CLAP_PLUGIN_FEATURE_ANALYZER, CLAP_PLUGIN_FEATURE_AUDIO_EFFECT,
    CLAP_PLUGIN_FEATURE_CHORUS, CLAP_PLUGIN_FEATURE_COMPRESSOR, CLAP_PLUGIN_FEATURE_DEESSER,
    CLAP_PLUGIN_FEATURE_DELAY, CLAP_PLUGIN_FEATURE_DISTORTION, CLAP_PLUGIN_FEATURE_DRUM,
    CLAP_PLUGIN_FEATURE_DRUM_MACHINE, CLAP_PLUGIN_FEATURE_EQUALIZER, CLAP_PLUGIN_FEATURE_EXPANDER,
    CLAP_PLUGIN_FEATURE_FILTER, CLAP_PLUGIN_FEATURE_FLANGER, CLAP_PLUGIN_FEATURE_FREQUENCY_SHIFTER,
    CLAP_PLUGIN_FEATURE_GATE, CLAP_PLUGIN_FEATURE_GLITCH, CLAP_PLUGIN_FEATURE_GRANULAR,
    CLAP_PLUGIN_FEATURE_INSTRUMENT, CLAP_PLUGIN_FEATURE_LIMITER, CLAP_PLUGIN_FEATURE_MASTERING,
    CLAP_PLUGIN_FEATURE_MIXING, CLAP_PLUGIN_FEATURE_MONO, CLAP_PLUGIN_FEATURE_MULTI_EFFECTS,
    CLAP_PLUGIN_FEATURE_NOTE_DETECTOR, CLAP_PLUGIN_FEATURE_NOTE_EFFECT,
    CLAP_PLUGIN_FEATURE_PHASE_VOCODER, CLAP_PLUGIN_FEATURE_PHASER,
    CLAP_PLUGIN_FEATURE_PITCH_CORRECTION, CLAP_PLUGIN_FEATURE_PITCH_SHIFTER,
    CLAP_PLUGIN_FEATURE_RESTORATION, CLAP_PLUGIN_FEATURE_REVERB, CLAP_PLUGIN_FEATURE_SAMPLER,
    CLAP_PLUGIN_FEATURE_STEREO, CLAP_PLUGIN_FEATURE_SURROUND, CLAP_PLUGIN_FEATURE_SYNTHESIZER,
    CLAP_PLUGIN_FEATURE_TRANSIENT_SHAPER, CLAP_PLUGIN_FEATURE_TREMOLO, CLAP_PLUGIN_FEATURE_UTILITY,
};
use clap_sys::version::CLAP_VERSION;

use crate::{PluginCore, PluginCoreContext};

pub(crate) type CreatePluginCore = fn(PluginCoreContext) -> Box<dyn PluginCore>;

#[derive(Debug, Clone, Copy)]
pub struct PluginDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor: &'static str,
    pub url: &'static str,
    pub manual_url: &'static str,
    pub support_url: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub features: &'static [PluginFeature],
    pub auv2: Option<Auv2Descriptor>,
}

#[derive(Debug, Clone, Copy)]
pub enum PluginFeature {
    AudioEffect,
    Analyzer,
    Ambisonic,
    Chorus,
    Compressor,
    DeEsser,
    Delay,
    Instrument,
    NoteEffect,
    NoteDetector,
    Drum,
    DrumMachine,
    Equalizer,
    Expander,
    Filter,
    Flanger,
    FrequencyShifter,
    Gate,
    Glitch,
    Granular,
    Distortion,
    Limiter,
    Mastering,
    Mixing,
    Mono,
    MultiEffects,
    Phaser,
    PhaseVocoder,
    PitchCorrection,
    PitchShifter,
    Restoration,
    Reverb,
    Sampler,
    Stereo,
    Surround,
    Synthesizer,
    TransientShaper,
    Tremolo,
    Utility,
}

impl PluginFeature {
    fn as_cstr(self) -> &'static CStr {
        match self {
            Self::AudioEffect => CLAP_PLUGIN_FEATURE_AUDIO_EFFECT,
            Self::Analyzer => CLAP_PLUGIN_FEATURE_ANALYZER,
            Self::Ambisonic => CLAP_PLUGIN_FEATURE_AMBISONIC,
            Self::Chorus => CLAP_PLUGIN_FEATURE_CHORUS,
            Self::Compressor => CLAP_PLUGIN_FEATURE_COMPRESSOR,
            Self::DeEsser => CLAP_PLUGIN_FEATURE_DEESSER,
            Self::Delay => CLAP_PLUGIN_FEATURE_DELAY,
            Self::Instrument => CLAP_PLUGIN_FEATURE_INSTRUMENT,
            Self::NoteEffect => CLAP_PLUGIN_FEATURE_NOTE_EFFECT,
            Self::NoteDetector => CLAP_PLUGIN_FEATURE_NOTE_DETECTOR,
            Self::Drum => CLAP_PLUGIN_FEATURE_DRUM,
            Self::DrumMachine => CLAP_PLUGIN_FEATURE_DRUM_MACHINE,
            Self::Equalizer => CLAP_PLUGIN_FEATURE_EQUALIZER,
            Self::Expander => CLAP_PLUGIN_FEATURE_EXPANDER,
            Self::Filter => CLAP_PLUGIN_FEATURE_FILTER,
            Self::Flanger => CLAP_PLUGIN_FEATURE_FLANGER,
            Self::FrequencyShifter => CLAP_PLUGIN_FEATURE_FREQUENCY_SHIFTER,
            Self::Gate => CLAP_PLUGIN_FEATURE_GATE,
            Self::Glitch => CLAP_PLUGIN_FEATURE_GLITCH,
            Self::Granular => CLAP_PLUGIN_FEATURE_GRANULAR,
            Self::Distortion => CLAP_PLUGIN_FEATURE_DISTORTION,
            Self::Limiter => CLAP_PLUGIN_FEATURE_LIMITER,
            Self::Mastering => CLAP_PLUGIN_FEATURE_MASTERING,
            Self::Mixing => CLAP_PLUGIN_FEATURE_MIXING,
            Self::Mono => CLAP_PLUGIN_FEATURE_MONO,
            Self::MultiEffects => CLAP_PLUGIN_FEATURE_MULTI_EFFECTS,
            Self::Phaser => CLAP_PLUGIN_FEATURE_PHASER,
            Self::PhaseVocoder => CLAP_PLUGIN_FEATURE_PHASE_VOCODER,
            Self::PitchCorrection => CLAP_PLUGIN_FEATURE_PITCH_CORRECTION,
            Self::PitchShifter => CLAP_PLUGIN_FEATURE_PITCH_SHIFTER,
            Self::Restoration => CLAP_PLUGIN_FEATURE_RESTORATION,
            Self::Reverb => CLAP_PLUGIN_FEATURE_REVERB,
            Self::Sampler => CLAP_PLUGIN_FEATURE_SAMPLER,
            Self::Stereo => CLAP_PLUGIN_FEATURE_STEREO,
            Self::Surround => CLAP_PLUGIN_FEATURE_SURROUND,
            Self::Synthesizer => CLAP_PLUGIN_FEATURE_SYNTHESIZER,
            Self::TransientShaper => CLAP_PLUGIN_FEATURE_TRANSIENT_SHAPER,
            Self::Tremolo => CLAP_PLUGIN_FEATURE_TREMOLO,
            Self::Utility => CLAP_PLUGIN_FEATURE_UTILITY,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Auv2Descriptor {
    pub manufacturer_code: [u8; 4],
    pub manufacturer_name: &'static str,
    pub plugin_type: [u8; 4],
    pub plugin_subtype: [u8; 4],
}

/// Descriptor and factory function fixed for the plugin binary.
///
/// The factory callback may be called at any time, so it is stored statically. Being
/// immutable allows it to be passed to the C ABI without relying on a global mutable
/// registry or pointer leaks.
pub struct PluginRegistration {
    pub(crate) descriptor: PluginDescriptor,
    pub(crate) create: CreatePluginCore,
    storage: OnceLock<PluginRegistrationStorage>,
}

// Safety: `descriptor` and `create` are immutable data in the static registration;
// mutable state is synchronized by `OnceLock`. Factory queries from multiple threads
// via the C ABI return only shared references.
unsafe impl Sync for PluginRegistration {}
unsafe impl Send for PluginRegistration {}

impl PluginRegistration {
    pub const fn new(descriptor: PluginDescriptor, create: CreatePluginCore) -> Self {
        Self {
            descriptor,
            create,
            storage: OnceLock::new(),
        }
    }

    pub(crate) fn storage(&'static self) -> &'static PluginRegistrationStorage {
        self.storage
            .get_or_init(|| PluginRegistrationStorage::new(self))
    }
}

pub(crate) struct PluginRegistrationStorage {
    pub clap_factory: ClapFactoryState,
    pub auv2_factory: Auv2FactoryState,
    pub descriptor: ClapDescriptorStorage,
}

// Safety: after creation the storage only reads out factory/descriptor pointers.
// Internal pointers point to buffers owned by this same storage, and `OnceLock`
// prevents initialization races.
unsafe impl Sync for PluginRegistrationStorage {}
unsafe impl Send for PluginRegistrationStorage {}

impl PluginRegistrationStorage {
    fn new(registration: &'static PluginRegistration) -> Self {
        let descriptor = ClapDescriptorStorage::new(registration.descriptor);
        Self {
            clap_factory: ClapFactoryState {
                factory: clap_plugin_factory {
                    get_plugin_count: Some(crate::abi::factory_get_plugin_count),
                    get_plugin_descriptor: Some(crate::abi::factory_get_plugin_descriptor),
                    create_plugin: Some(crate::abi::factory_create_plugin),
                },
                registration,
            },
            auv2_factory: Auv2FactoryState {
                factory: ClapPluginFactoryAsAuv2 {
                    manufacturer_code: descriptor.auv2_manufacturer_code_ptr(),
                    manufacturer_name: descriptor.auv2_manufacturer_name_ptr(),
                    get_auv2_info: Some(crate::abi::auv2_get_info),
                },
                registration,
            },
            descriptor,
        }
    }
}

// CLAP factory callbacks receive only a factory pointer, so the C ABI struct is placed
// as the first field and cast back to the state inside the callback.
#[repr(C)]
pub(crate) struct ClapFactoryState {
    pub factory: clap_plugin_factory,
    pub registration: &'static PluginRegistration,
}

unsafe impl Sync for ClapFactoryState {}
unsafe impl Send for ClapFactoryState {}

#[repr(C)]
pub(crate) struct Auv2FactoryState {
    pub factory: ClapPluginFactoryAsAuv2,
    pub registration: &'static PluginRegistration,
}

unsafe impl Sync for Auv2FactoryState {}
unsafe impl Send for Auv2FactoryState {}

#[repr(C)]
pub(crate) struct ClapPluginInfoAsAuv2 {
    pub au_type: [c_char; 5],
    pub au_subt: [c_char; 5],
}

#[repr(C)]
pub(crate) struct ClapPluginFactoryAsAuv2 {
    pub manufacturer_code: *const c_char,
    pub manufacturer_name: *const c_char,
    pub get_auv2_info: Option<
        unsafe extern "C" fn(
            factory: *const ClapPluginFactoryAsAuv2,
            index: u32,
            info: *mut ClapPluginInfoAsAuv2,
        ) -> bool,
    >,
}

unsafe impl Sync for ClapPluginFactoryAsAuv2 {}
unsafe impl Send for ClapPluginFactoryAsAuv2 {}

// `clap_plugin_descriptor` holds only C string pointers, so the owners of the CString
// and feature pointer arrays are placed in the same storage to keep their lifetimes
// aligned with the descriptor pointer.
pub(crate) struct ClapDescriptorStorage {
    _id: CString,
    _name: CString,
    _vendor: CString,
    _url: CString,
    _manual_url: CString,
    _support_url: CString,
    _version: CString,
    _description: CString,
    _feature_ptrs: Vec<*const c_char>,
    auv2_manufacturer_code: Option<CString>,
    auv2_manufacturer_name: Option<CString>,
    clap_descriptor: clap_plugin_descriptor,
}

// Safety: the descriptor storage is not mutated after initialization. Raw pointers point
// into CString/Vec fields owned by this struct, not external memory, so sharing them
// causes no data race.
unsafe impl Sync for ClapDescriptorStorage {}
unsafe impl Send for ClapDescriptorStorage {}

impl ClapDescriptorStorage {
    fn new(descriptor: PluginDescriptor) -> Self {
        let id = cstring(descriptor.id);
        let name = cstring(descriptor.name);
        let vendor = cstring(descriptor.vendor);
        let url = cstring(descriptor.url);
        let manual_url = cstring(descriptor.manual_url);
        let support_url = cstring(descriptor.support_url);
        let version = cstring(descriptor.version);
        let description = cstring(descriptor.description);

        let mut feature_ptrs = descriptor
            .features
            .iter()
            .map(|feature| feature.as_cstr().as_ptr())
            .collect::<Vec<_>>();
        feature_ptrs.push(ptr::null());

        let auv2_manufacturer_code = descriptor
            .auv2
            .map(|auv2| CString::new(auv2.manufacturer_code).expect("four char code"));
        let auv2_manufacturer_name = descriptor.auv2.map(|auv2| cstring(auv2.manufacturer_name));

        let clap_descriptor = clap_plugin_descriptor {
            clap_version: CLAP_VERSION,
            id: id.as_ptr(),
            name: name.as_ptr(),
            vendor: vendor.as_ptr(),
            url: url.as_ptr(),
            manual_url: manual_url.as_ptr(),
            support_url: support_url.as_ptr(),
            version: version.as_ptr(),
            description: description.as_ptr(),
            features: feature_ptrs.as_ptr(),
        };

        Self {
            _id: id,
            _name: name,
            _vendor: vendor,
            _url: url,
            _manual_url: manual_url,
            _support_url: support_url,
            _version: version,
            _description: description,
            _feature_ptrs: feature_ptrs,
            auv2_manufacturer_code,
            auv2_manufacturer_name,
            clap_descriptor,
        }
    }

    pub(crate) fn clap_descriptor(&self) -> *const clap_plugin_descriptor {
        &self.clap_descriptor
    }

    fn auv2_manufacturer_code_ptr(&self) -> *const c_char {
        self.auv2_manufacturer_code
            .as_ref()
            .map_or(ptr::null(), |value| value.as_ptr())
    }

    fn auv2_manufacturer_name_ptr(&self) -> *const c_char {
        self.auv2_manufacturer_name
            .as_ref()
            .map_or(ptr::null(), |value| value.as_ptr())
    }
}

fn cstring(value: &'static str) -> CString {
    CString::new(value).expect("plugin descriptor strings must not contain NUL bytes")
}

pub(crate) fn clap_factory_state(
    factory: *const clap_plugin_factory,
) -> Option<&'static ClapFactoryState> {
    if factory.is_null() {
        return None;
    }
    Some(unsafe { &*(factory as *const ClapFactoryState) })
}

pub(crate) fn auv2_factory_state(
    factory: *const ClapPluginFactoryAsAuv2,
) -> Option<&'static Auv2FactoryState> {
    if factory.is_null() {
        return None;
    }
    Some(unsafe { &*(factory as *const Auv2FactoryState) })
}

pub(crate) fn factory_ptr(storage: &'static PluginRegistrationStorage) -> *const c_void {
    &storage.clap_factory.factory as *const clap_plugin_factory as *const c_void
}

pub(crate) fn auv2_factory_ptr(storage: &'static PluginRegistrationStorage) -> *const c_void {
    &storage.auv2_factory.factory as *const ClapPluginFactoryAsAuv2 as *const c_void
}
