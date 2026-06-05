# WRAC Plugin Template

A template for implementing audio plugins with the WRAC stack.
You can copy this repository as a starting point for new projects.

> 日本語版: [README_JA.md](README_JA.md)

<img width="500" alt="wrac_gain" src="https://github.com/user-attachments/assets/4797b197-79ce-42d5-ab97-871eb3913db7" />


# What is the WRAC Stack?

The WRAC stack is a technology stack for audio plugin development, built around three core components: **Webview, Rust Audio, and CLAP**.

**W** (WebView): User interface implementation using HTML/CSS/JS.

**RA** (Rust Audio): Audio signal processing implementation in Rust.

**C** (CLAP): Interface with host applications via the CLever Audio Plug-in standard.

## Why WRAC?

Audio plugins have requirements that ordinary desktop WebView apps do not have: support for many DAWs and plugin formats, cooperative behavior with host applications, and hard real-time requirements on the audio thread.

It took our team a lot of iteration to satisfy those requirements with a WebView + Rust architecture.
Other developers should not have to repeat the same trial and error. With this template, you can start from working code that NovoNotes uses in production.

## Contents

The code in this repository implements a simple plugin called WRAC Gain.
It is also structured so it can be used as a template.

- CLAP plugin implementation in Rust using [clap-sys](https://github.com/micahrj/clap-sys)
- WebView GUI implementation using [wxp](https://github.com/novonotes/wxp)
- VST3 / AU plugin builds and a development standalone app via [clap-wrapper](https://github.com/free-audio/clap-wrapper)

## Quick Start

Want to try the bundled WRAC Gain plugin before building your own? Follow the minimal steps below.
With just Rust and Node.js you should be able to build CLAP.
For the prerequisites to build VST3 / AU or the development standalone app, see the [Setup doc](docs/setup.md#prerequisites).

```sh
# Clone the template. Add --recursive if you plan to build VST3 / AU or the development standalone app.
git clone https://github.com/novonotes/wrac-plugin-template.git
cd wrac-plugin-template

# Build and install the plugin
# Change the --target argument if you need AU or VST3
cargo xtask install --target=clap

# Debug builds load the GUI from the Vite dev server, so start it before launching your DAW
cd plugins/wrac-gain/src-gui
npm install
npm run dev
```

Then launch your DAW and insert **WRAC Gain** (a plugin rescan may be required).

To build your own plugin based on this template, see [Setup](docs/setup.md).

## FAQ

### Why WebView instead of a GPU-native UI stack?

For production plugins, predictability was the main reason. The web platform is mature, widely understood, and already has a known set of tradeoffs in desktop and plugin UI contexts. GPU-native UI stacks such as wgpu are promising, but there is still less production evidence to rely on inside DAW-hosted plugin environments.

### Is this a framework?

No. This repository is an implementation example and starting point, not a general-purpose framework. As a result, it does not provide a broad high-level API, and the adapter layers are intentionally kept thin. Adapting it to your own project should usually be straightforward. For the same reason, future changes will not provide API backwards compatibility or migration support.

### Can I use this for commercial plugins?

Yes. This repository is licensed under the MIT License, which permits commercial use. Open-source, freeware, and commercial releases built from this template are all welcome.

### What about AAX and AUv3 support?

AAX and AUv3 support are ongoing. `clap-wrapper` already has AAX support on its `next` branch, and NovoNotes is using it for internal macOS AAX builds. AUv3 support also has open `clap-wrapper` pull requests, but NovoNotes has not verified it yet. This template currently supports CLAP / VST3 / AU as plugin formats, plus a development-only standalone app target.

## Build

Common commands:

```bash
# Debug build for all plugin formats and the development standalone app
cargo xtask build
# Release build for all plugin formats and the development standalone app
cargo xtask build --release
# Debug build for VST3 only
cargo xtask build --target=vst3
# Release build for AU
cargo xtask build --target=au --release
# Build and validate plugins
cargo xtask validate
# Build and install plugins
cargo xtask install
```

`cargo xtask validate` runs WRAC production-readiness checks before external format validators.
For the check list and disable format, see [Production-Readiness Checks](docs/production-readiness-checks.md).
For source-level product implementation review, see [code-review-checklist.md](code-review-checklist.md).

Build and launch the development standalone app:

```bash
cargo xtask launch
```

The standalone app is a lightweight development and smoke-test host. It is not a release plugin format or a shipping artifact.

Supported plugin formats:

| OS | Supported formats |
|----|---------------------------|
| macOS | CLAP / VST3 / AU |
| Windows | CLAP / VST3 |
| Linux | CLAP / VST3 |

For build/debug commands, the `--target` option also accepts `standalone` as a development-only target.

For detailed usage:

```bash
# Overall help
cargo xtask --help
# Subcommand help
cargo xtask build --help
```

## Reference

For known DAW compatibility status, see the [DAW Compatibility Matrix](https://github.com/novonotes/wrac-plugin-template/wiki/DAW-Compatibility-Matrix).

For usage of the wxp crate, see the [wxp README](https://github.com/novonotes/wxp/tree/main/crates/wxp).

For additional plugin examples built from this template, see [wrac-examples](https://github.com/novonotes/wrac-examples).
