use std::path::Path;

use wrac_xtask::{XtaskConfig, run};

fn main() -> wrac_xtask::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be a direct child of the repository root")
        .to_path_buf();
    run(XtaskConfig {
        wrapper_dir: root.join("clap_wrapper_builder"),
        target_namespace: "wrac-plugins".to_string(),
        root,
    })
}
