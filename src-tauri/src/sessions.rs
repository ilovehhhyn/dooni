use std::path::Path;

/// Derive a stable session id from a JSONL log file path.
/// Uses the file stem (basename without extension).
pub fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Tauri window labels must match `^[a-zA-Z0-9_\-#/:]+$`.
/// Prefix with `session-` and sanitize any other chars to `-`.
pub fn window_label_for(session_id: &str) -> String {
    let sanitized: String = session_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') { c } else { '-' })
        .collect();
    format!("session-{sanitized}")
}

/// Extract the session id back out of a window label (inverse of `window_label_for`).
/// Returns `None` if the label is not a session window (e.g., "main").
#[allow(dead_code)]
pub fn session_id_from_label(label: &str) -> Option<&str> {
    label.strip_prefix("session-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn session_id_from_uuid_filename() {
        let p = PathBuf::from("/foo/bar/aff54b9a-1234-5678.jsonl");
        assert_eq!(session_id_from_path(&p), "aff54b9a-1234-5678");
    }

    #[test]
    fn session_id_from_test_filename() {
        let p = PathBuf::from("/tmp/welfare2-1783417741.jsonl");
        assert_eq!(session_id_from_path(&p), "welfare2-1783417741");
    }

    #[test]
    fn window_label_is_sanitized() {
        assert_eq!(window_label_for("abc-123"), "session-abc-123");
        assert_eq!(window_label_for("weird.name"), "session-weird-name");
        assert_eq!(window_label_for("with space"), "session-with-space");
    }

    #[test]
    fn round_trip_label() {
        let sid = "aff54b9a-1234";
        let label = window_label_for(sid);
        assert_eq!(session_id_from_label(&label), Some(sid));
    }

    #[test]
    fn main_label_is_not_a_session() {
        assert_eq!(session_id_from_label("main"), None);
    }
}
