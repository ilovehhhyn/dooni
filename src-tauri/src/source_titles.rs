//! Source titles are a read-only display overlay; user renames and stored fallbacks survive.
use crate::session_store::SessionMeta;
use std::collections::HashMap;

fn parse_index(contents: &str) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    for line in contents.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let (Some(id), Some(title)) = (value["id"].as_str(), value["thread_name"].as_str()) {
            if !title.trim().is_empty() {
                titles.insert(id.to_owned(), title.trim().to_owned());
            }
        }
    }
    titles
}

pub fn apply(sessions: &mut [SessionMeta]) {
    let home = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")));
    let Some(contents) =
        home.and_then(|home| std::fs::read_to_string(home.join("session_index.jsonl")).ok())
    else {
        return;
    };
    let titles = parse_index(&contents);
    for session in sessions {
        if session.agent != "codex" || session.title_locked {
            continue;
        }
        if let Some(title) = session
            .source_conversation_id
            .as_ref()
            .and_then(|id| titles.get(id))
            .or_else(|| titles.get(&session.session_id))
        {
            session.title = title.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latest_title_wins_and_partial_writes_do_not_erase_it() {
        let titles = parse_index("{\"id\":\"a\",\"thread_name\":\"Old\"}\n{\"id\":\"a\",\"thread_name\":\"New title\"}\n{\"id\":\"a\",\"thread_name\":\"  \"}\n{partial");
        assert_eq!(titles.get("a").unwrap(), "New title");
        assert_eq!(titles.len(), 1);
    }
}
