use crate::{AppState, Turn};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};
use tokio::fs::File;

const TRIGGER_EVERY_N_USER_PROMPTS: usize = 5;

pub async fn run(app: AppHandle, state: Arc<AppState>) -> Result<()> {
    let claude_root = dirs::home_dir().map(|h| h.join(".claude").join("projects"));
    let codex_root = dirs::home_dir().map(|h| h.join(".codex").join("sessions"));

    loop {
        // Poll every 2s for new/changed files.
        let mut files: Vec<PathBuf> = Vec::new();
        for root in [claude_root.clone(), codex_root.clone()].into_iter().flatten() {
            if root.exists() {
                for entry in walkdir::WalkDir::new(&root).into_iter().flatten() {
                    let p = entry.path();
                    if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        files.push(p.to_path_buf());
                    }
                }
            }
        }

        // Focus on the most recently modified file (current session)
        files.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
        if let Some(latest) = files.last().cloned() {
            if let Err(e) = process_file(&latest, &app, &state).await {
                eprintln!("[dooni] process_file error: {e:?}");
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn process_file(path: &Path, app: &AppHandle, state: &Arc<AppState>) -> Result<()> {
    let session_id = path.to_string_lossy().to_string();

    let (offset, prev_user_count) = {
        let mut map = state.sessions.lock().unwrap();
        let s = map.entry(session_id.clone()).or_default();
        (s.last_processed_offset, s.user_count_since_summary)
    };

    let mut file = File::open(path).await?;
    let meta = file.metadata().await?;
    let size = meta.len();
    if size <= offset {
        return Ok(());
    }
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut reader = BufReader::new(file);
    let mut new_turns: Vec<Turn> = Vec::new();
    let mut line = String::new();
    let mut new_offset = offset;
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 { break; }
        new_offset += n as u64;
        if let Some(t) = parse_turn(&line) {
            new_turns.push(t);
        }
    }

    let mut user_added = 0usize;
    for t in &new_turns {
        if t.role == "user" { user_added += 1; }
    }

    let (turns_snapshot, should_summarize) = {
        let mut map = state.sessions.lock().unwrap();
        let s = map.entry(session_id.clone()).or_default();
        s.turns.extend(new_turns.into_iter());
        s.last_processed_offset = new_offset;
        s.user_count_since_summary = prev_user_count + user_added;
        let trig = s.user_count_since_summary >= TRIGGER_EVERY_N_USER_PROMPTS;
        if trig { s.user_count_since_summary = 0; }
        (s.turns.clone(), trig)
    };

    if should_summarize {
        {
            let c = state.config.lock().unwrap();
            if !c.onboarded { return Ok(()); }
        }
        // Debounce briefly
        tokio::time::sleep(Duration::from_secs(3)).await;
        let current_topics = state.topics.lock().unwrap().clone();
        let (mode, name, api_key) = {
            let c = state.config.lock().unwrap();
            (c.mode.clone(), c.name.clone(), crate::config::effective_api_key(&c))
        };
        let recent: Vec<Turn> = turns_snapshot.iter().rev().take(10).cloned().collect::<Vec<_>>().into_iter().rev().collect();
        match crate::summarizer::update_topics(&current_topics, &recent, &mode, &name, &api_key).await {
            Ok(new_list) => {
                eprintln!("[dooni] summarizer OK, {} topics: {:?}", new_list.len(), new_list);
                {
                    let mut t = state.topics.lock().unwrap();
                    *t = new_list.clone();
                }
                match app.emit("topics-updated", &new_list) {
                    Ok(_) => eprintln!("[dooni] emitted topics-updated"),
                    Err(e) => eprintln!("[dooni] emit error: {e:?}"),
                }
            }
            Err(e) => eprintln!("[dooni] summarizer error: {e:?}"),
        }
    }

    Ok(())
}

fn parse_turn(line: &str) -> Option<Turn> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let t = v.get("type")?.as_str()?;
    // Claude Code format
    if t == "user" || t == "assistant" {
        let msg = v.get("message")?;
        let content = msg.get("content")?;
        let text = extract_text(content)?;
        if text.trim().is_empty() { return None; }
        return Some(Turn { role: t.to_string(), text });
    }
    // Codex format (best-effort): entries may have {role, content} or {type:"message", ...}
    if let Some(role) = v.get("role").and_then(|r| r.as_str()) {
        if role == "user" || role == "assistant" {
            let text = v.get("content").and_then(extract_text_val).unwrap_or_default();
            if !text.trim().is_empty() {
                return Some(Turn { role: role.to_string(), text });
            }
        }
    }
    None
}

fn extract_text(v: &serde_json::Value) -> Option<String> {
    if let Some(s) = v.as_str() { return Some(s.to_string()); }
    if let Some(arr) = v.as_array() {
        let mut out = String::new();
        for item in arr {
            if let Some(s) = item.as_str() { out.push_str(s); out.push('\n'); continue; }
            if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                out.push_str(t); out.push('\n');
            }
        }
        if !out.is_empty() { return Some(out); }
    }
    None
}
fn extract_text_val(v: &serde_json::Value) -> Option<String> { extract_text(v) }
