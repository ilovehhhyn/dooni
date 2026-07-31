use crate::session_store::{self, SessionMeta};
use crate::sessions::{
    agent_from_path, history_context, limit_title, project_label, session_id_from_path,
    title_without_project_prefix, window_label_for,
};
use crate::{AppState, Turn};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

const TRIGGER_EVERY_N_USER_PROMPTS: usize = 5;
const RECENT_WINDOW_SECS: u64 = 30 * 60; // consider files modified in the last 30 min as active
const POLL_INTERVAL_SECS: u64 = 2;
const MAX_TRACKED_SESSIONS: usize = 20;
/// A session is "running" if its JSONL was touched this recently.
const RUNNING_WINDOW_SECS: u64 = 3 * 60;

pub async fn run(app: AppHandle, state: Arc<AppState>) -> Result<()> {
    let claude_root = dirs::home_dir().map(|h| h.join(".claude").join("projects"));
    let codex_root = dirs::home_dir().map(|h| h.join(".codex").join("sessions"));
    let roots = [claude_root, codex_root]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    // Existing bytes are the launch baseline. A dormant chat stays hidden
    // until a new user turn is appended after this process starts.
    let mut activation_offsets: HashMap<String, u64> = collect_latest_files(&roots, usize::MAX)
        .into_iter()
        .filter_map(|path| {
            path.metadata()
                .map(|metadata| (path.to_string_lossy().to_string(), metadata.len()))
                .ok()
        })
        .collect();

    loop {
        let files = collect_latest_files(&roots, MAX_TRACKED_SESSIONS);
        let mut active_files: Vec<(PathBuf, bool)> = Vec::new();
        for path in files {
            let session_id = session_id_from_path(&path);
            let already_surfaced = state
                .surfaced_sessions
                .lock()
                .unwrap()
                .contains(&session_id);
            if already_surfaced {
                active_files.push((path, false));
                continue;
            }

            let path_key = path.to_string_lossy().to_string();
            let previous_offset = activation_offsets.get(&path_key).copied().unwrap_or(0);
            let (next_offset, has_new_user_turn) =
                appended_segment_has_user_turn(&path, previous_offset).await?;
            activation_offsets.insert(path_key, next_offset);
            if has_new_user_turn {
                active_files.push((path, true));
            }
        }

        let active_paths = active_files
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        refresh_session_meta(&active_paths, &app, &state);

        for (path, newly_surfaced) in active_files {
            let allow_ai_review = was_modified_within(&path, RECENT_WINDOW_SECS);
            match process_file(&path, &app, &state, allow_ai_review, newly_surfaced).await {
                Ok(()) if newly_surfaced => {
                    state
                        .surfaced_sessions
                        .lock()
                        .unwrap()
                        .insert(session_id_from_path(&path));
                    emit_surfaced_sessions(&app, &state);
                }
                Ok(()) => {}
                Err(error) => {
                    eprintln!(
                        "[dooni] process_file error for {}: {error:?}",
                        path.display()
                    );
                    if newly_surfaced {
                        activation_offsets.insert(path.to_string_lossy().to_string(), 0);
                    }
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
    }
}

async fn appended_segment_has_user_turn(path: &Path, previous_offset: u64) -> Result<(u64, bool)> {
    let mut file = File::open(path).await?;
    let size = file.metadata().await?.len();
    if size == previous_offset {
        return Ok((size, false));
    }
    let offset = if size < previous_offset {
        0
    } else {
        previous_offset
    };
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut has_user_turn = false;
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        if parse_turn(&line)
            .filter(|turn| turn.role == "user")
            .and_then(|turn| clean_user_prompt(&turn.text))
            .is_some()
        {
            has_user_turn = true;
        }
    }
    Ok((size, has_user_turn))
}

/// Return the newest top-level chat histories. Subagent histories are excluded
/// because they are work inside a chat, not user-facing conversations.
fn collect_latest_files(roots: &[PathBuf], max: usize) -> Vec<PathBuf> {
    let mut out: Vec<(SystemTime, PathBuf)> = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            if p.components().any(|c| c.as_os_str() == "subagents") {
                continue;
            }
            let Some(modified) = p.metadata().and_then(|m| m.modified()).ok() else {
                continue;
            };
            out.push((modified, p.to_path_buf()));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().take(max).map(|(_, p)| p).collect()
}

fn was_modified_within(path: &Path, seconds: u64) -> bool {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|mtime| SystemTime::now().duration_since(mtime).ok())
        .map(|age| age <= Duration::from_secs(seconds))
        .unwrap_or(false)
}

async fn process_file(
    path: &Path,
    app: &AppHandle,
    state: &Arc<AppState>,
    allow_ai_review: bool,
    force_title_review: bool,
) -> Result<()> {
    let session_id = session_id_from_path(path);
    let path_key = path.to_string_lossy().to_string();

    let (offset, prev_user_count) = {
        let mut map = state.sessions.lock().unwrap();
        let s = map.entry(path_key.clone()).or_default();
        (s.last_processed_offset, s.user_count_since_review)
    };

    let mut file = File::open(path).await?;
    let meta = file.metadata().await?;
    let size = meta.len();
    let truncated = size < offset;
    let offset = if truncated { 0 } else { offset };
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
        if n == 0 {
            break;
        }
        new_offset += n as u64;
        if let Some(t) = parse_turn(&line) {
            new_turns.push(t);
        }
    }

    let user_added = new_turns
        .iter()
        .filter(|turn| turn.role == "user" && clean_user_prompt(&turn.text).is_some())
        .count();

    let (turns_snapshot, should_review) = {
        let mut map = state.sessions.lock().unwrap();
        let s = map.entry(path_key.clone()).or_default();
        if truncated {
            s.turns.clear();
            s.user_count_since_review = 0;
        }
        s.turns.extend(new_turns.into_iter());
        s.last_processed_offset = new_offset;
        let previous_count = if truncated { 0 } else { prev_user_count };
        s.user_count_since_review = previous_count.saturating_add(user_added);
        let trig = allow_ai_review
            && (force_title_review || s.user_count_since_review >= TRIGGER_EVERY_N_USER_PROMPTS);
        if trig {
            s.user_count_since_review = 0;
        }
        (s.turns.clone(), trig)
    };

    let (all_user_prompts, all_user_prompt_timestamps) =
        user_prompt_data_from_turns(&turns_snapshot);
    let (current_title, title_locked, project_dir, mut excluded_indexes) = {
        let meta = state.session_meta.lock().unwrap();
        meta.get(&session_id)
            .map(|session| {
                (
                    session.title.clone(),
                    session.title_locked,
                    session.project_dir.clone(),
                    session.excluded_prompt_indexes.clone(),
                )
            })
            .unwrap_or_else(|| (session_id.clone(), false, None, Vec::new()))
    };
    if truncated {
        excluded_indexes.clear();
        let mut metadata = state.session_meta.lock().unwrap();
        if let Some(session) = metadata.get_mut(&session_id) {
            session.excluded_prompt_indexes.clear();
        }
    }
    excluded_indexes.sort_unstable();
    excluded_indexes.dedup();
    let visible_prompts = visible_user_prompts(&all_user_prompts, &excluded_indexes);
    let visible_timestamps = visible_user_prompt_timestamps(
        &all_user_prompts,
        &all_user_prompt_timestamps,
        &excluded_indexes,
    );
    persist_prompt_history(
        app,
        state,
        &session_id,
        visible_prompts,
        visible_timestamps,
        None,
    );

    if !should_review || !state.config.lock().unwrap().onboarded {
        return Ok(());
    }

    tokio::time::sleep(Duration::from_secs(3)).await;
    let (provider, api_key) = {
        let config = state.config.lock().unwrap();
        (
            config.runtime_provider.clone(),
            crate::config::effective_api_key(&config),
        )
    };
    const REVIEW_PROMPT_LIMIT: usize = 60;
    let prompt_window_start = all_user_prompts.len().saturating_sub(REVIEW_PROMPT_LIMIT);
    let prompt_window = &all_user_prompts[prompt_window_start..];
    let title_history = bounded_title_history(&turns_snapshot);
    let folder = project_label(project_dir.as_deref());
    let current_real_title = title_without_project_prefix(&current_title, folder.as_deref());

    match crate::summarizer::review_history(
        &current_real_title,
        prompt_window,
        &title_history,
        force_title_review && !title_locked,
        &provider,
        &api_key,
    )
    .await
    {
        Ok(review) => {
            excluded_indexes.retain(|index| *index < prompt_window_start);
            excluded_indexes.extend(
                review
                    .exclude_prompt_indexes
                    .into_iter()
                    .map(|index| prompt_window_start + index),
            );
            excluded_indexes.sort_unstable();
            excluded_indexes.dedup();

            let reviewed_title = limit_title(&review.title);
            let updated_title =
                (!title_locked && reviewed_title != current_title).then_some(reviewed_title);
            let visible_prompts = visible_user_prompts(&all_user_prompts, &excluded_indexes);
            let visible_timestamps = visible_user_prompt_timestamps(
                &all_user_prompts,
                &all_user_prompt_timestamps,
                &excluded_indexes,
            );
            {
                let mut metadata = state.session_meta.lock().unwrap();
                if let Some(session) = metadata.get_mut(&session_id) {
                    session.excluded_prompt_indexes = excluded_indexes;
                    if let Some(title) = updated_title.as_ref() {
                        session.title = title.clone();
                        session.title_ai_generated = true;
                    }
                }
            }
            persist_prompt_history(
                app,
                state,
                &session_id,
                visible_prompts,
                visible_timestamps,
                updated_title.clone(),
            );
            if let (Some(title), Some(window)) = (
                updated_title.as_ref(),
                app.get_webview_window(&window_label_for(&session_id)),
            ) {
                let _ = window.set_title(title);
            }
        }
        Err(error) => {
            eprintln!("[dooni] history review error: {error:?}");
            if force_title_review && !title_locked {
                let fallback = fallback_title(&all_user_prompts);
                {
                    let mut metadata = state.session_meta.lock().unwrap();
                    if let Some(session) = metadata.get_mut(&session_id) {
                        session.title = fallback.clone();
                    }
                }
                persist_prompt_history(
                    app,
                    state,
                    &session_id,
                    visible_user_prompts(&all_user_prompts, &excluded_indexes),
                    visible_user_prompt_timestamps(
                        &all_user_prompts,
                        &all_user_prompt_timestamps,
                        &excluded_indexes,
                    ),
                    Some(fallback),
                );
            }
        }
    }

    emit_surfaced_sessions(app, state);
    Ok(())
}

