//! Best-effort focus of the terminal window/tab running a given chat session.
//!
//! MVP strategy (macOS only):
//! 1. Try iTerm2 first: iterate windows/tabs/sessions, activate one whose
//!    working-directory ends with `project_dir`, or whose name contains
//!    the project basename.
//! 2. Fall back to Terminal.app: iterate windows and tabs, matching against
//!    the tab's custom title / tty working dir via a heuristic on the title.
//! 3. If nothing matches, just activate the frontmost terminal app so the user
//!    can find it themselves.
//!
//! Returns Ok(true) if a specific tab was focused, Ok(false) if we only
//! activated the app without pinpointing a tab.

use anyhow::{anyhow, Result};
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_foundation::url::CFURL;
use serde::Serialize;
use std::ffi::c_void;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const CG_HID_EVENT_TAP: u32 = 0;
const CG_EVENT_FLAG_COMMAND: u64 = 1 << 20;
const KEY_F: u16 = 3;
const KEY_V: u16 = 9;
const KEY_ESCAPE: u16 = 53;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: CFTypeRef,
    ) -> i32;
    fn CGEventCreateKeyboardEvent(
        source: *mut c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const c_void);
}

#[derive(Debug, Clone, Serialize)]
pub struct LocatePromptResult {
    pub exact_chat_opened: bool,
    pub search_succeeded: bool,
    pub excerpt_copied: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeDesktopAccessStatus {
    pub installed: bool,
    pub authorized: bool,
}

#[derive(Debug, Clone)]
pub struct ClaudeDesktopObservation {
    pub composer: Option<String>,
    pub conversation_id: Option<String>,
    pub window_title: Option<String>,
}

#[derive(Debug)]
struct ComposerCandidate {
    score: i32,
    text: String,
}

pub fn claude_desktop_access_status() -> ClaudeDesktopAccessStatus {
    let installed = std::path::Path::new("/Applications/Claude.app").exists()
        || dirs::home_dir()
            .map(|home| home.join("Applications").join("Claude.app").exists())
            .unwrap_or(false);
    ClaudeDesktopAccessStatus {
        installed,
        authorized: accessibility_enabled(),
    }
}

/// Asks macOS for Accessibility access. The system prompt registers whichever
/// binary is running right now, which matters because dooni is ad-hoc signed:
/// every rebuild changes its signature, so a grant made against an earlier build
/// no longer applies and the stale row in System Settings stays checked while
/// `AXIsProcessTrusted` keeps reporting false.
fn prompt_for_accessibility() -> bool {
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
    // SAFETY: the dictionary outlives the call and the API only reads from it.
    unsafe { AXIsProcessTrustedWithOptions(options.as_CFTypeRef() as *const c_void) != 0 }
}

pub fn open_claude_desktop_access() -> Result<()> {
    let status = if claude_desktop_access_status().installed {
        if prompt_for_accessibility() {
            return Ok(());
        }
        Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .status()?
    } else {
        Command::new("open")
            .arg("https://claude.com/download")
            .status()?
    };
    if !status.success() {
        return Err(anyhow!("could not open Claude Desktop setup"));
    }
    Ok(())
}

/// How long we keep re-asking Claude Desktop to expose its accessibility tree
/// before we settle for polling whatever the app has built so far.
const MANUAL_ACCESSIBILITY_DEADLINE: Duration = Duration::from_secs(5);

struct ManualAccessibility {
    pid: i32,
    requested_at: Instant,
    ready: bool,
}

static MANUAL_ACCESSIBILITY: OnceLock<Mutex<Option<ManualAccessibility>>> = OnceLock::new();

fn manual_accessibility_cell() -> &'static Mutex<Option<ManualAccessibility>> {
    MANUAL_ACCESSIBILITY.get_or_init(|| Mutex::new(None))
}

fn manual_accessibility_needs_request(ready: bool, elapsed: Duration) -> bool {
    !ready && elapsed < MANUAL_ACCESSIBILITY_DEADLINE
}

