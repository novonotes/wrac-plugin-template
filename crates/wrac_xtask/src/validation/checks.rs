use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap_sys::ext::audio_ports::CLAP_AUDIO_PORT_IS_MAIN;
use clap_sys::ext::params::{
    CLAP_PARAM_IS_BYPASS, CLAP_PARAM_IS_ENUM, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY,
    CLAP_PARAM_IS_STEPPED,
};
use clap_sys::plugin_features::{
    CLAP_PLUGIN_FEATURE_ANALYZER, CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_INSTRUMENT,
    CLAP_PLUGIN_FEATURE_NOTE_DETECTOR, CLAP_PLUGIN_FEATURE_NOTE_EFFECT,
    CLAP_PLUGIN_FEATURE_SAMPLER, CLAP_PLUGIN_FEATURE_SYNTHESIZER,
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
const RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST: &str =
    "macos-wrapper-info-plists-match-manifest";
const RULE_WRAPPER_TARGETS_REQUIRE_SINGLE_PRODUCT: &str = "wrapper-targets-require-single-product";
const RULE_PARAM_INFO_SHAPE: &str = "param-info-shape";
const RULE_STATE_EXTENSION_REQUIRED: &str = "state-extension-required";
const RULE_AUDIO_PORT_SHAPE: &str = "audio-port-shape";
const RULE_NOTE_PORT_SHAPE: &str = "note-port-shape";
const RULE_FEATURES_MATCH_CAPABILITIES: &str = "features-match-capabilities";
const RULE_GUI_ARTIFACT_SHAPE: &str = "gui-artifact-shape";
const RULE_TEMPLATE_PLACEHOLDERS_RENAMED: &str = "template-placeholders-renamed";
const RULE_SOURCE_METADATA_SINGLE_SOURCE: &str = "source-metadata-single-source";
const RULE_GUI_NATIVE_COMMANDS_MATCH: &str = "gui-native-commands-match";

const KNOWN_RULES: &[&str] = &[
    RULE_FENDER_SINGLE_KNOB,
    RULE_LUNA_VST3_PARAM_ID_MATCH_INDEX,
    RULE_BYPASS_PARAM_SHAPE,
    RULE_PLUGIN_REQUIRES_BYPASS,
    RULE_CLAP_DESCRIPTORS_MATCH_MANIFEST,
    RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
    RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
    RULE_WRAPPER_TARGETS_REQUIRE_SINGLE_PRODUCT,
    RULE_PARAM_INFO_SHAPE,
    RULE_STATE_EXTENSION_REQUIRED,
    RULE_AUDIO_PORT_SHAPE,
    RULE_NOTE_PORT_SHAPE,
    RULE_FEATURES_MATCH_CAPABILITIES,
    RULE_GUI_ARTIFACT_SHAPE,
    RULE_TEMPLATE_PLACEHOLDERS_RENAMED,
    RULE_SOURCE_METADATA_SINGLE_SOURCE,
    RULE_GUI_NATIVE_COMMANDS_MATCH,
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
    pub(crate) platform: Platform,
    pub(crate) targets: &'a [ValidateTarget],
    pub(crate) clap_bundle: &'a Path,
    pub(crate) vst3_bundle: &'a Path,
    pub(crate) au_bundle: &'a Path,
    pub(crate) standalone_artifact: &'a Path,
}

pub(crate) fn evaluate_bundle_checks(input: BundleCheckInputs<'_>) -> Vec<CheckResult> {
    let BundleCheckInputs {
        schemas,
        metadata,
        validation,
        location,
        platform,
        targets,
        clap_bundle,
        vst3_bundle,
        au_bundle,
        standalone_artifact,
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
        let wrapper_metadata_requested = targets
            .iter()
            .any(|target| matches!(target, Target::Vst3 | Target::Au | Target::Standalone));
        push_check_result_for_subject(
            &mut results,
            validation,
            &subject,
            RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            if wrapper_metadata_requested {
                CheckStatus::from_violations(wrapper_info_plist_violations(
                    metadata,
                    targets,
                    vst3_bundle,
                    au_bundle,
                    standalone_artifact,
                ))
            } else {
                CheckStatus::Skipped("No VST3, AU, or standalone validation target was requested.")
            },
        );
    } else {
        push_check_result_for_subject(
            &mut results,
            validation,
            &subject,
            RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
            CheckStatus::Skipped("macOS CLAP bundle metadata is not available on this platform."),
        );
        push_check_result_for_subject(
            &mut results,
            validation,
            &subject,
            RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            CheckStatus::Skipped(
                "macOS wrapper bundle metadata is not available on this platform.",
            ),
        );
    }

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

    push_check_result(
        &mut results,
        validation,
        schema,
        RULE_AUDIO_PORT_SHAPE,
        CheckStatus::from_violations(audio_port_shape_violations(schema, location)),
    );

    push_check_result(
        &mut results,
        validation,
        schema,
        RULE_NOTE_PORT_SHAPE,
        CheckStatus::from_violations(note_port_shape_violations(schema, location)),
    );

    push_check_result(
        &mut results,
        validation,
        schema,
        RULE_FEATURES_MATCH_CAPABILITIES,
        CheckStatus::from_violations(feature_capability_violations(schema, location)),
    );

    results
}

pub(crate) fn evaluate_source_checks(
    schemas: &[PluginSchema],
    metadata: &PluginMetadata,
    validation: &ValidationMetadata,
    location: &Path,
    repository_root: &Path,
    plugin_root: &Path,
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

    push_check_result_for_subject(
        &mut results,
        validation,
        &subject,
        RULE_SOURCE_METADATA_SINGLE_SOURCE,
        CheckStatus::from_violations(source_metadata_single_source_violations(
            metadata,
            location,
            plugin_root,
        )),
    );

    push_check_result_for_subject(
        &mut results,
        validation,
        &subject,
        RULE_GUI_NATIVE_COMMANDS_MATCH,
        CheckStatus::from_violations(gui_native_command_violations(
            metadata,
            location,
            plugin_root,
        )),
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

fn audio_port_shape_violations(schema: &PluginSchema, location: &Path) -> Vec<RuleViolation> {
    let mut violations = Vec::new();
    let has_audio_input = !schema.audio_inputs.is_empty();
    let has_audio_output = !schema.audio_outputs.is_empty();
    let is_audio_effect = has_feature(schema, CLAP_PLUGIN_FEATURE_AUDIO_EFFECT.to_str().unwrap());
    let is_note_only = has_feature(schema, CLAP_PLUGIN_FEATURE_NOTE_EFFECT.to_str().unwrap())
        || has_feature(schema, CLAP_PLUGIN_FEATURE_NOTE_DETECTOR.to_str().unwrap());
    let is_generator = has_feature(schema, CLAP_PLUGIN_FEATURE_INSTRUMENT.to_str().unwrap())
        || has_feature(schema, CLAP_PLUGIN_FEATURE_SYNTHESIZER.to_str().unwrap())
        || has_feature(schema, CLAP_PLUGIN_FEATURE_SAMPLER.to_str().unwrap());
    let is_analyzer = has_feature(schema, CLAP_PLUGIN_FEATURE_ANALYZER.to_str().unwrap());

    if is_audio_effect && (!has_audio_input || !has_audio_output) {
        violations.push(RuleViolation {
            plugin_id: schema.plugin_id.clone(),
            plugin_name: schema.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_AUDIO_PORT_SHAPE,
            message: format!(
                "Audio-effect plugins should expose at least one input and one output audio port. input_count={} output_count={}",
                schema.audio_inputs.len(),
                schema.audio_outputs.len()
            ),
            fix: "Expose one main audio input and one main audio output, or disable this rule with a documented reason for non-audio products.",
        });
    }
    if is_generator && !has_audio_output {
        violations.push(RuleViolation {
            plugin_id: schema.plugin_id.clone(),
            plugin_name: schema.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_AUDIO_PORT_SHAPE,
            message: "Instrument/synth/sampler plugins should expose an audio output port."
                .to_string(),
            fix: "Expose one main audio output, or disable this rule with a documented reason.",
        });
    }
    if !is_note_only && !is_analyzer {
        require_one_main_port(
            &mut violations,
            schema,
            location,
            &schema.audio_outputs,
            "output",
        );
        if is_audio_effect {
            require_one_main_port(
                &mut violations,
                schema,
                location,
                &schema.audio_inputs,
                "input",
            );
        }
    }
    validate_audio_port_list(
        &mut violations,
        schema,
        location,
        &schema.audio_inputs,
        "input",
    );
    validate_audio_port_list(
        &mut violations,
        schema,
        location,
        &schema.audio_outputs,
        "output",
    );

    violations
}

fn validate_audio_port_list(
    violations: &mut Vec<RuleViolation>,
    schema: &PluginSchema,
    location: &Path,
    ports: &[super::clap_schema::AudioPortSchema],
    direction: &'static str,
) {
    let mut seen_ids = HashSet::new();
    for port in ports {
        if !seen_ids.insert(port.id) {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_AUDIO_PORT_SHAPE,
                message: format!(
                    "Audio {direction} port IDs must be unique. duplicate_id={} name=\"{}\"",
                    port.id, port.name
                ),
                fix: "Assign one stable unique ID to each audio port.",
            });
        }
        if port.name.trim().is_empty() {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_AUDIO_PORT_SHAPE,
                message: format!(
                    "Audio {direction} port names must not be empty. id={}",
                    port.id
                ),
                fix: "Expose a non-empty stable name for each audio port.",
            });
        }
        if port.channel_count == 0 {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_AUDIO_PORT_SHAPE,
                message: format!(
                    "Audio {direction} port channel count must be non-zero. id={} name=\"{}\"",
                    port.id, port.name
                ),
                fix: "Expose a concrete channel count for each audio port.",
            });
        }
        if port.port_type.trim().is_empty() {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_AUDIO_PORT_SHAPE,
                message: format!(
                    "Audio {direction} port type should be declared. id={} name=\"{}\"",
                    port.id, port.name
                ),
                fix: "Declare the CLAP port type, such as mono or stereo.",
            });
        }
    }
}

