use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuturePrompt {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

/// Persistent metadata about a chat session, surfaced by the conversation list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    /// Session id (JSONL basename).
    pub session_id: String,
    /// "claude" | "codex" | "unknown".
    pub agent: String,
    /// AI-maintained title. Defaults to project dir or short id until the first review.
    pub title: String,
    /// Manual renames prevent later AI title changes.
    #[serde(default)]
    pub title_locked: bool,
    /// Whether the fallback title has been replaced by an AI-generated title.
    #[serde(default)]
    pub title_ai_generated: bool,
    /// Source-history user-prompt indexes classified as continuation-only.
    #[serde(default)]
    pub excluded_prompt_indexes: Vec<usize>,
    /// Best-effort project working directory (Claude only, decoded from slug).
    #[serde(default)]
    pub project_dir: Option<String>,
    /// Repository root name when available, otherwise the working-directory name.
    #[serde(default)]
    pub project_name: Option<String>,
    /// Absolute JSONL path.
    pub jsonl_path: String,
    /// Where the chat originated: "terminal", "codex-app", or "claude-app".
    #[serde(default = "default_surface")]
    pub surface: String,
    /// Unix seconds of last observed activity (JSONL mtime).
    #[serde(default)]
    pub last_active: u64,
    /// True if we consider the session actively running (recent activity).
    #[serde(default)]
    pub running: bool,
    /// Verbatim substantive user prompts from the source history.
    #[serde(default, alias = "summary")]
    pub asked_prompts: Vec<String>,
    /// Source timestamps aligned by index with `asked_prompts`.
    #[serde(default)]
    pub asked_prompt_timestamps: Vec<Option<String>>,
    /// User-authored prompts to try later.
    #[serde(default)]
    pub future_prompts: Vec<FuturePrompt>,
}

fn default_surface() -> String {
    "terminal".to_string()
}

fn store_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("dooni").join("sessions.json"))
}

pub fn load() -> HashMap<String, SessionMeta> {
    let Some(p) = store_path() else {
        return HashMap::new();
    };
    let Ok(bytes) = std::fs::read(&p) else {
        return HashMap::new();
    };
    let mut sessions: HashMap<String, SessionMeta> =
        serde_json::from_slice(&bytes).unwrap_or_default();
    for session in sessions.values_mut() {
        if session.title.trim().eq_ignore_ascii_case("dooni") {
            session.title = "Untitled chat".to_string();
        }
    }
    sessions
}

pub fn save(map: &HashMap<String, SessionMeta>) -> Result<()> {
    let Some(p) = store_path() else {
        return Ok(());
    };
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(map)?)?;
    Ok(())
}

/// Default title for a freshly discovered session.
pub fn default_title(project_dir: Option<&str>, _session_id: &str) -> String {
    if let Some(dir) = project_dir {
        if let Some(name) = std::path::Path::new(dir)
            .file_name()
            .and_then(|s| s.to_str())
        {
            if !name.is_empty() && !name.eq_ignore_ascii_case("dooni") {
                return name.to_string();
            }
        }
    }
    "Untitled chat".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_title_prefers_project_basename() {
        assert_eq!(
            default_title(Some("/Users/x/dooni"), "abc123def456ghi"),
            "Untitled chat"
        );
    }

    #[test]
    fn default_title_falls_back_to_short_id() {
        assert_eq!(default_title(None, "abc123def456ghi"), "Untitled chat");
    }
}
