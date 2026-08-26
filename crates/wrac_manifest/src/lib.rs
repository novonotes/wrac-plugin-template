//! Parser and validator for WRAC plugin manifests.
//!
//! `wrac-plugin.toml` is the product-owned manifest for host-visible metadata:
//! bundle identifiers, plugin IDs, wrapper descriptors, format policies, and
//! artifact metadata. This crate reads that file into typed Rust structures
//! for build scripts and xtask code; it does not perform plugin builds itself.

use std::{
    collections::HashSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Clone, Default)]
pub struct ManifestPackageInfo {
    pub package_name: Option<String>,
    pub version: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub package: ManifestPackageInfo,
    pub company_name: String,
    pub auv2_manufacturer_code: String,
    pub aax_manufacturer_id: Option<String>,
    pub bundle_name: String,
    pub bundle_identifier: String,
    pub homepage_url: String,
    pub manual_url: String,
    pub support_url: String,
    pub description: String,
    pub copyright: String,
    pub formats: Vec<PluginFormatDefinition>,
    pub plugins: Vec<PluginProduct>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginFormat {
    Clap,
    Vst3,
    Au,
    Aax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatDistribution {
    DevelopmentOnly,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct PluginFormatDefinition {
    #[serde(rename = "type")]
    pub format: PluginFormat,
    pub distribution: FormatDistribution,
}

impl PluginFormat {
    pub fn display(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Au => "AU",
            Self::Aax => "AAX",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginProduct {
    pub plugin_id: String,
    pub plugin_name: String,
    pub clap_features: Vec<String>,
    pub vst3_subcategories: String,
    pub vst3_component_id: String,
    pub standalone_name: String,
    /// Controls physical audio capture in the development standalone app independently of the
    /// plugin buses that remain visible to DAW hosts.
    #[serde(default = "default_true")]
    pub standalone_audio_input: bool,
    pub auv2_type: String,
    pub auv2_subtype: String,
    pub aax_categories: Option<Vec<String>>,
    pub aax_product_id: Option<String>,
    #[serde(default)]
    pub aax_stem_configs: Vec<AaxStemConfig>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct AaxStemConfig {
    pub name: String,
    pub input: String,
    pub output: String,
    pub plugin_id: String,
}

#[derive(Debug, Clone)]
pub enum ManifestSource {
    Dedicated(PathBuf),
}

pub fn discover_manifest(package_manifest_path: &Path) -> Result<ManifestSource> {
    let package_dir = package_manifest_path.parent().ok_or_else(|| {
        format!(
            "failed to derive package dir from {}",
            package_manifest_path.display()
        )
    })?;
    let package_manifest = package_dir.join("wrac-plugin.toml");
    if package_manifest.exists() {
        return Ok(ManifestSource::Dedicated(package_manifest));
    }
    Err(format!(
        "WRAC plugin manifest must exist at {}, but no manifest was found there",
        package_manifest.display()
    )
    .into())
}

pub fn read_manifest(source: &ManifestSource) -> Result<PluginManifest> {
    match source {
        ManifestSource::Dedicated(path) => read_dedicated_manifest(path),
    }
}

pub fn read_dedicated_manifest(path: &Path) -> Result<PluginManifest> {
    let manifest = fs::read_to_string(path)?;
    let dedicated: DedicatedManifest = toml::from_str(&manifest)?;
    if dedicated.schema_version != 1 {
        return Err(format!(
            "wrac-plugin.toml.schema_version must be 1, got {}",
            dedicated.schema_version
        )
        .into());
    }
    let metadata = PluginManifest {
        package: dedicated.package.unwrap_or_default(),
        company_name: dedicated.bundle.company_name,
        auv2_manufacturer_code: dedicated.bundle.auv2_manufacturer_code,
        aax_manufacturer_id: dedicated.bundle.aax_manufacturer_id,
        bundle_name: dedicated.bundle.bundle_name,
        bundle_identifier: dedicated.bundle.bundle_identifier,
        homepage_url: dedicated.bundle.homepage_url,
        manual_url: dedicated.bundle.manual_url,
        support_url: dedicated.bundle.support_url,
        description: dedicated.bundle.description,
        copyright: dedicated.bundle.copyright,
        formats: dedicated.bundle.formats,
        plugins: dedicated.plugins,
    };
    metadata.validate("wrac-plugin.toml")?;
    Ok(metadata)
}

pub fn read_cargo_package_info(path: &Path) -> Result<ManifestPackageInfo> {
    let manifest = fs::read_to_string(path)?;
    let cargo_manifest: CargoManifest = toml::from_str(&manifest)?;
    Ok(ManifestPackageInfo {
        package_name: Some(cargo_manifest.package.name),
        version: Some(cargo_manifest.package.version),
        repository: cargo_manifest.package.repository,
    })
}

impl PluginManifest {
    pub fn supports_format(&self, format: PluginFormat) -> bool {
        self.formats
            .iter()
            .any(|definition| definition.format == format)
    }

    pub fn validate(&self, label: &str) -> Result<()> {
        validate_required(&format!("{label}.company_name"), &self.company_name)?;
        validate_four_ascii(
            &format!("{label}.auv2_manufacturer_code"),
            &self.auv2_manufacturer_code,
        )?;
        validate_required(&format!("{label}.bundle_name"), &self.bundle_name)?;
        validate_required(
            &format!("{label}.bundle_identifier"),
            &self.bundle_identifier,
        )?;
        validate_required(&format!("{label}.homepage_url"), &self.homepage_url)?;
        validate_required(&format!("{label}.manual_url"), &self.manual_url)?;
        validate_required(&format!("{label}.support_url"), &self.support_url)?;
        validate_required(&format!("{label}.description"), &self.description)?;
        validate_required(&format!("{label}.copyright"), &self.copyright)?;
        if self.formats.is_empty() {
            return Err(format!("{label}.formats must not be empty").into());
        }
        let mut formats = HashSet::new();
        for definition in &self.formats {
            if !formats.insert(definition.format) {
                return Err(format!(
                    "duplicate {label}.formats entry: {}",
                    definition.format.display()
                )
                .into());
            }
        }
        let supports_aax = formats.contains(&PluginFormat::Aax);
        if supports_aax {
            let Some(aax_manufacturer_id) = self.aax_manufacturer_id.as_ref() else {
                return Err(format!(
                    "{label}.aax_manufacturer_id is required when formats contains aax"
                )
                .into());
            };
            validate_four_ascii(&format!("{label}.aax_manufacturer_id"), aax_manufacturer_id)?;
        }
        if self.plugins.is_empty() {
            return Err(format!("{label}.plugins must contain at least one plugin").into());
        }
        let mut plugin_ids = HashSet::new();
        let mut standalone_names = HashSet::new();
        let mut auv2_ids = HashSet::new();
        for plugin in &self.plugins {
            validate_required(&format!("{label}.plugins.plugin_id"), &plugin.plugin_id)?;
            validate_required(&format!("{label}.plugins.plugin_name"), &plugin.plugin_name)?;
            if plugin.clap_features.is_empty() {
                return Err(format!("{label}.plugins.clap_features must not be empty").into());
            }
            for feature in &plugin.clap_features {
                validate_clap_feature(feature).map_err(|_| {
                    format!("unsupported {label}.plugins.clap_features value: {feature}")
                })?;
            }
            validate_required(
                &format!("{label}.plugins.vst3_subcategories"),
                &plugin.vst3_subcategories,
            )?;
            vst3_component_id_bytes(&plugin.vst3_component_id)?;
            validate_required(
                &format!("{label}.plugins.standalone_name"),
                &plugin.standalone_name,
            )?;
            validate_four_ascii(&format!("{label}.plugins.auv2_type"), &plugin.auv2_type)?;
            validate_four_ascii(
                &format!("{label}.plugins.auv2_subtype"),
                &plugin.auv2_subtype,
            )?;
            if supports_aax {
                let Some(aax_categories) = plugin.aax_categories.as_ref() else {
                    return Err(format!(
                        "{label}.plugins.aax_categories is required when formats contains aax"
                    )
                    .into());
                };
                if aax_categories.is_empty() {
                    return Err(format!("{label}.plugins.aax_categories must not be empty").into());
                }
                for category in aax_categories {
                    aax_category_bits(category)?;
                }
                let Some(aax_product_id) = plugin.aax_product_id.as_ref() else {
                    return Err(format!(
                        "{label}.plugins.aax_product_id is required when formats contains aax"
                    )
                    .into());
                };
                validate_four_ascii(&format!("{label}.plugins.aax_product_id"), aax_product_id)?;
                if plugin.aax_stem_configs.is_empty() {
                    return Err(
                        format!("{label}.plugins.aax_stem_configs must not be empty").into(),
                    );
                }
            }
            let mut aax_plugin_ids = HashSet::new();
            for stem_config in &plugin.aax_stem_configs {
                validate_required(
                    &format!("{label}.plugins.aax_stem_configs.name"),
                    &stem_config.name,
                )?;
                aax_stem_format_value(&stem_config.input)?;
                aax_stem_format_value(&stem_config.output)?;
                validate_four_ascii(
                    &format!("{label}.plugins.aax_stem_configs.plugin_id"),
                    &stem_config.plugin_id,
                )?;
                if !aax_plugin_ids.insert(stem_config.plugin_id.as_str()) {
                    return Err(format!(
                        "duplicate {label}.plugins.aax_stem_configs plugin_id: {}",
                        stem_config.plugin_id
                    )
                    .into());
                }
            }
            if !plugin_ids.insert(plugin.plugin_id.as_str()) {
                return Err(
                    format!("duplicate {label}.plugins plugin_id: {}", plugin.plugin_id).into(),
                );
            }
            if !standalone_names.insert(plugin.standalone_name.as_str()) {
                return Err(format!(
                    "duplicate {label}.plugins standalone_name: {}",
                    plugin.standalone_name
                )
                .into());
            }
            if !auv2_ids.insert((plugin.auv2_type.as_str(), plugin.auv2_subtype.as_str())) {
                return Err(format!(
                    "duplicate {label}.plugins AUv2 type/subtype: {}/{}",
                    plugin.auv2_type, plugin.auv2_subtype
                )
                .into());
            }
        }
        Ok(())
    }
}

pub fn clap_feature_variant(feature: &str) -> Option<&'static str> {
    Some(match feature {
        "audio-effect" => "AudioEffect",
        "analyzer" => "Analyzer",
        "ambisonic" => "Ambisonic",
        "chorus" => "Chorus",
        "compressor" => "Compressor",
        "de-esser" => "DeEsser",
        "delay" => "Delay",
        "instrument" => "Instrument",
        "note-effect" => "NoteEffect",
        "note-detector" => "NoteDetector",
        "drum" => "Drum",
        "drum-machine" => "DrumMachine",
        "equalizer" => "Equalizer",
        "expander" => "Expander",
        "filter" => "Filter",
        "flanger" => "Flanger",
        "frequency-shifter" => "FrequencyShifter",
        "gate" => "Gate",
        "glitch" => "Glitch",
        "granular" => "Granular",
        "distortion" => "Distortion",
        "limiter" => "Limiter",
        "mastering" => "Mastering",
        "mixing" => "Mixing",
        "mono" => "Mono",
        "multi-effects" => "MultiEffects",
        "phaser" => "Phaser",
        "phase-vocoder" => "PhaseVocoder",
        "pitch-correction" => "PitchCorrection",
        "pitch-shifter" => "PitchShifter",
        "restoration" => "Restoration",
        "reverb" => "Reverb",
        "sampler" => "Sampler",
        "stereo" => "Stereo",
        "surround" => "Surround",
        "synthesizer" => "Synthesizer",
        "transient-shaper" => "TransientShaper",
        "tremolo" => "Tremolo",
        "utility" => "Utility",
        _ => return None,
    })
}

pub fn validate_clap_feature(feature: &str) -> Result<()> {
    clap_feature_variant(feature)
        .map(|_| ())
        .ok_or_else(|| format!("unsupported CLAP feature value: {feature}").into())
}

pub fn four_ascii_bytes(value: &str) -> Result<[u8; 4]> {
    if value.len() != 4 || !value.is_ascii() {
        return Err(format!("{value} must be exactly 4 ASCII bytes").into());
    }
    let bytes = value.as_bytes();
    Ok([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub fn fourcc(value: &str) -> Result<u32> {
    let bytes = four_ascii_bytes(value)?;
    Ok(((bytes[0] as u32) << 24)
        | ((bytes[1] as u32) << 16)
        | ((bytes[2] as u32) << 8)
        | (bytes[3] as u32))
}

pub fn vst3_component_id_bytes(value: &str) -> Result<[u8; 16]> {
    let hex = value.replace('-', "");
    if hex.len() != 32 || !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(format!("vst3_component_id must be a UUID: {value}").into());
    }
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&hex[start..start + 2], 16)
            .map_err(|error| format!("vst3_component_id must be a UUID: {error}"))?;
    }
    bytes.swap(0, 3);
    bytes.swap(1, 2);
    bytes.swap(4, 5);
    bytes.swap(6, 7);
    Ok(bytes)
}

pub fn aax_category_bits(category: &str) -> Result<u32> {
    Ok(match category {
        "eq" => 0x0000_0001,
        "dynamics" => 0x0000_0002,
        "pitch-shift" => 0x0000_0004,
        "reverb" => 0x0000_0008,
        "delay" => 0x0000_0010,
        "modulation" => 0x0000_0020,
        "harmonic" => 0x0000_0040,
        "noise-reduction" => 0x0000_0080,
        "dither" => 0x0000_0100,
        "sound-field" => 0x0000_0200,
        "hardware-generator" => 0x0000_0400,
        "software-generator" => 0x0000_0800,
        "wrapped-plugin" => 0x0000_1000,
        "effect" => 0x0000_2000,
        "midi-effect" => 0x0001_0000,
        _ => return Err(format!("unsupported AAX category value: {category}").into()),
    })
}

pub fn aax_stem_format_value(format: &str) -> Result<u32> {
    Ok(match format {
        "mono" => 1,
        "stereo" => 0x0001_0002,
        _ => return Err(format!("AAX stem format must be mono or stereo: {format}").into()),
    })
}

fn validate_required(key: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        Err(format!("{key} must not be empty").into())
    } else {
        Ok(())
    }
}

fn validate_four_ascii(key: &str, value: &str) -> Result<()> {
    four_ascii_bytes(value)
        .map(|_| ())
        .map_err(|_| format!("{key} must be exactly 4 ASCII bytes").into())
}

#[derive(Debug, Deserialize)]
struct DedicatedManifest {
    schema_version: u32,
    #[serde(default)]
    package: Option<ManifestPackageInfo>,
    bundle: DedicatedBundle,
    #[serde(default)]
    plugins: Vec<PluginProduct>,
}

#[derive(Debug, Deserialize)]
struct DedicatedBundle {
    company_name: String,
    auv2_manufacturer_code: String,
    aax_manufacturer_id: Option<String>,
    bundle_name: String,
    bundle_identifier: String,
    homepage_url: String,
    manual_url: String,
    support_url: String,
    description: String,
    copyright: String,
    formats: Vec<PluginFormatDefinition>,
}

impl<'de> Deserialize<'de> for ManifestPackageInfo {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            name: Option<String>,
            version: Option<String>,
            repository: Option<String>,
            #[allow(dead_code)]
            version_source: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            package_name: raw.name,
            version: raw.version,
            repository: raw.repository,
        })
    }
}

#[derive(Debug, Deserialize)]
struct CargoManifest {
    package: CargoPackage,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
    repository: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vst3_component_id_uses_clap_wrapper_inverse_tuid_order() {
        assert_eq!(
            vst3_component_id_bytes("c905bf36-234a-54d0-94f6-70d73f16a08e").unwrap(),
            [
                0x36, 0xbf, 0x05, 0xc9, 0x4a, 0x23, 0xd0, 0x54, 0x94, 0xf6, 0x70, 0xd7, 0x3f, 0x16,
                0xa0, 0x8e,
            ]
        );
    }

