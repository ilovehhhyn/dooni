use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_agents")]
    pub agents: Vec<String>,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub onboarded: bool,
}

fn default_agents() -> Vec<String> { vec!["claude".into(), "codex".into()] }
fn default_mode() -> String { "curt".into() }

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("dooni").join("config.json"))
}

pub fn load() -> Config {
    let Some(p) = config_path() else { return Config::default(); };
    let Ok(bytes) = std::fs::read(&p) else { return Config::default(); };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save(cfg: &Config) -> Result<()> {
    let Some(p) = config_path() else { return Ok(()); };
    if let Some(parent) = p.parent() { std::fs::create_dir_all(parent)?; }
    std::fs::write(&p, serde_json::to_vec_pretty(cfg)?)?;
    Ok(())
}

pub fn effective_api_key(cfg: &Config) -> String {
    if !cfg.api_key.is_empty() { return cfg.api_key.clone(); }
    std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
}
