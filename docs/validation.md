# Validation Rules

`cargo xtask validate` builds the requested plugin formats, runs WRAC validation rules, and then runs external format validators such as clap-validator, Steinberg's VST3 validator, and auval. WRAC rule violations are errors and return a non-zero exit code.

WRAC rules are NovoNotes-specific commercial plugin compatibility checks, not format-spec validators. A plugin can pass CLAP, VST3, and AU format validation and still fail WRAC validation when NovoNotes considers it risky for supported host workflows.

When a rule needs plugin metadata, WRAC currently reads it from the built CLAP artifact. Format-specific rules are enabled from the requested validation formats; for example, a VST3 compatibility rule can still read the WRAC/CLAP parameter schema and does not necessarily parse the VST3 binary directly.

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

**Expectation:** Production plugins that support Fender Studio Pro generic editor workflows expose either no visible non-bypass parameters or at least two visible non-bypass parameters.

**Reason:** Fender Studio Pro generic editors fail to render knobs for this shape. Bypass parameters do not count as knobs for this rule.

**Error condition:** When CLAP or VST3 validation is requested, the plugin exposes exactly one visible, non-bypass parameter.

**Fix:** Expose zero or at least two visible non-bypass parameters, or disable the rule with a documented reason when the product intentionally does not support Fender Studio Pro generic editor workflows.

### `luna-vst3-param-id-must-match-index`

**Expectation:** VST3-compatible plugins keep public parameter IDs equal to their parameter-list indices.

**Reason:** WRAC maps public parameter IDs to VST3 `ParamID`s. LUNA 2.0.3.4381 VST3 automation writes can fail when a VST3 `ParamID` differs from its parameter-list index.

**Error condition:** When VST3 validation is requested, a public parameter ID differs from its parameter-list index.

**Fix:** Reorder parameters or adjust public parameter IDs so each public parameter ID matches its index.

### `bypass-param-shape`

**Expectation:** Plugins expose at most one bypass parameter, and that parameter behaves as a boolean host bypass control.

**Reason:** Host bypass controls expect one boolean-shaped bypass parameter. External validators catch some invalid bypass shapes, but they do not fully enforce WRAC's required bypass shape.

**Error conditions:**

- More than one bypass parameter is exposed.
- A bypass parameter is not a stepped enum.
- A bypass parameter range is not `0..1`.
- A bypass parameter default is not `0` or `1`.

**Fix:** Expose a single bypass parameter with bypass, stepped, and enum flags, range `0..1`, and default `0` or `1`.

### `effect-plugin-without-bypass`

**Expectation:** Audio-effect plugins with user-facing parameters expose one valid bypass parameter.

**Reason:** Effect plugins with user-facing parameters need host bypass support so host-level bypass and automation behave consistently.

**Error condition:** An audio-effect plugin exposes parameters but does not expose a bypass parameter.

**Fix:** Add one bypass parameter, or disable the rule with a documented reason.
