use std::path::{Path, PathBuf};

use clap_sys::ext::params::{
    CLAP_PARAM_IS_BYPASS, CLAP_PARAM_IS_ENUM, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY,
    CLAP_PARAM_IS_STEPPED,
};

use crate::metadata::{PluginMetadata, ValidationMetadata};
use crate::targets::Platform;
use crate::targets::ValidateTarget;
use crate::{Result, targets::ValidateTarget as Target};

use super::clap_schema::{ParameterSchema, PluginSchema};

const RULE_FENDER_SINGLE_KNOB: &str = "fender-studio-pro-generic-editor-single-knob";
const RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX: &str = "luna-vst3-param-id-must-match-index";
const RULE_BYPASS_PARAM_SHAPE: &str = "bypass-param-shape";
const RULE_PLUGIN_REQUIRES_BYPASS: &str = "plugin-requires-bypass";
const RULE_CLAP_DESCRIPTORS_MATCH_MANIFEST: &str = "clap-descriptors-match-manifest";
const RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST: &str = "macos-clap-info-plist-matches-manifest";

const KNOWN_RULES: &[&str] = &[
    RULE_FENDER_SINGLE_KNOB,
    RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX,
    RULE_BYPASS_PARAM_SHAPE,
    RULE_PLUGIN_REQUIRES_BYPASS,
    RULE_CLAP_DESCRIPTORS_MATCH_MANIFEST,
    RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
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

pub(crate) fn evaluate_bundle_checks(
    schemas: &[PluginSchema],
    metadata: &PluginMetadata,
    validation: &ValidationMetadata,
    location: &Path,
    platform: Platform,
    clap_bundle: &Path,
) -> Vec<CheckResult> {
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

    if platform == Platform::Macos {
        push_check_result_for_subject(
            &mut results,
            validation,
            &subject,
            RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
            CheckStatus::from_violations(clap_info_plist_violations(
                metadata,
                &clap_bundle.join("Contents").join("Info.plist"),
            )),
        );
    } else {
        push_check_result_for_subject(
            &mut results,
            validation,
            &subject,
            RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
            CheckStatus::Skipped("macOS CLAP bundle metadata is not available on this platform."),
        );
    }

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

    results
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

fn clap_info_plist_violations(metadata: &PluginMetadata, plist_path: &Path) -> Vec<RuleViolation> {
    let subject = CheckSubject::bundle(metadata);
    let mut violations = Vec::new();
    let value = match plist::Value::from_file(plist_path) {
        Ok(value) => value,
        Err(error) => {
            violations.push(RuleViolation {
                plugin_id: subject.plugin_id.clone(),
                plugin_name: subject.plugin_name.clone(),
                location: plist_path.to_path_buf(),
                rule_id: RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
                message: format!("Failed to read CLAP Info.plist: {error}"),
                fix: "Build the CLAP artifact and keep Contents/Info.plist generated from package.metadata.wrac.",
            });
            return violations;
        }
    };
    let Some(dict) = value.as_dictionary() else {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: plist_path.to_path_buf(),
            rule_id: RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
            message: "CLAP Info.plist must be a dictionary.".to_string(),
            fix: "Regenerate the CLAP bundle Info.plist from package.metadata.wrac.",
        });
        return violations;
    };

    let primary = metadata.primary_plugin();
    check_plist_string(
        &mut violations,
        &subject,
        plist_path,
        dict,
        "CFBundleExecutable",
        &metadata.bundle_name,
    );
    check_plist_string(
        &mut violations,
        &subject,
        plist_path,
        dict,
        "CFBundleName",
        &metadata.bundle_name,
    );
    check_plist_string(
        &mut violations,
        &subject,
        plist_path,
        dict,
        "CFBundleDisplayName",
        &metadata.bundle_name,
    );
    check_plist_string(
        &mut violations,
        &subject,
        plist_path,
        dict,
        "CFBundleIdentifier",
        &primary.plugin_id,
    );
    check_plist_string(
        &mut violations,
        &subject,
        plist_path,
        dict,
        "CFBundleShortVersionString",
        &metadata.version,
    );
    check_plist_string(
        &mut violations,
        &subject,
        plist_path,
        dict,
        "CFBundleVersion",
        &metadata.version,
    );
    check_plist_bool(
        &mut violations,
        &subject,
        plist_path,
        dict,
        "NSHighResolutionCapable",
        true,
    );

    violations
}

fn check_plist_string(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    plist_path: &Path,
    dict: &plist::Dictionary,
    key: &'static str,
    expected: &str,
) {
    let actual = dict.get(key).and_then(plist::Value::as_string);
    if actual != Some(expected) {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: plist_path.to_path_buf(),
            rule_id: RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
            message: format!(
                "CLAP Info.plist {key} does not match manifest metadata. expected=\"{expected}\" actual=\"{}\"",
                actual.unwrap_or("<missing or non-string>")
            ),
            fix: "Keep CLAP bundle metadata generated from package.metadata.wrac.",
        });
    }
}

fn check_plist_bool(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    plist_path: &Path,
    dict: &plist::Dictionary,
    key: &'static str,
    expected: bool,
) {
    let actual = dict.get(key).and_then(plist::Value::as_boolean);
    if actual != Some(expected) {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: plist_path.to_path_buf(),
            rule_id: RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
            message: format!(
                "CLAP Info.plist {key} does not match manifest metadata. expected={expected} actual={}",
                actual
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<missing or non-boolean>".to_string())
            ),
            fix: "Keep CLAP bundle metadata generated from package.metadata.wrac.",
        });
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
            params,
        }
    }

    fn metadata() -> PluginMetadata {
        PluginMetadata {
            package_name: "test_plugin".to_string(),
            version: "1.0.0".to_string(),
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
