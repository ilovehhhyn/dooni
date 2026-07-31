use std::io::{BufRead, BufReader};
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
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("session-{sanitized}")
}

/// True if a transcript belongs to a background *subagent* rather than a chat
/// the user is having. Claude Code stores subagent (Task-tool) transcripts in a
/// `<session-uuid>/subagents/` directory, named `agent-*.jsonl`, with every
/// line marked `"isSidechain":true`. These are internal helpers — dooni should
/// never memo, record, or show them.
pub fn is_background_agent(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.contains("/subagents/") {
        return true;
    }
    matches!(
        path.file_stem().and_then(|x| x.to_str()),
        Some(stem) if stem.starts_with("agent-")
    )
}

/// Classify which agent a JSONL log path belongs to based on the roots
/// dooni watches. Returns "claude", "codex", or "unknown".
pub fn agent_from_path(path: &Path) -> &'static str {
    let s = path.to_string_lossy();
    if s.contains("/.claude/projects/") {
        "claude"
    } else if s.contains("/.codex/sessions/") || s.contains("/.codex/history") {
        "codex"
    } else {
        "unknown"
    }
}

/// Claude Code encodes the project directory as the parent folder of the
/// JSONL, with `/` replaced by `-` and a leading `-` (e.g.
/// `~/.claude/projects/-Users-helen-dooni/abc.jsonl` → `/Users/helen/dooni`).
/// Returns `None` when the path doesn't fit that pattern.
pub fn project_dir_from_path(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let name = parent.file_name()?.to_str()?;
    if !name.starts_with('-') {
        return None;
    }
    let decoded = name.replace('-', "/");
    if decoded.is_empty() {
        return None;
    }
    Some(decoded)
}

/// Read the small metadata records at the start of a history file. Codex
/// Desktop and Codex CLI share a history root, so the JSON is the reliable
/// way to distinguish their surface and recover the working directory.
pub fn history_context(path: &Path) -> (Option<String>, String) {
    let path_project = project_dir_from_path(path);
    let mut project_dir = path_project;
    let mut surface = "terminal".to_string();
    let Ok(file) = std::fs::File::open(path) else {
        return (project_dir, surface);
    };

    for line in BufReader::new(file).lines().take(12).flatten() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let payload = v.get("payload").unwrap_or(&v);
        if let Some(cwd) = payload.get("cwd").and_then(|x| x.as_str()) {
            project_dir = Some(cwd.to_string());
        }
        if let Some(originator) = payload.get("originator").and_then(|x| x.as_str()) {
            let origin = originator.to_ascii_lowercase();
            if origin.contains("codex desktop") {
                surface = "codex-app".to_string();
            } else if origin.contains("claude desktop") {
                surface = "claude-app".to_string();
            }
        }
    }
    (project_dir, surface)
}

/// Read the provider's technical conversation id from the history header.
/// Codex stores it in `session_meta.payload.id`; Claude Code repeats
/// `sessionId` on its records.
pub fn history_conversation_id(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    for line in BufReader::new(file).lines().take(20).flatten() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(|value| value.as_str()) == Some("session_meta") {
            if let Some(id) = v
                .get("payload")
                .and_then(|payload| payload.get("id"))
                .and_then(|id| id.as_str())
            {
                return Some(id.to_string());
            }
        }
        if let Some(id) = v.get("sessionId").and_then(|id| id.as_str()) {
            return Some(id.to_string());
        }
    }
    None
}

/// Prefer the repository root name when the working directory sits inside a
/// repository; otherwise use the working directory's final path component.
pub fn project_label(project_dir: Option<&str>) -> Option<String> {
    let project_dir = project_dir?.trim();
    if project_dir.is_empty() {
        return None;
    }
    let path = Path::new(project_dir);
    for ancestor in path.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string);
        }
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

pub const MAX_TITLE_CHARS: usize = 180;

/// Normalize a title and keep the full value within the UI's 180-character
/// limit. Prefer ending at a word boundary instead of clipping mid-word.
pub fn limit_title(title: &str) -> String {
    let normalized = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() < MAX_TITLE_CHARS {
        return normalized;
    }

    let clipped = normalized.chars().take(MAX_TITLE_CHARS).collect::<String>();
    let word_boundary = clipped
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, _)| index)
        .filter(|index| *index >= MAX_TITLE_CHARS / 2);
    word_boundary
        .map(|index| clipped[..index].trim_end().to_string())
        .unwrap_or(clipped)
}

pub fn title_without_project_prefix(title: &str, project: Option<&str>) -> String {
    if let Some(project) = project {
        if let Some(real_title) = title.strip_prefix(&format!("{project} · ")) {
            return real_title.to_string();
        }
    }
    title.to_string()
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
    fn reads_codex_conversation_id_from_history_header() {
        let path = std::env::temp_dir().join(format!(
            "dooni-conversation-id-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-123\",\"cwd\":\"/tmp\"}}\n",
        )
        .unwrap();
        assert_eq!(
            history_conversation_id(&path).as_deref(),
            Some("thread-123")
        );
        let _ = std::fs::remove_file(path);
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
    fn background_agent_detected() {
        assert!(is_background_agent(&PathBuf::from(
            "/Users/x/.claude/projects/-Users-x/uuid/subagents/agent-abc.jsonl"
        )));
        assert!(is_background_agent(&PathBuf::from(
            "/foo/agent-deadbeef.jsonl"
        )));
        assert!(!is_background_agent(&PathBuf::from(
            "/Users/x/.claude/projects/-Users-x/aff54b9a-1234.jsonl"
        )));
    }

    #[test]
    fn agent_from_path_classifies() {
        assert_eq!(
            agent_from_path(&PathBuf::from(
                "/Users/x/.claude/projects/-Users-x-repo/abc.jsonl"
            )),
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

    #[test]
    fn old_project_prefixes_can_be_removed() {
        assert_eq!(
            title_without_project_prefix("dooni · Track active AI chats", Some("dooni")),
            "Track active AI chats"
        );
    }

    #[test]
    fn titles_stop_at_a_word_boundary_within_180_characters() {
        let title = format!("{} complete", "word ".repeat(40));
        let limited = limit_title(&title);
        assert!(limited.chars().count() <= MAX_TITLE_CHARS);
        assert!(!limited.ends_with("compl"));
        assert!(!limited.ends_with(' '));
    }

    #[test]
    fn titles_already_stored_at_the_limit_drop_a_dangling_fragment() {
        let title = format!("{} short", "a".repeat(174));
        assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
        let limited = limit_title(&title);
        assert!(limited.chars().count() < MAX_TITLE_CHARS);
        assert!(!limited.ends_with("short"));
    }

    #[test]
    fn short_titles_are_only_whitespace_normalized() {
        assert_eq!(
            limit_title("  Track   active\nAI chats  "),
            "Track active AI chats"
        );
    }
}
