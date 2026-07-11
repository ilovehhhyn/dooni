use crate::sessions::{agent_from_path, project_dir_from_path, session_id_from_path, window_label_for};
use crate::session_store::{self, SessionMeta};
use crate::{AppState, Turn};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, WebviewUrl, WebviewWindowBuilder};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

const TRIGGER_EVERY_N_USER_PROMPTS: usize = 5;
const RECENT_WINDOW_SECS: u64 = 30 * 60; // consider files modified in the last 30 min as active
const POLL_INTERVAL_SECS: u64 = 2;
/// A session is "running" if its JSONL was touched this recently.
const RUNNING_WINDOW_SECS: u64 = 3 * 60;

pub async fn run(app: AppHandle, state: Arc<AppState>) -> Result<()> {
    let claude_root = dirs::home_dir().map(|h| h.join(".claude").join("projects"));
    let codex_root = dirs::home_dir().map(|h| h.join(".codex").join("sessions"));

    loop {
        let files = collect_recent_files(
            &[claude_root.clone(), codex_root.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            RECENT_WINDOW_SECS,
        );

        for path in &files {
            if let Err(e) = process_file(path, &app, &state).await {
                eprintln!("[dooni] process_file error for {}: {e:?}", path.display());
            }
        }

        refresh_session_meta(&files, &app, &state);

        tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
    }
}

/// Return all `*.jsonl` files under `roots` that were modified within the last `window_secs`.
fn collect_recent_files(roots: &[PathBuf], window_secs: u64) -> Vec<PathBuf> {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(window_secs))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut out = Vec::new();
    for root in roots {
        if !root.exists() { continue; }
        for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
            let p = entry.path();
            if !p.is_file() { continue; }
            if p.extension().and_then(|s| s.to_str()) != Some("jsonl") { continue; }
            // Background subagent transcripts are internal helpers — never track them.
            if crate::sessions::is_background_agent(p) { continue; }
            let modified = p.metadata().and_then(|m| m.modified()).ok();
            if let Some(m) = modified {
                if m >= cutoff {
                    out.push(p.to_path_buf());
                }
            }
        }
    }
    out
}

