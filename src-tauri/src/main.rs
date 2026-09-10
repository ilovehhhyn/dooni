#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod codex_runtime;
mod config;
mod focus;
mod session_store;
mod sessions;
mod source_titles;
mod summarizer;
mod watcher;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

const MAX_FUTURE_PROMPTS: usize = 100;

#[derive(Default)]
pub struct AppState {
    /// per-session parse state, keyed by session_id
    pub sessions: Mutex<HashMap<String, SessionState>>,
    /// Per-session verbatim user prompts, keyed by session id.
    pub prompts_by_session: Mutex<HashMap<String, Vec<String>>>,
    /// Persistent per-session metadata for the conversation list.
    pub session_meta: Mutex<HashMap<String, session_store::SessionMeta>>,
    /// Chats admitted to the list by a user turn during this app launch.
    pub surfaced_sessions: Mutex<HashSet<String>>,
    pub config: Mutex<config::Config>,
}

#[derive(Default, Clone)]
pub struct SessionState {
    pub turns: Vec<Turn>,
    pub user_count_since_review: usize,
    pub last_processed_offset: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Turn {
    pub role: String,
    pub text: String,
    pub timestamp: Option<String>,
    pub source_turn_id: Option<String>,
    pub history_position: u64,
}

fn surfaced_session_ids(sessions: &HashMap<String, session_store::SessionMeta>) -> HashSet<String> {
    sessions
        .values()
        .filter(|session| session.surface != session_store::MANUAL_SURFACE)
        .map(|session| session.session_id.clone())
        .collect()
}

#[tauri::command]
fn get_session(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
) -> Option<session_store::SessionMeta> {
    let mut session = state
        .session_meta
        .lock()
        .unwrap()
        .get(&session_id)
        .cloned()?;
    source_titles::apply(std::slice::from_mut(&mut session));
    Some(session)
}

#[tauri::command]
fn save_future_prompts(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    prompts: Vec<session_store::FuturePrompt>,
) -> Result<(), String> {
    let clean: Vec<_> = prompts
        .into_iter()
        .filter_map(|mut p| {
            p.text = p.text.trim().to_string();
            (!p.text.is_empty()).then_some(p)
        })
        .take(MAX_FUTURE_PROMPTS)
        .collect();
    let mut map = state.session_meta.lock().unwrap();
    let Some(meta) = map.get_mut(&session_id) else {
        return Err("session not found".to_string());
    };
    meta.future_prompts = clean;
    session_store::save(&map).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_config(state: tauri::State<Arc<AppState>>) -> config::PublicConfig {
    config::PublicConfig::from(&*state.config.lock().unwrap())
}

#[tauri::command]
fn save_config(
    state: tauri::State<Arc<AppState>>,
    api_key: String,
    runtime_provider: String,
    name: String,
    agents: Vec<String>,
) -> Result<config::PublicConfig, String> {
    if runtime_provider != "codex" && runtime_provider != "anthropic" {
        return Err("unsupported runtime provider".to_string());
    }
    if runtime_provider == "anthropic" && api_key.trim().is_empty() {
        return Err("Anthropic API key required".to_string());
    }
    let mut cfg = state.config.lock().unwrap();
    cfg.api_key = api_key.trim().to_string();
    cfg.runtime_provider = runtime_provider;
    cfg.name = name;
    cfg.agents = agents;
    cfg.onboarded = true;
    config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(config::PublicConfig::from(&*cfg))
}

#[tauri::command]
fn update_runtime_provider(
    state: tauri::State<Arc<AppState>>,
    runtime_provider: String,
    api_key: Option<String>,
) -> Result<config::PublicConfig, String> {
    if runtime_provider != "codex" && runtime_provider != "anthropic" {
        return Err("unsupported runtime provider".to_string());
    }
    let mut cfg = state.config.lock().unwrap();
    if runtime_provider == "anthropic" {
        if let Some(key) = api_key.map(|key| key.trim().to_string()) {
            if !key.is_empty() {
                cfg.api_key = key;
            }
        }
        if config::effective_api_key(&cfg).is_empty() {
            return Err("Anthropic API key required".to_string());
        }
    }
    cfg.runtime_provider = runtime_provider;
    config::save(&cfg).map_err(|error| error.to_string())?;
    Ok(config::PublicConfig::from(&*cfg))
}

#[tauri::command]
async fn get_codex_status() -> codex_runtime::CodexStatus {
    codex_runtime::status().await
}

#[tauri::command]
async fn start_codex_login() -> Result<(), String> {
    codex_runtime::start_login()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn open_codex_install() -> Result<(), String> {
    codex_runtime::open_install_page().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_claude_desktop_access_status() -> focus::ClaudeDesktopAccessStatus {
    focus::claude_desktop_access_status()
}

#[tauri::command]
fn open_claude_desktop_access() -> Result<(), String> {
    focus::open_claude_desktop_access().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_think_data() -> Result<serde_json::Value, String> {
    session_store::load_think_data().map_err(|e| e.to_string())
}

#[tauri::command]
fn save_think_data(data: serde_json::Value) -> Result<(), String> {
    session_store::save_think_data(&data).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_think_link(url: String) -> Result<(), String> {
    let parsed = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return Err("Only web links can be opened".into());
    }
    let status = std::process::Command::new("open")
        .arg(parsed.as_str())
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Could not open the browser".into());
    }
    Ok(())
}

fn codex_thought_url(prompt: Option<&str>) -> (reqwest::Url, bool) {
    // The installed desktop app handles this route as a new, unsent composer.
    // Keep large captures on the clipboard rather than exceeding macOS URL limits.
    let prompt = prompt.filter(|text| !text.is_empty() && text.len() <= 16_000);
    let mut url = reqwest::Url::parse("codex://threads/new").expect("static desktop URL");
    if let Some(prompt) = prompt {
        url.query_pairs_mut().append_pair("prompt", prompt);
    }
    (url, prompt.is_some())
}

#[tauri::command]
fn open_think_app(provider: String, prompt: Option<String>) -> Result<bool, String> {
    let bundle = match provider.as_str() {
        "Codex" | "ChatGPT" => "com.openai.codex",
        "Claude" => "com.anthropic.claudefordesktop",
        _ => return Err("Unknown desktop app".into()),
    };
    let mut command = std::process::Command::new("open");
    command.args(["-b", bundle]);
    let prefilled = if provider == "Codex" || provider == "ChatGPT" {
        let (url, prefilled) = codex_thought_url(prompt.as_deref());
        command.arg(url.as_str());
        prefilled
    } else {
        false
    };
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Could not open {provider}. Check that the desktop app is installed."
        ));
    }
    Ok(prefilled)
}

#[tauri::command]
async fn copy_think_content(text: String, images: Vec<String>) -> Result<(), String> {
    // AppKit preserves image attachments on the system clipboard for browser and desktop chats.
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        if images.is_empty() {
            let mut child = std::process::Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
            child.stdin.take().ok_or("Clipboard input unavailable")?.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            if !child.wait().map_err(|e| e.to_string())?.success() { return Err("Could not copy thought".into()); }
            return Ok(());
        }
        let script = r#"import AppKit
import Foundation
let input = FileHandle.standardInput.readDataToEndOfFile()
let value = try JSONSerialization.jsonObject(with: input) as! [String: Any]
let text = value["text"] as! String
let images = value["images"] as! [String]
var items: [NSPasteboardWriting] = []
for image in images {
    if let comma = image.firstIndex(of: ","), let data = Data(base64Encoded: String(image[image.index(after: comma)...])), let picture = NSBitmapImageRep(data: data), let png = picture.representation(using: .png, properties: [:]) {
        let attachment = NSPasteboardItem()
        attachment.setData(png, forType: .png)
        attachment.setString(text, forType: .string)
        items.append(attachment)
    }
}
let item = NSPasteboardItem()
item.setString(text, forType: .string)
if items.isEmpty { items.append(item) }
NSPasteboard.general.clearContents()
if !NSPasteboard.general.writeObjects(items) { exit(1) }
"#;
        let mut child = std::process::Command::new("swift").args(["-e", script])
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
        let payload = serde_json::json!({"text": text, "images": images}).to_string();
        child.stdin.take().ok_or("Clipboard input unavailable")?.write_all(payload.as_bytes()).map_err(|e| e.to_string())?;
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() { return Err("Could not copy this thought to the clipboard".into()); }
        Ok(())
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_sessions(state: tauri::State<Arc<AppState>>) -> Vec<session_store::SessionMeta> {
    let surfaced = state.surfaced_sessions.lock().unwrap().clone();
    let m = state.session_meta.lock().unwrap();
    let mut v: Vec<_> = m
        .values()
        .filter(|session| {
            surfaced.contains(&session.session_id)
                && session.surface != session_store::MANUAL_SURFACE
        })
        .cloned()
        .collect();
    drop(m);
    source_titles::apply(&mut v);
    v.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    v
}

#[tauri::command]
fn rename_session(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    title: String,
) -> Result<String, String> {
    let mut m = state.session_meta.lock().unwrap();
    let updated_title = {
        let meta = m
            .get_mut(&session_id)
            .ok_or_else(|| "session not found".to_string())?;
        meta.title = sessions::limit_title(&title);
        if meta.title.is_empty() {
            meta.title = "Untitled chat".to_string();
        }
        meta.title_locked = true;
        meta.title.clone()
    };
    session_store::save(&*m).map_err(|e| e.to_string())?;
    Ok(updated_title)
}

/// A chat the user added by hand. It has no history file to follow, so the
/// watcher never touches it: its prompts stay empty, its dot never lights up,
/// and the capture shortcuts cannot match it (they key off the frontmost app,
/// which is never `manual`). Thoughts are still editable.
fn manual_session(
    title: &str,
    url: Option<&str>,
    created_millis: u128,
) -> Result<session_store::SessionMeta, String> {
    let url = url.map(str::trim).filter(|url| !url.is_empty());
    if let Some(url) = url {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err("chat link must start with https://".to_string());
        }
    }
    let title = title.trim().to_string();
    let session_id = format!("{}{created_millis}", session_store::MANUAL_ID_PREFIX);
    Ok(session_store::SessionMeta {
        session_id: session_id.clone(),
        agent: session_store::MANUAL_SURFACE.to_string(),
        title: if title.is_empty() {
            "Untitled chat".to_string()
        } else {
            title
        },
        title_locked: true,
        title_ai_generated: false,
        excluded_prompt_indexes: Vec::new(),
        project_dir: None,
        project_name: None,
        jsonl_path: format!("manual://{session_id}"),
        history_bytes: 0,
        source_conversation_id: None,
        source_url: url.map(str::to_string),
        surface: session_store::MANUAL_SURFACE.to_string(),
        last_active: (created_millis / 1000) as u64,
        running: false,
        asked_prompts: Vec::new(),
        asked_prompt_timestamps: Vec::new(),
        asked_prompt_locators: Vec::new(),
        future_prompts: Vec::new(),
    })
}

#[tauri::command]
fn create_manual_session(
    app: tauri::AppHandle,
    state: tauri::State<Arc<AppState>>,
    title: String,
    url: Option<String>,
) -> Result<session_store::SessionMeta, String> {
    let created_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let session = manual_session(&title, url.as_deref(), created_millis)?;
    {
        let mut sessions = state.session_meta.lock().unwrap();
        sessions.insert(session.session_id.clone(), session.clone());
        session_store::save(&sessions).map_err(|error| error.to_string())?;
    }
    state
        .surfaced_sessions
        .lock()
        .unwrap()
        .insert(session.session_id.clone());
    watcher::emit_surfaced_sessions(&app, &state);
    Ok(session)
}

#[tauri::command]
fn focus_session(state: tauri::State<Arc<AppState>>, session_id: String) -> Result<bool, String> {
    let (project_dir, surface, source_url) = {
        let m = state.session_meta.lock().unwrap();
        let session = m.get(&session_id);
        (
            session.and_then(|s| s.project_dir.clone()),
            session
                .map(|s| s.surface.clone())
                .unwrap_or_else(|| "terminal".to_string()),
            session.and_then(|s| s.source_url.clone()),
        )
    };
    focus::focus_chat_for(&surface, project_dir.as_deref(), source_url.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn locate_asked_prompt(
    state: tauri::State<'_, Arc<AppState>>,
    session_id: String,
    prompt_index: usize,
) -> Result<focus::LocatePromptResult, String> {
    let (surface, project_dir, conversation_id, prompt, locator) = {
        let sessions = state.session_meta.lock().unwrap();
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| "session not found".to_string())?;
        let prompt = session
            .asked_prompts
            .get(prompt_index)
            .cloned()
            .ok_or_else(|| "prompt not found".to_string())?;
        (
            session.surface.clone(),
            session.project_dir.clone(),
            session.source_conversation_id.clone(),
            prompt,
            session
                .asked_prompt_locators
                .get(prompt_index)
                .cloned()
                .unwrap_or_default(),
        )
    };

    tauri::async_runtime::spawn_blocking(move || {
        focus::locate_prompt(
            &surface,
            conversation_id.as_deref(),
            project_dir.as_deref(),
            &prompt,
            locator.occurrence,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn launch_session_window(app: tauri::AppHandle, session_id: String) -> Result<(), String> {
    open_session_window(&app, &session_id)
}

fn open_session_window(app: &tauri::AppHandle, session_id: &str) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let label = sessions::window_label_for(&session_id);
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        return Ok(());
    }
    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("")
        .inner_size(420.0, 747.0)
        .min_inner_size(340.0, 420.0)
        .always_on_top(false)
        .resizable(true)
        .decorations(true);
    if let Some(manager_window) = app.get_webview_window("main") {
        if let Ok(position) = manager_window.outer_position() {
            let scale = manager_window.scale_factor().unwrap_or(1.0);
            builder = builder.position(
                position.x as f64 / scale + 28.0,
                position.y as f64 / scale + 28.0,
            );
        }
    }
    builder.build().map(|_| ()).map_err(|e| e.to_string())
}

fn show_shortcut_session(app: &tauri::AppHandle) {
    let Some(surface) = focus::frontmost_chat_surface() else {
        return;
    };
    let state = app.state::<Arc<AppState>>();
    if let Some(session_id) = shortcut_session_id(&state, &surface, None, true) {
        if let Err(error) = open_session_window(app, &session_id) {
            eprintln!("[dooni] shortcut window error: {error}");
        }
    }
}

fn shortcut_session_id(
    state: &Arc<AppState>,
    surface: &str,
    conversation_id: Option<&str>,
    surfaced_only: bool,
) -> Option<String> {
    let surfaced = state.surfaced_sessions.lock().unwrap().clone();
    let sessions = state.session_meta.lock().unwrap();
    if let Some(conversation_id) = conversation_id {
        if let Some(session) = sessions.values().find(|session| {
            (!surfaced_only || surfaced.contains(&session.session_id))
                && session.surface == surface
                && session.source_conversation_id.as_deref() == Some(conversation_id)
        }) {
            return Some(session.session_id.clone());
        }
    }
    sessions
        .values()
        .filter(|session| {
            (!surfaced_only || surfaced.contains(&session.session_id)) && session.surface == surface
        })
        .max_by_key(|session| session.last_active)
        .map(|session| session.session_id.clone())
}

fn capture_selection_as_thought(app: &tauri::AppHandle) -> Result<(), String> {
    let surface = focus::fresh_frontmost_chat_surface()
        .ok_or_else(|| "open a Codex, Claude, or terminal chat first".to_string())?;
    let conversation_id = if surface == "claude-app" {
        focus::observe_frontmost_claude_desktop_with_id()
            .ok()
            .flatten()
            .and_then(|observation| observation.conversation_id)
    } else {
        None
    };
    let Some(text) = focus::selected_text_from_frontmost().map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let state = app.state::<Arc<AppState>>();
    let session_id = shortcut_session_id(&state, &surface, conversation_id.as_deref(), false)
        .ok_or_else(|| "dooni could not match the current chat".to_string())?;

    let captured_prompt_id = format!(
        "captured-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let future_prompts = {
        let mut sessions = state.session_meta.lock().unwrap();
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| "session is no longer tracked".to_string())?;
        if session.future_prompts.len() >= MAX_FUTURE_PROMPTS {
            return Err("this chat already has 100 thoughts".to_string());
        }
        session.future_prompts.push(session_store::FuturePrompt {
            id: captured_prompt_id.clone(),
            text,
            done: false,
        });
        let future_prompts = session.future_prompts.clone();
        session_store::save(&sessions).map_err(|error| error.to_string())?;
        future_prompts
    };
    let newly_surfaced = state
        .surfaced_sessions
        .lock()
        .unwrap()
        .insert(session_id.clone());
    if newly_surfaced {
        watcher::emit_surfaced_sessions(app, &state);
    }

    let label = sessions::window_label_for(&session_id);
    let _ = app.emit_to(
        label.as_str(),
        "future-prompts-updated",
        serde_json::json!({
            "session_id": session_id,
            "future_prompts": future_prompts,
            "captured_prompt_id": captured_prompt_id,
        }),
    );
    Ok(())
}

fn main() {
    let cfg = config::load();
    let mut meta = session_store::load();
    for session in meta.values_mut() {
        let project = sessions::project_label(session.project_dir.as_deref());
        session.project_name = project.clone();
        if !session.title_locked {
            session.title = sessions::limit_title(&sessions::title_without_project_prefix(
                &session.title,
                project.as_deref(),
            ));
        }
        if session.title.is_empty() {
            session.title = "Untitled chat".to_string();
        }
    }
    let prompts_by_session = meta
        .iter()
        .map(|(id, session)| (id.clone(), session.asked_prompts.clone()))
        .collect();
    // Persisted chats remain visible across app restarts. The watcher refreshes
    // their histories immediately and continues polling them for appended turns.
    let surfaced_sessions = surfaced_session_ids(&meta);
    let state = Arc::new(AppState {
        sessions: Mutex::new(HashMap::new()),
        prompts_by_session: Mutex::new(prompts_by_session),
        session_meta: Mutex::new(meta),
        surfaced_sessions: Mutex::new(surfaced_sessions),
        config: Mutex::new(cfg),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};
                    if event.state == ShortcutState::Pressed
                        && shortcut.matches(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyD)
                    {
                        show_shortcut_session(app);
                    } else if event.state == ShortcutState::Pressed
                        && shortcut.matches(Modifiers::SUPER | Modifiers::SHIFT, Code::Space)
                    {
                        if let Err(error) = capture_selection_as_thought(app) {
                            eprintln!("[dooni] capture shortcut error: {error}");
                        }
                    }
                })
                .build(),
        )
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            update_runtime_provider,
            get_codex_status,
            start_codex_login,
            open_codex_install,
            get_claude_desktop_access_status,
            open_claude_desktop_access,
            get_think_data,
            save_think_data,
            open_think_link,
            open_think_app,
            copy_think_content,
            get_sessions,
            get_session,
            rename_session,
            create_manual_session,
            focus_session,
            locate_asked_prompt,
            launch_session_window,
            save_future_prompts
        ])
        .setup(move |app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            if let Err(error) = app.global_shortcut().register("CmdOrCtrl+Shift+D") {
                eprintln!("[dooni] could not register Cmd+Shift+D: {error}");
            }
            if let Err(error) = app.global_shortcut().register("Command+Shift+Space") {
                eprintln!("[dooni] could not register Command+Shift+Space: {error}");
            }

            let handle = app.handle().clone();
            let s = state.clone();

            let hw = handle.clone();
            let sw = s.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = watcher::run(hw, sw).await {
                    eprintln!("[dooni] watcher error: {e:?}");
                }
            });

            // Chat discovery uses the JSONL watcher above.

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("dooni failed to launch");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut_session(id: &str, last_active: u64) -> session_store::SessionMeta {
        serde_json::from_value(serde_json::json!({
            "session_id": id,
            "agent": "codex",
            "title": id,
            "jsonl_path": format!("/tmp/{id}.jsonl"),
            "surface": "codex-app",
            "last_active": last_active,
        }))
        .unwrap()
    }

    #[test]
    fn codex_handoff_preserves_prompt_without_submitting() {
        let text = "Why? #notes & a link=https://example.com/?a=1&b=2\n你好";
        let (url, prefilled) = codex_thought_url(Some(text));
        assert!(prefilled);
        assert_eq!(url.scheme(), "codex");
        assert_eq!(url.host_str(), Some("threads"));
        assert_eq!(url.path(), "/new");
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            vec![("prompt".into(), text.into())]
        );
    }

    #[test]
    fn large_captures_open_blank_composer_for_clipboard_paste() {
        let (url, prefilled) = codex_thought_url(Some(&"x".repeat(16001)));
        assert!(!prefilled);
        assert_eq!(url.as_str(), "codex://threads/new");
    }

    #[test]
    fn capture_shortcut_can_match_and_surface_a_cold_start_chat() {
        let state = Arc::new(AppState::default());
        state
            .session_meta
            .lock()
            .unwrap()
            .insert("older".to_string(), shortcut_session("older", 1));
        state
            .session_meta
            .lock()
            .unwrap()
            .insert("current".to_string(), shortcut_session("current", 2));
        state
            .surfaced_sessions
            .lock()
            .unwrap()
            .insert("older".to_string());

        assert_eq!(
            shortcut_session_id(&state, "codex-app", None, true).as_deref(),
            Some("older")
        );
        assert_eq!(
            shortcut_session_id(&state, "codex-app", None, false).as_deref(),
            Some("current")
        );
    }

    #[test]
    fn a_manual_chat_keeps_its_link_and_stays_static() {
        let session = manual_session(
            "  Reading list  ",
            Some("https://claude.ai/chat/abc"),
            1_700,
        )
        .expect("https links are accepted");
        assert_eq!(session.title, "Reading list");
        assert_eq!(session.surface, session_store::MANUAL_SURFACE);
        assert_eq!(
            session.source_url.as_deref(),
            Some("https://claude.ai/chat/abc")
        );
        assert_eq!(session.session_id, "manual-1700");
        assert_eq!(session.last_active, 1);
        assert!(!session.running);
        // A manual title is the user's, so AI review must never overwrite it.
        assert!(session.title_locked);
    }

    #[test]
    fn a_manual_chat_can_be_a_bare_title() {
        let session = manual_session("", None, 1_700).unwrap();
        assert_eq!(session.title, "Untitled chat");
        assert_eq!(session.source_url, None);
    }

    #[test]
    fn a_manual_chat_link_must_be_a_web_address() {
        // `open` would happily launch any scheme, so only http(s) is accepted.
        assert!(manual_session("x", Some("file:///etc/passwd"), 1).is_err());
        assert!(manual_session("x", Some("  "), 1)
            .unwrap()
            .source_url
            .is_none());
    }

    #[test]
    fn persisted_sessions_are_surfaced_on_restart() {
        let sessions = HashMap::from([
            ("one".to_string(), shortcut_session("one", 20)),
            ("two".to_string(), shortcut_session("two", 10)),
        ]);
        let surfaced = surfaced_session_ids(&sessions);
        assert_eq!(
            surfaced,
            HashSet::from(["one".to_string(), "two".to_string()])
        );
    }
}
