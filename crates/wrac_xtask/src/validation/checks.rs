use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap_sys::ext::params::{
    CLAP_PARAM_IS_BYPASS, CLAP_PARAM_IS_ENUM, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY,
    CLAP_PARAM_IS_STEPPED,
};

use crate::metadata::{PluginMetadata, ValidationMetadata};
use crate::targets::ValidateTarget;
use crate::{Result, targets::ValidateTarget as Target};

use super::clap_schema::{ParameterSchema, PluginSchema};

const RULE_FENDER_SINGLE_KNOB: &str = "fender-studio-pro-generic-editor-single-knob";
const RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX: &str = "luna-vst3-param-id-must-match-index";
const RULE_BYPASS_PARAM_SHAPE: &str = "bypass-param-shape";
const RULE_PLUGIN_REQUIRES_BYPASS: &str = "plugin-requires-bypass";
const RULE_CLAP_DESCRIPTORS_MATCH_MANIFEST: &str = "clap-descriptors-match-manifest";
const RULE_WRAPPER_TARGETS_REQUIRE_SINGLE_PRODUCT: &str = "wrapper-targets-require-single-product";
const RULE_PARAM_INFO_SHAPE: &str = "param-info-shape";
const RULE_STATE_EXTENSION_REQUIRED: &str = "state-extension-required";
const RULE_GUI_ARTIFACT_SHAPE: &str = "gui-artifact-shape";
const RULE_TEMPLATE_PLACEHOLDERS_RENAMED: &str = "template-placeholders-renamed";

const KNOWN_RULES: &[&str] = &[
    RULE_FENDER_SINGLE_KNOB,
    RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX,
    RULE_BYPASS_PARAM_SHAPE,
    RULE_PLUGIN_REQUIRES_BYPASS,
    RULE_CLAP_DESCRIPTORS_MATCH_MANIFEST,
    RULE_WRAPPER_TARGETS_REQUIRE_SINGLE_PRODUCT,
    RULE_PARAM_INFO_SHAPE,
    RULE_STATE_EXTENSION_REQUIRED,
    RULE_GUI_ARTIFACT_SHAPE,
    RULE_TEMPLATE_PLACEHOLDERS_RENAMED,
];

