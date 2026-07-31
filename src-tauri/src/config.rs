use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_runtime_provider", alias = "summary_provider")]
    pub runtime_provider: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_agents")]
    pub agents: Vec<String>,
    #[serde(default)]
    pub onboarded: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicConfig {
    pub runtime_provider: String,
    pub anthropic_connected: bool,
    pub name: String,
    pub agents: Vec<String>,
    pub onboarded: bool,
}

impl From<&Config> for PublicConfig {
    fn from(config: &Config) -> Self {
        Self {
            runtime_provider: config.runtime_provider.clone(),
            anthropic_connected: !effective_api_key(config).is_empty(),
            name: config.name.clone(),
            agents: config.agents.clone(),
            onboarded: config.onboarded,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            runtime_provider: default_runtime_provider(),
            name: String::new(),
            agents: default_agents(),
            onboarded: false,
        }
    }
}

fn default_agents() -> Vec<String> {
    vec!["claude".into(), "codex".into()]
}

fn default_runtime_provider() -> String {
    "codex".to_string()
}
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("dooni").join("config.json"))
}

pub fn load() -> Config {
    let Some(p) = config_path() else {
        return Config::default();
    };
    let Ok(bytes) = std::fs::read(&p) else {
        return Config::default();
    };
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> Config {
    let had_provider = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .get("runtime_provider")
                .or_else(|| value.get("summary_provider"))
                .cloned()
        })
        .is_some();
    let mut config: Config = serde_json::from_slice(bytes).unwrap_or_default();
    // Existing installs should see the new provider connection flow once
    // instead of silently attempting a runtime they have not connected.
    if config.onboarded && !had_provider {
        config.onboarded = false;
    }
    config
}

pub fn save(cfg: &Config) -> Result<()> {
    let Some(p) = config_path() else {
        return Ok(());
    };
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(cfg)?)?;
    Ok(())
}

pub fn effective_api_key(cfg: &Config) -> String {
    if !cfg.api_key.is_empty() {
        return cfg.api_key.clone();
    }
    std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_reopens_onboarding_with_codex_provider() {
        let config = parse(br#"{"api_key":"secret","onboarded":true}"#);
        assert_eq!(config.runtime_provider, "codex");
        assert!(!config.onboarded);
    }

    #[test]
    fn legacy_summary_provider_migrates_without_signing_out() {
        let config = parse(br#"{"summary_provider":"codex","onboarded":true}"#);
        assert_eq!(config.runtime_provider, "codex");
        assert!(config.onboarded);
    }

    #[test]
    fn public_config_never_contains_api_key() {
        let config = Config {
            api_key: "secret".to_string(),
            ..Config::default()
        };
        let value = serde_json::to_value(PublicConfig::from(&config)).unwrap();
        assert!(value.get("api_key").is_none());
        assert_eq!(value.get("anthropic_connected").unwrap(), true);
    }
}