async fn process_file(path: &Path, app: &AppHandle, state: &Arc<AppState>) -> Result<()> {
    let session_id = session_id_from_path(path);
    let path_key = path.to_string_lossy().to_string();

    let (offset, prev_user_count) = {
        let mut map = state.sessions.lock().unwrap();
        let s = map.entry(path_key.clone()).or_default();
        (s.last_processed_offset, s.user_count_since_summary)
    };

    let mut file = File::open(path).await?;
    let meta = file.metadata().await?;
    let size = meta.len();
    // Handle truncation/rotation: reset if the file shrunk.
    let offset = if size < offset { 0 } else { offset };
    if size == offset {
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

    let user_added = new_turns.iter().filter(|t| t.role == "user").count();

    // Memo windows are no longer auto-spawned. We still parse and summarize so
    // the memo is ready, but the window itself only opens when the user clicks
    // "focus" in the house manager (see focus_session -> open_session_window).

    let (turns_snapshot, should_summarize) = {
        let mut map = state.sessions.lock().unwrap();
        let s = map.entry(path_key.clone()).or_default();
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
        tokio::time::sleep(Duration::from_secs(3)).await;

        let current_topics = state
            .topics_by_session
            .lock()
            .unwrap()
            .get(&session_id)
            .cloned()
            .unwrap_or_default();

        let (mode, name, api_key) = {
            let c = state.config.lock().unwrap();
            (
                c.mode.clone(),
                c.name.clone(),
                crate::config::effective_api_key(&c),
            )
        };
        let recent: Vec<Turn> = turns_snapshot
            .iter()
            .rev()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        match crate::summarizer::update_topics(&current_topics, &recent, &mode, &name, &api_key)
            .await
        {
            Ok(new_list) => {
                eprintln!(
                    "[dooni] session={} summarizer OK, {} topics",
                    session_id,
                    new_list.len()
                );
                state
                    .topics_by_session
                    .lock()
                    .unwrap()
                    .insert(session_id.clone(), new_list.clone());

                // Auto-generate a topic title for this session's memo window,
                // unless the user has manually renamed it in the house manager.
                // Anchor on the session's first user prompt + the full memo so the
                // title reflects the overall topic, not just the latest messages.
                let first_prompt = turns_snapshot
                    .iter()
                    .find(|t| t.role == "user")
                    .map(|t| t.text.as_str());
                let title =
                    maybe_update_title(app, state, &session_id, first_prompt, &new_list, &api_key)
                        .await;

                let label = window_label_for(&session_id);
                // Emit both a scoped event and the payload with session id so
                // any interested window can filter if needed.
                let payload = serde_json::json!({
                    "session_id": session_id,
                    "topics": new_list,
                    "title": title,
                });
                if let Err(e) = app.emit_to(label.as_str(), "topics-updated", payload.clone()) {
                    eprintln!("[dooni] emit_to({label}) error: {e:?}");
                }
            }
            Err(e) => eprintln!("[dooni] summarizer error: {e:?}"),
        }
    }

    Ok(())
}

fn refresh_session_meta(files: &[PathBuf], app: &AppHandle, state: &Arc<AppState>) {
    let now = SystemTime::now();
    let mut changed = false;
    let snapshot: Vec<SessionMeta>;
    {
        let mut map = state.session_meta.lock().unwrap();
        // Mark all as not running; we'll flip the truly-active ones back below.
        for m in map.values_mut() {
            if m.running {
                m.running = false;
                changed = true;
            }
        }
        for path in files {
            let session_id = session_id_from_path(path);
            let agent = agent_from_path(path).to_string();
            let project_dir = project_dir_from_path(path);
            let jsonl_path = path.to_string_lossy().to_string();
            let mtime = path
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let running = now
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs().saturating_sub(mtime) <= RUNNING_WINDOW_SECS)
                .unwrap_or(false);
            let entry = map.entry(session_id.clone()).or_insert_with(|| {
                let title = session_store::default_title(project_dir.as_deref(), &session_id);
                changed = true;
                SessionMeta {
                    session_id: session_id.clone(),
                    agent: agent.clone(),
                    title,
                    auto_title: true,
                    project_dir: project_dir.clone(),
                    jsonl_path: jsonl_path.clone(),
                    last_active: mtime,
                    running,
                }
            });
            if entry.agent != agent { entry.agent = agent; changed = true; }
            if entry.project_dir != project_dir { entry.project_dir = project_dir; changed = true; }
            if entry.jsonl_path != jsonl_path { entry.jsonl_path = jsonl_path; changed = true; }
            if entry.last_active != mtime { entry.last_active = mtime; changed = true; }
            if entry.running != running { entry.running = running; changed = true; }
        }
        snapshot = map.values().cloned().collect();
    }
    if changed {
        let mut snapshot = snapshot;
        // Newest first.
        snapshot.sort_by(|a, b| b.last_active.cmp(&a.last_active));
        let payload = serde_json::json!({ "sessions": snapshot });
        if let Err(e) = app.emit_to("house-manager", "sessions-updated", payload.clone()) {
            eprintln!("[dooni] emit_to(house-manager) error: {e:?}");
        }
        // Also broadcast unscoped in case the window listens without scope.
        let _ = app.emit("sessions-updated", payload);
        // Persist best-effort.
        let map = state.session_meta.lock().unwrap();
        if let Err(e) = session_store::save(&*map) {
            eprintln!("[dooni] session_store save error: {e:?}");
        }
    }
}