pub(crate) fn validate_disabled_rules(validation: &ValidationMetadata) -> Result<()> {
    for rule_id in validation.disabled_rules.keys() {
        if !KNOWN_RULES.contains(&rule_id.as_str()) {
            return Err(format!(
                "unknown WRAC production-readiness rule in disabled_rules: {rule_id}"
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) struct BundleCheckInputs<'a> {
    pub(crate) schemas: &'a [PluginSchema],
    pub(crate) metadata: &'a PluginMetadata,
    pub(crate) validation: &'a ValidationMetadata,
    pub(crate) location: &'a Path,
    pub(crate) targets: &'a [ValidateTarget],
}

pub(crate) fn evaluate_bundle_checks(input: BundleCheckInputs<'_>) -> Vec<CheckResult> {
    let BundleCheckInputs {
        schemas,
        metadata,
        validation,
        location,
        targets,
    } = input;
    let subject = CheckSubject::bundle(metadata);
    let mut results = Vec::new();

    push_check_result_for_subject(
        &mut results,
        validation,
        &subject,
        RULE_CLAP_DESCRIPTORS_MATCH_MANIFEST,
        CheckStatus::from_violations(clap_descriptor_manifest_violations(
            schemas, metadata, location,
        )),
    );

    let wrapper_requested = targets
        .iter()
        .any(|target| matches!(target, Target::Vst3 | Target::Au));
    let wrapper_product_violations = if wrapper_requested && metadata.plugins.len() > 1 {
        vec![RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_WRAPPER_TARGETS_REQUIRE_SINGLE_PRODUCT,
            message: format!(
                "VST3/AU wrapper validation currently supports one manifest product per bundle. manifest_product_count={}",
                metadata.plugins.len()
            ),
            fix: "Release wrapper formats as single-product bundles, or disable this rule with a documented reason after confirming wrapper metadata and host scans for every product.",
        }]
    } else {
        Vec::new()
    };
    push_check_result_for_subject(
        &mut results,
        validation,
        &subject,
        RULE_WRAPPER_TARGETS_REQUIRE_SINGLE_PRODUCT,
        CheckStatus::from_violations(wrapper_product_violations),
    );

    results
}

pub(crate) fn evaluate_checks(
    schema: &PluginSchema,
    targets: &[ValidateTarget],
    validation: &ValidationMetadata,
    location: &Path,
) -> Vec<CheckResult> {
    let hidden_or_readonly = |param: &&ParameterSchema| {
        param.flags.contains(CLAP_PARAM_IS_HIDDEN) || param.flags.contains(CLAP_PARAM_IS_READONLY)
    };
    let visible_non_bypass_count = schema
        .params
        .iter()
        .filter(|param| !hidden_or_readonly(param) && !param.flags.contains(CLAP_PARAM_IS_BYPASS))
        .count();
    let bypass_params = schema
        .params
        .iter()
        .filter(|param| param.flags.contains(CLAP_PARAM_IS_BYPASS))
        .collect::<Vec<_>>();

    let mut results = Vec::new();
    push_check_result(
        &mut results,
        validation,
        schema,
        RULE_PARAM_INFO_SHAPE,
        CheckStatus::from_violations(param_info_shape_violations(schema, location)),
    );

    // Keep target-inapplicable checks in the report as `skipped`. Without this, CI logs
    // cannot distinguish "not relevant for this target" from "the check was never registered".
    if targets
        .iter()
        .any(|target| matches!(target, Target::Clap | Target::Vst3))
    {
        let violations = if visible_non_bypass_count == 1 {
            vec![RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_FENDER_SINGLE_KNOB,
                message: format!(
                    "Fender Studio Pro generic editors fail to render knobs when exactly one visible non-bypass parameter is exposed. visible_non_bypass_parameter_count={visible_non_bypass_count}"
                ),
                fix: "Expose zero or at least two visible non-bypass parameters, or disable this rule with a documented reason.",
            }]
        } else {
            Vec::new()
        };
        push_check_result(
            &mut results,
            validation,
            schema,
            RULE_FENDER_SINGLE_KNOB,
            CheckStatus::from_violations(violations),
        );
    } else {
        push_check_result(
            &mut results,
            validation,
            schema,
            RULE_FENDER_SINGLE_KNOB,
            CheckStatus::Skipped("CLAP or VST3 validation was not requested."),
        );
    }

    if targets.contains(&Target::Vst3) {
        let mut violations = Vec::new();
        for (index, param) in schema.params.iter().enumerate() {
            if param.id != index as u32 {
                violations.push(RuleViolation {
                    plugin_id: schema.plugin_id.clone(),
                    plugin_name: schema.plugin_name.clone(),
                    location: location.to_path_buf(),
                    rule_id: RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX,
                    message: format!(
                        "LUNA 2.0.3.4381 VST3 automation writes fail when ParamID differs from parameter index. index={index} id={} name=\"{}\"",
                        param.id, param.name
                    ),
                    fix: "Keep public VST3 parameter IDs equal to their parameter-list indices.",
                });
            }
        }
        push_check_result(
            &mut results,
            validation,
            schema,
            RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX,
            CheckStatus::from_violations(violations),
        );
    } else {
        push_check_result(
            &mut results,
            validation,
            schema,
            RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX,
            CheckStatus::Skipped("VST3 validation was not requested."),
        );
    }

    let mut bypass_shape_violations = Vec::new();
    if bypass_params.len() > 1 {
        bypass_shape_violations.push(RuleViolation {
            plugin_id: schema.plugin_id.clone(),
            plugin_name: schema.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_BYPASS_PARAM_SHAPE,
            message: format!(
                "Only one bypass parameter may be exposed. bypass_parameter_count={}",
                bypass_params.len()
            ),
            fix: "Expose a single host bypass parameter.",
        });
    }
    for param in bypass_params {
        let stepped = param.flags.contains(CLAP_PARAM_IS_STEPPED);
        let enum_flag = param.flags.contains(CLAP_PARAM_IS_ENUM);
        let default_is_boolean =
            nearly_equal(param.default_value, 0.0) || nearly_equal(param.default_value, 1.0);
        if !stepped
            || !enum_flag
            || !nearly_equal(param.min_value, 0.0)
            || !nearly_equal(param.max_value, 1.0)
            || !default_is_boolean
        {
            bypass_shape_violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_BYPASS_PARAM_SHAPE,
                message: format!(
                    "Bypass parameter must be a stepped enum with range 0..1 and a boolean default. id={} name=\"{}\" stepped={stepped} enum={enum_flag} min={} max={} default={}",
                    param.id, param.name, param.min_value, param.max_value, param.default_value
                ),
                fix: "Set bypass flags to stepped + enum + bypass, min=0, max=1, and default=0 or 1.",
            });
        }
    }
    push_check_result(
        &mut results,
        validation,
        schema,
        RULE_BYPASS_PARAM_SHAPE,
        CheckStatus::from_violations(bypass_shape_violations),
    );

    let bypass_required_violations = if schema
        .params
        .iter()
        .all(|param| !param.flags.contains(CLAP_PARAM_IS_BYPASS))
    {
        vec![RuleViolation {
            plugin_id: schema.plugin_id.clone(),
            plugin_name: schema.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_PLUGIN_REQUIRES_BYPASS,
            message: "Production plugins should expose a host bypass parameter.".to_string(),
            fix: "Add one bypass parameter, or disable this rule with a documented reason.",
        }]
    } else {
        Vec::new()
    };
    push_check_result(
        &mut results,
        validation,
        schema,
        RULE_PLUGIN_REQUIRES_BYPASS,
        CheckStatus::from_violations(bypass_required_violations),
    );

    push_check_result(
        &mut results,
        validation,
        schema,
        RULE_STATE_EXTENSION_REQUIRED,
        CheckStatus::from_violations(state_extension_violations(schema, location)),
    );

    results
}