fn require_one_main_port(
    violations: &mut Vec<RuleViolation>,
    schema: &PluginSchema,
    location: &Path,
    ports: &[super::clap_schema::AudioPortSchema],
    direction: &'static str,
) {
    let main_count = ports
        .iter()
        .filter(|port| port.flags.contains(CLAP_AUDIO_PORT_IS_MAIN))
        .count();
    if ports.is_empty() || main_count == 1 {
        return;
    }

    violations.push(RuleViolation {
        plugin_id: schema.plugin_id.clone(),
        plugin_name: schema.plugin_name.clone(),
        location: location.to_path_buf(),
        rule_id: RULE_AUDIO_PORT_SHAPE,
        message: format!(
            "Audio port list should expose exactly one main {direction} port when {direction} ports exist. {direction}_port_count={} main_{direction}_port_count={main_count}",
            ports.len()
        ),
        fix: "Mark one host-facing audio port as main and keep additional ports non-main.",
    });
}

fn feature_capability_violations(schema: &PluginSchema, location: &Path) -> Vec<RuleViolation> {
    let mut violations = Vec::new();
    let is_audio_effect = has_feature(schema, CLAP_PLUGIN_FEATURE_AUDIO_EFFECT.to_str().unwrap());
    let is_note_processor = has_feature(schema, CLAP_PLUGIN_FEATURE_NOTE_EFFECT.to_str().unwrap())
        || has_feature(schema, CLAP_PLUGIN_FEATURE_NOTE_DETECTOR.to_str().unwrap());
    let is_instrument = has_feature(schema, CLAP_PLUGIN_FEATURE_INSTRUMENT.to_str().unwrap())
        || has_feature(schema, CLAP_PLUGIN_FEATURE_SYNTHESIZER.to_str().unwrap())
        || has_feature(schema, CLAP_PLUGIN_FEATURE_SAMPLER.to_str().unwrap());

    if schema.plugin_features.is_empty() {
        violations.push(RuleViolation {
            plugin_id: schema.plugin_id.clone(),
            plugin_name: schema.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_FEATURES_MATCH_CAPABILITIES,
            message: "Plugin descriptor should expose at least one CLAP feature.".to_string(),
            fix: "Declare product features that match the plugin capability, such as audio-effect, instrument, or note-effect.",
        });
    }
    if is_audio_effect && (schema.audio_inputs.is_empty() || schema.audio_outputs.is_empty()) {
        violations.push(RuleViolation {
            plugin_id: schema.plugin_id.clone(),
            plugin_name: schema.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_FEATURES_MATCH_CAPABILITIES,
            message: "The audio-effect feature should match input/output audio ports.".to_string(),
            fix: "Expose audio input/output ports for audio effects, or change the feature list.",
        });
    }
    if is_instrument && schema.audio_outputs.is_empty() {
        violations.push(RuleViolation {
            plugin_id: schema.plugin_id.clone(),
            plugin_name: schema.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_FEATURES_MATCH_CAPABILITIES,
            message: "Instrument/synth/sampler features should match an audio output capability."
                .to_string(),
            fix: "Expose an audio output for generator products, or change the feature list.",
        });
    }
    if is_note_processor && schema.note_inputs.is_empty() && schema.note_outputs.is_empty() {
        violations.push(RuleViolation {
            plugin_id: schema.plugin_id.clone(),
            plugin_name: schema.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_FEATURES_MATCH_CAPABILITIES,
            message: "Note-processing features should match a note input or output capability."
                .to_string(),
            fix: "Expose note ports for note-processing products, or change the feature list.",
        });
    }

    violations
}

