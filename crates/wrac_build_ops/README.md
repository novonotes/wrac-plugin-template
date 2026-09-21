# wrac_build_ops

> Japanese: [README_JA.md](README_JA.md)

`wrac_build_ops` provides typed WRAC build, install, launch, and validation
operations for repository-local `cargo xtask` crates. It also exposes the
standard WRAC package-selection helpers and per-package task ID helper used by
those xtasks.

Repository xtasks still own workflow policy, task graph construction, final
artifact node selection, and task execution dispatch. That boundary keeps
product-specific tasks, such as attaching generated assets after a standard
package step, outside the WRAC operation layer.

Operations that can intentionally avoid work, such as a GUI build without a
frontend package or an up-to-date CMake configure, return `TaskOutcome` so the
repository workflow can print one task result with the concrete skip reason.

The standard `wrac-plugin.toml` schema is documented in
[`wrac_manifest`](../wrac_manifest/README.md).

## Validation operations

The crate exposes typed adapters for external format validators such as
clap-validator, VST3 validator, auval, or AAX Validator. The repository-local
xtask supplies validator configuration and decides how those operations are
ordered in its own workflow.