pub(crate) fn evaluate_source_checks(
    schemas: &[PluginSchema],
    metadata: &PluginMetadata,
    validation: &ValidationMetadata,
    location: &Path,
    repository_root: &Path,
    gui_dir: &Path,
) -> Vec<CheckResult> {
    let subject = CheckSubject::bundle(metadata);
    let mut results = Vec::new();

    // Only deterministic source checks live in the validate gate. Review-heavy source
    // concerns stay in the code review checklist so this remains a low-noise release gate.
    push_check_result_for_subject(
        &mut results,
        validation,
        &subject,
        RULE_GUI_ARTIFACT_SHAPE,
        CheckStatus::from_violations(gui_artifact_shape_violations(
            schemas, metadata, location, gui_dir,
        )),
    );

    push_check_result_for_subject(
        &mut results,
        validation,
        &subject,
        RULE_TEMPLATE_PLACEHOLDERS_RENAMED,
        if is_template_development_checkout(repository_root) {
            CheckStatus::Skipped(
                "Template placeholder metadata is expected in the template repository itself.",
            )
        } else {
            CheckStatus::from_violations(template_placeholder_violations(metadata, location))
        },
    );

    results
}

fn param_info_shape_violations(schema: &PluginSchema, location: &Path) -> Vec<RuleViolation> {
    let mut violations = Vec::new();
    let mut seen_ids = HashSet::new();

    for param in &schema.params {
        if !seen_ids.insert(param.id) {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_PARAM_INFO_SHAPE,
                message: format!(
                    "Public parameter IDs must be unique. duplicate_id={} name=\"{}\"",
                    param.id, param.name
                ),
                fix: "Assign one stable unique parameter ID to each public parameter.",
            });
        }
        if param.name.trim().is_empty() {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_PARAM_INFO_SHAPE,
                message: format!("Public parameter names must not be empty. id={}", param.id),
                fix: "Expose a non-empty stable name for every public parameter.",
            });
        }
        if !param.min_value.is_finite()
            || !param.max_value.is_finite()
            || !param.default_value.is_finite()
        {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_PARAM_INFO_SHAPE,
                message: format!(
                    "Parameter ranges and defaults must be finite. id={} name=\"{}\" min={} max={} default={}",
                    param.id, param.name, param.min_value, param.max_value, param.default_value
                ),
                fix: "Use finite min, max, and default values.",
            });
            continue;
        }
        if param.min_value >= param.max_value {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_PARAM_INFO_SHAPE,
                message: format!(
                    "Parameter min must be less than max. id={} name=\"{}\" min={} max={}",
                    param.id, param.name, param.min_value, param.max_value
                ),
                fix: "Set each parameter range so min < max.",
            });
        }
        if param.default_value < param.min_value || param.default_value > param.max_value {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_PARAM_INFO_SHAPE,
                message: format!(
                    "Parameter default must be inside its range. id={} name=\"{}\" min={} max={} default={}",
                    param.id, param.name, param.min_value, param.max_value, param.default_value
                ),
                fix: "Set each default value inside its declared range.",
            });
        }
    }

    violations
}

