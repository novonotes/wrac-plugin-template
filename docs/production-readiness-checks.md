# Production-Readiness Checks

> 日本語版: [production-readiness-checks-ja.md](production-readiness-checks-ja.md)

Production-readiness checks are WRAC-specific checks run by `cargo xtask validate`. Violations are errors and return a non-zero exit code.

These checks are opinionated NovoNotes release-policy checks for commercial plugins, not format-spec validators. Keep the rule list small. Add new rules only for observed host-compatibility or support problems, and only when preventing recurrence is difficult through other means.

## Disabling Rules

Rules can be disabled by rule ID in the plugin crate manifest. Every disabled rule must include a non-empty `reason`.

```toml
[package.metadata.wrac.validation.disabled_rules.fender-studio-pro-generic-editor-single-knob]
reason = "This product does not support Fender Studio Pro generic editor workflows."
```

## Adding Rules

When adding a rule, complete the following:

- **Justification:** Confirm that the rule covers a problem that has actually happened. Do not add rules for hypothetical risks.
- **Avoid duplication:** Do not duplicate problems that are already detected by other validators. Add a new rule only when the observed problem reproduces and `cargo xtask validate` still passes.
- **Document:** Add the expectation, reason, error condition, and fix to this document's Check List.
- **Manually Validate (Mandatory):** Unit tests alone are insufficient. You must:
  - Intentionally break a real template plugin and verify `cargo xtask validate` fails with the expected rule ID and message.
  - Disable the rule and verify validation passes. If it does not pass, another validator is likely detecting the same issue, so do not add the rule.
  - Restore the plugin, enable the rule, and verify the check passes.

## Check List

### `fender-studio-pro-generic-editor-single-knob`

**Expectation:** Products that support the Fender Studio Pro generic editor expose either zero non-bypass parameters or at least two non-bypass parameters.

**Reason:** Fender Studio Pro 8.0.3 generic editors render no knobs when the parameter shape results in exactly one knob. Users experience this as a plugin bug, so products should avoid it. Bypass parameters are not shown as knobs, so they do not count for this rule.

**Error condition:** When CLAP or VST3 validation is requested, the plugin exposes exactly one non-bypass parameter.

**Fix:** Expose either zero non-bypass parameters or at least two non-bypass parameters.

### `luna-vst3-param-id-must-match-index`

**Expectation:** Products that support LUNA VST3 keep parameter IDs equal to their parameter-list indices.

**Reason:** LUNA 2.0.3.4381 VST3 automation writes can fail when a VST3 parameter ID differs from its parameter-list index.

**Error condition:** When validation includes the VST3 target, a public parameter ID differs from its parameter-list index.

**Fix:** Before release, reorder parameters or adjust parameter IDs so each parameter ID matches its index. After release, do not change parameter IDs; disabling this rule is recommended instead.

### `bypass-param-shape`

**Expectation:** Plugins expose at most one bypass parameter, and that parameter behaves as a boolean host bypass control.

**Reason:** Host applications may provide dedicated UI for bypass parameters. To make that UI behave as users expect, the bypass parameter should satisfy this parameter shape.

**Error conditions:**

- More than one bypass parameter is exposed.
- A bypass parameter is not a stepped enum.
- A bypass parameter range is not `0..1`.
- A bypass parameter default is not `0` or `1`.

**Fix:** Expose a single bypass parameter with bypass, stepped, and enum flags, range `0..1`, and default `0` or `1`.

### `plugin-requires-bypass`

**Expectation:** Plugins expose one bypass parameter.

**Reason:** Bypass parameters have low implementation cost and reduce host-specific compatibility risk across plugin categories.

**Error condition:** The plugin does not expose a bypass parameter.

**Fix:** Add one bypass parameter. If the product intentionally does not provide bypass, disable the rule with a documented reason.

### `template-placeholders-renamed`

**Expectation:** Template placeholder names, IDs, and URLs are replaced with product-specific values.

**Reason:** Template-derived values must be replaced manually when creating a product, so they are easy to miss.

**Error condition:** Manifest metadata still contains template placeholders such as `Your Company`, `YrCo`, `com.your-company`, `example.com`, `WRAC Gain`, `wrac_gain_plugin`, `WtGn`, `WtGM`, `WtGS`, the template VST3 component UUID, or the template repository URL. This rule is skipped in the template repository itself.

**Fix:** Replace template metadata with product-specific metadata, or disable the rule with a documented reason when the repository is intentionally a template or example.
