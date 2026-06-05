# Production-Readiness Checks

`cargo xtask validate` builds the requested CLAP, VST3, and/or AU plugin targets, runs WRAC production-readiness checks, and then runs external format validators such as clap-validator, Steinberg's VST3 validator, and auval when they apply. WRAC check violations are errors and return a non-zero exit code.

WRAC production-readiness checks are NovoNotes release-policy checks for commercial plugins, not format-spec validators. Keep this gate small: a check should exist only when the failure mode has already caused a real problem, or when it is clearly easy for product implementors to miss and has a direct release risk.

Do not add checks for broad best practices, implementation style, or metadata that is already generated from a single source by xtask, CMake, or build scripts. Those paths have lower human-error risk and are better covered by ordinary tests for the generator.

The command logs every check as `pass`, `fail`, `disabled`, or `skipped` so CI logs show which release-policy checks were evaluated.

## Disabling Checks

Checks can be disabled by rule ID in the plugin crate manifest. Every disabled rule must include a non-empty `reason`.

```toml
[package.metadata.wrac.validation.disabled_rules.fender-studio-pro-generic-editor-single-knob]
reason = "This product does not support Fender Studio Pro generic editor workflows."
```

Unknown rule IDs and empty reasons are errors.

Disable checks only for intentional product decisions. If the plugin is expected to satisfy the release policy behind a check, fix the plugin instead.

## Adding Checks

New checks are release-policy changes, not just code changes. Before opening a PR, the author must complete the following:

- **Justify:** Confirm the rule covers either a real problem that has happened, or a clearly likely product-implementation mistake with direct release risk.
- **Avoid Duplication:** Do not duplicate external format validators or generator tests unless WRAC has a known business reason to be stricter.
- **Document:** Add the expectation, reason, error condition, and fix to this document's Check List.
- **Unit Test:** Cover `pass`, `fail`, `disabled`, `skipped`, and edge cases.
- **Manually Validate (Mandatory):** Unit tests alone are insufficient. You must:
  - Intentionally break a real template plugin and verify `cargo xtask validate` fails with the expected rule ID and message.
  - Restore the plugin and confirm the command now logs the check as `pass`, `disabled`, or `skipped`.

## Check List

### `fender-studio-pro-generic-editor-single-knob`

**Expectation:** Production plugins that support Fender Studio Pro generic editor workflows expose either no visible non-bypass parameters or at least two visible non-bypass parameters.

**Reason:** Fender Studio Pro 8.0.3 generic editors fail to render knobs for this shape. Bypass parameters do not count as knobs for this rule.

**Error condition:** When CLAP or VST3 validation is requested, the plugin exposes exactly one visible, non-bypass parameter.

**Fix:** Expose zero or at least two visible non-bypass parameters, or disable the rule with a documented reason when the product intentionally does not support Fender Studio Pro generic editor workflows.

### `luna-vst3-param-id-must-match-index`

**Expectation:** VST3-compatible plugins keep public parameter IDs equal to their parameter-list indices.

**Reason:** LUNA 2.0.3.4381 VST3 automation writes can fail when a VST3 parameter ID differs from its parameter-list index.

**Error condition:** When VST3 validation is requested, a public parameter ID differs from its parameter-list index.

**Fix:** Reorder parameters or adjust public parameter IDs so each public parameter ID matches its index.

### `param-info-shape`

**Expectation:** Public parameters have stable, host-safe identity and value metadata.

**Reason:** Parameter IDs, names, ranges, and defaults are product code, and small mistakes here can break automation, generic editors, control surfaces, or project recall.

**Error conditions:**

- A public parameter ID is duplicated.
- A public parameter name is empty.
- A public parameter min, max, or default is not finite.
- A public parameter min is greater than or equal to max.
- A public parameter default is outside its declared range.

**Fix:** Give every public parameter one stable unique ID, one non-empty name, a finite `min < max` range, and a finite default inside the range.

