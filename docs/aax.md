# AAX Build and Validation

> Japanese version: [aax-ja.md](aax-ja.md)

AAX support can be enabled by adding `aax` to
`wrac-plugin.toml` `formats`.

## Prerequisites

- AAX SDK
- AAX Validator/DSH archive
- CMake and the platform C++ toolchain supported by clap-wrapper

Several environment variables must be set. See `.env.example`.

## Sidechain input

To route the host's sidechain to the first auxiliary CLAP input, add this to the
plugin's `[[plugins]]` entry in `wrac-plugin.toml`:

```toml
aax_sidechain_input = "first-auxiliary"
```

AAX provides a mono sidechain. The wrapper supplies the same signal to every
channel of CLAP input port 1; other auxiliary inputs receive silence. The
plugin must expose that input port.
