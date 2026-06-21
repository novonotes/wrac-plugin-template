# wrac_xtask

> Japanese: [README_JA.md](README_JA.md)

`wrac_xtask` provides the shared WRAC `cargo xtask` command surface for building,
installing, launching, cleaning, and validating plugin artifacts.

The standard `wrac-plugin.toml` schema is documented in
[`wrac_manifest`](../wrac_manifest/README.md).

## Validation stages

`cargo xtask validate` normally builds the selected artifacts, runs WRAC
production-readiness checks, and runs external format validators.

- `--skip-readiness-checks`: skip WRAC production-readiness checks.
- `--skip-external-validators`: skip external format validators such as
  clap-validator, VST3 validator, auval, or AAX Validator.

If both flags are passed, validation becomes a build/package smoke check for the
selected targets.
