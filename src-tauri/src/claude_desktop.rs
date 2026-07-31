use crate::session_store::{self, AskedPromptLocator, SessionMeta};
use crate::sessions::{limit_title, window_label_for};
use crate::{AppState, Turn};
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

const POLL_MILLIS: u64 = 350;
const MAX_TRACKED_SESSIONS: usize = 20;

#[derive(Clone)]
struct Draft {
    text: String,
    conversation_id: Option<String>,
    window_title: Option<String>,
}

pub async fn run(app: AppHandle, state: Arc<AppState>) -> Result<()> {
    let mut draft: Option<Draft> = None;
    let mut latest_conversation_id: Option<String> = None;
    let mut latest_window_title: Option<String> = None;
    let mut local_session_id: Option<String> = None;

    loop {
        let observation =
            tauri::async_runtime::spawn_blocking(crate::focus::observe_frontmost_claude_desktop)
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();

        if let Some(observation) = observation {
            if observation.conversation_id.is_some() {
                if latest_conversation_id.is_some()
                    && observation.conversation_id != latest_conversation_id
                {
                    local_session_id = None;
                }
                latest_conversation_id = observation.conversation_id.clone();
            }
            if observation.window_title.is_some() {
                if latest_window_title.is_some()
                    && observation.window_title != latest_window_title
                    && latest_conversation_id.is_none()
                {
                    local_session_id = None;
                }
                latest_window_title = observation.window_title.clone();
            }
            if let Some(composer) = observation.composer {
                if composer.trim().is_empty() {
                    if let Some(sent) = draft.take() {
                        tokio::time::sleep(Duration::from_millis(650)).await;
                        let after_send = tauri::async_runtime::spawn_blocking(
                            crate::focus::observe_frontmost_claude_desktop_with_id,
                        )
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .flatten();
                        let conversation_id = after_send
                            .as_ref()
                            .and_then(|next| next.conversation_id.clone())
                            .or(sent.conversation_id)
                            .or_else(|| latest_conversation_id.clone());
                        let window_title = after_send
                            .and_then(|next| next.window_title)
                            .or(sent.window_title)
                            .or_else(|| latest_window_title.clone());
                        let has_conversation_id = conversation_id.is_some();
                        if let Some(session_id) = record_prompt(
                            &app,
                            &state,
                            conversation_id,
                            window_title,
                            local_session_id.clone(),
                            sent.text,
                        ) {
                            if has_conversation_id {
                                local_session_id = None;
                            } else {
                                local_session_id = Some(session_id.clone());
                            }
                            let review_app = app.clone();
                            let review_state = state.clone();
                            tauri::async_runtime::spawn(async move {
                                review_title(review_app, review_state, session_id).await;
                            });
                        }
                    }
                } else {
                    draft = Some(Draft {
                        text: composer,
                        conversation_id: observation
                            .conversation_id
                            .or_else(|| latest_conversation_id.clone()),
                        window_title: observation
                            .window_title
                            .or_else(|| latest_window_title.clone()),
                    });
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(POLL_MILLIS)).await;
    }
}

fn record_prompt(
    app: &AppHandle,
    state: &Arc<AppState>,
    conversation_id: Option<String>,
    window_title: Option<String>,
    fallback_session_id: Option<String>,
    prompt: String,
) -> Option<String> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() || crate::watcher::is_obvious_continuation(&prompt) {
        return None;
    }
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let session_id = conversation_id
        .as_ref()
        .map(|id| format!("claude-desktop-{id}"))
        .or(fallback_session_id)
        .unwrap_or_else(|| format!("claude-desktop-local-{}", now));
    let timestamp = Some(Utc::now().to_rfc3339());
    let mut removed = Vec::new();
    let prompts;

    {
        let mut sessions = state.session_meta.lock().unwrap();
        for session in sessions
            .values_mut()
            .filter(|session| session.jsonl_path.starts_with("claude-desktop://"))
        {
            session.running = false;
        }
        let entry = sessions
            .entry(session_id.clone())
            .or_insert_with(|| SessionMeta {
                session_id: session_id.clone(),
                agent: "claude".to_string(),
                title: window_title
                    .clone()
                    .filter(|title| !title.eq_ignore_ascii_case("Claude"))
                    .map(|title| limit_title(&title))
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| limit_title(&prompt)),
                title_locked: false,
                title_ai_generated: false,
                excluded_prompt_indexes: Vec::new(),
                project_dir: None,
                project_name: None,
                jsonl_path: format!(
                    "claude-desktop://{}",
                    conversation_id.as_deref().unwrap_or("active")
                ),
                source_conversation_id: conversation_id.clone(),
                surface: "claude-app".to_string(),
                last_active: now,
                running: true,
                asked_prompts: Vec::new(),
                asked_prompt_timestamps: Vec::new(),
                asked_prompt_locators: Vec::new(),
                future_prompts: Vec::new(),
            });
        entry.last_active = now;
        entry.running = true;
        if conversation_id.is_some() {
            entry.source_conversation_id = conversation_id;
        }
        let occurrence = entry
            .asked_prompts
            .iter()
            .filter(|existing| existing.as_str() == prompt)
            .count()
            + 1;
        entry.asked_prompts.push(prompt);
        entry.asked_prompt_timestamps.push(timestamp);
        entry.asked_prompt_locators.push(AskedPromptLocator {
            turn_id: None,
            history_position: entry.asked_prompts.len().saturating_sub(1) as u64,
            occurrence,
        });
        prompts = entry.asked_prompts.clone();

        if sessions.len() > MAX_TRACKED_SESSIONS {
            let mut by_age = sessions
                .values()
                .map(|session| (session.last_active, session.session_id.clone()))
                .collect::<Vec<_>>();
            by_age.sort_by(|left, right| right.0.cmp(&left.0));
            for (_, old_id) in by_age.into_iter().skip(MAX_TRACKED_SESSIONS) {
                sessions.remove(&old_id);
                removed.push(old_id);
            }
        }
        if let Err(error) = session_store::save(&sessions) {
            eprintln!("[dooni] Claude Desktop persistence error: {error:?}");
        }
    }

    state
        .prompts_by_session
        .lock()
        .unwrap()
        .insert(session_id.clone(), prompts);
    state
        .surfaced_sessions
        .lock()
        .unwrap()
        .insert(session_id.clone());
    for old_id in removed {
        state.prompts_by_session.lock().unwrap().remove(&old_id);
        state.surfaced_sessions.lock().unwrap().remove(&old_id);
        if let Some(window) = app.get_webview_window(&window_label_for(&old_id)) {
            let _ = window.close();
        }
    }
    emit_sessions(app, state);
    emit_prompt_update(app, state, &session_id, None);
    Some(session_id)
}

