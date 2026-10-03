use super::*;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn init_test_is_idempotent() {
    init_test();
    init_test();
}

#[test]
fn test_name_and_timestamp_are_available() {
    assert!(get_test_name().contains("test"));

    let timestamp = get_timestamp();
    assert_eq!(timestamp.len(), 19);
    assert_eq!(timestamp.chars().nth(8).unwrap(), '_');
    assert_eq!(timestamp.chars().nth(15).unwrap(), '_');
}

#[test]
fn logging_is_thread_safe_after_initialization() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    let counter = Arc::new(AtomicUsize::new(0));
    let handles = (0..10)
        .map(|i| {
            let counter = counter.clone();
            thread::spawn(move || {
                for j in 0..100 {
                    log::info!("Thread {i} - Message {j}");
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(counter.load(Ordering::SeqCst), 1000);
}

#[test]
fn log_file_stem_replaces_path_unsafe_characters() {
    assert_eq!(log_file_stem("TestApp"), "TestApp");
    assert_eq!(log_file_stem("Bad/Name:Plugin"), "Bad_Name_Plugin");
    assert_eq!(log_file_stem("   "), "Application");
}

#[test]
fn archive_existing_latest_log_moves_latest_to_timestamped_log() {
    let temp_dir = TempDir::new("wrac_log_archive_latest");
    let latest = temp_dir.path().join("TestApp Latest.log");
    std::fs::write(&latest, "previous session").unwrap();

    archive_existing_latest_log(&latest, "TestApp").unwrap();

    assert!(!latest.exists());
    let archived = log_files(temp_dir.path());
    assert_eq!(archived.len(), 1);
    let archived_name = archived[0].file_name().unwrap().to_string_lossy();
    assert!(archived_name.starts_with("TestApp "));
    assert!(archived_name.ends_with(".log"));
    assert_ne!(archived_name, "TestApp Latest.log");
    assert_eq!(
        std::fs::read_to_string(&archived[0]).unwrap(),
        "previous session",
    );
}

#[test]
fn collect_recent_log_files_includes_latest_first_and_respects_limit() {
    let temp_dir = TempDir::new("wrac_log_collect_recent");
    let latest = temp_dir.path().join("TestApp Latest.log");
    let archived1 = temp_dir.path().join("TestApp 20260101_000000_000.log");
    let archived2 = temp_dir.path().join("TestApp 20260102_000000_000.log");
    let other = temp_dir.path().join("Other 20260103_000000_000.log");
    std::fs::write(&latest, "latest").unwrap();
    std::fs::write(&archived1, "archived1").unwrap();
    std::fs::write(&archived2, "archived2").unwrap();
    std::fs::write(&other, "other").unwrap();

    let files = collect_recent_log_files_from_current(
        &latest,
        &RecentLogFilesOptions {
            max_files: 2,
            max_total_bytes: 1024,
        },
    )
    .unwrap();

    assert_eq!(files.len(), 2);
    assert_eq!(files[0], latest);
    assert!(files[1] == archived1 || files[1] == archived2);
    assert!(!files.contains(&other));
}

#[test]
fn rotate_logs_keeps_max_archived_logs() {
    let temp_dir = TempDir::new("wrac_log_rotate");
    for index in 0..(MAX_LOG_FILES + 2) {
        let file = temp_dir
            .path()
            .join(format!("TestApp 20260101_000000_{index:03}.log"));
        std::fs::write(file, format!("log {index}")).unwrap();
    }
    std::fs::write(temp_dir.path().join("TestApp Latest.log"), "latest").unwrap();

    rotate_logs(temp_dir.path(), "TestApp");

    let archived = log_files(temp_dir.path())
        .into_iter()
        .filter(|path| path.file_name().unwrap().to_string_lossy() != "TestApp Latest.log")
        .collect::<Vec<_>>();
    assert_eq!(archived.len(), MAX_LOG_FILES);
    assert!(temp_dir.path().join("TestApp Latest.log").exists());
}

#[test]
fn parse_dotenv_rust_log_reads_last_non_empty_rust_log() {
    let content = r#"
            # wrac_log reads this only in development builds
            OTHER=value
            export RUST_LOG="wrac_gain_plugin=debug,wrac_log=trace"
            RUST_LOG=wrac_gain_plugin=info # the last definition wins
        "#;

    assert_eq!(
        parse_dotenv_rust_log(content).as_deref(),
        Some("wrac_gain_plugin=info"),
    );
}

#[test]
fn parse_dotenv_rust_log_ignores_empty_rust_log() {
    assert_eq!(parse_dotenv_rust_log("RUST_LOG=\n"), None);
}

#[test]
fn debug_dotenv_path_prefers_repository_root() {
    let temp_dir = TempDir::new("wrac_log_dotenv_root");
    std::fs::create_dir(temp_dir.path().join(".git")).unwrap();
    std::fs::write(temp_dir.path().join(".env"), "RUST_LOG=info").unwrap();

    let crate_dir = temp_dir.path().join("plugins").join("gain");
    std::fs::create_dir_all(&crate_dir).unwrap();
    std::fs::write(crate_dir.join(".env"), "RUST_LOG=trace").unwrap();

    let expected = temp_dir.path().join(".env");
    assert_eq!(
        debug_dotenv_path(&crate_dir).as_deref(),
        Some(expected.as_path()),
    );
}

#[test]
fn debug_dotenv_path_falls_back_to_nearest_dotenv_when_repository_root_has_none() {
    let temp_dir = TempDir::new("wrac_log_dotenv_fallback");
    std::fs::create_dir(temp_dir.path().join(".git")).unwrap();

    let crate_dir = temp_dir.path().join("plugins").join("gain");
    std::fs::create_dir_all(&crate_dir).unwrap();
    std::fs::write(crate_dir.join(".env"), "RUST_LOG=trace").unwrap();

    let expected = crate_dir.join(".env");
    assert_eq!(
        debug_dotenv_path(&crate_dir).as_deref(),
        Some(expected.as_path()),
    );
}

#[test]
fn parse_dotenv_rust_log_strips_comment_after_quoted_value() {
    assert_eq!(
        parse_dotenv_rust_log(r#"RUST_LOG="debug" # comment"#).as_deref(),
        Some("debug"),
    );
    assert_eq!(
        parse_dotenv_rust_log("RUST_LOG='wrac_gain_plugin=trace' # comment").as_deref(),
        Some("wrac_gain_plugin=trace"),
    );
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}_{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .collect::<Vec<_>>();
    files.sort();
    files
}
