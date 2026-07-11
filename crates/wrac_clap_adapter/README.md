# wrac_clap_adapter

Maps the product contracts defined by `wrac_clap_adapter::interface` to the CLAP ABI.

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

`export_clap_entry!` exports the CLAP entry point. Product implementations use the traits and
related types from `wrac_clap_adapter::interface`; the remaining modules own the ABI callbacks,
registration storage, and concrete host proxy implementations.

## Instance lifecycle

`clap.plugin-factory.create_plugin` creates only the CLAP ABI shell. Product instance construction is deferred until `plugin.init`, where CLAP and clap-wrapper initialize the plugin-facing lifecycle. Capability objects such as ports, parameters, state, GUI, latency, and tail are frozen during `plugin.init` so later host callbacks can answer without taking the product lifecycle lock.

Calls that arrive before capability freeze do not wait for initialization to complete. They fail fast, return `null`/`false`/`0`, or no-op depending on the CLAP callback shape. Product-held host extension proxies also remain inert until capability freeze is complete, preventing init-time host callbacks from observing half-initialized plugin capabilities. Teardown callbacks are the exception: `deactivate` and `destroy` wait for in-flight processing access to finish so the adapter can reclaim processors before the host releases the instance.

## Limitations

This crate is provided as part of an implementation example, not as a general-purpose framework. Future changes will not provide API backwards compatibility or migration support.

Additionally, full CLAP ABI coverage is not yet complete. Known limitations:

- `configurable-audio-ports`: only layout negotiation while inactive is supported
- Host callback proxies are thin wrappers and do not marshal calls to a different thread
- Output event batching helpers are minimal (sample-accurate event ordering is the product's responsibility)
- The `audio-ports-activation` extension is not implemented
- Typed factories other than plugin factory and AUv2 wrapper info are not implemented yet
