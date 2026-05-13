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

    // debug build 時は zip を作らない (Vite dev server を使うため)。
    if env::var("PROFILE").ok().as_deref() != Some("release") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
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
