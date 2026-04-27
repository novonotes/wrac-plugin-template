//! WXP Example Gain Plugin
//!
//! A sample CLAP audio plugin built with wxp (WebView eXtension Platform).
//! A simple effect plugin that applies a gain (volume multiplier) to the input signal.
//!
//! ## Module layout
//! - `plugin` : Plugin definition, shared state, and command handler registration
//! - `audio`  : Real-time audio processing (runs on the audio thread)
//! - `params` : CLAP parameter exposure and host parameter synchronization
//! - `gui`    : GUI creation and resize management via wxp WebView

mod audio;
mod gui;
mod params;
mod plugin;

use std::ffi::CStr;

use clack_plugin::{clack_export_entry, entry::prelude::*};
use plugin::WxpExampleGainPluginFactory;

/// Entry point for the CLAP plugin.
/// When the host (DAW) loads the plugin's shared library, this type is instantiated first.
/// Entry is the top-level struct that manages the entire plugin lifecycle.
pub struct WxpExampleGainEntry {
    /// PluginFactoryWrapper is a wrapper provided by clack that exposes
    /// a custom PluginFactoryImpl to the host.
    plugin_factory: PluginFactoryWrapper<WxpExampleGainPluginFactory>,
}

impl Entry for WxpExampleGainEntry {
    /// Called exactly once immediately after the host loads the plugin.
    /// `_bundle_path` receives the path to the plugin file (.clap).
    fn new(_bundle_path: Option<&CStr>) -> Result<Self, EntryLoadError> {
        // Initialize the RunLoop on the main thread.
        // Because wxp's WebView and command handlers operate on the main thread (= RunLoop),
        // init() must be called as early as possible during plugin startup.
        // init/deinit use reference counting, so multiple calls are safe.
        novonotes_run_loop::RunLoop::init().map_err(|_| EntryLoadError)?;

        Ok(Self {
            plugin_factory: PluginFactoryWrapper::new(WxpExampleGainPluginFactory::new()),
        })
    }

    /// Register the factories provided by this plugin with the host.
    /// A single Entry can expose multiple plugin factories.
    fn declare_factories<'a>(&'a self, builder: &mut EntryFactories<'a>) {
        builder.register_factory(&self.plugin_factory);
    }
}

impl Drop for WxpExampleGainEntry {
    fn drop(&mut self) {
        // deinit() is the counterpart to init(). Entry being dropped means the plugin is unloaded.
        novonotes_run_loop::RunLoop::deinit();
    }
}

// Export the entry point symbol so the CLAP host can detect the plugin.
// This macro generates the global `clap_entry` symbol.
clack_export_entry!(WxpExampleGainEntry);

/// Some hosts cannot detect the `clap_entry` symbol directly,
/// so this function provides a fallback that returns the entry descriptor explicitly.
#[unsafe(no_mangle)]
pub extern "C" fn get_clap_entry() -> EntryDescriptor {
    clap_entry
}
