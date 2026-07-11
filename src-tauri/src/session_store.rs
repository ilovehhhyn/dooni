use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Persistent metadata about a chat session, surfaced by the house-manager window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    /// Session id (JSONL basename).
    pub session_id: String,
    /// "claude" | "codex" | "unknown".
    pub agent: String,
    /// User-editable title. Defaults to project dir or short id, then gets
    /// upgraded to an AI-generated topic summary while `auto_title` is true.
    pub title: String,
    /// True while `title` is still auto-managed. A manual rename flips this off
    /// so the summarizer stops overwriting the user's chosen title.
    #[serde(default = "default_true")]
    pub auto_title: bool,
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

fn default_true() -> bool { true }

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

    #[test]
    fn default_title_prefers_project_basename() {
        assert_eq!(default_title(Some("/Users/x/dooni"), "abc123def456ghi"), "dooni");
    }

    #[test]
    fn default_title_falls_back_to_short_id() {
        assert_eq!(default_title(None, "abc123def456ghi"), "abc123def456");
    }
}
