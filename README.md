# WRAC Plugin Template

A template for implementing audio plugins with the WRAC stack.
You can copy this repository as a starting point for new projects.

> 日本語版: [README_JA.md](README_JA.md)

# What is the WRAC Stack?

The WRAC stack is a technology stack for audio plugin development, built around three core components: **Webview, Rust Audio, and CLAP**.

**W** (WebView): User interface implementation using HTML/CSS/JS.

**RA** (Rust Audio): Audio signal processing implementation in Rust.

**C** (CLAP): Interface with host applications via the CLever Audio Plug-in standard.


## Contents

- WebView GUI implementation using [wxp](https://github.com/novonotes/wxp)
- CLAP plugin implementation in Rust using [clack](https://github.com/prokopyl/clack)
- VST3 and AU plugin builds via [clap-wrapper](https://github.com/free-audio/clap-wrapper)


## Setting Up a New Project

To create a new wxp plugin based on this repository, see [Setup](docs/setup.md).


## Give it a spin?

his template comes with a simple Gain plugin pre-implemented. Try loading it up and let us know how it works in your DAW! 
Even a quick comment like **"Works on Logic Pro 10.7"** is incredibly helpful for the community.

Feel free to drop a quick note here:
👉 [DAW Compatibility Reports](https://github.com/novonotes/wrac-plugin-template/discussions/6)

## Reference

For details on the thread model, communication flow, and parameter change flow, see [docs/architecture.md](docs/architecture.md).

For usage of the wxp crate, see the [wxp README](https://github.com/novonotes/wxp/tree/main/crates/wxp).

For known DAW compatibility status, see the [DAW Compatibility Matrix](https://github.com/novonotes/wrac-plugin-template/wiki/DAW-Compatibility-Matrix).