fn note_port_shape_violations(schema: &PluginSchema, location: &Path) -> Vec<RuleViolation> {
    let mut violations = Vec::new();
    validate_note_port_list(
        &mut violations,
        schema,
        location,
        &schema.note_inputs,
        "input",
    );
    validate_note_port_list(
        &mut violations,
        schema,
        location,
        &schema.note_outputs,
        "output",
    );
    violations
}

fn validate_note_port_list(
    violations: &mut Vec<RuleViolation>,
    schema: &PluginSchema,
    location: &Path,
    ports: &[super::clap_schema::NotePortSchema],
    direction: &'static str,
) {
    let mut seen_ids = HashSet::new();
    for port in ports {
        if !seen_ids.insert(port.id) {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_NOTE_PORT_SHAPE,
                message: format!(
                    "Note {direction} port IDs must be unique. duplicate_id={} name=\"{}\"",
                    port.id, port.name
                ),
                fix: "Assign one stable unique ID to each note port.",
            });
        }
        if port.name.trim().is_empty() {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_NOTE_PORT_SHAPE,
                message: format!(
                    "Note {direction} port names must not be empty. id={}",
                    port.id
                ),
                fix: "Expose a non-empty stable name for each note port.",
            });
        }
        if port.supported_dialects == 0 {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_NOTE_PORT_SHAPE,
                message: format!(
                    "Note {direction} port supported dialects must be non-empty. id={} name=\"{}\"",
                    port.id, port.name
                ),
                fix: "Declare at least one supported note dialect for each note port.",
            });
        }
        if port.preferred_dialect == 0 || port.preferred_dialect & port.supported_dialects == 0 {
            violations.push(RuleViolation {
                plugin_id: schema.plugin_id.clone(),
                plugin_name: schema.plugin_name.clone(),
                location: location.to_path_buf(),
                rule_id: RULE_NOTE_PORT_SHAPE,
                message: format!(
                    "Note {direction} port preferred dialect must be one of the supported dialects. id={} name=\"{}\" supported={} preferred={}",
                    port.id, port.name, port.supported_dialects, port.preferred_dialect
                ),
                fix: "Set the preferred note dialect to one of the supported dialects.",
            });
        }
    }
}