fn state_extension_violations(schema: &PluginSchema, location: &Path) -> Vec<RuleViolation> {
    if schema.has_state {
        return Vec::new();
    }

    vec![RuleViolation {
        plugin_id: schema.plugin_id.clone(),
        plugin_name: schema.plugin_name.clone(),
        location: location.to_path_buf(),
        rule_id: RULE_STATE_EXTENSION_REQUIRED,
        message: "Production plugins should expose the CLAP state extension for project recall."
            .to_string(),
        fix: "Implement state save/load for the plugin, or disable this rule with a documented reason.",
    }]
}

fn gui_artifact_shape_violations(
    schemas: &[PluginSchema],
    metadata: &PluginMetadata,
    location: &Path,
    gui_dir: &Path,
) -> Vec<RuleViolation> {
    let subject = CheckSubject::bundle(metadata);
    let mut violations = Vec::new();
    let gui_source_exists = gui_dir.exists();
    if gui_source_exists && schemas.iter().any(|schema| !schema.has_gui) {
        for schema in schemas.iter().filter(|schema| !schema.has_gui) {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_GUI_ARTIFACT_SHAPE,
                message: "Plugin has src-gui but does not expose the CLAP GUI extension."
                    .to_string(),
                fix: "Expose a GUI extension for products with src-gui, or disable this rule with a documented reason for headless products.",
            });
        }
    }
    if gui_source_exists && !gui_dir.join("dist").join("index.html").exists() {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id,
            plugin_name: subject.plugin_name,
            location: gui_dir.to_path_buf(),
            rule_id: RULE_GUI_ARTIFACT_SHAPE,
            message: "src-gui/dist/index.html must exist after validate builds the GUI."
                .to_string(),
            fix: "Run the frontend build through cargo xtask validate/build so release GUI assets are generated before the plugin is compiled.",
        });
    }
    violations
}

fn template_placeholder_violations(
    metadata: &PluginMetadata,
    location: &Path,
) -> Vec<RuleViolation> {
    let subject = CheckSubject::bundle(metadata);
    let mut violations = Vec::new();

    check_template_placeholder(
        &mut violations,
        &subject,
        location,
        "package.name",
        &metadata.package_name,
        "wrac_gain_plugin",
    );
    check_template_placeholder(
        &mut violations,
        &subject,
        location,
        "package.repository",
        metadata.repository.as_deref().unwrap_or_default(),
        "github.com/novonotes/wrac-plugin-template",
    );
    check_template_placeholder(
        &mut violations,
        &subject,
        location,
        "package.metadata.wrac.company_name",
        &metadata.company_name,
        "Your Company",
    );
    check_template_placeholder(
        &mut violations,
        &subject,
        location,
        "package.metadata.wrac.bundle_name",
        &metadata.bundle_name,
        "WRAC Gain",
    );
    check_template_placeholder(
        &mut violations,
        &subject,
        location,
        "package.metadata.wrac.standalone_name",
        &metadata.standalone_name,
        "WRAC Gain",
    );
    for (index, plugin) in metadata.plugins.iter().enumerate() {
        check_template_placeholder(
            &mut violations,
            &subject,
            location,
            "package.metadata.wrac.plugins.plugin_id",
            &plugin.plugin_id,
            "com.your-company",
        );
        check_template_placeholder(
            &mut violations,
            &subject,
            location,
            "package.metadata.wrac.plugins.plugin_name",
            &plugin.plugin_name,
            "WRAC Gain",
        );
        check_template_placeholder(
            &mut violations,
            &subject,
            location,
            "package.metadata.wrac.plugins.auv2_subtype",
            &plugin.auv2_subtype,
            "WtGn",
        );
        if plugin.auv2_type == "aufx" && metadata.package_name == "wrac_gain_plugin" {
            violations.push(RuleViolation {
                plugin_id: subject.plugin_id.clone(),
                plugin_name: subject.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_TEMPLATE_PLACEHOLDERS_RENAMED,
                message: format!(
                    "package.metadata.wrac.plugins[{index}].auv2_type still belongs to the unchanged WRAC Gain template identity. value=\"{}\"",
                    plugin.auv2_type
                ),
                fix: "Rename template placeholder metadata before shipping, or disable this rule with a documented reason for template/example repositories.",
            });
        }
    }
    violations
}