/// Claude Desktop is an Electron app, and Chromium keeps its web-content
/// accessibility tree switched off until an assistive client asks for it. Until
/// then the focused window has no descendants, so the composer is invisible and
/// no prompt is ever recorded. Setting `AXManualAccessibility` turns the tree on;
/// building it is asynchronous, so we keep asking until the composer shows up or
/// the deadline passes.
fn request_manual_accessibility(application: CFTypeRef, pid: i32) {
    let mut guard = manual_accessibility_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let restarted = guard.as_ref().map(|state| state.pid != pid).unwrap_or(true);
    if restarted {
        *guard = Some(ManualAccessibility {
            pid,
            requested_at: Instant::now(),
            ready: false,
        });
    }
    let Some(state) = guard.as_ref() else {
        return;
    };
    if !manual_accessibility_needs_request(state.ready, state.requested_at.elapsed()) {
        return;
    }
    let attribute = CFString::new("AXManualAccessibility");
    let enabled = CFBoolean::true_value();
    // SAFETY: the element, attribute, and value are live CF objects for the
    // duration of the call, and the setter does not take ownership of them.
    unsafe {
        AXUIElementSetAttributeValue(
            application as *const c_void,
            attribute.as_CFTypeRef() as *const c_void,
            enabled.as_CFTypeRef(),
        );
    }
}

/// Records that the tree finished building, so we stop re-asking for it.
fn note_manual_accessibility_ready(pid: i32) {
    let mut guard = manual_accessibility_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(state) = guard.as_mut() {
        if state.pid == pid {
            state.ready = true;
        }
    }
}

pub fn observe_frontmost_claude_desktop() -> Result<Option<ClaudeDesktopObservation>> {
    observe_frontmost_claude_desktop_inner(false)
}

pub fn observe_frontmost_claude_desktop_with_id() -> Result<Option<ClaudeDesktopObservation>> {
    observe_frontmost_claude_desktop_inner(true)
}

fn observe_frontmost_claude_desktop_inner(
    scan_for_conversation_id: bool,
) -> Result<Option<ClaudeDesktopObservation>> {
    if !accessibility_enabled() || frontmost_chat_surface().as_deref() != Some("claude-app") {
        return Ok(None);
    }
    let output = Command::new("pgrep").arg("-x").arg("Claude").output()?;
    let Some(pid) = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<i32>().ok())
    else {
        return Ok(None);
    };

    // SAFETY: AXUIElementCreateApplication returns an owned CF object for a
    // live PID. CFType assumes ownership and releases it when dropped.
    let application_ref = unsafe { AXUIElementCreateApplication(pid) };
    if application_ref.is_null() {
        return Ok(None);
    }
    let application = unsafe { CFType::wrap_under_create_rule(application_ref as CFTypeRef) };
    request_manual_accessibility(application.as_CFTypeRef(), pid);
    let mut observation = ClaudeDesktopObservation {
        composer: None,
        conversation_id: None,
        window_title: None,
    };

    let mut composer_candidate = None;
    if let Some(focused) = ax_attribute(application.as_CFTypeRef(), "AXFocusedUIElement") {
        scan_composer_tree(
            focused.as_CFTypeRef(),
            80,
            &mut composer_candidate,
            &mut 800usize,
        );
    }

    if let Some(window) = ax_attribute(application.as_CFTypeRef(), "AXFocusedWindow") {
        observation.window_title = ax_string(window.as_CFTypeRef(), "AXTitle")
            .filter(|title| !title.trim().is_empty() && !title.eq_ignore_ascii_case("Claude"));
        if composer_candidate
            .as_ref()
            .map(|candidate| candidate.score < 100)
            .unwrap_or(true)
        {
            scan_composer_tree(
                window.as_CFTypeRef(),
                0,
                &mut composer_candidate,
                &mut 2_000usize,
            );
        }
        observation.composer = composer_candidate.map(|candidate| candidate.text);
        if observation.composer.is_some() {
            note_manual_accessibility_ready(pid);
        }
        if scan_for_conversation_id {
            scan_accessibility_tree(window.as_CFTypeRef(), &mut observation, &mut 4_000usize);
        }
    }
    Ok(Some(observation))
}