fn has_feature(schema: &PluginSchema, feature: &str) -> bool {
    schema
        .plugin_features
        .iter()
        .any(|candidate| candidate == feature)
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

fn source_metadata_single_source_violations(
    metadata: &PluginMetadata,
    location: &Path,
    plugin_root: &Path,
) -> Vec<RuleViolation> {
    let subject = CheckSubject::bundle(metadata);
    let mut violations = Vec::new();
    let plugin_rs = read_source_file(
        &mut violations,
        &subject,
        plugin_root.join("src-plugin/src/plugin.rs"),
        RULE_SOURCE_METADATA_SINGLE_SOURCE,
    );
    let build_rs = read_source_file(
        &mut violations,
        &subject,
        plugin_root.join("src-plugin/build.rs"),
        RULE_SOURCE_METADATA_SINGLE_SOURCE,
    );
    let gui_source_exists = plugin_root.join("src-gui").exists();
    let vite_config = gui_source_exists.then(|| {
        read_source_file(
            &mut violations,
            &subject,
            plugin_root.join("src-gui/vite.config.ts"),
            RULE_SOURCE_METADATA_SINGLE_SOURCE,
        )
    });
    let frontend_main = gui_source_exists.then(|| {
        read_source_file(
            &mut violations,
            &subject,
            plugin_root.join("src-gui/src/main.ts"),
            RULE_SOURCE_METADATA_SINGLE_SOURCE,
        )
    });

    if let Some(plugin_rs) = plugin_rs.as_deref() {
        // These token checks protect the template contract rather than trying to parse
        // arbitrary Rust. The artifact checks still verify the actual descriptor values.
        for token in [
            r#"env!("WRAC_PLUGIN_0_ID")"#,
            r#"env!("WRAC_PLUGIN_0_NAME")"#,
            r#"env!("WRAC_COMPANY_NAME")"#,
            r#"env!("WRAC_PLUGIN_0_AUV2_TYPE")"#,
            r#"env!("WRAC_PLUGIN_0_AUV2_SUBTYPE")"#,
            r#"env!("WRAC_AUV2_MANUFACTURER_CODE")"#,
        ] {
            require_source_contains(
                &mut violations,
                &subject,
                location,
                SourceTokenCheck {
                    file_label: "src-plugin/src/plugin.rs",
                    source: plugin_rs,
                    token,
                    rule_id: RULE_SOURCE_METADATA_SINGLE_SOURCE,
                    fix: "Read descriptor identity from build-script env vars generated from package.metadata.wrac.",
                },
            );
        }
    }
    if let Some(build_rs) = build_rs.as_deref() {
        for token in [
            "cargo:rustc-env=WRAC_PLUGIN_{index}_ID",
            "cargo:rustc-env=WRAC_PLUGIN_{index}_NAME",
            "cargo:rustc-env=WRAC_PLUGIN_{index}_AUV2_TYPE",
            "cargo:rustc-env=WRAC_PLUGIN_{index}_AUV2_SUBTYPE",
            "cargo:rustc-env=WRAC_COMPANY_NAME",
            "cargo:rustc-env=WRAC_AUV2_MANUFACTURER_CODE",
        ] {
            require_source_contains(
                &mut violations,
                &subject,
                location,
                SourceTokenCheck {
                    file_label: "src-plugin/build.rs",
                    source: build_rs,
                    token,
                    rule_id: RULE_SOURCE_METADATA_SINGLE_SOURCE,
                    fix: "Emit descriptor identity env vars from package.metadata.wrac in build.rs.",
                },
            );
        }
    }
    if let Some(vite_config) = vite_config.as_ref().and_then(Option::as_deref) {
        for token in [
            "../src-plugin/Cargo.toml",
            "__WRAC_PLUGIN_METADATA__",
            "package.metadata.wrac.company_name",
            "package.metadata.wrac.plugins[0].plugin_id",
            "package.metadata.wrac.plugins[0].plugin_name",
        ] {
            require_source_contains(
                &mut violations,
                &subject,
                location,
                SourceTokenCheck {
                    file_label: "src-gui/vite.config.ts",
                    source: vite_config,
                    token,
                    rule_id: RULE_SOURCE_METADATA_SINGLE_SOURCE,
                    fix: "Inject frontend identity from src-plugin/Cargo.toml instead of hard-coding it.",
                },
            );
        }
    }
    if let Some(frontend_main) = frontend_main.as_ref().and_then(Option::as_deref) {
        require_source_contains(
            &mut violations,
            &subject,
            location,
            SourceTokenCheck {
                file_label: "src-gui/src/main.ts",
                source: frontend_main,
                token: "__WRAC_PLUGIN_METADATA__",
                rule_id: RULE_SOURCE_METADATA_SINGLE_SOURCE,
                fix: "Render frontend identity from metadata injected by Vite.",
            },
        );
    }

    violations
}

fn gui_native_command_violations(
    metadata: &PluginMetadata,
    location: &Path,
    plugin_root: &Path,
) -> Vec<RuleViolation> {
    let subject = CheckSubject::bundle(metadata);
    let mut violations = Vec::new();
    let rust_sources = read_files_with_extension(&plugin_root.join("src-plugin/src"), "rs");
    let mut ts_sources = read_files_with_extension(&plugin_root.join("src-gui/src"), "ts");
    ts_sources.extend(read_files_with_extension(
        &plugin_root.join("src-gui/src"),
        "tsx",
    ));
    let registered = rust_sources
        .iter()
        .flat_map(|source| extract_literal_calls(source, "register_sync("))
        .collect::<HashSet<_>>();
    let invoked = ts_sources
        .iter()
        .flat_map(|source| extract_invoke_literal_calls(source))
        .collect::<HashSet<_>>();

    // Enforce the call path that fails at runtime: a literal frontend invoke must exist
    // on the Rust side. Rust may register optional or page-specific commands that are
    // not invoked by the current frontend bundle, so the reverse direction is review-only.
    for command in invoked.difference(&registered) {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: RULE_GUI_NATIVE_COMMANDS_MATCH,
            message: format!(
                "TypeScript invoke command is not registered by Rust register_sync. command=\"{command}\""
            ),
            fix: "Register the command on the Rust side, rename the TypeScript invoke, or disable this rule with a documented reason for dynamic command routing.",
        });
    }

    violations
}

