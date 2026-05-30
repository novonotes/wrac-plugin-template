//! Build script that bundles `src-gui/dist` into a single zip and writes it to `OUT_DIR`
//! for release builds.
//!
//! The resulting zip is embedded into the plugin binary via `include_bytes!` in `gui.rs`
//! and served at runtime by the WebView under the `wxp-plugin://` scheme.
//! In debug builds Vite's dev server is used instead, so this script does nothing.

use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn main() {
    // Tell Cargo to rebuild (regenerate the zip) whenever frontend source files change.
    println!("cargo:rerun-if-changed=../src-gui/index.html");
    println!("cargo:rerun-if-changed=../src-gui/src");
    println!("cargo:rerun-if-changed=../src-gui/package.json");
    println!("cargo:rerun-if-changed=../src-gui/vite.config.ts");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let manifest_path = manifest_dir.join("Cargo.toml");
    // Make [package.metadata.wrac] in Cargo.toml the single source of truth for plugin
    // identity. If descriptor, AUv2 codes, GUI, and xtask each held their own values, a
    // template rename could leave some artifacts with the old name, so pass them to Rust
    // as compile-time env vars.
    let metadata = read_wrac_metadata(&manifest_path).expect("failed to read WRAC metadata");
    for (index, plugin) in metadata.plugins.iter().enumerate() {
        println!(
            "cargo:rustc-env=WRAC_PLUGIN_{index}_ID={}",
            plugin.plugin_id
        );
        println!(
            "cargo:rustc-env=WRAC_PLUGIN_{index}_NAME={}",
            plugin.plugin_name
        );
        println!(
            "cargo:rustc-env=WRAC_PLUGIN_{index}_AUV2_TYPE={}",
            plugin.auv2_type
        );
        println!(
            "cargo:rustc-env=WRAC_PLUGIN_{index}_AUV2_SUBTYPE={}",
            plugin.auv2_subtype
        );
    }
    println!(
        "cargo:rustc-env=WRAC_COMPANY_NAME={}",
        metadata.company_name
    );
    println!(
        "cargo:rustc-env=WRAC_AUV2_MANUFACTURER_CODE={}",
        metadata.auv2_manufacturer_code
    );

    // Skip zip creation for debug builds (the Vite dev server is used instead).
    if env::var("PROFILE").ok().as_deref() != Some("release") {
        return;
    }

    let gui_dist_dir = manifest_dir
        .parent()
        .expect("src-plugin must have a parent directory")
        .join("src-gui")
        .join("dist");
    let out_zip =
        PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("wrac_gain_plugin_gui.zip");

    // Fail early if `npm run build` was not run before the release build.
    if !gui_dist_dir.exists() {
        panic!(
            "frontend build output was not found at {}. Run `npm install && npm run build` in src-gui before release builds.",
            gui_dist_dir.display()
        );
    }

    create_zip(&gui_dist_dir, &out_zip).expect("failed to create frontend zip");
}

struct WracMetadata {
    company_name: String,
    auv2_manufacturer_code: String,
    plugins: Vec<WracPluginMetadata>,
}

struct WracPluginMetadata {
    plugin_id: String,
    plugin_name: String,
    auv2_type: String,
    auv2_subtype: String,
}

fn read_wrac_metadata(manifest_path: &Path) -> io::Result<WracMetadata> {
    let manifest = fs::read_to_string(manifest_path)?;
    let company_name = read_toml_string(&manifest, "package.metadata.wrac", "company_name")
        .ok_or_else(|| missing_metadata("company_name"))?;
    let auv2_manufacturer_code =
        read_toml_string(&manifest, "package.metadata.wrac", "auv2_manufacturer_code")
            .ok_or_else(|| missing_metadata("auv2_manufacturer_code"))?;
    validate_four_ascii("auv2_manufacturer_code", &auv2_manufacturer_code)?;
    let plugins = read_plugin_metadata(&manifest)?;
    if plugins.is_empty() {
        return Err(missing_metadata("plugins"));
    }
    for plugin in &plugins {
        validate_four_ascii("auv2_type", &plugin.auv2_type)?;
        validate_four_ascii("auv2_subtype", &plugin.auv2_subtype)?;
    }
    Ok(WracMetadata {
        company_name,
        auv2_manufacturer_code,
        plugins,
    })
}

fn missing_metadata(key: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("missing package.metadata.wrac.{key} in src-plugin/Cargo.toml"),
    )
}

fn read_toml_string(manifest: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line == format!("[{section}]");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((line_key, value)) = line.split_once('=') else {
            continue;
        };
        if line_key.trim() != key {
            continue;
        }
        return parse_toml_basic_string(value.trim());
    }
    None
}

fn read_plugin_metadata(manifest: &str) -> io::Result<Vec<WracPluginMetadata>> {
    let mut plugins = Vec::new();
    let mut current: Option<WracPluginMetadata> = None;
    let mut in_plugins = false;

    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(plugin) = current.take() {
                plugins.push(plugin);
            }
            in_plugins = line == "[[package.metadata.wrac.plugins]]";
            if in_plugins {
                current = Some(WracPluginMetadata {
                    plugin_id: String::new(),
                    plugin_name: String::new(),
                    auv2_type: String::new(),
                    auv2_subtype: String::new(),
                });
            }
            continue;
        }
        if !in_plugins {
            continue;
        }
        let Some((line_key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = parse_toml_basic_string(value.trim()) else {
            continue;
        };
        let plugin = current
            .as_mut()
            .expect("plugin table must create current metadata");
        match line_key.trim() {
            "plugin_id" => plugin.plugin_id = value,
            "plugin_name" => plugin.plugin_name = value,
            "auv2_type" => plugin.auv2_type = value,
            "auv2_subtype" => plugin.auv2_subtype = value,
            _ => {}
        }
    }
    if let Some(plugin) = current {
        plugins.push(plugin);
    }
    for plugin in &plugins {
        if plugin.plugin_id.is_empty()
            || plugin.plugin_name.is_empty()
            || plugin.auv2_type.is_empty()
            || plugin.auv2_subtype.is_empty()
        {
            return Err(missing_metadata("plugins.*"));
        }
    }
    Ok(plugins)
}

fn parse_toml_basic_string(value: &str) -> Option<String> {
    let value = value.strip_prefix('"')?;
    let value = value.strip_suffix('"')?;
    Some(value.replace("\\\"", "\"").replace("\\\\", "\\"))
}

fn validate_four_ascii(key: &str, value: &str) -> io::Result<()> {
    if value.len() == 4 && value.is_ascii() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("package.metadata.wrac.{key} must be exactly 4 ASCII bytes"),
        ))
    }
}

/// Compresses everything under `src_dir` into a deflate-compressed zip and writes it to `out_zip`.
fn create_zip(src_dir: &Path, out_zip: &Path) -> io::Result<()> {
    let file = File::create(out_zip)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    add_directory_contents(src_dir, src_dir, &mut zip, options)?;
    zip.finish()?;
    Ok(())
}

/// Recursively walks a directory and adds its contents to the zip.
///
/// Entries are sorted by path before processing to make the build deterministic
/// (same inputs always produce the same output).
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
        // Normalise internal zip paths to the OS-independent `/` separator (Windows fix).
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
