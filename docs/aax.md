# AAX Build and Validation

> Japanese version: [aax-ja.md](aax-ja.md)

AAX support can be enabled by adding `aax` to
`wrac-plugin.toml` `formats`.

## Prerequisites

- AAX SDK
- AAX Validator/DSH archive
- CMake and the platform C++ toolchain supported by clap-wrapper

Several environment variables must be set. See `.env.example`.

## MIDI input

For a plugin with a note input that should receive MIDI in Pro Tools, add an
ASCII node name to its `[[plugins]]` entry in `wrac-plugin.toml`:

```toml
aax_midi_input_name = "MIDI In"
```

AAX registers MIDI nodes before creating plugin instances. Without this setting,
the wrapper ignores note inputs in AAX and logs the missing declaration at
initialization. Categories such as `software-generator` do not declare MIDI input;
MIDI-controlled audio effects need the setting too. The plugin must expose a note
input port. WRAC currently does not expose an AAX MIDI output declaration.

## Sidechain input

To route the host's sidechain to the first auxiliary CLAP input, add this to the
plugin's `[[plugins]]` entry in `wrac-plugin.toml`:

```toml
aax_sidechain_input = "first-auxiliary"
```

AAX provides a mono sidechain. The wrapper supplies the same signal to every
channel of CLAP input port 1; other auxiliary inputs receive silence. After the
selected AAX configuration is applied, the plugin must expose a main input at
port 0 and an auxiliary input at port 1, both with channels. An invalid declaration
fails initialization; an unconnected host sidechain supplies silence normally.
Sidechains without a main input are not supported by this mapping.