fn read_source_file(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    path: PathBuf,
    rule_id: &'static str,
) -> Option<String> {
    match fs::read_to_string(&path) {
        Ok(source) => Some(source),
        Err(error) => {
            violations.push(RuleViolation {
                plugin_id: subject.plugin_id.clone(),
                plugin_name: subject.plugin_name.clone(),
                location: path,
                rule_id,
                message: format!("Failed to read source file for production-readiness check: {error}"),
                fix: "Keep the template source files in their expected locations, or disable this rule with a documented reason for custom project layouts.",
            });
            None
        }
    }
}

fn require_source_contains(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    location: &Path,
    check: SourceTokenCheck<'_>,
) {
    if !check.source.contains(check.token) {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: location.to_path_buf(),
            rule_id: check.rule_id,
            message: format!("{} must contain `{}`.", check.file_label, check.token),
            fix: check.fix,
        });
    }
}

struct SourceTokenCheck<'a> {
    file_label: &'static str,
    source: &'a str,
    token: &'static str,
    rule_id: &'static str,
    fix: &'static str,
}

fn read_files_with_extension(root: &Path, extension: &str) -> Vec<String> {
    let mut sources = Vec::new();
    collect_files_with_extension(root, extension, &mut sources);
    sources
}

fn collect_files_with_extension(root: &Path, extension: &str, sources: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_extension(&path, extension, sources);
        } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            let Ok(source) = fs::read_to_string(path) else {
                continue;
            };
            sources.push(source);
        }
    }
}

fn extract_literal_calls(source: &str, call: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = source;
    while let Some(index) = rest.find(call) {
        rest = &rest[index + call.len()..];
        let Some(stripped) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = stripped.find('"') else {
            continue;
        };
        values.push(stripped[..end].to_string());
        rest = &stripped[end + 1..];
    }
    values
}

