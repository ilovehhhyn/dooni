use anyhow::{anyhow, Result};
use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const CODEX_INSTALL_URL: &str = "https://developers.openai.com/codex/cli";

#[derive(Debug, Serialize)]
pub struct CodexStatus {
    pub installed: bool,
    pub authenticated: bool,
}

pub async fn status() -> CodexStatus {
    tokio::task::spawn_blocking(|| {
        let Some(binary) = find_binary() else {
            return CodexStatus {
                installed: false,
                authenticated: false,
            };
        };
        let authenticated = std::process::Command::new(binary)
            .args(["login", "status"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        CodexStatus {
            installed: true,
            authenticated,
        }
    })
    .await
    .unwrap_or(CodexStatus {
        installed: false,
        authenticated: false,
    })
}

pub async fn start_login() -> Result<()> {
    let binary = find_binary().ok_or_else(|| anyhow!("Codex CLI is not installed"))?;
    let mut child = tokio::process::Command::new(binary)
        .arg("login")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

pub fn open_install_page() -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };

    command
        .arg(CODEX_INSTALL_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

pub async fn complete(prompt: &str) -> Result<String> {
    let binary = find_binary().ok_or_else(|| anyhow!("Codex CLI is not installed"))?;
    let runtime_dir = std::env::temp_dir().join("dooni-codex-runtime");
    std::fs::create_dir_all(&runtime_dir)?;

    let mut child = tokio::process::Command::new(binary)
        .args([
            "exec",
            "--ephemeral",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--ignore-user-config",
            "--ignore-rules",
            "--color",
            "never",
            "-C",
        ])
        .arg(&runtime_dir)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to open Codex stdin"))?;
    stdin.write_all(prompt.as_bytes()).await?;
    drop(stdin);

    let output = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .map_err(|_| anyhow!("Codex timed out"))??;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(
            "Codex failed{}",
            if error.is_empty() {
                String::new()
            } else {
                format!(": {error}")
            }
        ));
    }

    let answer = String::from_utf8(output.stdout)?.trim().to_string();
    if answer.is_empty() {
        return Err(anyhow!("Codex returned an empty response"));
    }
    Ok(answer)
}

fn find_binary() -> Option<PathBuf> {
    if command_works(PathBuf::from("codex")) {
        return Some(PathBuf::from("codex"));
    }

    let home = dirs::home_dir();
    let mut candidates = vec![
        PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"),
        PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
        PathBuf::from("/usr/bin/codex"),
    ];
    if let Some(home) = home {
        candidates.extend([
            home.join(".local/bin/codex"),
            home.join(".npm-global/bin/codex"),
            home.join(".cargo/bin/codex"),
            home.join("Applications/ChatGPT.app/Contents/Resources/codex"),
            home.join("Applications/Codex.app/Contents/Resources/codex"),
        ]);
    }
    candidates
        .into_iter()
        .find(|path| command_works(path.clone()))
}

fn command_works(binary: PathBuf) -> bool {
    std::process::Command::new(binary)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
