# wrac_clap_adapter

Defines the traits that each product crate must implement,
and provides an adapter that maps those trait implementations to the CLAP ABI so they can be used as plugins.

Conversion to VST3 / AU / AAX is the responsibility of `clap-wrapper`. This crate focuses solely on implementing CLAP plugins and CLAP extensions on the Rust side.

## Purpose

When using a CLAP plugin through clap-wrapper with a VST3/AU/AAX host, certain contracts defined by CLAP's thread model and call-order guarantees may not be honored. This crate aims to handle those cases defensively.

## Differences from clack

The CLAP headers annotate the allowed thread for each function using comments such as `[main-thread]`, `[audio-thread]`, and `[thread-safe]`. For example, `init` is `[main-thread]`, `process` is `[audio-thread]`, and `get_extension` is `[thread-safe]`.

`clack` is designed assuming the host calls functions according to these annotations, and works straightforwardly with native CLAP hosts.

This crate, on the other hand, also targets VST3/AU/AAX hosts via `clap-wrapper`. When routing through those hosts, annotated `[main-thread]` queries may be called from a different thread, among other deviations from the spec. This crate handles such cases through locks and panic catching on the adapter side, without exposing `unsafe` to product code, while still operating correctly.

## Acknowledgements

`wrac_clap_adapter` is inspired by `clack`'s design for a safe, low-level CLAP wrapper — in particular, its approach to CLAP extension boundaries and audio buffer access. This crate is not derived from `clack`'s code; it is an independent implementation built directly on `clap-sys`.

## Public API

- `PluginEntry`: DSO-level lifecycle and typed factory provider
- `PluginFactory`: CLAP `clap.plugin-factory`
- `PluginCore`: instance lifecycle and declaration of supported extensions
- `PluginAudioPortsExtension`: CLAP `audio-ports`
- `PluginConfigurableAudioPortsExtension`: CLAP `configurable-audio-ports`
- `PluginNotePortsExtension`: CLAP `note-ports`
- `PluginParamsExtension`: CLAP `params`
- `PluginStateExtension`: CLAP `state`
- `PluginGuiExtension`: CLAP `gui`
- `PluginRenderExtension`: CLAP `render`
- `PluginTailExtension`: CLAP `tail`
- `PluginLatencyExtension`: CLAP `latency`
- `export_clap_entry!`: exports the CLAP entry point

Each trait is a thin Rust representation of the corresponding CLAP C ABI. This crate is not designed as a general plugin framework.

## Limitations

This crate is provided as part of an implementation example, not as a general-purpose framework. Future changes will not provide API backwards compatibility or migration support.

Additionally, full CLAP ABI coverage is not yet complete. Known limitations:

- `audio-ports`: exposes current port metadata only; dynamic port rescan notifications are not supported
- `configurable-audio-ports`: only layout negotiation while inactive is supported
- `params`: value rescan after state restore is supported, but a dynamic rescan API for the parameter schema itself is not provided
- Output event batching helpers are minimal (sample-accurate event ordering is the product's responsibility)
- The `audio-ports-activation` extension is not implemented
- Typed factories other than plugin factory and AUv2 wrapper info are not implemented yet