fn extract_invoke_literal_calls(source: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = source;
    while let Some(index) = rest.find("invoke") {
        rest = &rest[index + "invoke".len()..];
        if let Some(after_generics) = rest.strip_prefix('<') {
            let Some(end) = after_generics.find('>') else {
                continue;
            };
            rest = &after_generics[end + 1..];
        }
        let Some(after_paren) = rest.strip_prefix('(') else {
            continue;
        };
        let Some(stripped) = after_paren.strip_prefix('"') else {
            continue;
        };
        let Some(end) = stripped.find('"') else {
            continue;
        };
        values.push(stripped[..end].to_string());
        rest = &stripped[end + 1..];
    }
    values
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
    check_bundle_executable_exists(
        &mut violations,
        &subject,
        plist_path,
        &metadata.bundle_name,
        RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
        "Keep the CLAP bundle executable name and Contents/MacOS binary in sync.",
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

fn wrapper_info_plist_violations(
    metadata: &PluginMetadata,
    targets: &[ValidateTarget],
    vst3_bundle: &Path,
    au_bundle: &Path,
    standalone_artifact: &Path,
) -> Vec<RuleViolation> {
    let subject = CheckSubject::bundle(metadata);
    let mut violations = Vec::new();

    if targets.contains(&Target::Vst3) {
        check_vst3_info_plist(
            &mut violations,
            &subject,
            metadata,
            &vst3_bundle.join("Contents").join("Info.plist"),
        );
    }
    if targets.contains(&Target::Au) {
        check_au_info_plist(
            &mut violations,
            &subject,
            metadata,
            &au_bundle.join("Contents").join("Info.plist"),
        );
    }
    if targets.contains(&Target::Standalone) {
        check_standalone_info_plist(
            &mut violations,
            &subject,
            metadata,
            &standalone_artifact.join("Contents").join("Info.plist"),
        );
    }

    violations
}

fn check_vst3_info_plist(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    metadata: &PluginMetadata,
    plist_path: &Path,
) {
    let Some(dict) = read_plist_dict(violations, subject, plist_path, "VST3 Info.plist") else {
        return;
    };
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "VST3 Info.plist",
            key: "CFBundleExecutable",
            expected: &metadata.bundle_name,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep VST3 bundle metadata generated from package.metadata.wrac.",
        },
    );
    check_bundle_executable_exists(
        violations,
        subject,
        plist_path,
        &metadata.bundle_name,
        RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
        "Keep the VST3 bundle executable name and Contents/MacOS binary in sync.",
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "VST3 Info.plist",
            key: "CFBundleName",
            expected: &metadata.bundle_name,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep VST3 bundle metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "VST3 Info.plist",
            key: "CFBundleShortVersionString",
            expected: &metadata.version,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep VST3 bundle metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "VST3 Info.plist",
            key: "CFBundleVersion",
            expected: &metadata.version,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep VST3 bundle metadata generated from package.metadata.wrac.",
        },
    );
}

fn check_au_info_plist(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    metadata: &PluginMetadata,
    plist_path: &Path,
) {
    let Some(dict) = read_plist_dict(violations, subject, plist_path, "AU Info.plist") else {
        return;
    };
    let primary = metadata.primary_plugin();
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "AU Info.plist",
            key: "CFBundleExecutable",
            expected: &metadata.bundle_name,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AU bundle metadata generated from package.metadata.wrac.",
        },
    );
    check_bundle_executable_exists(
        violations,
        subject,
        plist_path,
        &metadata.bundle_name,
        RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
        "Keep the AU bundle executable name and Contents/MacOS binary in sync.",
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "AU Info.plist",
            key: "CFBundleName",
            expected: &metadata.bundle_name,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AU bundle metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "AU Info.plist",
            key: "CFBundleShortVersionString",
            expected: &metadata.version,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AU bundle metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "AU Info.plist",
            key: "CFBundleVersion",
            expected: &metadata.version,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AU bundle metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_bool_for_rule(
        violations,
        subject,
        plist_path,
        PlistBoolCheck {
            dict: &dict,
            section: "AU Info.plist",
            key: "NSHighResolutionCapable",
            expected: true,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AU bundle metadata generated from package.metadata.wrac.",
        },
    );

    let components = dict.get("AudioComponents").and_then(plist::Value::as_array);
    let Some(component) = components
        .and_then(|items| items.first())
        .and_then(plist::Value::as_dictionary)
    else {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: plist_path.to_path_buf(),
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            message: "AU Info.plist AudioComponents[0] must be present.".to_string(),
            fix: "Regenerate the AU bundle metadata from package.metadata.wrac.",
        });
        return;
    };
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: component,
            section: "AU AudioComponents[0]",
            key: "manufacturer",
            expected: &metadata.auv2_manufacturer_code,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AUv2 manufacturer metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: component,
            section: "AU AudioComponents[0]",
            key: "type",
            expected: &primary.auv2_type,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AUv2 type metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: component,
            section: "AU AudioComponents[0]",
            key: "subtype",
            expected: &primary.auv2_subtype,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AUv2 subtype metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: component,
            section: "AU AudioComponents[0]",
            key: "name",
            expected: &format!("{}: {}", metadata.company_name, primary.plugin_name),
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AUv2 display metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_integer_for_rule(
        violations,
        subject,
        plist_path,
        PlistIntegerCheck {
            dict: component,
            section: "AU AudioComponents[0]",
            key: "version",
            expected: auv2_encoded_version(&metadata.version),
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep AUv2 version metadata generated from the package version.",
        },
    );
}

