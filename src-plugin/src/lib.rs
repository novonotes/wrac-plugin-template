//! WRAC Gain plugin — the entry-point crate for this template.
//!
//! A minimal gain (volume) plugin. `src-plugin` contains only product-specific logic
//! (parameters, state, DSP, GUI); the messy CLAP ABI and FFI invariants are encapsulated
//! in the separate `wrac_clap_adapter` crate. When building a plugin from this template,
//! you'll mostly be editing the files in this crate.
//!
//! File layout:
//! - `plugin.rs`   : the plugin contract as seen by the host; details live under `plugin/`.
//! - `state.rs`    : lock-free state shared by the audio thread, GUI, and host.
//! - `audio.rs`    : DSP running on the audio thread (just applies gain in this sample).
//! - `gui.rs`      : WebView-based GUI integration; runtime/notifier live under `gui/`.
//! - `commands.rs` : Rust commands callable from the WebView frontend; resize helpers under `commands/`.
//!
//! Logging goes through the `log` facade and is initialized through the shared `wrac_log`
//! crate.

// In debug builds, swap in a custom allocator to detect allocations on the audio
// thread immediately (see process() in audio.rs for usage).
#[cfg(debug_assertions)]
use assert_no_alloc::*;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC_DISABLER: AllocDisabler = AllocDisabler;

mod audio;
mod commands;
mod gui;
mod plugin;
mod state;

// Export the CLAP entry point. The adapter owns the C ABI and calls the safe Rust entry.
wrac_clap_adapter::export_clap_entry! {
    entry: &crate::plugin::PLUGIN_ENTRY,
}