fn check_template_placeholder(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    location: &Path,
    field: &'static str,
    value: &str,
    placeholder: &'static str,
) {
    if value.contains(placeholder) {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_TEMPLATE_PLACEHOLDERS_RENAMED,
            message: format!(
                "{field} still contains template placeholder \"{placeholder}\". value=\"{value}\""
            ),
            fix: "Rename template placeholder metadata before shipping, or disable this rule with a documented reason for template/example repositories.",
        });
    }
}

fn is_template_development_checkout(repository_root: &Path) -> bool {
    let Ok(output) = Command::new("git")
        .args(["remote", "-v"])
        .current_dir(repository_root)
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let remotes = String::from_utf8_lossy(&output.stdout);
    [
        "github.com/novonotes/wrac-plugin-template",
        "github.com/satoshi-szk/wrac-plugin-template",
        "github.com/satoshi-assistant/wrac-plugin-template",
    ]
    .iter()
    .any(|needle| remotes.contains(needle))
}

fn clap_descriptor_manifest_violations(
    schemas: &[PluginSchema],
    metadata: &PluginMetadata,
    location: &Path,
) -> Vec<RuleViolation> {
    let subject = CheckSubject::bundle(metadata);
    let mut violations = Vec::new();
    if schemas.len() != metadata.plugins.len() {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_CLAP_DESCRIPTORS_MATCH_MANIFEST,
            message: format!(
                "CLAP descriptor count must match package.metadata.wrac.plugins. descriptor_count={} manifest_count={}",
                schemas.len(),
                metadata.plugins.len()
            ),
            fix: "Expose one CLAP descriptor for each package.metadata.wrac.plugins entry, in the same order.",
        });
    }

    for (index, expected) in metadata.plugins.iter().enumerate() {
        let Some(schema) = schemas.get(index) else {
            continue;
        };
        if schema.plugin_id != expected.plugin_id {
            violations.push(descriptor_manifest_violation(
                schema,
                location,
                index,
                "id",
                &expected.plugin_id,
                &schema.plugin_id,
            ));
        }
        if schema.plugin_name != expected.plugin_name {
            violations.push(descriptor_manifest_violation(
                schema,
                location,
                index,
                "name",
                &expected.plugin_name,
                &schema.plugin_name,
            ));
        }
        if schema.plugin_vendor != metadata.company_name {
            violations.push(descriptor_manifest_violation(
                schema,
                location,
                index,
                "vendor",
                &metadata.company_name,
                &schema.plugin_vendor,
            ));
        }
        if schema.plugin_version != metadata.version {
            violations.push(descriptor_manifest_violation(
                schema,
                location,
                index,
                "version",
                &metadata.version,
                &schema.plugin_version,
            ));
        }
    }
    violations
}

fn descriptor_manifest_violation(
    schema: &PluginSchema,
    location: &Path,
    index: usize,
    field: &str,
    expected: &str,
    actual: &str,
) -> RuleViolation {
    RuleViolation {
        plugin_id: schema.plugin_id.clone(),
        plugin_name: schema.plugin_name.clone(),
        location: location.to_path_buf(),
        rule_id: RULE_CLAP_DESCRIPTORS_MATCH_MANIFEST,
        message: format!(
            "CLAP descriptor {field} does not match manifest metadata. index={index} expected=\"{expected}\" actual=\"{actual}\""
        ),
        fix: "Keep CLAP descriptors generated from package.metadata.wrac instead of hard-coded product metadata.",
    }
}

fn push_check_result(
    results: &mut Vec<CheckResult>,
    validation: &ValidationMetadata,
    schema: &PluginSchema,
    rule_id: &'static str,
    status: CheckStatus,
) {
    // Disabled checks are still reported so reviewers can see that a release-policy check
    // exists and was intentionally bypassed with a reason.
    if let Some(disabled) = validation.disabled_rules.get(rule_id) {
        results.push(CheckResult {
            plugin_id: schema.plugin_id.clone(),
            plugin_name: schema.plugin_name.clone(),
            rule_id,
            status: CheckStatus::Disabled(disabled.reason.clone()),
        });
        return;
    }
    results.push(CheckResult {
        plugin_id: schema.plugin_id.clone(),
        plugin_name: schema.plugin_name.clone(),
        rule_id,
        status,
    });
}

