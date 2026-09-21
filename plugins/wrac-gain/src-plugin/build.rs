//! Build script that generates host-visible plugin descriptors and bundles
//! `src-gui/dist` into a single zip for release builds.

use std::env;
use std::path::PathBuf;

use wrac_build::{
    FrontendBundleConfig, PluginDescriptorCodegenConfig, build_frontend_bundle,
    generate_plugin_descriptors,
};

fn main() {
    generate_plugin_descriptors(PluginDescriptorCodegenConfig::default())
        .expect("failed to generate WRAC plugin descriptors");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let gui_dist_dir = manifest_dir
        .parent()
        .expect("src-plugin must have a parent directory")
        .join("src-gui")
        .join("dist");
    build_frontend_bundle(FrontendBundleConfig {
        dist_dir: gui_dist_dir,
        output_file_name: "wrac_gain_plugin_gui.zip",
        rerun_if_changed: &[
            "../src-gui/index.html",
            "../src-gui/src",
            "../src-gui/package.json",
            "../src-gui/vite.config.ts",
        ],
        missing_dist_build_command:
            "Run `npm install && npm run build` in src-gui before release builds.",
    })
    .expect("failed to create frontend zip");
}
