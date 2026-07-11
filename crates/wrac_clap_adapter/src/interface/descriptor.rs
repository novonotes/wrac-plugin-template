use std::ffi::c_char;

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
