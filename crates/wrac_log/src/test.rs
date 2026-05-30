#[cfg(test)]
mod tests {
    use crate::*;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_init_test_stdout() {
        init_test();
        init_test();
    }

    #[test]
    fn test_get_test_name_and_timestamp() {
        let test_name = crate::get_test_name();
        assert!(test_name.contains("test"));

        let timestamp = crate::get_timestamp();
        assert_eq!(timestamp.len(), 19);
        assert_eq!(timestamp.chars().nth(8).unwrap(), '_');
        assert_eq!(timestamp.chars().nth(15).unwrap(), '_');
    }

    #[test]
    fn test_thread_safety() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::thread;

        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];
        for i in 0..10 {
            let counter_clone = counter.clone();
            let handle = thread::spawn(move || {
                for j in 0..100 {
                    log::info!("Thread {} - Message {}", i, j);
                    counter_clone.fetch_add(1, Ordering::SeqCst);
                }
            });
            handles.push(handle);
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(counter.load(Ordering::SeqCst), 1000);
    }

    #[test]
    fn log_file_stem_replaces_path_unsafe_characters() {
        assert_eq!(crate::log_file_stem("TestApp"), "TestApp");
        assert_eq!(crate::log_file_stem("Bad/Name:Plugin"), "Bad_Name_Plugin");
        assert_eq!(crate::log_file_stem("   "), "Application");
    }

    #[test]
    fn archive_existing_latest_log_moves_latest_to_timestamped_log() {
        let temp_dir = TempDir::new("wrac_log_archive_latest");
        let latest = temp_dir.path().join("TestApp Latest.log");
        std::fs::write(&latest, "previous session").unwrap();

        crate::archive_existing_latest_log(&latest, "TestApp").unwrap();

        assert!(!latest.exists());
        let archived = log_files(temp_dir.path());
        assert_eq!(archived.len(), 1);
        let archived_name = archived[0].file_name().unwrap().to_string_lossy();
        assert!(archived_name.starts_with("TestApp "));
        assert!(archived_name.ends_with(".log"));
        assert_ne!(archived_name, "TestApp Latest.log");
        assert_eq!(
            std::fs::read_to_string(&archived[0]).unwrap(),
            "previous session"
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

        let files = crate::collect_recent_log_files_from_current(
            &latest,
            &RecentLogFilesOptions {
                max_files: 2,
                max_total_bytes: 1024,
                ..RecentLogFilesOptions::default()
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
        for index in 0..(crate::MAX_LOG_FILES + 2) {
            let file = temp_dir
                .path()
                .join(format!("TestApp 20260101_000000_{index:03}.log"));
            std::fs::write(file, format!("log {index}")).unwrap();
        }
        std::fs::write(temp_dir.path().join("TestApp Latest.log"), "latest").unwrap();

        crate::rotate_logs(temp_dir.path(), "TestApp");

        let archived = log_files(temp_dir.path())
            .into_iter()
            .filter(|path| path.file_name().unwrap().to_string_lossy() != "TestApp Latest.log")
            .collect::<Vec<_>>();
        assert_eq!(archived.len(), crate::MAX_LOG_FILES);
        assert!(temp_dir.path().join("TestApp Latest.log").exists());
    }

    #[test]
    fn parse_dotenv_rust_log_reads_rust_log_without_mutating_environment() {
        let content = r#"
            # wrac_log reads this only in development builds
            OTHER=value
            export RUST_LOG="snapclip_plugin=debug,snapclip_core_device=trace"
            RUST_LOG=snapclip_plugin=info # the last definition wins
        "#;

        assert_eq!(
            crate::parse_dotenv_rust_log(content).as_deref(),
            Some("snapclip_plugin=info")
        );
    }

    #[test]
    fn parse_dotenv_rust_log_ignores_empty_rust_log() {
        assert_eq!(crate::parse_dotenv_rust_log("RUST_LOG=\n"), None);
    }

    #[test]
    fn debug_dotenv_path_prefers_repository_root() {
        let temp_dir = TempDir::new("wrac_log_dotenv_root");
        std::fs::create_dir(temp_dir.path().join(".git")).unwrap();
        std::fs::write(temp_dir.path().join(".env"), "RUST_LOG=info").unwrap();

        let crate_dir = temp_dir.path().join("snapclip").join("snapclip_plugin");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(crate_dir.join(".env"), "RUST_LOG=trace").unwrap();

        let expected = temp_dir.path().join(".env");
        assert_eq!(
            crate::debug_dotenv_path(crate_dir.to_str().unwrap()).as_deref(),
            Some(expected.as_path())
        );
    }

    #[test]
    fn debug_dotenv_path_falls_back_to_nearest_dotenv_when_repository_root_has_none() {
        let temp_dir = TempDir::new("wrac_log_dotenv_fallback");
        std::fs::create_dir(temp_dir.path().join(".git")).unwrap();

        let crate_dir = temp_dir.path().join("snapclip").join("snapclip_plugin");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(crate_dir.join(".env"), "RUST_LOG=trace").unwrap();

        let expected = crate_dir.join(".env");
        assert_eq!(
            crate::debug_dotenv_path(crate_dir.to_str().unwrap()).as_deref(),
            Some(expected.as_path())
        );
    }

    #[test]
    fn parse_dotenv_rust_log_strips_comment_after_quoted_value() {
        assert_eq!(
            crate::parse_dotenv_rust_log(r#"RUST_LOG="debug" # comment"#).as_deref(),
            Some("debug")
        );
        assert_eq!(
            crate::parse_dotenv_rust_log("RUST_LOG='snapclip_plugin=trace' # comment").as_deref(),
            Some("snapclip_plugin=trace")
        );
    }

    #[test]
    fn rt_drain_stops_before_unpublished_slot() {
        let log = crate::RtLogInner::new("test");
        log.next_sequence
            .store(1, std::sync::atomic::Ordering::Release);

        log.drain_to_log();
        assert_eq!(
            log.drain_sequence
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );

        log.slots[0].write(
            0,
            log::Level::Debug,
            "test",
            10,
            20,
            format_args!("published"),
        );
        log.drain_to_log();
        assert_eq!(
            log.drain_sequence
                .load(std::sync::atomic::Ordering::Acquire),
            1
        );
    }

    #[test]
    fn rt_fixed_message_truncates_at_utf8_boundary() {
        let mut message = crate::FixedMessage::new();
        let value = "a".repeat(crate::RT_MESSAGE_CAPACITY - 1) + "あ";

        std::fmt::Write::write_str(&mut message, &value).unwrap();

        assert_eq!(message.len, crate::RT_MESSAGE_CAPACITY - 1);
        assert_eq!(
            std::str::from_utf8(message.as_bytes()).unwrap().len(),
            message.len
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
}
