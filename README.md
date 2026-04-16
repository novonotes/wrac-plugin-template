# wxp-gain-example

A reference implementation of a gain plugin built with [wxp](https://github.com/novonotes/wxp).
You can also copy this repository as a starting point for new projects.

> 日本語版: [README_JA.md](README_JA.md)

## Contents

| Path | Description |
|------|-------------|
| `src-plugin` | CLAP plugin core written in Rust |
| `src-gui` | GUI written in TypeScript + HTML/CSS |
| `script` | Build and installation scripts |
| `clap_wrapper_builder` | Helper build environment that wraps CLAP into VST3 / AUv2 / Standalone |

## Setting Up a New Project

To create a new wxp plugin based on this repository, see [Setup](docs/setup.md).

## Architecture

For details on the thread model, communication flow, and parameter change flow, see [docs/architecture.md](docs/architecture.md).

For usage of the wxp crate, see the [wxp README](https://github.com/novonotes/wxp/tree/main/crates/wxp).
