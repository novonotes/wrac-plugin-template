# wrac_wxp_gui

`wrac_wxp_gui` is a helper crate that connects `wrac_clap_adapter`'s `PluginGuiExtension` with the wxp WebView runtime.

It has two responsibilities: converting the host window handle provided by `wrac_clap_adapter` into `raw-window-handle` types for passing to wxp, and holding a WebView runtime (which can only be operated from a specific thread) on the host UI thread. Interaction with the CLAP C ABI is the responsibility of `wrac_clap_adapter`, not this crate.

## Assumptions

- `set_parent()` fixes the UI thread, and the GUI runtime is created on that thread when `show()` is called
- A single host UI thread per process is assumed
- Hosts that use multiple UI threads are treated as unsupported and will fail
- Floating windows are not handled by this helper
- This is part of an implementation example, not a general-purpose framework. Future changes will not provide API backwards compatibility or migration support.

## Reference
For wxp crate usage, see the [wxp README](https://github.com/novonotes/wxp/tree/main/crates/wxp).
