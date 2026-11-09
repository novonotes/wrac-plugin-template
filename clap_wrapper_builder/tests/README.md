# Wrapper regression tests

> 日本語版: [README_JA.md](README_JA.md)

On Windows or macOS, build the release WRAC Gain VST3 or AUv2 bundle, then run
the tests for the current platform:

```sh
cargo xtask build --release --target=vst3 # Windows; use --target=au on macOS
cargo xtask test-wrapper
```

CTest registrations live in [CMakeLists.txt](CMakeLists.txt). Each test source
documents its purpose and any partial skips.

On macOS, building both formats with `--target=vst3,au` also enables the VST3
parameter-edit regression test alongside the AUv2 tests.

To include the AAX regression tests, configure CMake with the local AAX SDK
and the built WRAC Gain bundle, then build and run the tests:

```sh
cmake -S clap_wrapper_builder/tests -B target/wrapper-tests \
  -DWRAC_TEST_PLUGIN="/path/to/WRAC Gain.component" \
  -DWRAC_TEST_AAX_SDK="/path/to/aax-sdk"
cmake --build target/wrapper-tests --config Release
ctest --test-dir target/wrapper-tests -C Release --output-on-failure
```

On Windows, point `WRAC_TEST_PLUGIN` to the VST3 bundle instead. The optional
AAX tests are omitted when no SDK path is supplied.