fn user_prompt_data_from_turns(turns: &[Turn]) -> (Vec<String>, Vec<Option<String>>) {
    let mut prompts = Vec::new();
    let mut timestamps = Vec::new();
    for turn in turns.iter().filter(|turn| turn.role == "user") {
        if let Some(prompt) = clean_user_prompt(&turn.text) {
            prompts.push(prompt);
            timestamps.push(turn.timestamp.clone());
        }
    }
    (prompts, timestamps)
}

#[cfg(test)]
fn user_prompts_from_turns(turns: &[Turn]) -> Vec<String> {
    user_prompt_data_from_turns(turns).0
}

fn clean_user_prompt(prompt: &str) -> Option<String> {
    if is_internal_user_payload(prompt) {
        return None;
    }

    let mut skipping_files = false;
    let mut kept = Vec::new();
    for line in prompt.lines() {
        let trimmed = line.trim();
        if trimmed == "# Files mentioned by the user:" {
            skipping_files = true;
            continue;
        }
        if skipping_files {
            if trimmed == "## My request for Codex:" {
                skipping_files = false;
            }
            continue;
        }
        if trimmed.starts_with("<image name=") && trimmed.contains(" path=") {
            continue;
        }
        kept.push(line);
    }
    let cleaned = kept.join("\n").trim().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn cleaned_turn(turn: &Turn) -> Option<Turn> {
    if turn.role == "user" {
        clean_user_prompt(&turn.text).map(|text| Turn {
            role: turn.role.clone(),
            text,
            timestamp: turn.timestamp.clone(),
        })
    } else {
        Some(turn.clone())
    }
}

fn bounded_title_history(turns: &[Turn]) -> Vec<Turn> {
    const MAX_TITLE_HISTORY_CHARS: usize = 80_000;
    const FIRST_TURNS: usize = 12;
    let cleaned = turns.iter().filter_map(cleaned_turn).collect::<Vec<_>>();
    let mut selected = cleaned
        .iter()
        .take(FIRST_TURNS)
        .cloned()
        .collect::<Vec<_>>();
    let mut used = selected.iter().map(|turn| turn.text.len()).sum::<usize>();
    let first_end = selected.len();
    let mut tail = Vec::new();
    for (index, turn) in cleaned.iter().enumerate().rev() {
        if index < first_end {
            break;
        }
        if used + turn.text.len() > MAX_TITLE_HISTORY_CHARS {
            continue;
        }
        used += turn.text.len();
        tail.push(turn.clone());
    }
    tail.reverse();
    selected.extend(tail);
    selected
}

fn fallback_title(prompts: &[String]) -> String {
    let title = prompts
        .iter()
        .find(|prompt| !is_obvious_continuation(prompt))
        .map(|prompt| prompt.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Untitled chat".to_string());
    limit_title(&title)
}

fn visible_user_prompts(prompts: &[String], excluded_indexes: &[usize]) -> Vec<String> {
    prompts
        .iter()
        .enumerate()
        .filter(|(index, prompt)| {
            excluded_indexes.binary_search(index).is_err() && !is_obvious_continuation(prompt)
        })
        .map(|(_, prompt)| prompt.clone())
        .collect()
}

fn visible_user_prompt_timestamps(
    prompts: &[String],
    timestamps: &[Option<String>],
    excluded_indexes: &[usize],
) -> Vec<Option<String>> {
    prompts
        .iter()
        .enumerate()
        .filter(|(index, prompt)| {
            excluded_indexes.binary_search(index).is_err() && !is_obvious_continuation(prompt)
        })
        .map(|(index, _)| timestamps.get(index).cloned().unwrap_or(None))
        .collect()
}

fn is_internal_user_payload(prompt: &str) -> bool {
    let text = prompt.trim_start();
    [
        "<environment_context>",
        "<permissions instructions>",
        "<app-context>",
        "<collaboration_mode>",
        "<skills_instructions>",
        "<apps_instructions>",
        "<plugins_instructions>",
        "<recommended_plugins>",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn is_obvious_continuation(prompt: &str) -> bool {
    let normalized = prompt
        .trim()
        .to_lowercase()
        .trim_matches(|character: char| {
            character.is_whitespace() || character.is_ascii_punctuation()
        })
        .to_string();
    matches!(
        normalized.as_str(),
        "ok" | "okay"
            | "yes"
            | "yep"
            | "yeah"
            | "sure"
            | "proceed"
            | "continue"
            | "go ahead"
            | "do it"
            | "please do"
            | "carry on"
            | "sounds good"
            | "let's do it"
    )
}

fn persist_prompt_history(
    app: &AppHandle,
    state: &Arc<AppState>,
    session_id: &str,
    prompts: Vec<String>,
    timestamps: Vec<Option<String>>,
    title: Option<String>,
) {
    state
        .prompts_by_session
        .lock()
        .unwrap()
        .insert(session_id.to_string(), prompts.clone());
    {
        let mut metadata = state.session_meta.lock().unwrap();
        if let Some(session) = metadata.get_mut(session_id) {
            session.asked_prompts = prompts.clone();
            session.asked_prompt_timestamps = timestamps.clone();
        }
        if let Err(error) = session_store::save(&metadata) {
            eprintln!("[dooni] prompt persistence error: {error:?}");
        }
    }
    let label = window_label_for(session_id);
    let payload = serde_json::json!({
        "session_id": session_id,
        "asked_prompts": prompts,
        "asked_prompt_timestamps": timestamps,
        "title": title,
    });
    if app.get_webview_window(&label).is_some() {
        if let Err(error) = app.emit_to(label.as_str(), "asked-prompts-updated", payload) {
            eprintln!("[dooni] emit_to({label}) error: {error:?}");
        }
    }
}

fn emit_surfaced_sessions(app: &AppHandle, state: &Arc<AppState>) {
    let surfaced = state.surfaced_sessions.lock().unwrap().clone();
    let mut sessions = state
        .session_meta
        .lock()
        .unwrap()
        .values()
        .filter(|session| surfaced.contains(&session.session_id))
        .cloned()
        .collect::<Vec<_>>();
    sessions.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    let _ = app.emit(
        "sessions-updated",
        serde_json::json!({ "sessions": sessions }),
    );
}

fn refresh_session_meta(files: &[PathBuf], app: &AppHandle, state: &Arc<AppState>) {
    let now = SystemTime::now();
    let mut changed = false;
    let mut removed_ids = Vec::new();
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
            let (project_dir, surface) = history_context(path);
            let project_name = project_label(project_dir.as_deref());
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
                    title_locked: false,
                    title_ai_generated: false,
                    excluded_prompt_indexes: Vec::new(),
                    project_dir: project_dir.clone(),
                    project_name: project_name.clone(),
                    jsonl_path: jsonl_path.clone(),
                    surface: surface.clone(),
                    last_active: mtime,
                    running,
                    asked_prompts: Vec::new(),
                    asked_prompt_timestamps: Vec::new(),
                    future_prompts: Vec::new(),
                }
            });
            if entry.agent != agent {
                entry.agent = agent;
                changed = true;
            }
            if entry.project_dir != project_dir {
                entry.project_dir = project_dir;
                changed = true;
            }
            if entry.project_name != project_name {
                entry.project_name = project_name;
                changed = true;
            }
            if entry.jsonl_path != jsonl_path {
                entry.jsonl_path = jsonl_path;
                changed = true;
            }
            if entry.surface != surface {
                entry.surface = surface;
                changed = true;
            }
            if entry.last_active != mtime {
                entry.last_active = mtime;
                changed = true;
            }
            if entry.running != running {
                entry.running = running;
                changed = true;
            }
        }

        if map.len() > MAX_TRACKED_SESSIONS {
            let mut by_age: Vec<_> = map
                .values()
                .map(|m| (m.last_active, m.session_id.clone()))
                .collect();
            by_age.sort_by(|a, b| b.0.cmp(&a.0));
            for (_, old_id) in by_age.into_iter().skip(MAX_TRACKED_SESSIONS) {
                map.remove(&old_id);
                state.prompts_by_session.lock().unwrap().remove(&old_id);
                removed_ids.push(old_id.clone());
                let label = window_label_for(&old_id);
                if let Some(window) = app.get_webview_window(&label) {
                    let _ = window.close();
                }
                changed = true;
            }
        }
    }
    if !removed_ids.is_empty() {
        let mut surfaced = state.surfaced_sessions.lock().unwrap();
        for session_id in removed_ids {
            surfaced.remove(&session_id);
        }
    }
    if changed {
        emit_surfaced_sessions(app, state);
        // Persist best-effort.
        let map = state.session_meta.lock().unwrap();
        if let Err(e) = session_store::save(&*map) {
            eprintln!("[dooni] session_store save error: {e:?}");
        }
    }
}

