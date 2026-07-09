#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod watcher;
mod process_watch;
mod summarizer;
mod config;
mod sessions;
mod session_store;
mod focus;

use std::sync::{Arc, Mutex};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct AppState {
    /// per-session parse state, keyed by session_id
    pub sessions: Mutex<HashMap<String, SessionState>>,
    /// per-session topic memo, keyed by session_id
    pub topics_by_session: Mutex<HashMap<String, Vec<String>>>,
    /// session_ids that already have a spawned window
    pub windows_spawned: Mutex<HashSet<String>>,
    /// persistent per-session metadata for the house-manager window
    pub session_meta: Mutex<HashMap<String, session_store::SessionMeta>>,
    pub config: Mutex<config::Config>,
}

#[derive(Default, Clone)]
pub struct SessionState {
    pub turns: Vec<Turn>,
    pub user_count_since_summary: usize,
    pub last_processed_offset: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Turn {
    pub role: String,
    pub text: String,
}

#[tauri::command]
fn get_topics(state: tauri::State<Arc<AppState>>, session_id: String) -> Vec<String> {
    state
        .topics_by_session
        .lock()
        .unwrap()
        .get(&session_id)
        .cloned()
        .unwrap_or_default()
}

#[tauri::command]
fn clear_topics(state: tauri::State<Arc<AppState>>, session_id: String) -> Vec<String> {
    let mut m = state.topics_by_session.lock().unwrap();
    m.insert(session_id.clone(), Vec::new());
    Vec::new()
}

#[tauri::command]
fn get_config(state: tauri::State<Arc<AppState>>) -> config::Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(
    state: tauri::State<Arc<AppState>>,
    api_key: String,
    name: String,
    agents: Vec<String>,
    mode: String,
) -> Result<config::Config, String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.api_key = api_key;
    cfg.name = name;
    cfg.agents = agents;
    cfg.mode = mode;
    cfg.onboarded = true;
    config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(cfg.clone())
}

#[tauri::command]
fn get_sessions(state: tauri::State<Arc<AppState>>) -> Vec<session_store::SessionMeta> {
    let m = state.session_meta.lock().unwrap();
    let mut v: Vec<_> = m.values().cloned().collect();
    v.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    v
}

#[tauri::command]
fn rename_session(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    let mut m = state.session_meta.lock().unwrap();
    if let Some(meta) = m.get_mut(&session_id) {
        meta.title = title;
        session_store::save(&*m).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn focus_session(
    app: tauri::AppHandle,
    state: tauri::State<Arc<AppState>>,
    session_id: String,
) -> Result<bool, String> {
    let project_dir = {
        let m = state.session_meta.lock().unwrap();
        m.get(&session_id).and_then(|s| s.project_dir.clone())
    };
    // Also raise the corresponding memo window if it exists.
    let label = sessions::window_label_for(&session_id);
    if let Some(win) = tauri::Manager::get_webview_window(&app, &label) {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
    focus::focus_terminal_for(project_dir.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_mode(state: tauri::State<Arc<AppState>>, mode: String) -> Result<config::Config, String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.mode = mode;
    config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(cfg.clone())
}

fn main() {
    let cfg = config::load();
    let meta = session_store::load();
    let state = Arc::new(AppState {
        sessions: Mutex::new(HashMap::new()),
        topics_by_session: Mutex::new(HashMap::new()),
        windows_spawned: Mutex::new(HashSet::new()),
        session_meta: Mutex::new(meta),
        config: Mutex::new(cfg),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            get_topics, clear_topics, get_config, save_config, set_mode,
            get_sessions, rename_session, focus_session
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let s = state.clone();

            // House-manager window: persistent overview of all chat sessions.
            {
                use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
                if handle.get_webview_window("house-manager").is_none() {
                    let builder = WebviewWindowBuilder::new(
                        &handle,
                        "house-manager",
                        WebviewUrl::App("manager.html".into()),
                    )
                    .title("dooni · house manager")
                    .inner_size(420.0, 560.0)
                    .always_on_top(true)
                    .resizable(true)
                    .decorations(true);
                    if let Err(e) = builder.build() {
                        eprintln!("[dooni] failed to spawn house-manager: {e:?}");
                    }
                }
            }

            let hw = handle.clone();
            let sw = s.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = watcher::run(hw, sw).await {
                    eprintln!("[dooni] watcher error: {e:?}");
                }
            });

            let hp = handle.clone();
            tauri::async_runtime::spawn(async move {
                process_watch::run(hp).await;
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("dooni failed to launch");
}
