# clap_wrapper_builder

> 日本語版: [README_JA.md](README_JA.md)

Reusable CMake project that wraps a prebuilt CLAP plugin **static library**
into VST3 / AUv2 / AAX / Standalone bundles.

## Contents

- `CMakeLists.txt` - The wrapper build definition. The full input-variable
  reference lives in the header comment at the top of this file.
- `clap-wrapper` - Vendored wrapper source, tracked as a git subtree.
- `clap` / `vst3sdk` / `AudioUnitSDK` - Dependency SDKs, checked out as git
  submodules.

## Usage

Call this project directly, passing at least the static library to wrap:

```bash
cmake -S clap_wrapper_builder -B build \
  -DCLAP_WRAPPER_BUILDER_TARGET_LIB=/path/to/libyour_plugin.a \
  -DCLAP_WRAPPER_BUILDER_OUTPUT_NAME="Your Plugin"
cmake --build build
```

See the `CLAP_WRAPPER_BUILDER_*` variable list in `CMakeLists.txt` for
output naming, format toggles (VST3/AUv2/AAX/Standalone), and artifact
staging.

> Building this template's own plugin? Use `cargo xtask build` from the
> repository root instead — see the root `README.md`.

## AAX

AAX is off by default and requires the Avid AAX SDK. Enable it with
`-DCLAP_WRAPPER_BUILDER_BUILD_AAX=ON` once the SDK is available.

## Maintaining the clap-wrapper subtree

`clap-wrapper` is vendored as ordinary files, not as an active submodule, but it
keeps git-subtree metadata in history. The subtree tracks upstream release tags,
not `next`: `next` carries unreleased features that widen the change set of an
update. Put project-specific follow-up changes in later, small logical commits.
An unreleased upstream fix that is needed before the next release is
cherry-picked on its own with `-x`, so the next tag update can recognize it.

Update command example:

```bash
git subtree pull \
  --prefix=clap_wrapper_builder/clap-wrapper \
  https://github.com/free-audio/clap-wrapper.git \
  v0.17.0 \
  -m "Update clap-wrapper subtree to v0.17.0"
```

When local patches conflict broadly with an update, set the subtree content to
the plain upstream tag in the merge commit and reapply each local patch as its
own commit on top, naming the commit it was reapplied from.

Cherry-picking a single upstream commit:

```bash
git cherry-pick -x -Xsubtree=clap_wrapper_builder/clap-wrapper <upstream-commit>
```
