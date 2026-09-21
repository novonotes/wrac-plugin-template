use std::path::{Path, PathBuf};

#[cfg(debug_assertions)]
pub(super) fn rust_log_from_debug_dotenv(search_dir: Option<&Path>) -> Option<String> {
    if std::env::var("RUST_LOG").is_ok() {
        return None;
    }

    let dotenv_path = debug_dotenv_path(search_dir?)?;
    let Ok(content) = std::fs::read_to_string(&dotenv_path) else {
        return None;
    };
    parse_dotenv_rust_log(&content)
}

#[cfg(not(debug_assertions))]
pub(super) fn rust_log_from_debug_dotenv(search_dir: Option<&Path>) -> Option<String> {
    let _ = search_dir;
    None
}

#[cfg(debug_assertions)]
pub(super) fn debug_dotenv_path(search_dir: &Path) -> Option<PathBuf> {
    let mut fallback = None;

    for ancestor in search_dir.ancestors() {
        let candidate = ancestor.join(".env");
        if ancestor.join(".git").exists() {
            if candidate.is_file() {
                return Some(candidate);
            }
            break;
        }
        if fallback.is_none() && candidate.is_file() {
            fallback = Some(candidate);
        }
    }
    fallback
}

#[cfg(debug_assertions)]
pub(super) fn parse_dotenv_rust_log(content: &str) -> Option<String> {
    let mut rust_log = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "RUST_LOG" {
            continue;
        }

        let value = parse_dotenv_value(value.trim());
        if !value.is_empty() {
            rust_log = Some(value);
        }
    }
    rust_log
}

#[cfg(debug_assertions)]
fn parse_dotenv_value(value: &str) -> String {
    if let Some(stripped) = value.strip_prefix('"') {
        if let Some(end) = stripped.find('"') {
            return stripped[..end].to_string();
        }
    } else if let Some(stripped) = value.strip_prefix('\'')
        && let Some(end) = stripped.find('\'')
    {
        return stripped[..end].to_string();
    }

    value
        .split_once(" #")
        .map(|(value, _)| value.trim_end())
        .unwrap_or(value)
        .to_string()
}
