# wrac_manifest

> Japanese: [README_JA.md](README_JA.md)

`wrac_manifest` parses the WRAC plugin manifest (`wrac-plugin.toml`) into typed
Rust metadata. This README is the primary schema reference for WRAC-owned
manifest fields.

## Scope

`wrac-plugin.toml` describes host-visible plugin identity, wrapper metadata,
default build targets, and WRAC validation exceptions.

WRAC ignores unknown fields and tables. Users may add their own namespaced
extension tables.

## Location Contract

A workspace member is a WRAC plugin exactly when its Cargo manifest is located
at `<plugin-root>/src-plugin/Cargo.toml`. The plugin root is derived as the
parent of `src-plugin`; it is not declared in configuration. Its WRAC manifest
must be located only at `<plugin-root>/src-plugin/wrac-plugin.toml`.

`wrac-plugin.toml` at the plugin root or in any other source directory is an
error, as is a missing manifest for a `src-plugin` workspace member. Ordinary
Cargo workspace members outside `src-plugin` are not treated as WRAC plugins.

```toml
[acme.ci]
validation_profile = "prototype"
```

## Schema

### Root Table

| Field | Type | Required | Accepted values | Meaning |
| --- | --- | --- | --- | --- |
| `schema_version` | integer | Recommended | `1` | WRAC manifest schema version. Current manifests should write `1`. The parser currently accepts an omitted value for older manifests. |

### `[package]`

`[package]` is optional. When `wrac_build_ops` reads a manifest from a Cargo
package, missing `name`, `version`, and `repository` values are filled from the
package `Cargo.toml`.

| Field | Type | Required | Accepted values | Meaning |
| --- | --- | --- | --- | --- |
| `name` | string | No | Non-empty package name when supplied | Overrides the Cargo package name used by WRAC artifact metadata. Usually omit this and use `Cargo.toml`. |
| `version` | string | No | Non-empty version when supplied | Overrides the Cargo package version used in generated plugin descriptors and bundles. Usually omit this and use `Cargo.toml`. |
| `repository` | string | No | Any non-empty repository URL when supplied | Overrides the Cargo package repository metadata used by readiness checks. |
| `version_source` | string | No | `cargo` by convention | Compatibility/documentation field. WRAC currently fills missing `version` from Cargo metadata and does not support another version source. |

### `[bundle]`

`[bundle]` is required. These fields are shared by every plugin product exposed
from the same binary bundle.

| Field | Type | Required | Accepted values | Meaning |
| --- | --- | --- | --- | --- |
| `company_name` | string | Yes | Non-empty | Host-facing vendor/manufacturer name. Generated into CLAP descriptors and wrapper metadata. It may be visible in host browsers and scanners. |
| `auv2_manufacturer_code` | string | Yes | Exactly 4 ASCII bytes | AUv2 manufacturer code. Generated into the AUv2 wrapper descriptor. Keep it stable after release. |
| `aax_manufacturer_id` | string | Conditional | Exactly 4 ASCII bytes | AAX manufacturer ID. Required when `supported_formats` contains `aax`; ignored otherwise. Keep it stable after release. |
| `bundle_name` | string | Yes | Non-empty | Product bundle name. Used for artifact names such as `.clap`, `.vst3`, `.component`, and `.aaxplugin`, and for macOS bundle display names. |
| `bundle_identifier` | string | Yes | Non-empty reverse-DNS style identifier recommended | macOS CLAP `CFBundleIdentifier`. This is bundle identity, not a per-plugin ID. Keep it stable after release. |
| `homepage_url` | string | Yes | Non-empty URL string | Generated into CLAP descriptor `url`. Hosts or scanners may expose it to users. |
| `manual_url` | string | Yes | Non-empty URL string | Generated into CLAP descriptor `manual_url`. Hosts or scanners may expose it to users. |
| `support_url` | string | Yes | Non-empty URL string | Generated into CLAP descriptor `support_url`. Hosts or scanners may expose it to users. |
| `description` | string | Yes | Non-empty | Generated into CLAP descriptor `description`. This is a short user-facing product description that may appear in host or scanner UI. |
| `copyright` | string | Yes | Non-empty | Generated into macOS bundle copyright metadata and used by readiness checks. |
| `supported_formats` | array of `PluginFormat` | Yes | `clap`, `vst3`, `au`, `aax`; non-empty; no duplicates | Product policy for default `cargo xtask build`, `install`, and `validate` plugin targets. Explicit `--target` plugin formats must also be listed here. |

### `[[plugins]]`

At least one `[[plugins]]` entry is required. Each entry is one host-visible
plugin product exposed by the bundle. A bundle may expose multiple plugin
products.

