//! release build 時に `src-gui/dist` を 1 つの zip にまとめて `OUT_DIR` に出力する build script。
//!
//! 出力された zip は `gui.rs` で `include_bytes!` により plugin バイナリへ
//! 埋め込まれ、実行時に WebView が `wxp-plugin://` scheme として配信する。
//! debug build では Vite dev server を使うので、このスクリプトは何もしない。

use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn main() {
    // frontend ソースが変わったら再ビルドする (zip を作り直す) ように Cargo に伝える。
    println!("cargo:rerun-if-changed=../src-gui/index.html");
    println!("cargo:rerun-if-changed=../src-gui/src");
    println!("cargo:rerun-if-changed=../src-gui/package.json");
    println!("cargo:rerun-if-changed=../src-gui/vite.config.ts");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let manifest_path = manifest_dir.join("Cargo.toml");
    // Cargo.toml の [package.metadata.wrac] を plugin identity の SoT にする。
    // descriptor / AUv2 codes / GUI / xtask が別々の値を持つと、テンプレートを
    // 改名した時に一部 artifact だけ古い名前で出るため、Rust へ compile-time env で渡す。
    let metadata = read_wrac_metadata(&manifest_path).expect("failed to read WRAC metadata");
    println!("cargo:rustc-env=WRAC_PLUGIN_ID={}", metadata.plugin_id);
    println!("cargo:rustc-env=WRAC_PLUGIN_NAME={}", metadata.plugin_name);
    println!(
        "cargo:rustc-env=WRAC_COMPANY_NAME={}",
        metadata.company_name
    );
    println!("cargo:rustc-env=WRAC_AUV2_TYPE={}", metadata.auv2_type);
    println!(
        "cargo:rustc-env=WRAC_AUV2_SUBTYPE={}",
        metadata.auv2_subtype
    );
    println!(
        "cargo:rustc-env=WRAC_AUV2_MANUFACTURER_CODE={}",
        metadata.auv2_manufacturer_code
    );

    // debug build 時は zip を作らない (Vite dev server を使うため)。
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

    // release build の前に `npm run build` を回し忘れていた場合は早めに止める。
    if !gui_dist_dir.exists() {
        panic!(
            "frontend build output was not found at {}. Run `npm install && npm run build` in src-gui before release builds.",
            gui_dist_dir.display()
        );
    }

    create_zip(&gui_dist_dir, &out_zip).expect("failed to create frontend zip");
}

struct WracMetadata {
    plugin_id: String,
    plugin_name: String,
    company_name: String,
    auv2_type: String,
    auv2_subtype: String,
    auv2_manufacturer_code: String,
}

fn read_wrac_metadata(manifest_path: &Path) -> io::Result<WracMetadata> {
    let manifest = fs::read_to_string(manifest_path)?;
    let plugin_id = read_toml_string(&manifest, "package.metadata.wrac", "plugin_id")
        .ok_or_else(|| missing_metadata("plugin_id"))?;
    let plugin_name = read_toml_string(&manifest, "package.metadata.wrac", "plugin_name")
        .ok_or_else(|| missing_metadata("plugin_name"))?;
    let company_name = read_toml_string(&manifest, "package.metadata.wrac", "company_name")
        .ok_or_else(|| missing_metadata("company_name"))?;
    let auv2_type = read_toml_string(&manifest, "package.metadata.wrac", "auv2_type")
        .ok_or_else(|| missing_metadata("auv2_type"))?;
    let auv2_subtype = read_toml_string(&manifest, "package.metadata.wrac", "auv2_subtype")
        .ok_or_else(|| missing_metadata("auv2_subtype"))?;
    let auv2_manufacturer_code =
        read_toml_string(&manifest, "package.metadata.wrac", "auv2_manufacturer_code")
            .ok_or_else(|| missing_metadata("auv2_manufacturer_code"))?;
    validate_four_ascii("auv2_type", &auv2_type)?;
    validate_four_ascii("auv2_subtype", &auv2_subtype)?;
    validate_four_ascii("auv2_manufacturer_code", &auv2_manufacturer_code)?;
    Ok(WracMetadata {
        plugin_id,
        plugin_name,
        company_name,
        auv2_type,
        auv2_subtype,
        auv2_manufacturer_code,
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

/// `src_dir` 以下を丸ごと deflate 圧縮の zip にまとめて `out_zip` に書き出す。
fn create_zip(src_dir: &Path, out_zip: &Path) -> io::Result<()> {
    let file = File::create(out_zip)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    add_directory_contents(src_dir, src_dir, &mut zip, options)?;
    zip.finish()?;
    Ok(())
}

/// directory を再帰的に walk して zip へ追加する。
///
/// build を decisive (= 同じ入力なら常に同じ出力) にするため、entry を
/// path 順に sort してから処理する。
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
        // zip 内部は OS 非依存の `/` 区切りに揃える (Windows 対策)。
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
