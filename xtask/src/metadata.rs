use std::fs;
use std::path::Path;

use crate::Result;

#[derive(Debug, Clone)]
pub(crate) struct PluginMetadata {
    pub(crate) plugin_id: String,
    pub(crate) plugin_name: String,
    pub(crate) company_name: String,
    pub(crate) auv2_type: String,
    pub(crate) auv2_subtype: String,
    pub(crate) auv2_manufacturer_code: String,
    pub(crate) standalone_name: String,
}

impl PluginMetadata {
    pub(crate) fn read(manifest_path: &Path) -> Result<Self> {
        let manifest = fs::read_to_string(manifest_path)?;
        let metadata = Self {
            plugin_id: required_toml_string(&manifest, "plugin_id")?,
            plugin_name: required_toml_string(&manifest, "plugin_name")?,
            company_name: required_toml_string(&manifest, "company_name")?,
            auv2_type: required_toml_string(&manifest, "auv2_type")?,
            auv2_subtype: required_toml_string(&manifest, "auv2_subtype")?,
            auv2_manufacturer_code: required_toml_string(&manifest, "auv2_manufacturer_code")?,
            standalone_name: required_toml_string(&manifest, "standalone_name")?,
        };
        metadata.validate()?;
        Ok(metadata)
    }

    pub(crate) fn clap_bundle_name(&self) -> String {
        format!("{}.clap", self.plugin_name)
    }

    pub(crate) fn vst3_bundle_name(&self) -> String {
        format!("{}.vst3", self.plugin_name)
    }

    pub(crate) fn au_bundle_name(&self) -> String {
        format!("{}.component", self.plugin_name)
    }

    fn validate(&self) -> Result<()> {
        validate_four_ascii("auv2_type", &self.auv2_type)?;
        validate_four_ascii("auv2_subtype", &self.auv2_subtype)?;
        validate_four_ascii("auv2_manufacturer_code", &self.auv2_manufacturer_code)?;
        Ok(())
    }
}

fn required_toml_string(manifest: &str, key: &str) -> Result<String> {
    read_toml_string(manifest, "package.metadata.wrac", key).ok_or_else(|| {
        format!("missing package.metadata.wrac.{key} in src-plugin/Cargo.toml").into()
    })
}

fn read_toml_string(manifest: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line == format!("[{section}]");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((line_key, value)) = line.split_once('=') else {
            continue;
        };
        if line_key.trim() != key {
            continue;
        }
        return parse_toml_basic_string(value.trim());
    }
    None
}

fn parse_toml_basic_string(value: &str) -> Option<String> {
    let value = value.strip_prefix('"')?;
    let value = value.strip_suffix('"')?;
    Some(value.replace("\\\"", "\"").replace("\\\\", "\\"))
}

fn validate_four_ascii(key: &str, value: &str) -> Result<()> {
    if value.len() == 4 && value.is_ascii() {
        Ok(())
    } else {
        Err(format!("package.metadata.wrac.{key} must be exactly 4 ASCII bytes").into())
    }
}