async fn review_title(app: AppHandle, state: Arc<AppState>, session_id: String) {
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (current_title, title_locked, prompts, timestamps) = {
        let sessions = state.session_meta.lock().unwrap();
        let Some(session) = sessions.get(&session_id) else {
            return;
        };
        (
            session.title.clone(),
            session.title_locked,
            session.asked_prompts.clone(),
            session.asked_prompt_timestamps.clone(),
        )
    };
    if title_locked || (prompts.len() != 1 && prompts.len() % 5 != 0) {
        return;
    }
    let (provider, api_key, onboarded) = {
        let config = state.config.lock().unwrap();
        (
            config.runtime_provider.clone(),
            crate::config::effective_api_key(&config),
            config.onboarded,
        )
    };
    if !onboarded {
        return;
    }
    let history = prompts
        .iter()
        .enumerate()
        .map(|(index, prompt)| Turn {
            role: "user".to_string(),
            text: prompt.clone(),
            timestamp: timestamps.get(index).cloned().unwrap_or(None),
            source_turn_id: None,
            history_position: index as u64,
        })
        .collect::<Vec<_>>();
    let Ok(review) = crate::summarizer::review_history(
        &current_title,
        &prompts,
        &history,
        true,
        &provider,
        &api_key,
    )
    .await
    else {
        return;
    };

    let updated_title = {
        let mut sessions = state.session_meta.lock().unwrap();
        let Some(session) = sessions.get_mut(&session_id) else {
            return;
        };
        if session.title_locked {
            return;
        }
        session.title = limit_title(&review.title);
        session.title_ai_generated = true;
        for index in review.exclude_prompt_indexes.into_iter().rev() {
            if index < session.asked_prompts.len() {
                session.asked_prompts.remove(index);
                session.asked_prompt_timestamps.remove(index);
                session.asked_prompt_locators.remove(index);
            }
        }
        let updated_title = Some(session.title.clone());
        state
            .prompts_by_session
            .lock()
            .unwrap()
            .insert(session_id.clone(), session.asked_prompts.clone());
        if let Err(error) = session_store::save(&sessions) {
            eprintln!("[dooni] Claude Desktop title persistence error: {error:?}");
        }
        updated_title
    };
    emit_sessions(&app, &state);
    emit_prompt_update(&app, &state, &session_id, updated_title);
}

fn emit_sessions(app: &AppHandle, state: &Arc<AppState>) {
    let surfaced = state.surfaced_sessions.lock().unwrap().clone();
    let mut sessions = state
        .session_meta
        .lock()
        .unwrap()
        .values()
        .filter(|session| surfaced.contains(&session.session_id))
        .cloned()
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| right.last_active.cmp(&left.last_active));
    let _ = app.emit(
        "sessions-updated",
        serde_json::json!({ "sessions": sessions }),
    );
}

fn emit_prompt_update(
    app: &AppHandle,
    state: &Arc<AppState>,
    session_id: &str,
    title: Option<String>,
) {
    let label = window_label_for(session_id);
    if app.get_webview_window(&label).is_none() {
        return;
    }
    let sessions = state.session_meta.lock().unwrap();
    let Some(session) = sessions.get(session_id) else {
        return;
    };
    let _ = app.emit_to(
        label,
        "asked-prompts-updated",
        serde_json::json!({
            "session_id": session_id,
            "asked_prompts": session.asked_prompts,
            "asked_prompt_timestamps": session.asked_prompt_timestamps,
            "asked_prompt_locators": session.asked_prompt_locators,
            "title": title,
        }),
    );
}
