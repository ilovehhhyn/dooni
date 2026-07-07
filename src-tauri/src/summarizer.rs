use crate::Turn;
use anyhow::{anyhow, Result};
use serde_json::json;

const MODEL: &str = "claude-haiku-4-5-20251001";

pub async fn update_topics(
    current: &[String],
    recent: &[Turn],
    mode: &str,
    user_name: &str,
    api_key: &str,
) -> Result<Vec<String>> {
    if api_key.is_empty() {
        return Err(anyhow!("ANTHROPIC_API_KEY not set"));
    }

    let recent_text = recent
        .iter()
        .map(|t| {
            let mut body = t.text.clone();
            if body.len() > 800 { body.truncate(800); body.push_str("…"); }
            format!("[{}] {}", t.role, body)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let current_list = if current.is_empty() {
        "(empty)".to_string()
    } else {
        current.iter().enumerate().map(|(i, s)| format!("{}. {}", i + 1, s)).collect::<Vec<_>>().join("\n")
    };

    let mode_instructions = if mode == "wordy" {
        format!(
            "Mode: WORDY. New entries should be full descriptive sentences of the form \
            '{name} asked <thing>, and the assistant <did/said> <thing>.' \
            If a single user prompt covers multiple distinct asks, split it into multiple bullets. \
            When something is a clear 'aha' / breakthrough moment (root cause found, decision made), prefix the entry with 💡.",
            name = user_name
        )
    } else {
        "Mode: CURT. New entries should be short bullet-point topics (≤7 words each). \
        When something is a clear 'aha' / breakthrough moment, prefix the entry with 💡."
            .to_string()
    };

    let system = format!(
        "You maintain a running memo of a live coding chat session. The memo is a MONOTONICALLY GROWING list — \
         you may ADD new entries and lightly CLARIFY existing wording, but you must NEVER remove or shorten prior entries, \
         and you must NEVER change an existing entry's style (a curt bullet stays curt forever; a wordy sentence stays wordy). \
         The user may toggle modes between calls; only NEW entries follow the current mode. \
         \n\n{mode_instructions}\n\n\
         Given the CURRENT LIST and the RECENT MESSAGES, return the UPDATED LIST as a strict JSON array of strings. \
         Preserve every prior entry verbatim (or with minor clarifying tweaks). Append new entries at the end, oldest→newest. \
         Cap at 200 items. Output ONLY the JSON array — no prose, no code fences."
    );

    let user = format!(
        "CURRENT LIST:\n{}\n\nRECENT MESSAGES:\n{}\n\nReturn the updated JSON array now.",
        current_list, recent_text
    );

    let body = json!({
        "model": MODEL,
        "max_tokens": 2048,
        "system": system,
        "messages": [{ "role": "user", "content": user }]
    });

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Anthropic API {}: {}", status, text));
    }

    let v: serde_json::Value = resp.json().await?;
    let text = v
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.iter().find_map(|b| b.get("text").and_then(|t| t.as_str())))
        .ok_or_else(|| anyhow!("no text in response"))?
        .to_string();

    let cleaned = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let list: Vec<String> = serde_json::from_str(cleaned)
        .map_err(|e| anyhow!("failed to parse topic list: {e}; raw: {cleaned}"))?;
    Ok(list.into_iter().take(200).collect())
}
