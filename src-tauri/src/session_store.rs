use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// House-manager entries disappear after five days without transcript activity.
pub const UNUSED_RETENTION_SECS: u64 = 5 * 24 * 60 * 60;

/// Persistent metadata about a chat session, surfaced by the house-manager window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    /// Session id (JSONL basename).
    pub session_id: String,
    /// "claude" | "codex" | "unknown".
    pub agent: String,
    /// User-editable title. Defaults to project dir or short id.
    pub title: String,
    /// Best-effort project working directory (Claude only, decoded from slug).
    #[serde(default)]
    pub project_dir: Option<String>,
    /// Absolute JSONL path.
    pub jsonl_path: String,
    /// Unix seconds of last observed activity (JSONL mtime).
    #[serde(default)]
    pub last_active: u64,
    /// True if we consider the session actively running (recent activity).
    #[serde(default)]
    pub running: bool,
}

fn store_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("dooni").join("sessions.json"))
}

pub fn load() -> HashMap<String, SessionMeta> {
    let Some(p) = store_path() else { return HashMap::new(); };
    let Ok(bytes) = std::fs::read(&p) else { return HashMap::new(); };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save(map: &HashMap<String, SessionMeta>) -> Result<()> {
    let Some(p) = store_path() else { return Ok(()); };
    if let Some(parent) = p.parent() { std::fs::create_dir_all(parent)?; }
    std::fs::write(&p, serde_json::to_vec_pretty(map)?)?;
    Ok(())
}

/// Remove entries whose last observed transcript activity is at least five days old.
/// This only changes Dooni's metadata; the underlying transcript files are never deleted.
pub fn prune_unused(map: &mut HashMap<String, SessionMeta>, now_secs: u64) -> bool {
    let before = map.len();
    map.retain(|_, session| {
        now_secs.saturating_sub(session.last_active) < UNUSED_RETENTION_SECS
    });
    map.len() != before
}

/// Default title for a freshly discovered session.
pub fn default_title(project_dir: Option<&str>, session_id: &str) -> String {
    if let Some(dir) = project_dir {
        if let Some(name) = std::path::Path::new(dir).file_name().and_then(|s| s.to_str()) {
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    let short = if session_id.len() > 12 { &session_id[..12] } else { session_id };
    short.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, last_active: u64) -> SessionMeta {
        SessionMeta {
            session_id: id.to_string(),
            agent: "codex".to_string(),
            title: id.to_string(),
            project_dir: None,
            jsonl_path: format!("/tmp/{id}.jsonl"),
            last_active,
            running: false,
        }
    }

    #[test]
    fn default_title_prefers_project_basename() {
        assert_eq!(default_title(Some("/Users/x/dooni"), "abc123def456ghi"), "dooni");
    }

    #[test]
    fn default_title_falls_back_to_short_id() {
        assert_eq!(default_title(None, "abc123def456ghi"), "abc123def456");
    }

    #[test]
    fn prune_unused_removes_entries_at_five_days() {
        let now = 1_000_000;
        let mut sessions = HashMap::from([
            ("recent".to_string(), session("recent", now - UNUSED_RETENTION_SECS + 1)),
            ("expired".to_string(), session("expired", now - UNUSED_RETENTION_SECS)),
            ("future".to_string(), session("future", now + 60)),
        ]);

        assert!(prune_unused(&mut sessions, now));
        assert!(sessions.contains_key("recent"));
        assert!(sessions.contains_key("future"));
        assert!(!sessions.contains_key("expired"));
    }

    #[test]
    fn prune_unused_reports_when_nothing_changed() {
        let now = 1_000_000;
        let mut sessions = HashMap::from([(
            "recent".to_string(),
            session("recent", now - UNUSED_RETENTION_SECS + 1),
        )]);

        assert!(!prune_unused(&mut sessions, now));
    }
}
