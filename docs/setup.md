# Setup

> 日本語版: [setup_JA.md](setup_JA.md)

This guide explains how to create a new wxp plugin starting from `wrac-plugin-template`.

## Prerequisites

### Building CLAP only

- Rust (latest stable)
- Node.js (npm)

### Building VST3 / AU / Standalone as well

To generate VST3 / AU / Standalone using clap-wrapper, the following are additionally required.

**macOS:**
- Xcode or Xcode Command Line Tools
- CMake (3.15 or later recommended)

**Windows:**
- Visual Studio 2022 (with C++ build tools)
- CMake (3.15 or later recommended)

**Linux:**
- CLAP only is supported at this time.

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

### 2. Bulk Replace Identifiers

Several kinds of identifiers are scattered throughout the repository.
Use your IDE's find-and-replace, `rg`, or an LLM agent to search all files and replace them all at once.

**Replacement table:**

| Kind | Current value | Example replacement |
|------|--------------|---------------------|
| Rust crate name | `wxp_example_gain_plugin` | `my_plugin` |
| Plugin display name | `WXP Example Gain` | `My Plugin` |
| Plugin ID (reverse-domain recommended) | `com.novo-notes.wxp-example-gain` | `com.your-company.my-plugin` |
| kebab-case name in GUI / scripts / etc. | `wxp-example-gain-plugin` | `my-plugin` |

> **Important:** The plugin ID must be globally unique. It cannot be changed once published.

**Steps:**

Check the target files and remaining count.

Example using rg:

```sh
rg --hidden "wxp_example_gain_plugin|WXP Example Gain|com\.novo-notes\.wxp-example-gain|wxp-example-gain-plugin" \
    --glob '!node_modules' --glob '!dist' --glob '!*.lock' \
    --glob '!package-lock.json' --glob '!*.zip' --glob '!docs/setup.md'
```

Once confirmed, **replace all occurrences** according to the table above.
Re-run the same command after replacing and verify the output is zero matches.

### 3. Build & Install

Run the following from the repository root.

```sh
cd /path/to/my_plugin
./script/build_and_install.sh
```

The built plugin will be installed to the following directories:

| OS | Install path |
|----|-------------|
| macOS | `~/Library/Audio/Plug-Ins/CLAP/` |
| Windows | `%LOCALAPPDATA%/Programs/Common/CLAP/` |
| Linux | `~/.clap/` |

VST3 / AU are installed at the same time.

### 4. Verify

Debug builds fetch GUI resources from the Vite dev server (`localhost:5173`).
Before launching the plugin in your DAW, start the dev server with the following commands.

```sh
cd /path/to/my_plugin/src-gui
npm install
npm run dev
```

Launch your DAW and try inserting the plugin.
Some DAWs may require a plugin rescan.
The GUI supports hot reload — try editing the HTML files.

### 5. Debug

Attaching a debugger to a DAW can be difficult, so we recommend debugging as a standalone application first.
In VS Code, select the "Debug gain plugin standalone" configuration and run it.

> **Note:** Audio feedback is present in standalone mode. **Use headphones.**

## Next Steps

Read [architecture.md](architecture.md) to learn about the thread model, communication flow, and parameter change flow.

For wxp usage, see the [wxp README](https://github.com/novonotes/wxp/tree/main/crates/wxp).
