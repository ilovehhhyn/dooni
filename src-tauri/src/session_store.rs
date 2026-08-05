use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuturePrompt {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AskedPromptLocator {
    /// Source-provider turn/message id when the history format exposes one.
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Byte position of the source JSONL record.
    #[serde(default)]
    pub history_position: u64,
    /// One-based occurrence of identical prompt text in this conversation.
    #[serde(default)]
    pub occurrence: usize,
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
    /// Source byte length after the last successful history parse.
    #[serde(default)]
    pub history_bytes: u64,
    /// Provider conversation/thread id used by supported desktop deep links.
    #[serde(default)]
    pub source_conversation_id: Option<String>,
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
    /// Source records aligned by index with `asked_prompts`.
    #[serde(default)]
    pub asked_prompt_locators: Vec<AskedPromptLocator>,
    /// User-authored prompts to try later.
    #[serde(default)]
    pub future_prompts: Vec<FuturePrompt>,
}

fn default_surface() -> String {
    "terminal".to_string()
}

const LEGACY_MIGRATION_KEY: &str = "legacy_sessions_json_migrated";
pub const MAX_TRACKED_SESSIONS: usize = 20;

fn database_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("dooni").join("sessions.sqlite3"))
}

fn legacy_store_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("dooni").join("sessions.json"))
}

pub fn load() -> HashMap<String, SessionMeta> {
    let (Some(database), Some(legacy)) = (database_path(), legacy_store_path()) else {
        return HashMap::new();
    };
    match load_from_paths(&database, &legacy) {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("[dooni] session database load error: {error:?}");
            HashMap::new()
        }
    }
}

pub fn save(map: &HashMap<String, SessionMeta>) -> Result<()> {
    let Some(path) = database_path() else {
        return Ok(());
    };
    save_to_path(&path, map)
}

fn open_database(path: &std::path::Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(3))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT PRIMARY KEY NOT NULL,
            payload TEXT NOT NULL,
            last_active INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS sessions_last_active
            ON sessions(last_active DESC);
        CREATE TABLE IF NOT EXISTS metadata (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        );",
    )?;
    Ok(connection)
}

fn load_from_paths(
    database: &std::path::Path,
    legacy: &std::path::Path,
) -> Result<HashMap<String, SessionMeta>> {
    let mut connection = open_database(database)?;
    migrate_legacy_json(&mut connection, legacy)?;
    connection.execute(
        "DELETE FROM sessions
         WHERE session_id NOT IN (
             SELECT session_id FROM sessions
             ORDER BY last_active DESC, session_id DESC
             LIMIT ?1
         )",
        [MAX_TRACKED_SESSIONS as i64],
    )?;

    let mut statement = connection.prepare(
        "SELECT session_id, payload FROM sessions
         ORDER BY last_active DESC, session_id DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut sessions = HashMap::new();
    for row in rows {
        let (session_id, payload) = row?;
        match serde_json::from_str::<SessionMeta>(&payload) {
            Ok(mut session) => {
                session.session_id = session_id.clone();
                if session.title.trim().eq_ignore_ascii_case("dooni") {
                    session.title = "Untitled chat".to_string();
                }
                sessions.insert(session_id, session);
            }
            Err(error) => {
                eprintln!("[dooni] skipping corrupt session row {session_id}: {error}");
            }
        }
    }
    Ok(sessions)
}

fn migrate_legacy_json(connection: &mut Connection, legacy: &std::path::Path) -> Result<()> {
    let migrated = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = ?1",
            [LEGACY_MIGRATION_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some();
    if migrated {
        return Ok(());
    }

    let row_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
    if row_count == 0 && legacy.exists() {
        match std::fs::read(legacy)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| {
                serde_json::from_slice::<HashMap<String, SessionMeta>>(&bytes).map_err(Into::into)
            }) {
            Ok(sessions) => persist_map(connection, &sessions)?,
            Err(error) => eprintln!("[dooni] legacy session migration skipped: {error:?}"),
        }
    }
    connection.execute(
        "INSERT INTO metadata(key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [LEGACY_MIGRATION_KEY],
    )?;
    Ok(())
}

fn save_to_path(path: &std::path::Path, map: &HashMap<String, SessionMeta>) -> Result<()> {
    let mut connection = open_database(path)?;
    persist_map(&mut connection, map)
}

fn persist_map(connection: &mut Connection, map: &HashMap<String, SessionMeta>) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM sessions", [])?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO sessions(session_id, payload, last_active) VALUES (?1, ?2, ?3)",
        )?;
        for session in map.values() {
            statement.execute(params![
                session.session_id,
                serde_json::to_string(session)?,
                session.last_active as i64,
            ])?;
        }
    }
    transaction.commit()?;
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

    fn session(id: &str, title: &str) -> SessionMeta {
        serde_json::from_value(serde_json::json!({
            "session_id": id,
            "agent": "codex",
            "title": title,
            "jsonl_path": format!("/tmp/{id}.jsonl"),
            "last_active": 42,
            "asked_prompts": ["first question"],
            "future_prompts": [{"id": "thought-1", "text": "try this later", "done": false}]
        }))
        .unwrap()
    }

    fn test_directory(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("dooni-{name}-{}-{nonce}", std::process::id()))
    }

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

    #[test]
    fn sqlite_round_trip_persists_history_and_thoughts() {
        let directory = test_directory("sqlite-round-trip");
        let database = directory.join("sessions.sqlite3");
        let legacy = directory.join("sessions.json");
        let mut expected = session("one", "Persistent title");
        expected.title_locked = true;
        save_to_path(
            &database,
            &HashMap::from([("one".to_string(), expected.clone())]),
        )
        .unwrap();

        let loaded = load_from_paths(&database, &legacy).unwrap();
        let actual = loaded.get("one").unwrap();
        assert_eq!(actual.title, expected.title);
        assert!(actual.title_locked);
        assert_eq!(actual.asked_prompts, expected.asked_prompts);
        assert_eq!(actual.future_prompts[0].text, "try this later");

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_json_is_imported_only_once() {
        let directory = test_directory("legacy-migration");
        std::fs::create_dir_all(&directory).unwrap();
        let database = directory.join("sessions.sqlite3");
        let legacy = directory.join("sessions.json");
        let original = session("one", "From JSON");
        std::fs::write(
            &legacy,
            serde_json::to_vec(&HashMap::from([("one".to_string(), original)])).unwrap(),
        )
        .unwrap();

        let mut imported = load_from_paths(&database, &legacy).unwrap();
        assert_eq!(imported["one"].title, "From JSON");
        imported.get_mut("one").unwrap().title = "From SQLite".to_string();
        save_to_path(&database, &imported).unwrap();

        let loaded_again = load_from_paths(&database, &legacy).unwrap();
        assert_eq!(loaded_again["one"].title, "From SQLite");

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn database_retains_only_the_newest_twenty_sessions() {
        let directory = test_directory("retention");
        let database = directory.join("sessions.sqlite3");
        let legacy = directory.join("sessions.json");
        let sessions = (0..=MAX_TRACKED_SESSIONS)
            .map(|index| {
                let id = format!("session-{index:02}");
                let mut value = session(&id, &id);
                value.last_active = index as u64;
                (id, value)
            })
            .collect::<HashMap<_, _>>();
        save_to_path(&database, &sessions).unwrap();

        let loaded = load_from_paths(&database, &legacy).unwrap();
        assert_eq!(loaded.len(), MAX_TRACKED_SESSIONS);
        assert!(!loaded.contains_key("session-00"));
        assert!(loaded.contains_key("session-20"));

        std::fs::remove_dir_all(directory).unwrap();
    }
}
