# wrac_manifest

> Japanese: [README_JA.md](README_JA.md)

`wrac_manifest` parses and validates WRAC plugin manifests. The schema,
validation, and usage requirements are defined in [src/lib.rs](src/lib.rs).

See the [WRAC Gain manifest](../../plugins/wrac-gain/src-plugin/wrac-plugin.toml)
for an example.

For how xtask uses the manifest, see the following `wrac_build_ops` code:

- [Metadata loading (metadata.rs)](../wrac_build_ops/src/metadata.rs)
- [Target selection (target_resolution.rs)](../wrac_build_ops/src/target_resolution.rs)
- [Context API (context.rs)](../wrac_build_ops/src/context.rs)