fn parse_turn(line: &str) -> Option<Turn> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let timestamp = v
        .get("timestamp")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let t = v.get("type").and_then(|x| x.as_str());
    if t == Some("user") || t == Some("assistant") {
        let msg = v.get("message")?;
        let content = msg.get("content")?;
        let text = extract_text(content)?;
        if text.trim().is_empty() {
            return None;
        }
        return Some(Turn {
            role: t.unwrap().to_string(),
            text,
            timestamp: timestamp.clone(),
        });
    }
    if let Some(role) = v.get("role").and_then(|r| r.as_str()) {
        if role == "user" || role == "assistant" {
            let text = v.get("content").and_then(extract_text).unwrap_or_default();
            if !text.trim().is_empty() {
                return Some(Turn {
                    role: role.to_string(),
                    text,
                    timestamp: timestamp.clone(),
                });
            }
        }
    }
    if v.get("type").and_then(|x| x.as_str()) == Some("response_item") {
        let payload = v.get("payload")?;
        if payload.get("type").and_then(|x| x.as_str()) != Some("message") {
            return None;
        }
        let role = payload.get("role").and_then(|r| r.as_str())?;
        if role != "user" && role != "assistant" {
            return None;
        }
        let text = payload
            .get("content")
            .and_then(extract_text)
            .unwrap_or_default();
        if !text.trim().is_empty() {
            return Some(Turn {
                role: role.to_string(),
                text,
                timestamp,
            });
        }
    }
    None
}