fn push_check_result_for_subject(
    results: &mut Vec<CheckResult>,
    validation: &ValidationMetadata,
    subject: &CheckSubject,
    rule_id: &'static str,
    status: CheckStatus,
) {
    if let Some(disabled) = validation.disabled_rules.get(rule_id) {
        results.push(CheckResult {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            rule_id,
            status: CheckStatus::Disabled(disabled.reason.clone()),
        });
        return;
    }
    results.push(CheckResult {
        plugin_id: subject.plugin_id.clone(),
        plugin_name: subject.plugin_name.clone(),
        rule_id,
        status,
    });
}

struct CheckSubject {
    plugin_id: String,
    plugin_name: String,
}

impl CheckSubject {
    fn bundle(metadata: &PluginMetadata) -> Self {
        let primary = metadata.primary_plugin();
        Self {
            plugin_id: primary.plugin_id.clone(),
            plugin_name: metadata.bundle_name.clone(),
        }
    }
}

fn nearly_equal(a: f64, b: f64) -> bool {
    (a - b).abs() < f64::EPSILON
}

#[derive(Debug)]
pub(crate) struct CheckResult {
    pub(crate) plugin_id: String,
    pub(crate) plugin_name: String,
    pub(crate) rule_id: &'static str,
    pub(crate) status: CheckStatus,
}

#[derive(Debug)]
pub(crate) enum CheckStatus {
    Passed,
    Failed(Vec<RuleViolation>),
    Skipped(&'static str),
    Disabled(String),
}

impl CheckStatus {
    fn from_violations(violations: Vec<RuleViolation>) -> Self {
        if violations.is_empty() {
            Self::Passed
        } else {
            Self::Failed(violations)
        }
    }
}

#[derive(Debug)]
pub(crate) struct RuleViolation {
    pub(crate) plugin_id: String,
    pub(crate) plugin_name: String,
    pub(crate) location: PathBuf,
    pub(crate) rule_id: &'static str,
    pub(crate) message: String,
    pub(crate) fix: &'static str,
}

trait FlagContains {
    fn contains(self, flag: u32) -> bool;
}

impl FlagContains for u32 {
    fn contains(self, flag: u32) -> bool {
        self & flag != 0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use crate::metadata::{
        DisabledValidationRule, PluginMetadata, PluginProductMetadata, ValidationMetadata,
    };
    use crate::targets::ValidateTarget;

    use super::super::clap_schema::{ParameterSchema, PluginSchema};
    use super::*;

    fn schema(params: Vec<ParameterSchema>) -> PluginSchema {
        PluginSchema {
            plugin_id: "com.example.test".to_string(),
            plugin_name: "Test Plugin".to_string(),
            plugin_vendor: "Example".to_string(),
            plugin_version: "1.0.0".to_string(),
            has_gui: true,
            has_state: true,
            params,
        }
    }

    fn metadata() -> PluginMetadata {
        PluginMetadata {
            package_name: "test_plugin".to_string(),
            version: "1.0.0".to_string(),
            repository: Some("https://github.com/example/test-plugin".to_string()),
            company_name: "Example".to_string(),
            auv2_manufacturer_code: "ExCo".to_string(),
            bundle_name: "Test Plugin".to_string(),
            standalone_name: "Test Plugin Standalone".to_string(),
            plugins: vec![PluginProductMetadata {
                plugin_id: "com.example.test".to_string(),
                plugin_name: "Test Plugin".to_string(),
                auv2_type: "aufx".to_string(),
                auv2_subtype: "TstP".to_string(),
            }],
            validation: ValidationMetadata::default(),
        }
    }

    fn param(id: u32, flags: u32) -> ParameterSchema {
        ParameterSchema {
            id,
            name: format!("Param {id}"),
            flags,
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
        }
    }

    fn no_disabled_rules() -> ValidationMetadata {
        ValidationMetadata::default()
    }

    fn status_for<'a>(results: &'a [CheckResult], rule_id: &str) -> &'a CheckStatus {
        &results
            .iter()
            .find(|result| result.rule_id == rule_id)
            .expect("rule result should exist")
            .status
    }

    fn rule_failed(results: &[CheckResult], rule_id: &str) -> bool {
        matches!(status_for(results, rule_id), CheckStatus::Failed(_))
    }

