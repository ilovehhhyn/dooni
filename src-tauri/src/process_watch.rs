use std::time::Duration;
use sysinfo::{ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager};

pub async fn run(app: AppHandle) {
    let mut sys = System::new();
    let mut visible = true;
    loop {
        sys.refresh_processes(ProcessesToUpdate::All);
        let mut active = false;
        for (_pid, proc_) in sys.processes() {
            let name = proc_.name().to_string_lossy().to_lowercase();
            if name == "claude" || name == "codex" || name.contains("claude-code") {
                active = true;
                break;
            }
            // also check cmdline first arg
            if let Some(exe) = proc_.exe() {
                let s = exe.to_string_lossy().to_lowercase();
                if s.ends_with("/claude") || s.ends_with("/codex") {
                    active = true;
                    break;
                }
            }
        }
        let _ = app.emit("session-active", active);
        if let Some(win) = app.get_webview_window("main") {
            if active && !visible {
                let _ = win.show();
                visible = true;
            } else if !active && visible {
                // keep window visible on first launch; only hide after we've seen an active session
                // For MVP: keep always visible so user can inspect. Comment out hide.
                // let _ = win.hide();
                // visible = false;
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
