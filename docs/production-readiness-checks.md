# Production-Readiness Checks

`cargo xtask validate` builds the requested plugin formats, runs WRAC production-readiness checks, and then runs external format validators such as clap-validator, Steinberg's VST3 validator, and auval. WRAC check violations are errors and return a non-zero exit code.

WRAC production-readiness checks are opinionated NovoNotes release-policy checks for commercial plugins, not format-spec validators. They can require a low-cost convention when NovoNotes expects it to reduce compatibility risk, support burden, or product inconsistency, even without a known format-spec violation or confirmed host-specific bug.

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

### `bypass-param-shape`

**Expectation:** Plugins expose at most one bypass parameter, and that parameter behaves as a boolean host bypass control.

**Reason:** Host bypass UI, bypass automation, generic editors, and control surfaces are most predictable when bypass is exposed as one boolean-shaped parameter.

**Error conditions:**

- More than one bypass parameter is exposed.
- A bypass parameter is not a stepped enum.
- A bypass parameter range is not `0..1`.
- A bypass parameter default is not `0` or `1`.

**Fix:** Expose a single bypass parameter with bypass, stepped, and enum flags, range `0..1`, and default `0` or `1`.

### `plugin-requires-bypass`

**Expectation:** Production plugins expose one valid bypass parameter.

**Reason:** Host bypass UI, bypass automation, generic editors, and control surfaces commonly expect plugins to provide a host-visible bypass control. A valid bypass parameter has low implementation cost and reduces host-specific compatibility risk across plugin categories.

**Error condition:** The plugin does not expose a bypass parameter.

**Fix:** Add one bypass parameter, or disable the rule with a documented reason when the product intentionally does not provide host bypass.

### `clap-descriptors-match-manifest`

**Expectation:** The CLAP factory descriptors match `package.metadata.wrac.plugins`.

**Reason:** Hosts and wrappers read product identity from the built plugin binary, while WRAC build tools read identity from the manifest. If those diverge, products can be scanned, displayed, automated, or wrapped under stale metadata.

**Error conditions:**

- The CLAP descriptor count differs from `package.metadata.wrac.plugins`.
- A CLAP descriptor ID, name, vendor, or version differs from manifest metadata.

**Fix:** Generate CLAP descriptors from `package.metadata.wrac` instead of hard-coding product metadata.

### `macos-clap-info-plist-matches-manifest`

**Expectation:** The macOS CLAP bundle `Info.plist` matches `package.metadata.wrac`.

**Reason:** macOS hosts and plugin scanners inspect bundle metadata before or alongside the plugin binary. Stale bundle identifiers, names, versions, or HiDPI flags can cause scan cache, display, loading, or editor behavior problems even when the CLAP descriptor is correct.

**Error conditions:**

- `Contents/Info.plist` is missing or unreadable.
- `CFBundleExecutable`, `CFBundleName`, or `CFBundleDisplayName` differs from `bundle_name`.
- `CFBundleIdentifier` differs from the primary plugin ID.
- `CFBundleShortVersionString` or `CFBundleVersion` differs from the package version.
- `NSHighResolutionCapable` is not `true`.

**Fix:** Keep CLAP bundle metadata generated from `package.metadata.wrac`.
