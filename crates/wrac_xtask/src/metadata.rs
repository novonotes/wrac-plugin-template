use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::Result;

#[derive(Debug, Clone)]
pub(crate) struct PluginMetadata {
    pub(crate) package_name: String,
    pub(crate) company_name: String,
    pub(crate) auv2_manufacturer_code: String,
    pub(crate) bundle_name: String,
    pub(crate) standalone_name: String,
    pub(crate) plugins: Vec<PluginProductMetadata>,
}

#[derive(Debug, Clone)]
pub(crate) struct PluginProductMetadata {
    pub(crate) plugin_id: String,
    pub(crate) plugin_name: String,
    pub(crate) auv2_type: String,
    pub(crate) auv2_subtype: String,
}

impl PluginMetadata {
    pub(crate) fn read(manifest_path: &Path) -> Result<Self> {
        let manifest = fs::read_to_string(manifest_path)?;
        let metadata = Self {
            package_name: read_toml_string(&manifest, "package", "name")
                .ok_or("missing package.name in plugin Cargo.toml")?,
            company_name: required_toml_string(&manifest, "company_name")?,
            auv2_manufacturer_code: required_toml_string(&manifest, "auv2_manufacturer_code")?,
            bundle_name: required_toml_string(&manifest, "bundle_name")?,
            standalone_name: required_toml_string(&manifest, "standalone_name")?,
            plugins: read_plugin_products(&manifest)?,
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

    pub(crate) fn au_bundle_name(&self) -> String {
        format!("{}.component", self.bundle_name)
    }

    pub(crate) fn primary_plugin(&self) -> &PluginProductMetadata {
        // WRAC bundles may expose multiple plugin products from one binary, but wrapper
        // fallbacks and standalone launch still need one stable identity. The first
        // metadata entry is that primary product; validation and generated Rust metadata
        // still cover every entry in `plugins`.
        self.plugins
            .first()
            .expect("validated metadata must contain at least one plugin")
    }

    fn validate(&self) -> Result<()> {
        if self.plugins.is_empty() {
            return Err("package.metadata.wrac.plugins must contain at least one plugin".into());
        }
        validate_four_ascii("auv2_manufacturer_code", &self.auv2_manufacturer_code)?;
        let mut plugin_ids = HashSet::new();
        let mut auv2_ids = HashSet::new();
        for plugin in &self.plugins {
            validate_four_ascii("auv2_type", &plugin.auv2_type)?;
            validate_four_ascii("auv2_subtype", &plugin.auv2_subtype)?;
            if !plugin_ids.insert(plugin.plugin_id.as_str()) {
                return Err(format!(
                    "duplicate package.metadata.wrac.plugins plugin_id: {}",
                    plugin.plugin_id
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
        Ok(())
    }
}

fn required_toml_string(manifest: &str, key: &str) -> Result<String> {
    read_toml_string(manifest, "package.metadata.wrac", key).ok_or_else(|| {
        format!("missing package.metadata.wrac.{key} in src-plugin/Cargo.toml").into()
    })
}

fn read_plugin_products(manifest: &str) -> Result<Vec<PluginProductMetadata>> {
    let mut plugins = Vec::new();
    let mut current: Option<PluginProductMetadata> = None;
    let mut in_plugins = false;

    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(plugin) = current.take() {
                plugins.push(plugin);
            }
            in_plugins = line == "[[package.metadata.wrac.plugins]]";
            if in_plugins {
                current = Some(PluginProductMetadata {
                    plugin_id: String::new(),
                    plugin_name: String::new(),
                    auv2_type: String::new(),
                    auv2_subtype: String::new(),
                });
            }
            continue;
        }
        if !in_plugins {
            continue;
        }
        let Some((line_key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = parse_toml_basic_string(value.trim()) else {
            continue;
        };
        let plugin = current
            .as_mut()
            .expect("plugin table must create current metadata");
        match line_key.trim() {
            "plugin_id" => plugin.plugin_id = value,
            "plugin_name" => plugin.plugin_name = value,
            "auv2_type" => plugin.auv2_type = value,
            "auv2_subtype" => plugin.auv2_subtype = value,
            _ => {}
        }
    }
    if let Some(plugin) = current {
        plugins.push(plugin);
    }
    for plugin in &plugins {
        if plugin.plugin_id.is_empty()
            || plugin.plugin_name.is_empty()
            || plugin.auv2_type.is_empty()
            || plugin.auv2_subtype.is_empty()
        {
            return Err("incomplete package.metadata.wrac.plugins entry".into());
        }
    }
    Ok(plugins)
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
