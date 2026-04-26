# clap_wrapper_builder

`clap_wrapper_builder` is a helper build environment for wrapping a CLAP plugin
from the `wrac-plugin-template` reference implementation into VST3 / AUv2 / AAX / Standalone.

This is not a stable public API. Treat it as example code that supports sample
implementations such as the root gain plugin — breaking changes may be introduced.

## Contents

- `build_wrapper_plugin.sh` - Build a VST3 / AUv2 / AAX wrapper from a CLAP bundle
- `build_wrapper_plugin_static.sh` - Build VST3 / AUv2 / AAX / Standalone from a static library
- `install_wrapper_plugin.sh` - Install the generated VST3
- `clap-wrapper` / `clap` / `vst3sdk` / `AudioUnitSDK` - Dependency SDKs / toolchain

## AAX

Set `CLAP_WRAPPER_BUILDER_BUILD_AAX=ON` to enable AAX in the wrapper scripts.
When the SDK is already available locally, pass it via `CLAP_WRAPPER_BUILDER_AAX_SDK_ROOT`
or `AAX_SDK_ROOT`. To download the public slim SDK automatically through `clap-wrapper`,
set `CLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=ON`.

## Intended use

- Distribute the reference plugin of `wrac-plugin-template` in multiple formats
- Use as a reference implementation from other projects such as `xdevice-private`

This is not treated as a long-term stable public interface; the configuration and scripts may be revised as needed.
