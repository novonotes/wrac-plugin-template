# Production-Readiness Checks

Production-readiness checks run inside `cargo xtask validate`. Rule violations are errors, and the command returns a non-zero exit code.

These checks are NovoNotes-specific release-policy checks for commercial plugins. They are not format-spec validators. Keep this check set small: rules should be limited to failure modes that have already caused real problems, or risks that product implementors are clearly likely to miss and that directly affect releases.

## Disabling Rules

Checks can be disabled by rule ID in the plugin crate manifest. Every disabled rule must include a non-empty `reason`.

```toml
[package.metadata.wrac.validation.disabled_rules.fender-studio-pro-generic-editor-single-knob]
reason = "This product does not support Fender Studio Pro generic editor workflows."
```

## Adding Rules

Adding a new rule is a release-policy change, not just a code change. Before opening a PR, authors must complete the following:

- **Justification:** Confirm that the rule covers a problem that has actually happened, or a mistake that product implementors are clearly likely to make and that directly affects releases.
- **Avoid duplication:** Do not duplicate checks that are already covered by other validators. Add a new rule only when the problem reproduces but `cargo xtask validate` still passes.
- **Document:** Add the new rule to this document's rule list.
- **Manual validation required:** Unit tests are not enough.
  - Intentionally break a real template plugin and confirm that `cargo xtask validate` fails with the expected rule ID and message.
  - Restore the plugin and confirm that the check passes.

## Rule List

### `fender-studio-pro-generic-editor-single-knob`

**Expectation:** Production plugins that support Fender Studio Pro generic editor workflows should expose either zero visible non-bypass parameters or at least two visible non-bypass parameters.

**Reason:** Fender Studio Pro 8.0.3 generic editors do not render knobs for this shape. Bypass parameters do not count toward the knob count for this rule.

**Error condition:** When CLAP or VST3 validation is requested, the plugin exposes exactly one visible non-bypass parameter.

**Fix:** Expose zero or at least two visible non-bypass parameters.

### `luna-vst3-param-id-must-match-index`

**Expectation:** VST3-compatible plugins should keep public parameter IDs equal to their parameter-list indices.

**Reason:** LUNA 2.0.3.4381 can fail to write VST3 automation when a VST3 parameter ID differs from its parameter-list index.

**Error condition:** A public parameter ID differs from its parameter-list index.

**Fix:** Reorder parameters or adjust public parameter IDs so each public parameter ID matches its index.

### `param-info-shape`

**Expectation:** Public parameters should have stable, host-safe identity and value metadata.

**Reason:** Parameter IDs, names, ranges, and defaults are product code. Small mistakes can break automation, generic editors, control surfaces, or project recall.

**Error conditions:**

- A public parameter ID is duplicated.
- A public parameter name is empty.
- A public parameter min, max, or default is not finite.
- A public parameter min is greater than or equal to max.
- A public parameter default is outside its declared range.

**Fix:** Give every public parameter a stable unique ID, a non-empty name, a finite `min < max` range, and a finite default inside the range.

### `bypass-param-shape`

**Expectation:** Plugins should expose at most one bypass parameter, and that parameter should behave as a boolean host bypass control.

**Reason:** Bypass is cheap to implement but visible to hosts, automation, generic editors, and control surfaces. Shape mistakes are easy to make when adding the parameter manually.

**Error conditions:**

- More than one bypass parameter is exposed.
- A bypass parameter is not a stepped enum.
- A bypass parameter range is not `0..1`.
- A bypass parameter default is not `0` or `1`.

**Fix:** Expose one bypass parameter with bypass, stepped, and enum flags, range `0..1`, and default `0` or `1`.

### `plugin-requires-bypass`

**Expectation:** Production plugins should expose one valid bypass parameter.

**Reason:** A valid bypass parameter has low implementation cost and reduces host-specific compatibility risk across plugin categories.

**Error condition:** The plugin does not expose a bypass parameter.

**Fix:** Add one bypass parameter. If the product intentionally does not provide host bypass, disable the rule with a documented reason.

### `template-placeholders-renamed`

**Expectation:** Template placeholder names, IDs, and URLs should be replaced with product-specific values.

**Reason:** Template values must be manually replaced during product setup, so they are easy to miss. Placeholder company names, plugin IDs, plugin names, AU codes, and repository URLs can leak into host scan caches, plugin menus, AU registration, logs, and support diagnostics. This rule is skipped in the template repository itself.

**Error condition:**

- Manifest metadata still contains template placeholders such as `Your Company`, `com.your-company`, `WRAC Gain`, `wrac_gain_plugin`, `WtGn`, or the template repository URL.

**Fix:** Replace template metadata with product-specific metadata. If the repository is intentionally a template or example, disable the rule with a documented reason.
