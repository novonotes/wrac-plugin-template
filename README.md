# WRAC Plugin Template

⚠️ Currently in **pre-release testing** for the official launch.
- Feedback is welcome via issues, discussions, DM, email, or any other means.
- The scope is currently intentionally limited to acquaintances and stakeholders.
- Official launch is planned for early May 2026. Your shares and support at that time would be greatly appreciated!

---

A template for implementing audio plugins with the WRAC stack.
You can copy this repository as a starting point for new projects.

> 日本語版: [README_JA.md](README_JA.md)

# What is the WRAC Stack?

The WRAC stack is a technology stack for audio plugin development, built around three core components: **Webview, Rust Audio, and CLAP**.

**W** (WebView): User interface implementation using HTML/CSS/JS.

**RA** (Rust Audio): Audio signal processing implementation in Rust.

**C** (CLAP): Interface with host applications via the CLever Audio Plug-in standard.


## Contents

The code in this repository implements a simple plugin called WRAC Gain.
It is also structured so it can be used as a template.

- WebView GUI implementation using [wxp](https://github.com/novonotes/wxp)
- CLAP plugin implementation in Rust using `clap-sys`
- VST3 and AU plugin builds via [clap-wrapper](https://github.com/free-audio/clap-wrapper)

## Build

```bash
cargo xtask build
cargo xtask build --release
cargo xtask build --target=vst3
cargo xtask build --target=au,standalone
cargo xtask validate
cargo xtask install
```

`cargo xtask build` builds every target supported by the current OS:
CLAP/VST3/AU/standalone on macOS and CLAP/VST3/standalone on Windows and Linux.
Use `build --target` with a comma-separated list of `clap`, `vst3`,
`au`, and `standalone` to build a smaller set. `install` and `uninstall` accept
plugin formats only: `clap`, `vst3`, and `au`. `install --scope` accepts `user`
or `system`; `uninstall --scope` accepts `user`, `system`, or `all`.

Plugin artifacts are staged under `target/wrac/plugins/<profile>/`, and
standalone apps are staged under `target/wrac/standalone/<profile>/`. On macOS,
`cargo xtask validate` runs clap-validator, the VST3 validator, and
`auval -v aufx WtGn YrCo`. On Windows and Linux, validation runs clap-validator
and the VST3 validator. CLAP validation downloads clap-validator 0.3.2 into
`target/tools` when needed. AU validation installs the built component under
`~/Library/Audio/Plug-Ins/Components/` and fails if the same bundle exists under
`/Library/Audio/Plug-Ins/Components/`.


## Setting Up a New Project

To create a new wxp plugin based on this repository, see [Setup](docs/setup.md).


## Give it a spin?

This template comes with a simple Gain plugin pre-implemented. Try loading it up and let us know how it works in your DAW!
Even a quick comment like **"Works on Logic Pro 10.7"** is incredibly helpful for the community.

Feel free to drop a quick note here:
👉 [DAW Compatibility Reports](https://github.com/novonotes/wrac-plugin-template/discussions/6)

## Reference

For usage of the wxp crate, see the [wxp README](https://github.com/novonotes/wxp/tree/main/crates/wxp).

For known DAW compatibility status, see the [DAW Compatibility Matrix](https://github.com/novonotes/wrac-plugin-template/wiki/DAW-Compatibility-Matrix).