fn scan_composer_tree(
    element: CFTypeRef,
    focus_bonus: i32,
    best: &mut Option<ComposerCandidate>,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }
    *remaining -= 1;

    let role = ax_string(element, "AXRole").unwrap_or_default();
    if matches!(
        role.as_str(),
        "AXTextArea"
            | "AXTextField"
            | "AXSearchField"
            | "AXComboBox"
            | "AXWebArea"
            | "AXGroup"
            | "AXUnknown"
    ) {
        let metadata = [
            "AXDescription",
            "AXHelp",
            "AXPlaceholderValue",
            "AXRoleDescription",
            "AXTitle",
            "AXDOMIdentifier",
            "AXDOMClassList",
        ]
        .into_iter()
        .filter_map(|attribute| ax_string(element, attribute))
        .collect::<Vec<_>>()
        .join(" ");
        if let (Some(score), Some(text)) = (
            composer_candidate_score(&role, &metadata, focus_bonus),
            ax_string(element, "AXValue"),
        ) {
            if text.chars().count() <= 20_000
                && best
                    .as_ref()
                    .map(|candidate| score > candidate.score)
                    .unwrap_or(true)
            {
                *best = Some(ComposerCandidate { score, text });
            }
        }
    }

    let Some(children_value) = ax_attribute(element, "AXChildren") else {
        return;
    };
    let Some(children) = children_value.downcast::<CFArray>() else {
        return;
    };
    for child in children.iter() {
        let child_ref = *child as CFTypeRef;
        if !child_ref.is_null() {
            scan_composer_tree(child_ref, focus_bonus, best, remaining);
            if *remaining == 0 {
                return;
            }
        }
    }
}

fn composer_candidate_score(role: &str, metadata: &str, focus_bonus: i32) -> Option<i32> {
    let metadata = metadata.to_ascii_lowercase();
    let mentions_composer = [
        "message",
        "reply",
        "ask claude",
        "chat input",
        "prompt",
        "composer",
    ]
    .iter()
    .any(|hint| metadata.contains(hint));
    let mentions_search = metadata.contains("search") || role == "AXSearchField";
    let mut score = match role {
        "AXTextArea" => 80,
        "AXTextField" | "AXSearchField" | "AXComboBox" => 40,
        "AXWebArea" | "AXGroup" | "AXUnknown" if mentions_composer => 55,
        _ => return None,
    };
    if mentions_composer {
        score += 100;
    }
    if mentions_search {
        score -= 200;
    }
    Some(score + focus_bonus)
}

fn ax_attribute(element: CFTypeRef, name: &str) -> Option<CFType> {
    let attribute = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: both inputs are valid CF objects for the duration of the call;
    // a successful copy follows Core Foundation's create rule.
    let result = unsafe {
        AXUIElementCopyAttributeValue(
            element as *const c_void,
            attribute.as_CFTypeRef() as *const c_void,
            &mut value,
        )
    };
    if result != 0 || value.is_null() {
        return None;
    }
    Some(unsafe { CFType::wrap_under_create_rule(value) })
}

fn ax_string(element: CFTypeRef, name: &str) -> Option<String> {
    ax_attribute(element, name).and_then(|value| cf_text(&value))
}

fn cf_text(value: &CFType) -> Option<String> {
    if let Some(string) = value.downcast::<CFString>() {
        return Some(string.to_string());
    }
    value
        .downcast::<CFURL>()
        .map(|url| url.get_string().to_string())
}

fn scan_accessibility_tree(
    element: CFTypeRef,
    observation: &mut ClaudeDesktopObservation,
    remaining: &mut usize,
) {
    if *remaining == 0 || observation.conversation_id.is_some() {
        return;
    }
    *remaining -= 1;
    for attribute in ["AXURL", "AXValue", "AXDescription", "AXTitle"] {
        if let Some(value) = ax_string(element, attribute) {
            if let Some(id) = conversation_id_from_text(&value) {
                observation.conversation_id = Some(id);
                return;
            }
        }
    }
    let Some(children_value) = ax_attribute(element, "AXChildren") else {
        return;
    };
    let Some(children) = children_value.downcast::<CFArray>() else {
        return;
    };
    for child in children.iter() {
        let child_ref = *child as CFTypeRef;
        if !child_ref.is_null() {
            scan_accessibility_tree(child_ref, observation, remaining);
            if observation.conversation_id.is_some() {
                return;
            }
        }
    }
}

fn conversation_id_from_text(text: &str) -> Option<String> {
    for marker in ["/chat/", "/conversation/"] {
        let Some(start) = text.find(marker).map(|index| index + marker.len()) else {
            continue;
        };
        let id = text[start..]
            .chars()
            .take_while(|character| character.is_ascii_hexdigit() || *character == '-')
            .collect::<String>();
        if id.len() == 36 && safe_conversation_id(&id) {
            return Some(id);
        }
    }
    None
}

/// Map the application that currently owns the menu bar to the surface values
/// stored with chat sessions. The shortcut intentionally does nothing when it
/// is pressed from an unrelated application.
/// How long a frontmost-app reading stays good enough to reuse.
const FRONTMOST_CACHE_MILLIS: u64 = 900;

static FRONTMOST_CACHE: OnceLock<Mutex<Option<(Instant, Option<String>)>>> = OnceLock::new();

