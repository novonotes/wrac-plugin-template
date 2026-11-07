# Wrapper regression tests

> 日本語版: [README_JA.md](README_JA.md)

On Windows or macOS, build the release WRAC Gain VST3 or AUv2 bundle, then run
the tests for the current platform:

```sh
cargo xtask build --release --target=vst3 # Windows; use --target=au on macOS
cargo xtask test-wrapper
```

The VST3 silence-buffer regression can also run directly without a plugin bundle:

```sh
c++ -std=c++17 -I clap_wrapper_builder/vst3sdk \
  clap_wrapper_builder/tests/vst3_silence.cpp -o /tmp/vst3_silence
/tmp/vst3_silence
```

CTest registrations live in [CMakeLists.txt](CMakeLists.txt). Each test source
documents its purpose and any partial skips.
