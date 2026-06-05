# Validation Rules

`cargo xtask validate` builds the requested plugin formats, runs WRAC validation rules, and then runs external format validators such as clap-validator, Steinberg's VST3 validator, and auval.

WRAC validation rule violations are errors. Any violation returns a non-zero exit code.

## Disabling Rules

Rules can be disabled in the plugin crate manifest. Every disabled rule must include a non-empty `reason`.

```toml
[package.metadata.wrac.validation.disabled_rules.fender-studio-pro-generic-editor-single-knob]
reason = "This product does not support Fender Studio Pro generic editor workflows."
```

Unknown rule IDs and empty reasons are errors.

Disable rules only for intentional product decisions. If the plugin is expected to work in the affected host or format, fix the plugin instead.

## Rule List

### `fender-studio-pro-generic-editor-single-knob`

**Targets:** CLAP and VST3 validation.

**Error condition:** The plugin exposes exactly one visible, non-bypass parameter.

Fender Studio Pro generic editors fail to render knobs for this shape. A plugin with no visible non-bypass parameters is allowed, and a plugin with two or more visible non-bypass parameters is allowed. Bypass parameters do not count as knobs for this rule.

**Fix:** Expose zero or at least two visible non-bypass parameters, or disable the rule with a documented reason when the product intentionally does not support Fender Studio Pro generic editor workflows.

### `luna-vst3-param-id-must-match-index`

**Targets:** VST3 validation.

**Error condition:** A public parameter's VST3 `ParamID` differs from its parameter-list index.

LUNA 2.0.3.4381 VST3 automation writes can fail when `ParamID` and parameter index differ.

**Fix:** Keep public VST3 parameter IDs equal to their parameter-list indices.

### `bypass-param-shape`

**Targets:** All validation targets.

**Error conditions:**

- More than one bypass parameter is exposed.
- A bypass parameter is not a stepped enum.
- A bypass parameter range is not `0..1`.
- A bypass parameter default is not `0` or `1`.

Host bypass controls expect one boolean-shaped bypass parameter.

**Fix:** Expose a single bypass parameter with bypass, stepped, and enum flags, range `0..1`, and default `0` or `1`.

### `effect-plugin-without-bypass`

**Targets:** All validation targets.

**Error condition:** An audio-effect plugin exposes parameters but does not expose a bypass parameter.

Effect plugins with user-facing parameters should provide a host bypass parameter so host-level bypass and automation behave consistently.

**Fix:** Add one bypass parameter, or disable the rule with a documented reason.
