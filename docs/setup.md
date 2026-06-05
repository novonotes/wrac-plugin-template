# Setup

> 日本語版: [setup-ja.md](setup-ja.md)

This guide explains how to create a new wxp plugin starting from `wrac-plugin-template`.

## Prerequisites

### Building CLAP only

- Rust (latest stable)
- Node.js (npm)

### Building VST3 / AU or the development standalone app

To generate VST3 / AU using clap-wrapper, or to build the development standalone app, the following are additionally required.

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

### 2. Configure Plugin Identity

Plugin identity is centralized in the plugin package manifest, initially `plugins/wrac-gain/src-plugin/Cargo.toml`.
Edit `[package.metadata.wrac]` first.

```toml
[package.metadata.wrac]
company_name = "Your Company"
auv2_manufacturer_code = "YrCo"
bundle_name = "My Plugin"

[[package.metadata.wrac.plugins]]
plugin_id = "com.your-company.my-plugin"
plugin_name = "My Plugin"
standalone_name = "My Plugin Standalone"
auv2_type = "aufx"
auv2_subtype = "MyPl"
```

> **Important:** The plugin ID must be globally unique. It cannot be changed once published.
> AUv2 `auv2_type`, `auv2_subtype`, and `auv2_manufacturer_code` must each be exactly 4 ASCII bytes.

### 3. Bulk Replace Remaining Identifiers

Several kinds of identifiers are scattered throughout the repository.
Use your IDE's find-and-replace, `rg`, or an LLM agent to search all files and replace them all at once.

**Replacement table:**

| Kind | Current value | Example replacement |
|------|--------------|---------------------|
| WRAC plugin package name (Cargo package) | `wrac_gain_plugin` | `my_plugin` |
| kebab-case name in GUI / scripts / etc. | `wrac-gain-plugin` | `my-plugin` |
| Repository URL in `Cargo.toml` files | `https://github.com/novonotes/wrac-plugin-template` | `https://github.com/your-org/my-plugin` |

The repository URL points to this template by default. After generating a new project, update it to your own repository if you publish the crate metadata.

**Steps:**

Check the target files and remaining count.

Example using rg:

```sh
rg --hidden "wrac_gain_plugin|WRAC Gain|com\.your-company\.wrac-gain|wrac-gain-plugin" \
    --glob '!node_modules' --glob '!dist' --glob '!*.lock' \
    --glob '!package-lock.json' --glob '!*.zip' \
    --glob '!docs/setup.md' --glob '!docs/setup-ja.md'

rg --hidden 'repository = "https://github.com/novonotes/wrac-plugin-template"' --glob 'Cargo.toml'
```

Once confirmed, **replace all occurrences** according to the table above.
Re-run the same commands after replacing and verify the output is zero matches.

### 4. Build & Install

Run the following from the repository root.

```sh
cd /path/to/my_plugin
cargo xtask install
```

`cargo xtask install` builds the selected plugin formats before installing them.
Use `-p/--package` with the Cargo package name when the workspace contains multiple WRAC plugin packages.
`cargo xtask install` installs to user-local paths by default.
Use `cargo xtask install --scope=system` for hosts that only scan system-wide plugin folders.
The `--target` option accepts `clap`, `vst3`, and `au` as comma-separated values.

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

The standalone app is a lightweight development host, not a release plugin format or shipping artifact.

> **Note:** Audio feedback is present in standalone mode. **Use headphones.**

### Reading Debug Logs

Debug build logs are written to `.log/<plugin_name> Latest.log`.
To follow the log, use `tail -f ".log/<plugin_name> Latest.log"` on macOS/Linux, or `Get-Content ".log\<plugin_name> Latest.log" -Wait` in Windows PowerShell.
