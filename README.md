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
- VST3 / AU / Standalone builds via [clap-wrapper](https://github.com/free-audio/clap-wrapper)

## Quick Start

Want to try the bundled WRAC Gain plugin before building your own? Follow the minimal steps below.
With just Rust and Node.js you should be able to build CLAP.
For the prerequisites to build VST3 / AU / Standalone, see the [Setup doc](docs/setup.md#prerequisites).

```sh
# Clone with submodules (submodules are only needed for VST3 / AU / Standalone)
git clone --recursive https://github.com/novonotes/wrac-plugin-template.git
cd wrac-plugin-template

# Build and install the plugin
# Change the --target argument if you need AU or VST3
cargo xtask build --target=clap
cargo xtask install --target=clap

# Debug builds load the GUI from the Vite dev server, so start it before launching your DAW
cd plugins/wrac-gain/src-gui
npm install
npm run dev
```

Then launch your DAW and insert **WRAC Gain** (a plugin rescan may be required).

If it works, a quick note at [DAW Compatibility Reports](https://github.com/novonotes/wrac-plugin-template/discussions/6) is a big help for the community!

To build your own plugin based on this template, see [Setup](docs/setup.md).

## FAQ

### Why WebView instead of a GPU-native UI stack?

For production plugins, predictability was the main reason. The web platform is mature, widely understood, and already has a known set of tradeoffs in desktop and plugin UI contexts. GPU-native UI stacks such as wgpu are promising, but there is still less production evidence to rely on inside DAW-hosted plugin environments.

### Is this a framework?

No. This repository is an implementation example and starting point, not a general-purpose framework. As a result, it does not provide a broad high-level API, and the adapter layers are intentionally kept thin. Adapting it to your own project should usually be straightforward. For the same reason, future changes will not provide API backwards compatibility or migration support.

### Can I use this for commercial plugins?

Yes. This repository is licensed under the MIT License, which permits commercial use. Open-source, freeware, and commercial releases built from this template are all welcome.

### What about AAX and AUv3 support?

AAX and AUv3 support are ongoing. `clap-wrapper` already has AAX support on its `next` branch, and NovoNotes is using it for internal macOS AAX builds. AUv3 support also has open `clap-wrapper` pull requests, but NovoNotes has not verified it yet. This template still exposes only CLAP / VST3 / AU / Standalone as supported `xtask` targets.

## Build

Common commands:

```bash
# Debug build for all formats
cargo xtask build
# Release build for all formats
cargo xtask build --release
# Debug build for VST3 only
cargo xtask build --target=vst3
# Release build for AU and Standalone
cargo xtask build --target=au,standalone --release
# Validate built plugins
cargo xtask validate
# Install built plugins
cargo xtask install
```

Launch the standalone app after building it:

```bash
cargo xtask build --target=standalone
cargo xtask launch
```

Supported formats:

| OS | Supported formats |
|----|---------------------------|
| macOS | CLAP / VST3 / AU / Standalone |
| Windows | CLAP / VST3 / Standalone |
| Linux | CLAP / VST3 / Standalone |

The `--target` option accepts `clap`, `vst3`, `au`, and `standalone` as comma-separated values.

For detailed usage:

```bash
# Overall help
cargo xtask --help
# Subcommand help
cargo xtask build --help
```

## Built with WRAC

Built a plugin with this template? Share it in the [Showcase Discussion](https://github.com/novonotes/wrac-plugin-template/discussions/43).
Open-source, freeware, and commercial releases are all welcome.

## Reference

For known DAW compatibility status, see the [DAW Compatibility Matrix](https://github.com/novonotes/wrac-plugin-template/wiki/DAW-Compatibility-Matrix).

For usage of the wxp crate, see the [wxp README](https://github.com/novonotes/wxp/tree/main/crates/wxp).

For additional plugin examples built from this template, see [wrac-examples](https://github.com/novonotes/wrac-examples).