fn check_standalone_info_plist(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    metadata: &PluginMetadata,
    plist_path: &Path,
) {
    let Some(dict) = read_plist_dict(violations, subject, plist_path, "Standalone Info.plist")
    else {
        return;
    };
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "Standalone Info.plist",
            key: "CFBundleExecutable",
            expected: &metadata.standalone_name,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep standalone app metadata generated from package.metadata.wrac.",
        },
    );
    check_bundle_executable_exists(
        violations,
        subject,
        plist_path,
        &metadata.standalone_name,
        RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
        "Keep the standalone app executable name and Contents/MacOS binary in sync.",
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "Standalone Info.plist",
            key: "CFBundleName",
            expected: &metadata.standalone_name,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep standalone app metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "Standalone Info.plist",
            key: "CFBundleShortVersionString",
            expected: &metadata.version,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep standalone app metadata generated from package.metadata.wrac.",
        },
    );
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict: &dict,
            section: "Standalone Info.plist",
            key: "CFBundleVersion",
            expected: &metadata.version,
            rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
            fix: "Keep standalone app metadata generated from package.metadata.wrac.",
        },
    );
}

fn check_plist_string(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    plist_path: &Path,
    dict: &plist::Dictionary,
    key: &'static str,
    expected: &str,
) {
    check_plist_string_for_rule(
        violations,
        subject,
        plist_path,
        PlistStringCheck {
            dict,
            section: "CLAP Info.plist",
            key,
            expected,
            rule_id: RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
            fix: "Keep CLAP bundle metadata generated from package.metadata.wrac.",
        },
    );
}

fn check_plist_string_for_rule(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    plist_path: &Path,
    check: PlistStringCheck<'_>,
) {
    let actual = check.dict.get(check.key).and_then(plist::Value::as_string);
    if actual != Some(check.expected) {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: plist_path.to_path_buf(),
            rule_id: check.rule_id,
            message: format!(
                "{} {} does not match manifest metadata. expected=\"{}\" actual=\"{}\"",
                check.section,
                check.key,
                check.expected,
                actual.unwrap_or("<missing or non-string>")
            ),
            fix: check.fix,
        });
    }
}

struct PlistStringCheck<'a> {
    dict: &'a plist::Dictionary,
    section: &'static str,
    key: &'static str,
    expected: &'a str,
    rule_id: &'static str,
    fix: &'static str,
}

fn check_plist_bool(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    plist_path: &Path,
    dict: &plist::Dictionary,
    key: &'static str,
    expected: bool,
) {
    check_plist_bool_for_rule(
        violations,
        subject,
        plist_path,
        PlistBoolCheck {
            dict,
            section: "CLAP Info.plist",
            key,
            expected,
            rule_id: RULE_MACOS_CLAP_INFO_PLIST_MATCHES_MANIFEST,
            fix: "Keep CLAP bundle metadata generated from package.metadata.wrac.",
        },
    );
}

fn check_plist_bool_for_rule(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    plist_path: &Path,
    check: PlistBoolCheck<'_>,
) {
    let actual = check.dict.get(check.key).and_then(plist::Value::as_boolean);
    if actual != Some(check.expected) {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: plist_path.to_path_buf(),
            rule_id: check.rule_id,
            message: format!(
                "{} {} does not match manifest metadata. expected={} actual={}",
                check.section,
                check.key,
                check.expected,
                actual
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<missing or non-boolean>".to_string())
            ),
            fix: check.fix,
        });
    }
}

struct PlistBoolCheck<'a> {
    dict: &'a plist::Dictionary,
    section: &'static str,
    key: &'static str,
    expected: bool,
    rule_id: &'static str,
    fix: &'static str,
}

fn check_bundle_executable_exists(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    plist_path: &Path,
    executable_name: &str,
    rule_id: &'static str,
    fix: &'static str,
) {
    let Some(contents_dir) = plist_path.parent() else {
        return;
    };
    let executable_path = contents_dir.join("MacOS").join(executable_name);
    if !executable_path.exists() {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: executable_path,
            rule_id,
            message: format!(
                "Bundle executable declared by Info.plist does not exist. executable=\"{executable_name}\""
            ),
            fix,
        });
    }
}

fn check_plist_integer_for_rule(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    plist_path: &Path,
    check: PlistIntegerCheck<'_>,
) {
    let actual = check
        .dict
        .get(check.key)
        .and_then(plist::Value::as_signed_integer);
    if actual != Some(check.expected) {
        violations.push(RuleViolation {
            plugin_id: subject.plugin_id.clone(),
            plugin_name: subject.plugin_name.clone(),
            location: plist_path.to_path_buf(),
            rule_id: check.rule_id,
            message: format!(
                "{} {} does not match manifest metadata. expected={} actual={}",
                check.section,
                check.key,
                check.expected,
                actual
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<missing or non-integer>".to_string())
            ),
            fix: check.fix,
        });
    }
}

struct PlistIntegerCheck<'a> {
    dict: &'a plist::Dictionary,
    section: &'static str,
    key: &'static str,
    expected: i64,
    rule_id: &'static str,
    fix: &'static str,
}

