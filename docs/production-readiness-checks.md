# Production-Readiness Checks

`cargo xtask validate` builds the requested CLAP, VST3, and/or AU plugin targets, runs WRAC production-readiness checks, and then runs external format validators such as clap-validator, Steinberg's VST3 validator, and auval when they apply. The command can also build the development-only standalone app for local smoke testing; standalone validation currently runs WRAC checks only. WRAC check violations are errors and return a non-zero exit code.

WRAC production-readiness checks are opinionated NovoNotes release-policy checks for commercial plugins, not format-spec validators. They can require a low-cost convention when NovoNotes expects it to reduce compatibility risk, support burden, or product inconsistency, even without a known format-spec violation or confirmed host-specific bug.

The command logs every check as `pass`, `fail`, `disabled`, or `skipped` so CI logs show which release-policy checks were evaluated.

Source-level implementation review is a separate layer. For AI-assisted review,
ask the reviewer to use the repository root
[`code-review-checklist.md`](../code-review-checklist.md) as context.

Some source-level checks are still deterministic enough to run as production-readiness checks. These checks cover template placeholders, metadata injection paths, and literal frontend/native command names. More contextual source concerns, such as realtime lock usage or state migration quality, remain code review checklist items.

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

### `param-info-shape`

**Expectation:** Public parameters have stable, host-safe identity and value metadata.

**Reason:** Hosts, automation lanes, generic editors, control surfaces, and project recall depend on parameter IDs, names, ranges, and defaults being deterministic and coherent.

**Error conditions:**

- A public parameter ID is duplicated.
- A public parameter name is empty.
- A public parameter min, max, or default is not finite.
- A public parameter min is greater than or equal to max.
- A public parameter default is outside its declared range.

**Fix:** Give every public parameter one stable unique ID, one non-empty name, a finite `min < max` range, and a finite default inside the range.

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

### `state-extension-required`

**Expectation:** Production plugins expose a state extension.

**Reason:** DAW project recall, preset workflows, duplicate/restore behavior, and wrapper-host state bridging are release requirements for commercial products.

**Error condition:** The built plugin does not expose the CLAP state extension.

**Fix:** Implement plugin state save/load, or disable the rule with a documented reason when the product intentionally has no project-recall state.

### `audio-port-shape`

**Expectation:** Audio-capable products expose coherent audio port metadata.

**Reason:** Hosts and wrappers use audio port lists to scan capabilities, create tracks, choose channel layouts, validate buses, and route audio.

**Error conditions:**

- An `audio-effect` plugin does not expose at least one audio input and one audio output.
- An `instrument`, `synthesizer`, or `sampler` plugin does not expose an audio output.
- A non-note-only, non-analyzer plugin exposes multiple main ports in the same direction.
- Audio port IDs are duplicated within one direction.
- An audio port name is empty.
- An audio port channel count is zero.
- An audio port type is empty.

**Fix:** Expose stable named audio ports with unique IDs, concrete channel counts, declared port types, and exactly one main host-facing port per direction when ports exist.

### `note-port-shape`

**Expectation:** Note-capable products expose coherent note port metadata.

**Reason:** Hosts and wrappers use note port lists to route MIDI/CLAP note events and to decide whether note-processing workflows are available.

**Error conditions:**

- Note port IDs are duplicated within one direction.
- A note port name is empty.
- A note port supports no note dialects.
- A note port preferred dialect is empty or not included in its supported dialects.

**Fix:** Expose stable named note ports with unique IDs and a preferred dialect that is included in the supported dialect set.

### `features-match-capabilities`

**Expectation:** CLAP descriptor features match the capabilities exposed by the built plugin.

**Reason:** Hosts and plugin browsers use descriptor features for categorization, track creation, routing, and search/filter behavior.

**Error conditions:**

- The CLAP descriptor exposes no features.
- The `audio-effect` feature is present without audio input and output ports.
- An `instrument`, `synthesizer`, or `sampler` feature is present without an audio output.
- A `note-effect` or `note-detector` feature is present without note input or output ports.

**Fix:** Declare descriptor features that match the plugin's actual ports and capabilities.

### `gui-artifact-shape`

**Expectation:** Products with `src-gui` expose a GUI extension and have built frontend output available when validation runs.

**Reason:** Release builds embed frontend assets into the plugin binary. A product can pass format validation while still shipping without usable GUI assets if the GUI build or GUI extension path regresses.

**Error conditions:**

- The plugin source has `src-gui`, but the built plugin does not expose the CLAP GUI extension.
- `src-gui/dist/index.html` is missing after `cargo xtask validate` builds the GUI.

**Fix:** Build the frontend through `cargo xtask validate/build` before compiling the plugin, expose the GUI extension, or disable this rule with a documented reason for headless products.

### `template-placeholders-renamed`

**Expectation:** Product repositories replace template identity placeholders before shipping.

