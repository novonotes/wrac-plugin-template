use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use wrac_gain_parameter_contract::render_typescript_parameter_contract;

fn main() -> io::Result<()> {
    let output_path = output_path();
    let expected = render_typescript_parameter_contract();
    let actual = fs::read_to_string(&output_path)?;
    if actual == expected {
        return Ok(());
    }
    Err(stale_generated_file_error(&output_path))
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

fn stale_generated_file_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "{} is stale. Run `cargo run -p wrac_gain_parameter_contract --bin generate_typescript_parameters`.",
            path.display()
        ),
    )
}
