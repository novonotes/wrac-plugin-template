# wrac_manifest

> Japanese: [README_JA.md](README_JA.md)

`wrac_manifest` parses the WRAC plugin manifest (`wrac-plugin.toml`) into typed
Rust metadata. It owns the standard manifest schema used by WRAC build scripts
and xtask commands.

## `wrac-plugin.toml`

WRAC plugin packages are described by a repository-owned `wrac-plugin.toml`.
The manifest contains host-visible product metadata and validation exceptions.

### Standard fields

- `schema_version`: WRAC manifest schema version.
- `[package]`: package-level metadata overrides. `version_source = "cargo"` uses
  `Cargo.toml` as the bundle and descriptor version source.
- `[bundle]`: metadata shared by every plugin product in the bundle, including
  company name, bundle identifiers, URLs, description, copyright, and
  `supported_formats`.
- `[[plugins]]`: host-visible plugin products exposed from the bundle. Each
  entry defines IDs, names, CLAP features, wrapper descriptors, and optional
  AAX metadata.
- `[validation]`: validation exceptions such as disabled production-readiness
  rules and external validator skip filters.

### Repository extensions

WRAC ignores unknown fields and tables in `wrac-plugin.toml`. Downstream
repositories may place their own metadata in a namespaced table and parse that
metadata from repository-local automation.

```toml
[acme.ci]
validation_profile = "prototype"
```

WRAC does not interpret extension tables. Repository automation should translate
repository-specific policy into explicit WRAC command-line options.