/// The Claude Desktop observer polls at 350ms, and each `frontmost_chat_surface`
/// call spawns an `osascript` process. Left uncached that is roughly three
/// process launches a second for as long as dooni runs, which is far more
/// expensive than the question deserves. Reuse a recent answer instead; being
/// under a second stale only delays noticing an app switch by one poll.
pub fn frontmost_chat_surface() -> Option<String> {
    let cell = FRONTMOST_CACHE.get_or_init(|| Mutex::new(None));
    if let Some((measured_at, cached)) = cell
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        if measured_at.elapsed() < Duration::from_millis(FRONTMOST_CACHE_MILLIS) {
            return cached.clone();
        }
    }
    let surface = read_frontmost_chat_surface();
    *cell.lock().unwrap_or_else(|error| error.into_inner()) =
        Some((Instant::now(), surface.clone()));
    surface
}

fn read_frontmost_chat_surface() -> Option<String> {
    let name = run_osascript(
        r#"tell application "System Events"
    return name of first application process whose frontmost is true
end tell"#,
    )
    .ok()?
    .trim()
    .to_ascii_lowercase();

    if name.contains("codex") || name.contains("chatgpt") {
        return Some("codex-app".to_string());
    }
    if name == "claude" || name.contains("claude") {
        return Some("claude-app".to_string());
    }
    if [
        "terminal",
        "iterm",
        "warp",
        "ghostty",
        "wezterm",
        "kitty",
        "alacritty",
    ]
    .iter()
    .any(|terminal| name.contains(terminal))
    {
        return Some("terminal".to_string());
    }
    None
}

pub fn focus_chat_for(surface: &str, project_dir: Option<&str>) -> Result<bool> {
    if surface == "codex-app" {
        return activate_running_app(&["ChatGPT", "Codex"]);
    }
    if surface == "claude-app" {
        return activate_running_app(&["Claude"]);
    }
    focus_terminal_for(project_dir)
}

/// Open the exact provider chat when a supported deep link exists, then use
/// the app's regular Find command to move to a distinctive prompt excerpt.
/// Search automation requires macOS Accessibility permission; the excerpt is
/// left on the clipboard as a useful fallback when that permission is absent.
pub fn locate_prompt(
    surface: &str,
    conversation_id: Option<&str>,
    project_dir: Option<&str>,
    prompt: &str,
    _occurrence: usize,
) -> Result<LocatePromptResult> {
    let excerpt = prompt_search_excerpt(prompt);
    if excerpt.is_empty() {
        return Err(anyhow!("prompt has no searchable text"));
    }
    copy_to_clipboard(&excerpt)?;

    let mut exact_chat_opened = false;
    let process_names: &[&str] = match surface {
        "codex-app" => {
            if let Some(id) = conversation_id.filter(|id| safe_conversation_id(id)) {
                open_deep_link(&format!("codex://threads/{id}"))?;
                exact_chat_opened = true;
            } else {
                let _ = activate_running_app(&["ChatGPT", "Codex"])?;
            }
            &["ChatGPT", "Codex"]
        }
        "claude-app" => {
            if let Some(id) = conversation_id.filter(|id| safe_conversation_id(id)) {
                open_deep_link(&format!("claude://claude.ai/chat/{id}"))?;
                exact_chat_opened = true;
            } else {
                let _ = activate_running_app(&["Claude"])?;
            }
            &["Claude"]
        }
        _ => {
            let focused = focus_terminal_for(project_dir)?;
            if !focused {
                return Ok(LocatePromptResult {
                    exact_chat_opened: false,
                    search_succeeded: false,
                    excerpt_copied: true,
                    message: "No matching terminal tab was found. Search text copied.".to_string(),
                });
            }
            &["iTerm2", "iTerm", "Terminal", "Warp", "Ghostty"]
        }
    };

    // Give a provider deep link a moment to finish changing conversations.
    if exact_chat_opened {
        std::thread::sleep(Duration::from_millis(850));
    }

    if !accessibility_enabled() {
        let destination = if exact_chat_opened {
            "Opened the chat"
        } else {
            "Focused the chat app"
        };
        return Ok(LocatePromptResult {
            exact_chat_opened,
            search_succeeded: false,
            excerpt_copied: true,
            message: format!(
                "{destination}. Search text copied; allow dooni in Accessibility to auto-locate."
            ),
        });
    }

    let Some(process_name) = process_names
        .iter()
        .find(|name| app_is_installed(name))
        .copied()
    else {
        return Ok(LocatePromptResult {
            exact_chat_opened,
            search_succeeded: false,
            excerpt_copied: true,
            message: "Chat opened. Search text copied, but its app window was not found."
                .to_string(),
        });
    };

    if run_find_shortcut(process_name).is_err() {
        return Ok(LocatePromptResult {
            exact_chat_opened,
            search_succeeded: false,
            excerpt_copied: true,
            message:
                "Chat opened. Search text copied; allow dooni in Accessibility to auto-locate."
                    .to_string(),
        });
    }
    Ok(LocatePromptResult {
        exact_chat_opened,
        search_succeeded: true,
        excerpt_copied: true,
        message: if exact_chat_opened {
            "Opened the original chat and located this prompt.".to_string()
        } else {
            "Focused the chat and searched for this prompt.".to_string()
        },
    })
}