    fn valid_bypass_param(id: u32) -> ParameterSchema {
        param(
            id,
            CLAP_PARAM_IS_BYPASS | CLAP_PARAM_IS_STEPPED | CLAP_PARAM_IS_ENUM,
        )
    }

    #[test]
    fn single_visible_non_bypass_parameter_fails_for_clap_and_vst3() {
        let results = evaluate_checks(
            &schema(vec![param(0, 0), param(1, CLAP_PARAM_IS_BYPASS)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_FENDER_SINGLE_KNOB));
    }

    #[test]
    fn single_visible_non_bypass_parameter_is_skipped_for_au_only() {
        let results = evaluate_checks(
            &schema(vec![param(0, 0), valid_bypass_param(1)]),
            &[ValidateTarget::Au],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(matches!(
            status_for(&results, RULE_FENDER_SINGLE_KNOB),
            CheckStatus::Skipped(_)
        ));
    }

    #[test]
    fn zero_visible_non_bypass_parameters_are_allowed() {
        let results = evaluate_checks(
            &schema(vec![valid_bypass_param(0)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(matches!(
            status_for(&results, RULE_FENDER_SINGLE_KNOB),
            CheckStatus::Passed
        ));
    }

    #[test]
    fn two_visible_non_bypass_parameters_are_allowed() {
        let results = evaluate_checks(
            &schema(vec![valid_bypass_param(0), param(1, 0), param(2, 0)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(matches!(
            status_for(&results, RULE_FENDER_SINGLE_KNOB),
            CheckStatus::Passed
        ));
    }

    #[test]
    fn hidden_readonly_and_bypass_parameters_do_not_count_as_visible_knobs() {
        let results = evaluate_checks(
            &schema(vec![
                valid_bypass_param(0),
                param(1, CLAP_PARAM_IS_HIDDEN),
                param(2, CLAP_PARAM_IS_READONLY),
            ]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(matches!(
            status_for(&results, RULE_FENDER_SINGLE_KNOB),
            CheckStatus::Passed
        ));
    }

    #[test]
    fn disabled_rules_are_reported() {
        let mut disabled_rules = HashMap::new();
        disabled_rules.insert(
            RULE_FENDER_SINGLE_KNOB.to_string(),
            DisabledValidationRule {
                reason: "not a supported host workflow".to_string(),
            },
        );
        let validation = ValidationMetadata { disabled_rules };
        let results = evaluate_checks(
            &schema(vec![param(0, 0), param(1, CLAP_PARAM_IS_BYPASS)]),
            &[ValidateTarget::Clap],
            &validation,
            Path::new("Cargo.toml"),
        );
        assert!(matches!(
            status_for(&results, RULE_FENDER_SINGLE_KNOB),
            CheckStatus::Disabled(reason) if reason == "not a supported host workflow"
        ));
    }

    #[test]
    fn vst3_param_id_must_match_index() {
        let results = evaluate_checks(
            &schema(vec![param(1, 0), param(2, 0)]),
            &[ValidateTarget::Vst3],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX));
    }

    #[test]
    fn vst3_only_rule_is_skipped_without_vst3_target() {
        let results = evaluate_checks(
            &schema(vec![valid_bypass_param(0)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(matches!(
            status_for(&results, RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX),
            CheckStatus::Skipped(_)
        ));
    }

    #[test]
    fn vst3_param_ids_matching_indices_pass() {
        let results = evaluate_checks(
            &schema(vec![valid_bypass_param(0), param(1, 0)]),
            &[ValidateTarget::Vst3],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(matches!(
            status_for(&results, RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX),
            CheckStatus::Passed
        ));
    }

    #[test]
    fn bypass_shape_requires_stepped_flag() {
        let results = evaluate_checks(
            &schema(vec![param(0, CLAP_PARAM_IS_BYPASS | CLAP_PARAM_IS_ENUM)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_BYPASS_PARAM_SHAPE));
    }

    #[test]
    fn bypass_shape_requires_enum_flag() {
        let results = evaluate_checks(
            &schema(vec![param(0, CLAP_PARAM_IS_BYPASS | CLAP_PARAM_IS_STEPPED)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_BYPASS_PARAM_SHAPE));
    }

    #[test]
    fn bypass_shape_requires_boolean_range() {
        let mut bypass = valid_bypass_param(0);
        bypass.max_value = 2.0;
        let results = evaluate_checks(
            &schema(vec![bypass]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_BYPASS_PARAM_SHAPE));
    }

    #[test]
    fn bypass_shape_requires_boolean_default() {
        let mut bypass = valid_bypass_param(0);
        bypass.default_value = 0.5;
        let results = evaluate_checks(
            &schema(vec![bypass]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_BYPASS_PARAM_SHAPE));
    }

    #[test]
    fn bypass_shape_allows_one_valid_bypass_parameter() {
        let results = evaluate_checks(
            &schema(vec![valid_bypass_param(0)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(matches!(
            status_for(&results, RULE_BYPASS_PARAM_SHAPE),
            CheckStatus::Passed
        ));
    }

    #[test]
    fn bypass_shape_rejects_multiple_bypass_parameters() {
        let results = evaluate_checks(
            &schema(vec![valid_bypass_param(0), valid_bypass_param(1)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_BYPASS_PARAM_SHAPE));
    }

    #[test]
    fn plugin_requires_bypass() {
        let results = evaluate_checks(
            &schema(Vec::new()),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_PLUGIN_REQUIRES_BYPASS));
    }

    #[test]
    fn plugin_requires_bypass_when_only_non_bypass_parameters_exist() {
        let results = evaluate_checks(
            &schema(vec![param(0, 0), param(1, 0)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_PLUGIN_REQUIRES_BYPASS));
    }

    #[test]
    fn plugin_requires_bypass_passes_with_valid_bypass_parameter() {
        let results = evaluate_checks(
            &schema(vec![valid_bypass_param(0)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(matches!(
            status_for(&results, RULE_PLUGIN_REQUIRES_BYPASS),
            CheckStatus::Passed
        ));
    }

    #[test]
    fn parameter_info_requires_unique_ids() {
        let results = evaluate_checks(
            &schema(vec![valid_bypass_param(0), param(0, 0)]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_PARAM_INFO_SHAPE));
    }

    #[test]
    fn parameter_info_requires_default_inside_range() {
        let mut gain = param(1, 0);
        gain.default_value = 2.0;
        let results = evaluate_checks(
            &schema(vec![valid_bypass_param(0), gain]),
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_PARAM_INFO_SHAPE));
    }

    #[test]
    fn state_extension_is_required() {
        let mut schema = schema(vec![valid_bypass_param(0)]);
        schema.has_state = false;
        let results = evaluate_checks(
            &schema,
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_STATE_EXTENSION_REQUIRED));
    }

    #[test]
    fn gui_artifact_requires_gui_extension_when_gui_source_exists() {
        let mut schema = schema(vec![valid_bypass_param(0)]);
        schema.has_gui = false;
        let violations = gui_artifact_shape_violations(
            &[schema],
            &metadata(),
            Path::new("Cargo.toml"),
            Path::new("."),
        );
        assert!(
            violations
                .iter()
                .any(|violation| violation.rule_id == RULE_GUI_ARTIFACT_SHAPE)
        );
    }

    #[test]
    fn placeholder_check_rejects_template_identity() {
        let mut metadata = metadata();
        metadata.package_name = "wrac_gain_plugin".to_string();
        metadata.company_name = "Your Company".to_string();
        metadata.plugins[0].plugin_id = "com.your-company.wrac-gain".to_string();

        let violations = template_placeholder_violations(&metadata, Path::new("Cargo.toml"));

        assert!(
            violations
                .iter()
                .any(|violation| violation.rule_id == RULE_TEMPLATE_PLACEHOLDERS_RENAMED)
        );
    }

    #[test]
    fn clap_descriptors_must_match_manifest_metadata() {
        let mut schema = schema(vec![valid_bypass_param(0)]);
        schema.plugin_name = "Wrong Name".to_string();
        let violations =
            clap_descriptor_manifest_violations(&[schema], &metadata(), Path::new("Cargo.toml"));

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, RULE_CLAP_DESCRIPTORS_MATCH_MANIFEST);
        assert!(violations[0].message.contains("name"));
    }

    #[test]
    fn clap_descriptors_pass_when_manifest_metadata_matches() {
        let violations = clap_descriptor_manifest_violations(
            &[schema(vec![valid_bypass_param(0)])],
            &metadata(),
            Path::new("Cargo.toml"),
        );

        assert!(violations.is_empty());
    }
}
