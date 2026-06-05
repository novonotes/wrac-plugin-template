use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::Result;

#[derive(Debug, Clone)]
pub(crate) struct PluginMetadata {
    pub(crate) package_name: String,
    pub(crate) version: String,
    pub(crate) repository: Option<String>,
    pub(crate) company_name: String,
    pub(crate) auv2_manufacturer_code: String,
    pub(crate) bundle_name: String,
    pub(crate) bundle_identifier: String,
    pub(crate) homepage_url: String,
    pub(crate) manual_url: String,
    pub(crate) support_url: String,
    pub(crate) description: String,
    pub(crate) copyright: String,
    pub(crate) plugins: Vec<PluginProductMetadata>,
    pub(crate) validation: ValidationMetadata,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ValidationMetadata {
    #[serde(default)]
    pub(crate) disabled_rules: HashMap<String, DisabledValidationRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DisabledValidationRule {
    pub(crate) reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PluginProductMetadata {
    pub(crate) plugin_id: String,
    pub(crate) plugin_name: String,
    pub(crate) clap_features: Vec<String>,
    pub(crate) standalone_name: String,
    pub(crate) auv2_type: String,
    pub(crate) auv2_subtype: String,
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
            bundle_name: wrac.bundle_name,
            bundle_identifier: wrac.bundle_identifier,
            homepage_url: wrac.homepage_url,
            manual_url: wrac.manual_url,
            support_url: wrac.support_url,
            description: wrac.description,
            copyright: wrac.copyright,
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

    pub(crate) fn au_bundle_name(&self, plugin: &PluginProductMetadata) -> String {
        format!("{}.component", plugin.plugin_name)
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
                "package.metadata.wrac.plugins.standalone_name",
                &plugin.standalone_name,
            )?;
            validate_four_ascii("auv2_type", &plugin.auv2_type)?;
            validate_four_ascii("auv2_subtype", &plugin.auv2_subtype)?;
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
    bundle_name: String,
    bundle_identifier: String,
    homepage_url: String,
    manual_url: String,
    support_url: String,
    description: String,
    copyright: String,
    #[serde(default)]
    plugins: Vec<PluginProductMetadata>,
    validation: Option<ValidationMetadata>,
}
