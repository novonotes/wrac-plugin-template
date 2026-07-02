# wrac_build_ops

> Japanese: [README_JA.md](README_JA.md)

`wrac_build_ops` provides typed WRAC build, install, launch, and validation
operations for repository-local `cargo xtask` crates. Repository xtasks own
package selection, workflow policy, task graph construction, and task execution
dispatch.

The standard `wrac-plugin.toml` schema is documented in
[`wrac_manifest`](../wrac_manifest/README.md).

## Validation operations

The crate exposes typed operations for running WRAC production-readiness checks
and external format validators such as clap-validator, VST3 validator, auval, or
AAX Validator. The repository-local xtask decides how those operations are
ordered in its own workflow.
