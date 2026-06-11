//! Repository-local entry point for `cargo xtask`.
//!
//! This binary only wires the WRAC template's root paths into `wrac_xtask`; the
//! reusable command behavior lives in the `wrac_xtask` crate.

use std::path::Path;

use wrac_xtask::{WracWorkspace, XtaskConfig};

fn main() -> wrac_xtask::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be a direct child of the repository root")
        .to_path_buf();

    let workspace = WracWorkspace::new(XtaskConfig {
        wrapper_dir: root.join("clap_wrapper_builder"),
        target_namespace: "wrac-plugins".to_string(),
        root,
    })?;
    workspace.run(wrac_xtask::command_from_args())
}