fn extract_text(v: &serde_json::Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = v.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                parts.push(s.to_string());
                continue;
            }
            if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                parts.push(t.to_string());
                continue;
            }
            if let Some(t) = item.get("input_text").and_then(|x| x.as_str()) {
                parts.push(t.to_string());
                continue;
            }
            if let Some(t) = item.get("output_text").and_then(|x| x.as_str()) {
                parts.push(t.to_string());
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
    fn parse_codex_response_item_shape() {
        let line = r#"{"timestamp":"2026-07-30T23:55:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello desktop"}]}}"#;
        let t = parse_turn(line).unwrap();
        assert_eq!(t.role, "user");
        assert_eq!(t.text.trim(), "hello desktop");
        assert_eq!(t.timestamp.as_deref(), Some("2026-07-30T23:55:00Z"));
    }

    #[test]
    fn removes_only_obvious_continuation_prompts_locally() {
        assert!(is_obvious_continuation("Proceed."));
        assert!(is_obvious_continuation("  go ahead  "));
        assert!(!is_obvious_continuation("Proceed with the Rust version"));
        assert!(!is_obvious_continuation("why?"));
    }

    #[test]
    fn skips_injected_user_context() {
        assert!(is_internal_user_payload(
            "<environment_context>\n<cwd>/tmp</cwd>"
        ));
        assert!(!is_internal_user_payload(
            "Please update the environment context parser"
        ));
    }

    #[test]
    fn prompt_history_stays_verbatim_and_ordered() {
        let turns = vec![
            Turn {
                role: "user".into(),
                text: "First prompt\nwith a second line".into(),
                timestamp: Some("2026-07-29T23:50:00Z".into()),
            },
            Turn {
                role: "assistant".into(),
                text: "answer".into(),
                timestamp: Some("2026-07-29T23:51:00Z".into()),
            },
            Turn {
                role: "user".into(),
                text: "proceed".into(),
                timestamp: Some("2026-07-30T00:01:00Z".into()),
            },
        ];
        let prompts = user_prompts_from_turns(&turns);
        assert_eq!(prompts, vec!["First prompt\nwith a second line", "proceed"]);
        assert_eq!(
            visible_user_prompts(&prompts, &[]),
            vec!["First prompt\nwith a second line"]
        );
        let (prompts, timestamps) = user_prompt_data_from_turns(&turns);
        assert_eq!(
            visible_user_prompt_timestamps(&prompts, &timestamps, &[]),
            vec![Some("2026-07-29T23:50:00Z".to_string())]
        );
    }

    #[test]
    fn removes_generated_file_attachment_scaffolding() {
        let prompt = r#"Please keep this part.

# Files mentioned by the user:

## Screenshot 2026-07-30.png: /tmp/screenshot.png

## My request for Codex:
Change the launch icon.
<image name=[Image #1] path="/tmp/screenshot.png">"#;
        assert_eq!(
            clean_user_prompt(prompt),
            Some("Please keep this part.\n\nChange the launch icon.".to_string())
        );
    }

    #[test]
    fn title_history_uses_cleaned_user_prompts() {
        let turns = vec![Turn {
            role: "user".into(),
            text: "# Files mentioned by the user:\n\n## Screenshot.png: /tmp/a.png\n\n## My request for Codex:\nKeep me".into(),
            timestamp: Some("2026-07-30T02:00:00Z".into()),
        }];
        assert_eq!(bounded_title_history(&turns)[0].text, "Keep me");
    }

    #[tokio::test]
    async fn launch_baseline_waits_for_a_new_user_turn() {
        let path = std::env::temp_dir().join(format!(
            "dooni-activation-{}-{}.jsonl",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "{\"role\":\"user\",\"content\":\"an old prompt\"}\n").unwrap();
        let baseline = path.metadata().unwrap().len();

        let (_, active) = appended_segment_has_user_turn(&path, baseline)
            .await
            .unwrap();
        assert!(!active);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(
            file,
            "{{\"role\":\"assistant\",\"content\":\"a new assistant turn\"}}"
        )
        .unwrap();
        drop(file);
        let (assistant_offset, active) = appended_segment_has_user_turn(&path, baseline)
            .await
            .unwrap();
        assert!(!active);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(
            file,
            "{{\"role\":\"user\",\"content\":\"a new user prompt\"}}"
        )
        .unwrap();
        drop(file);
        let (_, active) = appended_segment_has_user_turn(&path, assistant_offset)
            .await
            .unwrap();
        assert!(active);
        std::fs::remove_file(path).unwrap();
    }
}
