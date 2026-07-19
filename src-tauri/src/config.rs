use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_TERMINAL_RETENTION_DAYS: u64 = 5;
pub const MAX_TERMINAL_RETENTION_DAYS: u64 = 3650;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_terminal_retention_days")]
    pub terminal_retention_days: u64,
}

fn default_agents() -> Vec<String> { vec!["claude".into(), "codex".into()] }
fn default_mode() -> String { "curt".into() }
fn default_terminal_retention_days() -> u64 { DEFAULT_TERMINAL_RETENTION_DAYS }

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            name: String::new(),
            agents: default_agents(),
            mode: default_mode(),
            onboarded: false,
            terminal_retention_days: default_terminal_retention_days(),
        }
    }
}

pub fn effective_terminal_retention_days(days: u64) -> u64 {
    if (1..=MAX_TERMINAL_RETENTION_DAYS).contains(&days) {
        days
    } else {
        DEFAULT_TERMINAL_RETENTION_DAYS
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_retention_uses_five_day_default() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.terminal_retention_days, DEFAULT_TERMINAL_RETENTION_DAYS);
        assert_eq!(Config::default().terminal_retention_days, DEFAULT_TERMINAL_RETENTION_DAYS);
    }

    #[test]
    fn invalid_retention_falls_back_to_default() {
        assert_eq!(effective_terminal_retention_days(0), DEFAULT_TERMINAL_RETENTION_DAYS);
        assert_eq!(
            effective_terminal_retention_days(MAX_TERMINAL_RETENTION_DAYS + 1),
            DEFAULT_TERMINAL_RETENTION_DAYS
        );
        assert_eq!(effective_terminal_retention_days(30), 30);
    }
}
