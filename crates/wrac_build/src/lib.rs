use std::{
    env,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

pub struct FrontendBundleConfig<'a> {
    pub dist_dir: PathBuf,
    pub output_file_name: &'a str,
    pub rerun_if_changed: &'a [&'a str],
    pub missing_dist_build_command: &'a str,
}

pub fn build_frontend_bundle(config: FrontendBundleConfig<'_>) -> io::Result<Option<PathBuf>> {
    for path in config.rerun_if_changed {
        println!("cargo:rerun-if-changed={path}");
    }

    if env::var("PROFILE").ok().as_deref() != Some("release") {
        return Ok(None);
    }

    if !config.dist_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "frontend build output was not found at {}. {}",
                config.dist_dir.display(),
                config.missing_dist_build_command
            ),
        ));
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|error| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("OUT_DIR is not available: {error}"),
        )
    })?);
    let output_path = out_dir.join(config.output_file_name);
    create_zip(&config.dist_dir, &output_path)?;
    Ok(Some(output_path))
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
