//! Parser and validator for WRAC plugin manifests.
//!
//! `wrac-plugin.toml` is the product-owned manifest for host-visible metadata:
//! bundle identifiers, plugin IDs, wrapper descriptors, supported formats, and
//! validation exceptions. This crate reads that file into typed Rust structures
//! for build scripts and xtask code; it does not perform plugin builds itself.

use std::{
    collections::{HashMap, HashSet},
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
    pub supported_formats: Vec<PluginFormat>,
    pub plugins: Vec<PluginProduct>,
    pub validation: ValidationMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginFormat {
    Clap,
    Vst3,
    Au,
    Aax,
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ValidationMetadata {
    #[serde(default)]
    pub disabled_rules: HashMap<String, DisabledValidationRule>,
    #[serde(default)]
    pub clap_validator: ClapValidatorMetadata,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DisabledValidationRule {
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClapValidatorMetadata {
    pub skip_test_filter: Option<String>,
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginProduct {
    pub plugin_id: String,
    pub plugin_name: String,
    pub clap_features: Vec<String>,
    pub vst3_subcategories: String,
    pub vst3_component_id: String,
    pub standalone_name: String,
    pub auv2_type: String,
    pub auv2_subtype: String,
    pub aax_categories: Option<Vec<String>>,
    pub aax_product_id: Option<String>,
    #[serde(default)]
    pub aax_stem_configs: Vec<AaxStemConfig>,
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
    LegacyCargoMetadata(PathBuf),
}

pub fn discover_manifest(
    package_manifest_path: &Path,
    plugin_root: &Path,
) -> Result<ManifestSource> {
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
    let plugin_root_manifest = plugin_root.join("wrac-plugin.toml");
    if plugin_root_manifest.exists() {
        return Ok(ManifestSource::Dedicated(plugin_root_manifest));
    }
    if let Some(relative) = legacy_manifest_reference(package_manifest_path)? {
        return Ok(ManifestSource::Dedicated(package_dir.join(relative)));
    }
    Ok(ManifestSource::LegacyCargoMetadata(
        package_manifest_path.to_path_buf(),
    ))
}

pub fn read_manifest(source: &ManifestSource) -> Result<PluginManifest> {
    match source {
        ManifestSource::Dedicated(path) => read_dedicated_manifest(path),
        ManifestSource::LegacyCargoMetadata(path) => read_legacy_cargo_metadata(path),
    }
}

pub fn read_dedicated_manifest(path: &Path) -> Result<PluginManifest> {
    let manifest = fs::read_to_string(path)?;
    let dedicated: DedicatedManifest = toml::from_str(&manifest)?;
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
        supported_formats: dedicated.bundle.supported_formats,
        plugins: dedicated.plugins,
        validation: dedicated.validation.unwrap_or_default(),
    };
    metadata.validate("wrac-plugin.toml")?;
    Ok(metadata)
}

pub fn read_legacy_cargo_metadata(path: &Path) -> Result<PluginManifest> {
    let manifest = fs::read_to_string(path)?;
    let cargo_manifest: CargoManifest = toml::from_str(&manifest)?;
    let wrac = cargo_manifest
        .package
        .metadata
        .wrac
        .ok_or_else(|| format!("missing package.metadata.wrac in {}", path.display()))?;
    if wrac.manifest.is_some() {
        return Err(
            "package.metadata.wrac.manifest must be resolved before reading legacy metadata".into(),
        );
    }
    let metadata = PluginManifest {
        package: ManifestPackageInfo {
            package_name: Some(cargo_manifest.package.name),
            version: Some(cargo_manifest.package.version),
            repository: cargo_manifest.package.repository,
        },
        company_name: wrac
            .company_name
            .ok_or("missing package.metadata.wrac.company_name")?,
        auv2_manufacturer_code: wrac
            .auv2_manufacturer_code
            .ok_or("missing package.metadata.wrac.auv2_manufacturer_code")?,
        aax_manufacturer_id: wrac.aax_manufacturer_id,
        bundle_name: wrac
            .bundle_name
            .ok_or("missing package.metadata.wrac.bundle_name")?,
        bundle_identifier: wrac
            .bundle_identifier
            .ok_or("missing package.metadata.wrac.bundle_identifier")?,
        homepage_url: wrac
            .homepage_url
            .ok_or("missing package.metadata.wrac.homepage_url")?,
        manual_url: wrac
            .manual_url
            .ok_or("missing package.metadata.wrac.manual_url")?,
        support_url: wrac
            .support_url
            .ok_or("missing package.metadata.wrac.support_url")?,
        description: wrac
            .description
            .ok_or("missing package.metadata.wrac.description")?,
        copyright: wrac
            .copyright
            .ok_or("missing package.metadata.wrac.copyright")?,
        supported_formats: wrac.supported_formats.unwrap_or_default(),
        plugins: wrac.plugins,
        validation: wrac.validation.unwrap_or_default(),
    };
    metadata.validate("package.metadata.wrac")?;
    Ok(metadata)
}

pub fn legacy_manifest_reference(path: &Path) -> Result<Option<String>> {
    let manifest = fs::read_to_string(path)?;
    let cargo_manifest: CargoManifest = toml::from_str(&manifest)?;
    Ok(cargo_manifest
        .package
        .metadata
        .wrac
        .and_then(|wrac| wrac.manifest))
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
        if self.supported_formats.is_empty() {
            return Err(format!("{label}.supported_formats must not be empty").into());
        }
        let mut supported_formats = HashSet::new();
        for format in &self.supported_formats {
            if !supported_formats.insert(*format) {
                return Err(format!(
                    "duplicate {label}.supported_formats entry: {}",
                    format.display()
                )
                .into());
            }
        }
        let supports_aax = supported_formats.contains(&PluginFormat::Aax);
        if supports_aax {
            let Some(aax_manufacturer_id) = self.aax_manufacturer_id.as_ref() else {
                return Err(format!(
                    "{label}.aax_manufacturer_id is required when supported_formats contains aax"
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
                    return Err(format!("{label}.plugins.aax_categories is required when supported_formats contains aax").into());
                };
                if aax_categories.is_empty() {
                    return Err(format!("{label}.plugins.aax_categories must not be empty").into());
                }
                for category in aax_categories {
                    aax_category_bits(category)?;
                }
                let Some(aax_product_id) = plugin.aax_product_id.as_ref() else {
                    return Err(format!("{label}.plugins.aax_product_id is required when supported_formats contains aax").into());
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
        for (rule_id, disabled) in &self.validation.disabled_rules {
            validate_required(
                &format!("{label}.validation.disabled_rules.{rule_id}.reason"),
                disabled.reason.trim(),
            )?;
        }
        if let Some(filter) = self.validation.clap_validator.skip_test_filter.as_deref() {
            validate_required(
                &format!("{label}.validation.clap_validator.skip_test_filter"),
                filter.trim(),
            )?;
            validate_required(
                &format!("{label}.validation.clap_validator.skip_reason"),
                self.validation
                    .clap_validator
                    .skip_reason
                    .as_deref()
                    .unwrap_or_default()
                    .trim(),
            )?;
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
    #[allow(dead_code)]
    schema_version: Option<u32>,
    #[serde(default)]
    package: Option<ManifestPackageInfo>,
    bundle: DedicatedBundle,
    #[serde(default)]
    plugins: Vec<PluginProduct>,
    validation: Option<ValidationMetadata>,
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
    supported_formats: Vec<PluginFormat>,
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
    #[serde(default)]
    metadata: PackageMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct PackageMetadata {
    wrac: Option<LegacyWracMetadata>,
}

#[derive(Debug, Deserialize)]
struct LegacyWracMetadata {
    manifest: Option<String>,
    company_name: Option<String>,
    auv2_manufacturer_code: Option<String>,
    aax_manufacturer_id: Option<String>,
    bundle_name: Option<String>,
    bundle_identifier: Option<String>,
    homepage_url: Option<String>,
    manual_url: Option<String>,
    support_url: Option<String>,
    description: Option<String>,
    copyright: Option<String>,
    supported_formats: Option<Vec<PluginFormat>>,
    #[serde(default)]
    plugins: Vec<PluginProduct>,
    validation: Option<ValidationMetadata>,
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
}
