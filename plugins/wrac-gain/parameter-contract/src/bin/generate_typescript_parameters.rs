use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use wrac_gain_parameter_contract::render_typescript_parameter_contract;

fn main() -> io::Result<()> {
    let output_path = output_path();
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, render_typescript_parameter_contract())
}

fn output_path() -> PathBuf {
    env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_output_path)
}

fn default_output_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("src-gui")
        .join("src")
        .join("generated")
        .join("parameters.ts")
}
