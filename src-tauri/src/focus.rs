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

use anyhow::Result;
use std::process::Command;

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

fn run_osascript(script: &str) -> Result<String> {
    let out = Command::new("osascript").arg("-e").arg(script).output()?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
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