### `bypass-param-shape`

**Expectation:** Plugins expose at most one bypass parameter, and that parameter behaves as a boolean host bypass control.

**Reason:** Bypass is cheap to implement but visible to hosts, automation, generic editors, and control surfaces. Shape mistakes are easy to make when adding the parameter manually.

**Error conditions:**

- More than one bypass parameter is exposed.
- A bypass parameter is not a stepped enum.
- A bypass parameter range is not `0..1`.
- A bypass parameter default is not `0` or `1`.

**Fix:** Expose a single bypass parameter with bypass, stepped, and enum flags, range `0..1`, and default `0` or `1`.

### `plugin-requires-bypass`

**Expectation:** Production plugins expose one valid bypass parameter.

**Reason:** A valid bypass parameter has low implementation cost and reduces host-specific compatibility risk across plugin categories.

**Error condition:** The plugin does not expose a bypass parameter.

**Fix:** Add one bypass parameter, or disable the rule with a documented reason when the product intentionally does not provide host bypass.

### `state-extension-required`

**Expectation:** Production plugins expose a state extension.

**Reason:** DAW project recall, preset workflows, duplicate/restore behavior, and wrapper-host state bridging are release requirements for commercial products.

**Error condition:** The built plugin does not expose the CLAP state extension.

**Fix:** Implement plugin state save/load, or disable the rule with a documented reason when the product intentionally has no project-recall state.

### `gui-artifact-shape`

**Expectation:** Products with `src-gui` expose a GUI extension and have built frontend output available when validation runs.

**Reason:** Release builds embed frontend assets into the plugin binary. A product can pass format validation while still shipping without usable GUI assets if the GUI build or GUI extension path regresses.

**Error conditions:**

- The plugin source has `src-gui`, but the built plugin does not expose the CLAP GUI extension.
- `src-gui/dist/index.html` is missing after `cargo xtask validate` builds the GUI.

**Fix:** Build the frontend through `cargo xtask validate/build` before compiling the plugin, expose the GUI extension, or disable this rule with a documented reason for headless products.

### `template-placeholders-renamed`

**Expectation:** Product repositories replace template identity placeholders before shipping.

**Reason:** Template placeholders are changed manually during product setup, so the human-error risk is high. Placeholder names and IDs can leak into host scan caches, plugin menus, AU registration, logs, and support diagnostics. This rule is skipped in the template repository itself.

**Error conditions:**

- Manifest metadata still contains template placeholders such as `Your Company`, `com.your-company`, `WRAC Gain`, `wrac_gain_plugin`, `WtGn`, or the template repository URL.

**Fix:** Rename template metadata to product metadata, or disable this rule with a documented reason for template/example repositories.

### `clap-descriptors-match-manifest`

**Expectation:** The CLAP factory descriptors match `package.metadata.wrac.plugins`.

**Reason:** Hosts and wrappers read product identity from the built plugin binary, while WRAC build tools read identity from the manifest. If those diverge, products can be scanned, displayed, automated, or wrapped under stale metadata.

**Error conditions:**

- The CLAP descriptor count differs from `package.metadata.wrac.plugins`.
- A CLAP descriptor ID, name, vendor, or version differs from manifest metadata.

**Fix:** Generate CLAP descriptors from `package.metadata.wrac` instead of hard-coding product metadata.

### `wrapper-targets-require-single-product`

**Expectation:** VST3 and AU wrapper validation releases one product per wrapper bundle.

**Reason:** The CLAP factory can expose multiple products, but wrapper formats currently use one primary product identity for bundle metadata and host registration. Failing wrapper validation for multi-product bundles avoids silently validating only the primary product.

**Error condition:** VST3 or AU validation is requested and `package.metadata.wrac.plugins` contains more than one product.

**Fix:** Release wrapper formats as single-product bundles, or disable this rule with a documented reason after confirming wrapper metadata and host scans for every product.
