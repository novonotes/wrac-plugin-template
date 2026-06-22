# wrac_xtask

> Japanese: [README_JA.md](README_JA.md)

`wrac_xtask` provides typed WRAC task primitives and shared execution helpers for
repository-local `cargo xtask` crates. Repository xtasks own package selection,
policy decisions, and task graph construction.

The standard `wrac-plugin.toml` schema is documented in
[`wrac_manifest`](../wrac_manifest/README.md).

## Validation tasks

The crate exposes task kinds for building artifacts, running WRAC
production-readiness checks, and running external format validators such as
clap-validator, VST3 validator, auval, or AAX Validator. The repository-local
xtask decides which of those tasks belong in a validation plan.
