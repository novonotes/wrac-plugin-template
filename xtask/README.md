# xtask

Repository-local build commands for the WRAC plugin template.

```bash
cargo xtask build
cargo xtask build --release
cargo xtask build --validate
cargo xtask install
cargo xtask validate
```

`build` creates the CLAP bundle first and then builds VST3/AU wrappers through
`clap_wrapper_builder`. On macOS, `validate` runs Steinberg's VST3 validator and
`auval -v aufx WtGn YrCo`.