/// Regenerate the memo window's title from recent messages while the session's
/// title is still auto-managed. Applies it to the native window title, persists
/// it, and notifies the house-manager. Returns the effective title for callers
/// to forward to the memo window (`None` if nothing could be determined).
async fn maybe_update_title(
    app: &AppHandle,
    state: &Arc<AppState>,
    session_id: &str,
    first_prompt: Option<&str>,
    topics: &[String],
    api_key: &str,
) -> Option<String> {
    // Only auto-generate while the user hasn't set their own title.
    let (auto, existing, project_dir) = {
        let map = state.session_meta.lock().unwrap();
        match map.get(session_id) {
            Some(m) => (m.auto_title, Some(m.title.clone()), m.project_dir.clone()),
            None => (true, None, None),
        }
    };
    if !auto {
        return existing;
    }

    let topic = match crate::summarizer::generate_title(first_prompt, topics, api_key).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[dooni] title gen error for {session_id}: {e:?}");
            return existing;
        }
    };

    // Titles follow "repo: topic". The repo comes from the project dir basename
    // when we know it (Claude sessions); otherwise we fall back to topic only.
    let repo = project_dir.as_deref().and_then(|d| {
        std::path::Path::new(d)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    });
    let title = match repo {
        Some(r) => format!("{r}: {topic}"),
        None => topic,
    };

    // Persist onto the session meta (only if still auto).
    let snapshot: Vec<SessionMeta> = {
        let mut map = state.session_meta.lock().unwrap();
        if let Some(m) = map.get_mut(session_id) {
            if !m.auto_title {
                return Some(m.title.clone());
            }
            m.title = title.clone();
        }
        if let Err(e) = session_store::save(&*map) {
            eprintln!("[dooni] session_store save error: {e:?}");
        }
        map.values().cloned().collect()
    };

    // Update the native memo-window title bar.
    let label = window_label_for(session_id);
    if let Some(win) = tauri::Manager::get_webview_window(app, &label) {
        let _ = win.set_title(&format!("dooni · {title}"));
    }

    // Refresh the house-manager list so it shows the new title too.
    let mut snapshot = snapshot;
    snapshot.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    let payload = serde_json::json!({ "sessions": snapshot });
    let _ = app.emit_to("house-manager", "sessions-updated", payload.clone());
    let _ = app.emit("sessions-updated", payload);

    Some(title)
}

/// Open (or raise) the memo window for a session. Memo windows are created
/// lazily — only when the user asks for one via the house manager — so this
/// builds the window on first request and just focuses it thereafter.
pub fn open_session_window(app: &AppHandle, session_id: &str) {
    let label = window_label_for(session_id);
    if let Some(win) = tauri::Manager::get_webview_window(app, &label) {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        return;
    }
    let title = format!("dooni · {}", short_id(session_id));
    let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(340.0, 500.0)
        .always_on_top(false)
        .resizable(true)
        .decorations(true);
    match builder.build() {
        Ok(_) => eprintln!("[dooni] opened memo window for session {session_id} (label={label})"),
        Err(e) => eprintln!("[dooni] failed to open window for {session_id}: {e:?}"),
    }
}

fn short_id(id: &str) -> String {
    if id.len() <= 12 { id.to_string() } else { id[..12].to_string() }
}

fn parse_turn(line: &str) -> Option<Turn> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let t = v.get("type").and_then(|x| x.as_str());
    if t == Some("user") || t == Some("assistant") {
        let msg = v.get("message")?;
        let content = msg.get("content")?;
        let text = extract_text(content)?;
        if text.trim().is_empty() { return None; }
        return Some(Turn { role: t.unwrap().to_string(), text });
    }
    if let Some(role) = v.get("role").and_then(|r| r.as_str()) {
        if role == "user" || role == "assistant" {
            let text = v.get("content").and_then(extract_text).unwrap_or_default();
            if !text.trim().is_empty() {
                return Some(Turn { role: role.to_string(), text });
            }
        }
    }
    None
}

fn extract_text(v: &serde_json::Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = v.as_array() {
        let mut out = String::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                out.push_str(s);
                out.push('\n');
                continue;
            }
            if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                out.push_str(t);
                out.push('\n');
            }
        }
        if !out.is_empty() { return Some(out); }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_user_string_content() {
        let line = r#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        let t = parse_turn(line).unwrap();
        assert_eq!(t.role, "user");
        assert_eq!(t.text.trim(), "hi");
    }

    #[test]
    fn parse_claude_assistant_array_content() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}"#;
        let t = parse_turn(line).unwrap();
        assert_eq!(t.role, "assistant");
        assert!(t.text.contains("hello"));
    }

    #[test]
    fn parse_skips_permission_mode() {
        let line = r#"{"type":"permission-mode","permissionMode":"default"}"#;
        assert!(parse_turn(line).is_none());
    }

    #[test]
    fn parse_skips_empty_text() {
        let line = r#"{"type":"user","message":{"role":"user","content":""}}"#;
        assert!(parse_turn(line).is_none());
    }

    #[test]
    fn parse_codex_role_shape() {
        let line = r#"{"role":"user","content":"hello codex"}"#;
        let t = parse_turn(line).unwrap();
        assert_eq!(t.role, "user");
        assert_eq!(t.text.trim(), "hello codex");
    }

    #[test]
    fn parse_bad_json_returns_none() {
        assert!(parse_turn("not json").is_none());
    }

    #[test]
    fn short_id_truncates() {
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(short_id("abcdefghijklmnop"), "abcdefghijkl");
    }
}
