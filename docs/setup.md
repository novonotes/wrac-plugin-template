# Setup

> 日本語版: [setup-ja.md](setup-ja.md)

This guide explains how to create a new wxp plugin starting from `wrac-plugin-template`.

## Prerequisites

### Building CLAP only

- Rust (latest stable)
- Node.js (npm)

### Building VST3 / AU / AAX or the development standalone app

To generate VST3 / AU / AAX using clap-wrapper, or to build the development standalone app, the following are additionally required.

**macOS:**
- Xcode or Xcode Command Line Tools
- CMake (3.15 or later recommended)

**Windows:**
- Visual Studio 2022 (with C++ build tools)
- CMake (3.15 or later recommended)

**Linux:**
- C++ compiler and build tools
- CMake (3.15 or later recommended)
- Development packages for WebKitGTK, GTK 3, GDK X11, and X11

### Debugging

VS Code debug configurations are included.
The [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb) extension is required to use them.

## Creating Your First Plugin

### 1. Repository Setup

Use the `Use this template` button in the upper right of the [wrac-plugin-template](https://github.com/novonotes/wrac-plugin-template) page on GitHub to create a new repository.
After creating it, clone the new repository and initialize the submodules.

```sh
git clone https://github.com/your-org/my-plugin.git
cd my_plugin
git submodule update --init --recursive
```

Submodules are not needed if you are only building CLAP.
The SDK submodules used by clap-wrapper are required when building VST3 / AU, building the development standalone app, or validating VST3 / AU.
AAX builds additionally require the private AAX SDK. Put local AAX paths in `.env`; see [AAX Build and Validation](aax.md).

### 2. Configure Plugin Identity

Plugin identity is centralized in `plugins/wrac-gain/src-plugin/wrac-plugin.toml`.
Edit that manifest instead of duplicating host-visible IDs in Rust code or Cargo metadata.

> **Important:** The plugin ID must be globally unique. It cannot be changed once published.
> AUv2 `auv2_type`, `auv2_subtype`, and `auv2_manufacturer_code` must each be exactly 4 ASCII bytes.
> `clap_features` must match the plugin's real audio/MIDI behavior because CLAP hosts read it directly.
> `supported_formats` is the product policy used by default `xtask` build/install/validate commands.
> `vst3_subcategories` controls VST3 host browser categories; use Steinberg-style `|`-separated values such as `Fx|Dynamics`.
> `vst3_component_id` must be a stable UUID. Generate it once before release and never change it for the same product.
> `aax_manufacturer_id`, `aax_product_id`, and each AAX stem config `plugin_id` must be stable 4-byte ASCII IDs.
> AAX stem configs should list only the channel layouts the product actually supports.

### 3. Bulk Replace Remaining Identifiers

Several kinds of identifiers are scattered throughout the repository.
Use your IDE's find-and-replace, `rg`, or an LLM agent to search all files and replace them all at once.

**Replacement table:**

| Kind | Current value | Example replacement |
|------|--------------|---------------------|
| WRAC plugin package name (Cargo package) | `wrac_gain_plugin` | `my_plugin` |
| kebab-case name in GUI / scripts / etc. | `wrac-gain-plugin` | `my-plugin` |
| Repository URL in `Cargo.toml` files | `https://github.com/novonotes/wrac-plugin-template` | `https://github.com/your-org/my-plugin` |

### 4. Build & Install

Run the following from the repository root.

```sh
cd /path/to/my_plugin
cargo xtask install
```

`cargo xtask install` builds and installs the selected plugin formats.
For detailed usage of each xtask command, see `cargo xtask --help`.

### Plugin Resources

Plugins can package additional runtime resources by placing files in one of the
resource directories below.

```text
plugins/<plugin>/resources/                 # source-managed resources
target/wrac-plugins/<plugin>/wrac/resources/ # generated resources
```

`xtask` merges these directories into a clean staging directory before packaging.
Source-managed resources are copied first, then generated resources are copied on
top of them. This lets generated files intentionally replace source-managed files
with the same relative path while keeping the final packaged resources identical
across the supported formats.

Resource packaging is currently supported for:

- CLAP on macOS, Windows, and Linux
- VST3 on macOS

If plugin resources are present and an unsupported wrapper format is requested,
`xtask` fails during wrapper configure instead of producing an artifact that is
missing required files. AUv2, AAX, standalone, Windows VST3, and Linux VST3
resource packaging should be implemented in the wrapper layer before enabling
those combinations.

### 5. Verify

Debug builds fetch GUI resources from the Vite dev server (`localhost:5173`).
Before launching the plugin in your DAW, start the dev server with the following commands.
If the WebView cannot connect to the configured URL, the plugin shows a low-level load error
instead of a blank editor so you can see the failed URL and socket error directly.

```sh
cd /path/to/my_plugin/plugins/wrac-gain/src-gui
npm install
npm run dev
```

For release builds, `src-plugin/build.rs` zips the sibling `src-gui/dist` and embeds it in the plugin binary, so the dev server is not needed.

Launch your DAW and try inserting the plugin.
Some DAWs may require a plugin rescan.
The GUI supports hot reload — try editing the HTML files.

### 6. Debug

Attaching a debugger to a DAW can be difficult, so we recommend debugging with the development standalone app first.
In VS Code, select the "Debug gain plugin standalone" configuration and run it.

> **Note:** Audio feedback is present in standalone mode. **Use headphones.**

### Reading Debug Logs

Debug build logs are written to `.log/<plugin_name> Latest.log`.
To follow the log, use `tail -f ".log/<plugin_name> Latest.log"` on macOS/Linux, or `Get-Content ".log\<plugin_name> Latest.log" -Wait` in Windows PowerShell.
For details about logging, see the `crates/wrac_log` directory.
