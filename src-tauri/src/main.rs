#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod watcher;
mod process_watch;
mod summarizer;
mod config;
mod sessions;

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
fn set_mode(state: tauri::State<Arc<AppState>>, mode: String) -> Result<config::Config, String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.mode = mode;
    config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(cfg.clone())
}

fn main() {
    let cfg = config::load();
    let state = Arc::new(AppState {
        sessions: Mutex::new(HashMap::new()),
        topics_by_session: Mutex::new(HashMap::new()),
        windows_spawned: Mutex::new(HashSet::new()),
        config: Mutex::new(cfg),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            get_topics, clear_topics, get_config, save_config, set_mode
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let s = state.clone();

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
