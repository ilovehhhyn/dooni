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

/// Classify which agent a JSONL log path belongs to based on the roots
/// dooni watches. Returns "claude", "codex", or "unknown".
pub fn agent_from_path(path: &Path) -> &'static str {
    let s = path.to_string_lossy();
    if s.contains("/.claude/projects/") { "claude" }
    else if s.contains("/.codex/sessions/") || s.contains("/.codex/history") { "codex" }
    else { "unknown" }
}

/// Claude Code encodes the project directory as the parent folder of the
/// JSONL, with `/` replaced by `-` and a leading `-` (e.g.
/// `~/.claude/projects/-Users-helen-dooni/abc.jsonl` → `/Users/helen/dooni`).
/// Returns `None` when the path doesn't fit that pattern.
pub fn project_dir_from_path(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let name = parent.file_name()?.to_str()?;
    if !name.starts_with('-') { return None; }
    let decoded = name.replace('-', "/");
    if decoded.is_empty() { return None; }
    Some(decoded)
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

    #[test]
    fn agent_from_path_classifies() {
        assert_eq!(
            agent_from_path(&PathBuf::from("/Users/x/.claude/projects/-Users-x-repo/abc.jsonl")),
            "claude"
        );
        assert_eq!(
            agent_from_path(&PathBuf::from("/Users/x/.codex/sessions/2026/01/foo.jsonl")),
            "codex"
        );
        assert_eq!(
            agent_from_path(&PathBuf::from("/Users/x/.codex/history/foo.jsonl")),
            "codex"
        );
        assert_eq!(
            agent_from_path(&PathBuf::from("/tmp/random.jsonl")),
            "unknown"
        );
    }

    #[test]
    fn project_dir_decodes_claude_slug() {
        let p = PathBuf::from("/Users/x/.claude/projects/-Users-helen-dooni/abc.jsonl");
        assert_eq!(
            project_dir_from_path(&p),
            Some("/Users/helen/dooni".to_string())
        );
    }

    #[test]
    fn project_dir_none_when_not_slug() {
        let p = PathBuf::from("/tmp/plain/abc.jsonl");
        assert_eq!(project_dir_from_path(&p), None);
    }
}