    #[test]
    fn fourcc_is_big_endian_ascii() {
        assert_eq!(fourcc("SnCl").unwrap(), 0x536E_436C);
    }

    #[test]
    fn package_ignores_repository_specific_extensions() {
        let info: ManifestPackageInfo = toml::from_str("version_source = \"cargo\"").unwrap();

        assert_eq!(info.package_name, None);

        let manifest: DedicatedManifest = toml::from_str(
            r#"
schema_version = 1

[package]
version_source = "cargo"

[bundle]
company_name = "Example"
auv2_manufacturer_code = "ExCo"
bundle_name = "Test Plugin"
bundle_identifier = "com.example.test-plugin"
homepage_url = "https://example.com"
manual_url = "https://example.com/manual"
support_url = "https://example.com/support"
description = "Test plugin"
copyright = "Copyright Example"
formats = [
    { type = "clap", distribution = "development-only" },
    { type = "vst3", distribution = "public" },
]

[acme.ci]
release_track = "prototype"

[[plugins]]
plugin_id = "com.example.test-plugin"
plugin_name = "Test Plugin"
clap_features = ["audio-effect", "stereo"]
vst3_subcategories = "Fx"
vst3_component_id = "5c65bb45-6f84-527b-915a-a51a30ea5854"
standalone_name = "Test Plugin Standalone"
auv2_type = "aufx"
auv2_subtype = "TstP"
"#,
        )
        .unwrap();
        assert!(manifest.plugins[0].standalone_audio_input);
        assert_eq!(
            manifest.bundle.formats,
            vec![
                PluginFormatDefinition {
                    format: PluginFormat::Clap,
                    distribution: FormatDistribution::DevelopmentOnly,
                },
                PluginFormatDefinition {
                    format: PluginFormat::Vst3,
                    distribution: FormatDistribution::Public,
                },
            ]
        );
    }
}