| Field | Type | Required | Accepted values | Meaning |
| --- | --- | --- | --- | --- |
| `plugin_id` | string | Yes | Non-empty; unique in the manifest | CLAP plugin ID. Hosts use this as product identity. Changing it after release is a compatibility break for saved sessions. |
| `plugin_name` | string | Yes | Non-empty | Host-facing plugin name. Generated into CLAP descriptors and shown by host browsers/scanners. |
| `clap_features` | array of `ClapFeature` | Yes | One or more predefined CLAP feature strings; see [CLAP feature values](#clap-feature-values) | Generated into the CLAP descriptor. CLAP hosts read these directly, so they must match the plugin's actual audio/MIDI behavior. |
| `vst3_subcategories` | string | Yes | Non-empty Steinberg-style subcategory string separated by vertical bars, for example `Fx&#124;Dynamics` | Generated into the VST3 wrapper descriptor. WRAC only validates that this is non-empty; choose values accepted by VST3 hosts. |
| `vst3_component_id` | string | Yes | UUID string | VST3 component identity. WRAC converts the UUID to the byte order expected by clap-wrapper. Generate once before release and keep it stable for the same product. |
| `standalone_name` | string | Yes | Non-empty; unique in the manifest | Standalone app artifact name. Used for `.app`, `.exe`, or Linux standalone file names. |
| `auv2_type` | string | Yes | Exactly 4 ASCII bytes | AUv2 component type, such as `aufx` or `aumu`. Generated into the AUv2 wrapper descriptor. |
| `auv2_subtype` | string | Yes | Exactly 4 ASCII bytes; the `(auv2_type, auv2_subtype)` pair must be unique in the manifest | AUv2 component subtype. Keep it stable after release because hosts use it for identity. |
| `aax_categories` | array of `AaxCategory` | Conditional | One or more predefined AAX category strings; see [AAX category values](#aax-category-values) | Required when `supported_formats` contains `aax`; ignored otherwise. Generated into the AAX wrapper descriptor. |
| `aax_product_id` | string | Conditional | Exactly 4 ASCII bytes | Required when `supported_formats` contains `aax`; ignored otherwise. AAX product identity. Keep it stable after release. |
| `aax_stem_configs` | array of tables | Conditional | Non-empty when `supported_formats` contains `aax` | Required when `supported_formats` contains `aax`; ignored otherwise. Defines AAX input/output stem layouts. |

### `[[plugins.aax_stem_configs]]`

| Field | Type | Required | Accepted values | Meaning |
| --- | --- | --- | --- | --- |
| `name` | string | Yes | Non-empty | Human-readable AAX stem configuration name. |
| `input` | `AaxStemFormat` | Yes | `mono`, `stereo` | AAX input stem format. |
| `output` | `AaxStemFormat` | Yes | `mono`, `stereo` | AAX output stem format. |
| `plugin_id` | string | Yes | Exactly 4 ASCII bytes; unique within the parent plugin's stem configs | AAX stem-specific plugin ID. Keep it stable after release. |

### `[validation]`

`[validation]` is optional. It defines WRAC validation exceptions, not product
category or release policy.

| Field | Type | Required | Accepted values | Meaning |
| --- | --- | --- | --- | --- |
| `disabled_rules` | table | No | Keys are WRAC readiness rule IDs | Disables specific WRAC production-readiness rules. Each disabled rule must include a reason. |
| `clap_validator` | table | No | See below | Configures skips for external `clap-validator` tests. |

#### `[validation.disabled_rules.<rule_id>]`

| Field | Type | Required | Accepted values | Meaning |
| --- | --- | --- | --- | --- |
| `reason` | string | Yes | Non-empty | Explanation for disabling this WRAC readiness rule. |

#### `[validation.clap_validator]`

| Field | Type | Required | Accepted values | Meaning |
| --- | --- | --- | --- | --- |
| `skip_test_filter` | string | No | Non-empty regular-expression/filter string | Passed to the external CLAP validator skip mechanism. If present, `skip_reason` is required. |
| `skip_reason` | string | Conditional | Non-empty | Required when `skip_test_filter` is present. Explains why the external validator tests are skipped. |

## Predefined Values

### `PluginFormat` Values

- `clap`
- `vst3`
- `au`
- `aax`

### CLAP Feature Values

- `audio-effect`
- `analyzer`
- `ambisonic`
- `chorus`
- `compressor`
- `de-esser`
- `delay`
- `instrument`
- `note-effect`
- `note-detector`
- `drum`
- `drum-machine`
- `equalizer`
- `expander`
- `filter`
- `flanger`
- `frequency-shifter`
- `gate`
- `glitch`
- `granular`
- `distortion`
- `limiter`
- `mastering`
- `mixing`
- `mono`
- `multi-effects`
- `phaser`
- `phase-vocoder`
- `pitch-correction`
- `pitch-shifter`
- `restoration`
- `reverb`
- `sampler`
- `stereo`
- `surround`
- `synthesizer`
- `transient-shaper`
- `tremolo`
- `utility`

### AAX Category Values

- `eq`
- `dynamics`
- `pitch-shift`
- `reverb`
- `delay`
- `modulation`
- `harmonic`
- `noise-reduction`
- `dither`
- `sound-field`
- `hardware-generator`
- `software-generator`
- `wrapped-plugin`
- `effect`
- `midi-effect`

### AAX Stem Format Values

- `mono`
- `stereo`