**Reason:** Placeholder names and IDs can leak into host scan caches, plugin menus, AU registration, logs, and support diagnostics. This rule is skipped in the template repository itself.

**Error conditions:**

- Manifest metadata still contains template placeholders such as `Your Company`, `com.your-company`, `WRAC Gain`, `wrac_gain_plugin`, `WtGn`, or the template repository URL.

**Fix:** Rename template metadata to product metadata, or disable this rule with a documented reason for template/example repositories.

### `source-metadata-single-source`

**Expectation:** Template source code reads product identity from `src-plugin/Cargo.toml`.

**Reason:** Rust descriptors, wrapper arguments, GUI About text, logs, and bundle metadata should not drift when a product is renamed.

**Error conditions:**

- The Rust descriptor source no longer uses build-script env vars generated from `package.metadata.wrac`.
- `build.rs` no longer emits WRAC identity env vars from `package.metadata.wrac`.
- `src-gui/vite.config.ts` no longer reads `../src-plugin/Cargo.toml` and injects `__WRAC_PLUGIN_METADATA__`.
- The frontend no longer renders identity from `__WRAC_PLUGIN_METADATA__`.

**Fix:** Keep `src-plugin/Cargo.toml` as the source of truth, or disable this rule with a documented reason for custom metadata generation.

### `gui-native-commands-match`

**Expectation:** Literal TypeScript `invoke(...)` command names are registered by Rust.

**Reason:** Frontend/native command drift usually appears only when the GUI path is exercised. A simple literal-name check catches stale TypeScript calls before host smoke testing.

**Error condition:** A string-literal TypeScript `invoke(...)` command is not registered by Rust `register_sync(...)`.

**Fix:** Register the command on the Rust side, rename the TypeScript invoke, or disable this rule with a documented reason for dynamic command routing.

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

### `macos-clap-info-plist-matches-manifest`

**Expectation:** The macOS CLAP bundle `Info.plist` matches `package.metadata.wrac`.

**Reason:** macOS hosts and plugin scanners inspect bundle metadata before or alongside the plugin binary. Stale bundle identifiers, names, versions, or HiDPI flags can cause scan cache, display, loading, or editor behavior problems even when the CLAP descriptor is correct.

**Error conditions:**

- `Contents/Info.plist` is missing or unreadable.
- `CFBundleExecutable`, `CFBundleName`, or `CFBundleDisplayName` differs from `bundle_name`.
- The executable named by `CFBundleExecutable` is missing from `Contents/MacOS`.
- `CFBundleIdentifier` differs from the primary plugin ID.
- `CFBundleShortVersionString` or `CFBundleVersion` differs from the package version.
- `NSHighResolutionCapable` is not `true`.

**Fix:** Keep CLAP bundle metadata generated from `package.metadata.wrac`.

### `macos-wrapper-info-plists-match-manifest`

**Expectation:** macOS VST3 and AU bundle metadata match `package.metadata.wrac`. When the development standalone app is requested, its app metadata also matches `package.metadata.wrac`.

**Reason:** macOS hosts, plugin scanners, AU registration, and user-facing plugin lists inspect bundle metadata separately from the plugin binary. Stale wrapper metadata can cause scan cache, display, registration, or loading problems. The standalone app is a development-only smoke-test host, but keeping its metadata generated from the same manifest avoids confusing local debug sessions.

**Error conditions:**

- A requested VST3 or AU `Contents/Info.plist` is missing or unreadable.
- VST3 `CFBundleExecutable`, `CFBundleName`, `CFBundleShortVersionString`, or `CFBundleVersion` differs from manifest metadata.
- The executable named by VST3 or AU `CFBundleExecutable` is missing from `Contents/MacOS`.
- AU `CFBundleExecutable`, `CFBundleName`, `CFBundleShortVersionString`, `CFBundleVersion`, or `NSHighResolutionCapable` differs from manifest metadata.
- AU `AudioComponents[0]` is missing.
- AU `AudioComponents[0].manufacturer`, `type`, `subtype`, `name`, or `version` differs from manifest metadata.
- When standalone validation is requested, standalone `Contents/Info.plist` is missing or unreadable.
- When standalone validation is requested, standalone `CFBundleExecutable`, `CFBundleName`, `CFBundleShortVersionString`, or `CFBundleVersion` differs from manifest metadata.
- When standalone validation is requested, the executable named by standalone `CFBundleExecutable` is missing from `Contents/MacOS`.

**Fix:** Keep wrapper bundle metadata, and the development standalone app metadata when built, generated from `package.metadata.wrac`.

VST3 factory vendor/company metadata is not stored in the macOS VST3 `Info.plist`. WRAC checks the CLAP descriptor vendor against `package.metadata.wrac.company_name`; the VST3 wrapper derives its factory vendor from that descriptor, and Steinberg's VST3 validator prints the resulting factory metadata during validation.
