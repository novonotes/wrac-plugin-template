use std::path::Path;

use crate::Result;
use crate::targets::PluginFormat;

pub(crate) use wrac_manifest::{AaxStemConfig as AaxStemConfigMetadata, ValidationMetadata};

#[derive(Debug, Clone)]
#[allow(dead_code)]
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
    pub(crate) supported_formats: Vec<PluginFormat>,
    pub(crate) plugins: Vec<PluginProductMetadata>,
    pub(crate) validation: ValidationMetadata,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
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
    pub(crate) aax_stem_configs: Vec<AaxStemConfigMetadata>,
}

impl PluginMetadata {
    pub(crate) fn read_discovered(manifest_path: &Path, plugin_root: &Path) -> Result<Self> {
        let source = wrac_manifest::discover_manifest(manifest_path, plugin_root)?;
        let mut manifest = wrac_manifest::read_manifest(&source)?;
        let cargo_package = wrac_manifest::read_cargo_package_info(manifest_path)?;
        if manifest.package.package_name.is_none() {
            manifest.package.package_name = cargo_package.package_name;
        }
        if manifest.package.version.is_none() {
            manifest.package.version = cargo_package.version;
        }
        if manifest.package.repository.is_none() {
            manifest.package.repository = cargo_package.repository;
        }
        Self::from_manifest(manifest)
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
        self.plugins
            .first()
            .expect("validated metadata must contain at least one plugin")
    }

    fn from_manifest(manifest: wrac_manifest::PluginManifest) -> Result<Self> {
        let package_name = manifest
            .package
            .package_name
            .ok_or("WRAC manifest package name is required when used from xtask")?;
        let version = manifest
            .package
            .version
            .ok_or("WRAC manifest package version is required when used from xtask")?;
        Ok(Self {
            package_name,
            version,
            repository: manifest.package.repository,
            company_name: manifest.company_name,
            auv2_manufacturer_code: manifest.auv2_manufacturer_code,
            aax_manufacturer_id: manifest.aax_manufacturer_id,
            bundle_name: manifest.bundle_name,
            bundle_identifier: manifest.bundle_identifier,
            homepage_url: manifest.homepage_url,
            manual_url: manifest.manual_url,
            support_url: manifest.support_url,
            description: manifest.description,
            copyright: manifest.copyright,
            supported_formats: manifest
                .supported_formats
                .into_iter()
                .map(convert_plugin_format)
                .collect(),
            plugins: manifest
                .plugins
                .into_iter()
                .map(|plugin| PluginProductMetadata {
                    plugin_id: plugin.plugin_id,
                    plugin_name: plugin.plugin_name,
                    clap_features: plugin.clap_features,
                    vst3_subcategories: plugin.vst3_subcategories,
                    vst3_component_id: plugin.vst3_component_id,
                    standalone_name: plugin.standalone_name,
                    auv2_type: plugin.auv2_type,
                    auv2_subtype: plugin.auv2_subtype,
                    aax_categories: plugin.aax_categories,
                    aax_product_id: plugin.aax_product_id,
                    aax_stem_configs: plugin.aax_stem_configs,
                })
                .collect(),
            validation: manifest.validation,
        })
    }
}

fn convert_plugin_format(format: wrac_manifest::PluginFormat) -> PluginFormat {
    match format {
        wrac_manifest::PluginFormat::Clap => PluginFormat::Clap,
        wrac_manifest::PluginFormat::Vst3 => PluginFormat::Vst3,
        wrac_manifest::PluginFormat::Au => PluginFormat::Au,
        wrac_manifest::PluginFormat::Aax => PluginFormat::Aax,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use wrac_manifest::ClapValidatorMetadata;

    fn metadata() -> PluginMetadata {
        PluginMetadata {
            package_name: "test_plugin".to_string(),
            version: "1.0.0".to_string(),
            repository: None,
            company_name: "Example".to_string(),
            auv2_manufacturer_code: "ExCo".to_string(),
            aax_manufacturer_id: None,
            bundle_name: "Test Plugin".to_string(),
            bundle_identifier: "com.example.test-plugin".to_string(),
            homepage_url: "https://example.com".to_string(),
            manual_url: "https://example.com/manual".to_string(),
            support_url: "https://example.com/support".to_string(),
            description: "Test plugin".to_string(),
            copyright: "Copyright Example".to_string(),
            supported_formats: vec![PluginFormat::Clap, PluginFormat::Vst3, PluginFormat::Au],
            plugins: vec![PluginProductMetadata {
                plugin_id: "com.example.test-plugin".to_string(),
                plugin_name: "Test Plugin".to_string(),
                clap_features: vec!["audio-effect".to_string(), "stereo".to_string()],
                vst3_subcategories: "Fx".to_string(),
                vst3_component_id: "5c65bb45-6f84-527b-915a-a51a30ea5854".to_string(),
                standalone_name: "Test Plugin Standalone".to_string(),
                auv2_type: "aufx".to_string(),
                auv2_subtype: "TstP".to_string(),
                aax_categories: None,
                aax_product_id: None,
                aax_stem_configs: Vec::new(),
            }],
            validation: ValidationMetadata {
                disabled_rules: HashMap::new(),
                clap_validator: ClapValidatorMetadata::default(),
            },
        }
    }

    #[test]
    fn bundle_names_use_bundle_name() {
        let metadata = metadata();
        assert_eq!(metadata.clap_bundle_name(), "Test Plugin.clap");
        assert_eq!(metadata.vst3_bundle_name(), "Test Plugin.vst3");
        assert_eq!(metadata.au_bundle_name(), "Test Plugin.component");
    }
}