pub fn focus_terminal_for(project_dir: Option<&str>) -> Result<bool> {
    let dir = project_dir.unwrap_or("").to_string();
    let basename = std::path::Path::new(&dir)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Try iTerm2 first.
    if app_is_installed("iTerm") {
        let script = iterm_script(&dir, &basename);
        if let Ok(out) = run_osascript(&script) {
            if out.trim() == "matched" {
                return Ok(true);
            }
        }
    }

    // Fall back to Terminal.app.
    if app_is_installed("Terminal") {
        let script = terminal_script(&dir, &basename);
        if let Ok(out) = run_osascript(&script) {
            if out.trim() == "matched" {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn activate_running_app(names: &[&str]) -> Result<bool> {
    for name in names {
        if app_is_installed(name) {
            let script = format!(
                r#"tell application "{}" to activate"#,
                name.replace('"', "\\\"")
            );
            run_osascript(&script)?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn run_osascript(script: &str) -> Result<String> {
    let out = Command::new("osascript").arg("-e").arg(script).output()?;
    if !out.status.success() {
        let message = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(anyhow!(if message.is_empty() {
            "AppleScript failed".to_string()
        } else {
            message
        }));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn open_deep_link(url: &str) -> Result<()> {
    let status = Command::new("open").arg(url).status()?;
    if !status.success() {
        return Err(anyhow!("could not open chat link"));
    }
    Ok(())
}

fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut child = Command::new("pbcopy").stdin(Stdio::piped()).spawn()?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow!("could not open clipboard"))?
        .write_all(text.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        return Err(anyhow!("could not copy search text"));
    }
    Ok(())
}

fn accessibility_enabled() -> bool {
    // SAFETY: AXIsProcessTrusted takes no arguments and returns a CoreServices
    // Boolean. Calling it attributes the permission check to Dooni itself.
    unsafe { AXIsProcessTrusted() != 0 }
}

fn run_find_shortcut(process_name: &str) -> Result<()> {
    let status = Command::new("open").arg("-a").arg(process_name).status()?;
    if !status.success() {
        return Err(anyhow!("could not focus chat app"));
    }
    std::thread::sleep(Duration::from_millis(180));
    post_key(KEY_F, true)?;
    std::thread::sleep(Duration::from_millis(220));
    post_key(KEY_V, true)?;
    std::thread::sleep(Duration::from_millis(320));
    post_key(KEY_ESCAPE, false)
}

fn post_key(key: u16, command: bool) -> Result<()> {
    for key_down in [true, false] {
        // SAFETY: CoreGraphics accepts a null event source, returns an owned
        // CGEvent, and permits posting it to the HID event tap. We release each
        unsafe {
            let event = CGEventCreateKeyboardEvent(std::ptr::null_mut(), key, key_down);
            if event.is_null() {
                return Err(anyhow!("could not create keyboard event"));
            }
            if command {
                CGEventSetFlags(event, CG_EVENT_FLAG_COMMAND);
            }
            CGEventPost(CG_HID_EVENT_TAP, event);
            CFRelease(event);
        }
    }
    Ok(())
}

fn safe_conversation_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

fn prompt_search_excerpt(prompt: &str) -> String {
    const MAX_CHARS: usize = 80;
    let best_line = prompt
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .max_by_key(|line| line.chars().count())
        .unwrap_or_default();
    if best_line.chars().count() <= MAX_CHARS {
        return best_line;
    }
    let clipped = best_line.chars().take(MAX_CHARS).collect::<String>();
    clipped
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, _)| clipped[..index].trim_end().to_string())
        .filter(|excerpt| excerpt.chars().count() >= 40)
        .unwrap_or(clipped)
}

fn app_is_installed(name: &str) -> bool {
    // Returns true only when the app is currently running; this prevents us
    // from launching iTerm/Terminal just to look for a matching tab.
    let script = format!(
        r#"try
    tell application "System Events"
        if (exists application process "{n}") then return "true"
    end tell
end try
return "false""#,
        n = name
    );
    matches!(run_osascript(&script), Ok(s) if s.trim() == "true")
}

fn iterm_script(dir: &str, basename: &str) -> String {
    // Escape quotes.
    let d = dir.replace('"', "\\\"");
    let b = basename.replace('"', "\\\"");
    format!(
        r#"tell application "iTerm"
    activate
    set matched to false
    try
        repeat with w in windows
            repeat with t in tabs of w
                repeat with s in sessions of t
                    set nm to ""
                    try
                        set nm to name of s
                    end try
                    set cwd to ""
                    try
                        set cwd to (variable named "session.path" of s)
                    end try
                    if ("{d}" is not "" and cwd ends with "{d}") or ("{b}" is not "" and (nm contains "{b}" or cwd contains "{b}")) then
                        tell w to select
                        tell t to select
                        select s
                        set matched to true
                        exit repeat
                    end if
                end repeat
                if matched then exit repeat
            end repeat
            if matched then exit repeat
        end repeat
    end try
    if matched then
        return "matched"
    else
        return "activated"
    end if
end tell"#,
        d = d,
        b = b
    )
}

fn terminal_script(dir: &str, basename: &str) -> String {
    let d = dir.replace('"', "\\\"");
    let b = basename.replace('"', "\\\"");
    format!(
        r#"tell application "Terminal"
    activate
    set matched to false
    try
        repeat with w in windows
            repeat with t in tabs of w
                set nm to ""
                try
                    set nm to custom title of t
                end try
                if nm is missing value then set nm to ""
                set proc to ""
                try
                    set proc to (processes of t) as string
                end try
                if ("{b}" is not "" and (nm contains "{b}" or proc contains "{b}")) or ("{d}" is not "" and nm contains "{d}") then
                    set selected of t to true
                    set frontmost of w to true
                    set matched to true
                    exit repeat
                end if
            end repeat
            if matched then exit repeat
        end repeat
    end try
    if matched then
        return "matched"
    else
        return "activated"
    end if
end tell"#,
        d = d,
        b = b
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_is_single_line_and_bounded() {
        let excerpt = prompt_search_excerpt(
            "short\nThis is the much longer searchable prompt line with enough words to clip cleanly at a boundary and not cross a newline.",
        );
        assert!(!excerpt.contains('\n'));
        assert!(excerpt.chars().count() <= 80);
        assert!(excerpt.chars().count() >= 40);
    }

    #[test]
    fn conversation_ids_reject_url_syntax() {
        assert!(safe_conversation_id("019fb0db-b463-79b0-b306-85cdc4b48878"));
        assert!(!safe_conversation_id("../../settings"));
        assert!(!safe_conversation_id("thread?id=x"));
    }

    #[test]
    fn reads_claude_conversation_id_from_accessible_url() {
        assert_eq!(
            conversation_id_from_text(
                "https://claude.ai/chat/77109920-2746-4688-8f72-741372e71d64"
            )
            .as_deref(),
            Some("77109920-2746-4688-8f72-741372e71d64")
        );
    }

    #[test]
    fn composer_scoring_prefers_prompt_area_over_search() {
        let prompt = composer_candidate_score("AXTextArea", "Message Claude", 0).unwrap();
        let search = composer_candidate_score("AXTextField", "Search", 80).unwrap();
        assert!(prompt > search);
    }

    #[test]
    fn manual_accessibility_retries_until_ready_or_deadline() {
        assert!(manual_accessibility_needs_request(
            false,
            Duration::from_secs(0)
        ));
        assert!(manual_accessibility_needs_request(
            false,
            MANUAL_ACCESSIBILITY_DEADLINE - Duration::from_millis(1)
        ));
        assert!(!manual_accessibility_needs_request(
            false,
            MANUAL_ACCESSIBILITY_DEADLINE
        ));
        assert!(!manual_accessibility_needs_request(
            true,
            Duration::from_secs(0)
        ));
    }

    #[test]
    fn composer_scoring_accepts_hinted_electron_groups() {
        assert!(composer_candidate_score("AXGroup", "chat input composer", 0).is_some());
        assert!(composer_candidate_score("AXGroup", "conversation content", 80).is_none());
    }
}
