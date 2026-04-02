use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn main() {
    println!("cargo:rerun-if-changed=../src-gui/index.html");
    println!("cargo:rerun-if-changed=../src-gui/src");
    println!("cargo:rerun-if-changed=../src-gui/package.json");
    println!("cargo:rerun-if-changed=../src-gui/vite.config.ts");

    if env::var("PROFILE").ok().as_deref() != Some("release") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let gui_dist_dir = manifest_dir
        .parent()
        .expect("src-plugin must have a parent directory")
        .join("src-gui")
        .join("dist");
    let out_zip = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"))
        .join("wxp_example_gain_plugin_gui.zip");

    if !gui_dist_dir.exists() {
        panic!(
            "frontend build output was not found at {}. Run `npm install && npm run build` in wxp-examples/examples/gain_plugin/src-gui before release builds.",
            gui_dist_dir.display()
        );
    }

    create_zip(&gui_dist_dir, &out_zip).expect("failed to create frontend zip");
}

fn create_zip(src_dir: &Path, out_zip: &Path) -> io::Result<()> {
    let file = File::create(out_zip)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    add_directory_contents(src_dir, src_dir, &mut zip, options)?;
    zip.finish()?;
    Ok(())
}

fn add_directory_contents(
    root: &Path,
    current: &Path,
    zip: &mut ZipWriter<File>,
    options: SimpleFileOptions,
) -> io::Result<()> {
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("walked path must be inside root");
        let zip_path = relative.to_string_lossy().replace('\\', "/");

        if path.is_dir() {
            zip.add_directory(format!("{zip_path}/"), options)?;
            add_directory_contents(root, &path, zip, options)?;
            continue;
        }

        zip.start_file(zip_path, options)?;
        let bytes = fs::read(&path)?;
        zip.write_all(&bytes)?;
    }

    Ok(())
}
