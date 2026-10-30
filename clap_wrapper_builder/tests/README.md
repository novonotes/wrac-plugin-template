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