fn read_plist_dict(
    violations: &mut Vec<RuleViolation>,
    subject: &CheckSubject,
    plist_path: &Path,
    section: &'static str,
) -> Option<plist::Dictionary> {
    match plist::Value::from_file(plist_path) {
        Ok(value) => match value.into_dictionary() {
            Some(dict) => Some(dict),
            None => {
                violations.push(RuleViolation {
                    plugin_id: subject.plugin_id.clone(),
                    plugin_name: subject.plugin_name.clone(),
                    location: plist_path.to_path_buf(),
                    rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
                    message: format!("{section} must be a dictionary."),
                    fix: "Regenerate wrapper bundle metadata from package.metadata.wrac.",
                });
                None
            }
        },
        Err(error) => {
            violations.push(RuleViolation {
                plugin_id: subject.plugin_id.clone(),
                plugin_name: subject.plugin_name.clone(),
                location: plist_path.to_path_buf(),
                rule_id: RULE_MACOS_WRAPPER_INFO_PLISTS_MATCH_MANIFEST,
                message: format!("Failed to read {section}: {error}"),
                fix: "Build the requested wrapper artifact and keep Contents/Info.plist generated from package.metadata.wrac.",
            });
            None
        }
    }
}

fn auv2_encoded_version(version: &str) -> i64 {
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<i64>().unwrap_or_default());
    let major = parts.next().unwrap_or_default().clamp(0, 255);
    let minor = parts.next().unwrap_or_default().clamp(0, 255);
    let patch = parts.next().unwrap_or_default().clamp(0, 255);
    (major << 16) | (minor << 8) | patch
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

    use super::super::clap_schema::{
        AudioPortSchema, NotePortSchema, ParameterSchema, PluginSchema,
    };
    use super::*;

    fn schema(params: Vec<ParameterSchema>) -> PluginSchema {
        PluginSchema {
            plugin_id: "com.example.test".to_string(),
            plugin_name: "Test Plugin".to_string(),
            plugin_vendor: "Example".to_string(),
            plugin_version: "1.0.0".to_string(),
            plugin_features: vec!["audio-effect".to_string(), "stereo".to_string()],
            has_gui: true,
            has_state: true,
            audio_inputs: vec![main_stereo_port(0, "Input")],
            audio_outputs: vec![main_stereo_port(1, "Output")],
            note_inputs: Vec::new(),
            note_outputs: Vec::new(),
            params,
        }
    }

    fn main_stereo_port(id: u32, name: &str) -> AudioPortSchema {
        AudioPortSchema {
            id,
            name: name.to_string(),
            flags: CLAP_AUDIO_PORT_IS_MAIN,
            channel_count: 2,
            port_type: "stereo".to_string(),
        }
    }

    fn note_port(id: u32, name: &str) -> NotePortSchema {
        NotePortSchema {
            id,
            name: name.to_string(),
            supported_dialects: 1,
            preferred_dialect: 1,
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
    fn audio_effect_requires_input_and_output_audio_ports() {
        let mut schema = schema(vec![valid_bypass_param(0)]);
        schema.audio_outputs = Vec::new();
        let results = evaluate_checks(
            &schema,
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_AUDIO_PORT_SHAPE));
        assert!(rule_failed(&results, RULE_FEATURES_MATCH_CAPABILITIES));
    }

    #[test]
    fn audio_ports_reject_multiple_main_outputs() {
        let mut schema = schema(vec![valid_bypass_param(0)]);
        schema.audio_outputs = vec![
            main_stereo_port(1, "Output 1"),
            main_stereo_port(2, "Output 2"),
        ];
        let results = evaluate_checks(
            &schema,
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_AUDIO_PORT_SHAPE));
    }

    #[test]
    fn descriptor_features_are_required() {
        let mut schema = schema(vec![valid_bypass_param(0)]);
        schema.plugin_features = Vec::new();
        let results = evaluate_checks(
            &schema,
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_FEATURES_MATCH_CAPABILITIES));
    }

    #[test]
    fn note_ports_require_preferred_dialect_to_be_supported() {
        let mut schema = schema(vec![valid_bypass_param(0)]);
        let mut notes = note_port(0, "Notes");
        notes.preferred_dialect = 2;
        schema.note_inputs = vec![notes];
        let results = evaluate_checks(
            &schema,
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_NOTE_PORT_SHAPE));
    }

    #[test]
    fn note_processing_features_require_note_ports() {
        let mut schema = schema(vec![valid_bypass_param(0)]);
        schema.plugin_features = vec!["note-effect".to_string()];
        let results = evaluate_checks(
            &schema,
            &[ValidateTarget::Clap],
            &no_disabled_rules(),
            Path::new("Cargo.toml"),
        );
        assert!(rule_failed(&results, RULE_FEATURES_MATCH_CAPABILITIES));
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
    fn literal_call_extraction_reads_double_quoted_calls() {
        let values = extract_literal_calls(
            r#"invoke("set_parameter_value", {}); invoke(dynamicName); invoke("write_to_log");"#,
            "invoke(",
        );

        assert_eq!(values, vec!["set_parameter_value", "write_to_log"]);
    }

    #[test]
    fn invoke_extraction_reads_generic_invocations() {
        let values = extract_invoke_literal_calls(
            r#"invoke<State>("get_parameter_state", {}); invoke("write_to_log"); invoke(dynamicName);"#,
        );

        assert_eq!(values, vec!["get_parameter_state", "write_to_log"]);
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
