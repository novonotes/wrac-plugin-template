use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::Result;
use crate::targets::PluginFormat;

#[derive(Debug, Clone)]
pub(crate) struct PluginMetadata {
    pub(crate) package_name: String,
    pub(crate) version: String,
    pub(crate) repository: Option<String>,
    pub(crate) company_name: String,
    pub(crate) auv2_manufacturer_code: String,
    pub(crate) aax_manufacturer_id: Option<String>,
    pub(crate) bundle_name: String,
    pub(crate) bundle_identifier: String,
    pub(crate) homepage_url: String,
    pub(crate) manual_url: String,
    pub(crate) support_url: String,
    pub(crate) description: String,
    pub(crate) copyright: String,
    // Product-level plugin format policy. xtask uses this for default build,
    // install, and validate selections so AAX adoption is decided once in
    // metadata instead of repeated on every command line.
    pub(crate) supported_formats: Vec<PluginFormat>,
    pub(crate) plugins: Vec<PluginProductMetadata>,
    pub(crate) validation: ValidationMetadata,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ValidationMetadata {
    #[serde(default)]
    pub(crate) disabled_rules: HashMap<String, DisabledValidationRule>,
    #[serde(default)]
    pub(crate) clap_validator: ClapValidatorMetadata,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DisabledValidationRule {
    pub(crate) reason: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ClapValidatorMetadata {
    pub(crate) skip_test_filter: Option<String>,
    pub(crate) skip_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PluginProductMetadata {
    pub(crate) plugin_id: String,
    pub(crate) plugin_name: String,
    pub(crate) clap_features: Vec<String>,
    pub(crate) vst3_subcategories: String,
    pub(crate) vst3_component_id: String,
    pub(crate) standalone_name: String,
    pub(crate) auv2_type: String,
    pub(crate) auv2_subtype: String,
    pub(crate) aax_categories: Option<Vec<String>>,
    pub(crate) aax_product_id: Option<String>,
    #[serde(default)]
    pub(crate) aax_stem_configs: Vec<AaxStemConfigMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AaxStemConfigMetadata {
    pub(crate) name: String,
    pub(crate) input: String,
    pub(crate) output: String,
    pub(crate) plugin_id: String,
}

impl PluginMetadata {
    pub(crate) fn read(manifest_path: &Path) -> Result<Self> {
        let manifest = fs::read_to_string(manifest_path)?;
        let cargo_manifest: CargoManifest = toml::from_str(&manifest)?;
        let wrac = cargo_manifest.package.metadata.wrac.ok_or_else(|| {
            format!(
                "missing package.metadata.wrac in {}",
                manifest_path.display()
            )
        })?;
        let metadata = Self {
            package_name: cargo_manifest.package.name,
            version: cargo_manifest.package.version,
            repository: cargo_manifest.package.repository,
            company_name: wrac.company_name,
            auv2_manufacturer_code: wrac.auv2_manufacturer_code,
            aax_manufacturer_id: wrac.aax_manufacturer_id,
            bundle_name: wrac.bundle_name,
            bundle_identifier: wrac.bundle_identifier,
            homepage_url: wrac.homepage_url,
            manual_url: wrac.manual_url,
            support_url: wrac.support_url,
            description: wrac.description,
            copyright: wrac.copyright,
            supported_formats: wrac.supported_formats,
            plugins: wrac.plugins,
            validation: wrac.validation.unwrap_or_default(),
        };
        metadata.validate()?;
        Ok(metadata)
    }

    pub(crate) fn clap_bundle_name(&self) -> String {
        format!("{}.clap", self.bundle_name)
    }

    pub(crate) fn vst3_bundle_name(&self) -> String {
        format!("{}.vst3", self.bundle_name)
    }

    pub(crate) fn aax_bundle_name(&self) -> String {
        format!("{}.aaxplugin", self.bundle_name)
    }

    pub(crate) fn au_bundle_name(&self) -> String {
        format!("{}.component", self.bundle_name)
    }

    pub(crate) fn bundle_identity_plugin(&self) -> &PluginProductMetadata {
        // CLAP bundle Info.plist has one CFBundleIdentifier even when the CLAP
        // factory exposes multiple products. Use the first metadata entry only
        // for that bundle-level identifier; product-specific outputs must still
        // iterate over `plugins`.
        self.plugins
            .first()
            .expect("validated metadata must contain at least one plugin")
    }

    fn validate(&self) -> Result<()> {
        validate_required("package.name", &self.package_name)?;
        validate_required("package.version", &self.version)?;
        validate_required("package.metadata.wrac.company_name", &self.company_name)?;
        validate_four_ascii("auv2_manufacturer_code", &self.auv2_manufacturer_code)?;
        validate_required("package.metadata.wrac.bundle_name", &self.bundle_name)?;
        validate_required(
            "package.metadata.wrac.bundle_identifier",
            &self.bundle_identifier,
        )?;
        validate_required("package.metadata.wrac.homepage_url", &self.homepage_url)?;
        validate_required("package.metadata.wrac.manual_url", &self.manual_url)?;
        validate_required("package.metadata.wrac.support_url", &self.support_url)?;
        validate_required("package.metadata.wrac.description", &self.description)?;
        validate_required("package.metadata.wrac.copyright", &self.copyright)?;
        if self.supported_formats.is_empty() {
            return Err("package.metadata.wrac.supported_formats must not be empty".into());
        }
        // Treat duplicates as metadata errors rather than silently deduplicating:
        // supported_formats is commercial product policy, so ambiguity here is
        // more likely to hide a setup mistake than help a caller.
        let mut supported_formats = HashSet::new();
        for format in &self.supported_formats {
            if !supported_formats.insert(*format) {
                return Err(format!(
                    "duplicate package.metadata.wrac.supported_formats entry: {}",
                    format.display()
                )
                .into());
            }
        }
        let supports_aax = supported_formats.contains(&PluginFormat::Aax);
        if supports_aax {
            let Some(aax_manufacturer_id) = self.aax_manufacturer_id.as_ref() else {
                return Err("package.metadata.wrac.aax_manufacturer_id is required when supported_formats contains aax".into());
            };
            validate_four_ascii("aax_manufacturer_id", aax_manufacturer_id)?;
        }
        if self.plugins.is_empty() {
            return Err("package.metadata.wrac.plugins must contain at least one plugin".into());
        }
        let mut plugin_ids = HashSet::new();
        let mut standalone_names = HashSet::new();
        let mut auv2_ids = HashSet::new();
        for plugin in &self.plugins {
            validate_required("package.metadata.wrac.plugins.plugin_id", &plugin.plugin_id)?;
            validate_required(
                "package.metadata.wrac.plugins.plugin_name",
                &plugin.plugin_name,
            )?;
            if plugin.clap_features.is_empty() {
                return Err("package.metadata.wrac.plugins.clap_features must not be empty".into());
            }
            for feature in &plugin.clap_features {
                validate_required("package.metadata.wrac.plugins.clap_features", feature)?;
                validate_clap_feature(feature)?;
            }
            validate_required(
                "package.metadata.wrac.plugins.vst3_subcategories",
                &plugin.vst3_subcategories,
            )?;
            validate_uuid(
                "package.metadata.wrac.plugins.vst3_component_id",
                &plugin.vst3_component_id,
            )?;
            validate_required(
                "package.metadata.wrac.plugins.standalone_name",
                &plugin.standalone_name,
            )?;
            validate_four_ascii("auv2_type", &plugin.auv2_type)?;
            validate_four_ascii("auv2_subtype", &plugin.auv2_subtype)?;
            if supports_aax {
                let Some(aax_categories) = plugin.aax_categories.as_ref() else {
                    return Err("package.metadata.wrac.plugins.aax_categories is required when supported_formats contains aax".into());
                };
                if aax_categories.is_empty() {
                    return Err(
                        "package.metadata.wrac.plugins.aax_categories must not be empty".into(),
                    );
                }
                for category in aax_categories {
                    validate_aax_category(category)?;
                }
                let Some(aax_product_id) = plugin.aax_product_id.as_ref() else {
                    return Err("package.metadata.wrac.plugins.aax_product_id is required when supported_formats contains aax".into());
                };
                validate_four_ascii("plugins.aax_product_id", aax_product_id)?;
                if plugin.aax_stem_configs.is_empty() {
                    return Err(
                        "package.metadata.wrac.plugins.aax_stem_configs must not be empty".into(),
                    );
                }
                let mut aax_plugin_ids = HashSet::new();
                for stem_config in &plugin.aax_stem_configs {
                    validate_required(
                        "package.metadata.wrac.plugins.aax_stem_configs.name",
                        &stem_config.name,
                    )?;
                    validate_aax_stem_format(
                        "package.metadata.wrac.plugins.aax_stem_configs.input",
                        &stem_config.input,
                    )?;
                    validate_aax_stem_format(
                        "package.metadata.wrac.plugins.aax_stem_configs.output",
                        &stem_config.output,
                    )?;
                    validate_four_ascii(
                        "plugins.aax_stem_configs.plugin_id",
                        &stem_config.plugin_id,
                    )?;
                    if !aax_plugin_ids.insert(stem_config.plugin_id.as_str()) {
                        return Err(format!(
                            "duplicate package.metadata.wrac.plugins.aax_stem_configs plugin_id: {}",
                            stem_config.plugin_id
                        )
                        .into());
                    }
                }
            }
            if !plugin_ids.insert(plugin.plugin_id.as_str()) {
                return Err(format!(
                    "duplicate package.metadata.wrac.plugins plugin_id: {}",
                    plugin.plugin_id
                )
                .into());
            }
            if !standalone_names.insert(plugin.standalone_name.as_str()) {
                return Err(format!(
                    "duplicate package.metadata.wrac.plugins standalone_name: {}",
                    plugin.standalone_name
                )
                .into());
            }
            if !auv2_ids.insert((plugin.auv2_type.as_str(), plugin.auv2_subtype.as_str())) {
                return Err(format!(
                    "duplicate package.metadata.wrac.plugins AUv2 type/subtype: {}/{}",
                    plugin.auv2_type, plugin.auv2_subtype
                )
                .into());
            }
        }
        for (rule_id, disabled) in &self.validation.disabled_rules {
            validate_required(
                &format!("package.metadata.wrac.validation.disabled_rules.{rule_id}.reason"),
                disabled.reason.trim(),
            )?;
        }
        if let Some(filter) = self.validation.clap_validator.skip_test_filter.as_deref() {
            validate_required(
                "package.metadata.wrac.validation.clap_validator.skip_test_filter",
                filter.trim(),
            )?;
            validate_required(
                "package.metadata.wrac.validation.clap_validator.skip_reason",
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

fn validate_clap_feature(feature: &str) -> Result<()> {
    match feature {
        "audio-effect" | "analyzer" | "ambisonic" | "chorus" | "compressor" | "de-esser"
        | "delay" | "instrument" | "note-effect" | "note-detector" | "drum" | "drum-machine"
        | "equalizer" | "expander" | "filter" | "flanger" | "frequency-shifter" | "gate"
        | "glitch" | "granular" | "distortion" | "limiter" | "mastering" | "mixing" | "mono"
        | "multi-effects" | "phaser" | "phase-vocoder" | "pitch-correction" | "pitch-shifter"
        | "restoration" | "reverb" | "sampler" | "stereo" | "surround" | "synthesizer"
        | "transient-shaper" | "tremolo" | "utility" => Ok(()),
        _ => Err(format!(
            "unsupported package.metadata.wrac.plugins.clap_features value: {feature}"
        )
        .into()),
    }
}

fn validate_aax_category(category: &str) -> Result<()> {
    match category {
        "eq" | "dynamics" | "pitch-shift" | "reverb" | "delay" | "modulation" | "harmonic"
        | "noise-reduction" | "dither" | "sound-field" | "hardware-generator"
        | "software-generator" | "wrapped-plugin" | "effect" | "midi-effect" => Ok(()),
        _ => Err(format!(
            "unsupported package.metadata.wrac.plugins.aax_categories value: {category}"
        )
        .into()),
    }
}

fn validate_aax_stem_format(label: &str, format: &str) -> Result<()> {
    match format {
        "mono" | "stereo" => Ok(()),
        _ => Err(format!("{label} must be mono or stereo").into()),
    }
}

fn validate_required(key: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        Err(format!("{key} must not be empty").into())
    } else {
        Ok(())
    }
}

fn validate_four_ascii(key: &str, value: &str) -> Result<()> {
    if value.len() == 4 && value.is_ascii() {
        Ok(())
    } else {
        Err(format!("package.metadata.wrac.{key} must be exactly 4 ASCII bytes").into())
    }
}

fn validate_uuid(label: &str, value: &str) -> Result<()> {
    let hex = value.replace('-', "");
    if hex.len() == 32 && hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        Ok(())
    } else {
        Err(format!("{label} must be a UUID").into())
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
    wrac: Option<WracMetadata>,
}

#[derive(Debug, Deserialize)]
struct WracMetadata {
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
    #[serde(default)]
    plugins: Vec<PluginProductMetadata>,
    validation: Option<ValidationMetadata>,
}
