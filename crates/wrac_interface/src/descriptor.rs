use std::ffi::{CStr, c_char};

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
    pub vst3: Option<Vst3Descriptor>,
    pub aax: Option<AaxDescriptor>,
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
    /// Returns the CLAP feature identifier represented by this product-facing value.
    pub fn as_cstr(self) -> &'static CStr {
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

#[derive(Debug, Clone, Copy)]
pub struct Vst3Descriptor {
    /// VST3 PClassInfo2 subCategories string, such as `Fx|Tools`.
    pub subcategories: &'static str,
    /// Stable VST3 class ID. Changing this after release breaks host project recall.
    pub component_id: [u8; 16],
}

#[derive(Debug, Clone, Copy)]
pub struct AaxDescriptor {
    pub package_name: &'static str,
    /// AAX package version encoded as 0xMMmmppbb.
    pub package_version: u32,
    pub categories: u32,
    /// Avid-facing FourCC identity. Changing these IDs after release breaks recall.
    pub manufacturer_id: u32,
    pub product_id: u32,
    /// AAX wrapper asks for stem metadata before creating plugin instances.
    /// Keep these callbacks independent from product runtime state.
    pub get_num_stem_configs: unsafe extern "C" fn() -> u32,
    pub get_stem_config: unsafe extern "C" fn(index: u32) -> *const AaxStemConfig,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AaxStemConfig {
    pub name: *const c_char,
    pub format_in: u32,
    pub format_out: u32,
    pub plugin_id: u32,
}

// Safety: generated stem configs point at immutable, NUL-terminated static strings.
// clap-wrapper reads them during factory-time metadata collection only.
unsafe impl Sync for AaxStemConfig {}
unsafe impl Send for AaxStemConfig {}
