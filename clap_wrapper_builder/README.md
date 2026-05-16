# clap_wrapper_builder

Reusable CMake project that wraps a prebuilt CLAP plugin **static library**
into VST3 / AUv2 / AAX / Standalone bundles.

## Contents

- `CMakeLists.txt` - The wrapper build definition. The full input-variable
  reference lives in the header comment at the top of this file.
- `clap` / `clap-wrapper` / `vst3sdk` / `AudioUnitSDK` - Dependency SDKs,
  checked out as git submodules.

## Usage

Call this project directly, passing at least the static library to wrap:

```bash
cmake -S clap_wrapper_builder -B build \
  -DCLAP_WRAPPER_BUILDER_TARGET_LIB=/path/to/libyour_plugin.a \
  -DCLAP_WRAPPER_BUILDER_OUTPUT_NAME="Your Plugin"
cmake --build build
```

See the `CLAP_WRAPPER_BUILDER_*` variable list in `CMakeLists.txt` for
output naming, format toggles (VST3/AUv2/AAX/Standalone), and artifact
staging.

> Building this template's own plugin? Use `cargo xtask build` from the
> repository root instead — see the root `README.md`.

## AAX

AAX is off by default and requires the Avid AAX SDK. Enable it with
`-DCLAP_WRAPPER_BUILDER_BUILD_AAX=ON` once the SDK is available.
