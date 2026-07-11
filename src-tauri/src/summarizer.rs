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
        "Mode: CURT. Entries are short, glanceable bullets (≤7 words) that capture the OUTCOME or decision — \
        what was done, found, or decided, not just the topic. Prefer 'Fixed always-on-top toggle' over \
        'Lock button', or 'Chose SQLite over JSON store' over 'Storage'. \
        Before adding, scan the list: if an entry already covers this, REFINE that entry instead of adding a \
        near-duplicate. Prefix a clear 'aha' / breakthrough moment with 💡."
            .to_string()
    };

    let system = format!(
        "You maintain a running memo of a live coding chat session — a compact, evolving record of what has happened. \
         Favor stability: keep existing entries and APPEND new ones as the conversation moves on. But you MAY also \
         MERGE two entries that describe the same thing into one, REFINE an entry's wording to be clearer or more \
         specific, or UPDATE an entry once its outcome becomes known (e.g. a question that later got answered, or a \
         plan that got carried out). Do NOT wipe the list or drop meaningful history, and keep each entry in its \
         original mode's style (a curt bullet stays curt, a wordy sentence stays wordy). \
         The user may toggle modes between calls; new entries follow the current mode. \
         \n\n{mode_instructions}\n\n\
         Given the CURRENT LIST and the RECENT MESSAGES, return the UPDATED LIST as a strict JSON array of strings, \
         ordered oldest→newest. Cap at 200 items. Output ONLY the JSON array — no prose, no code fences."
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

/// Produce a short (≈3–7 word) human-readable title that recaps the OVERALL
/// topic of a chat session, used as the memo window's title instead of
/// "session-123".
///
/// Anchored on the session's original request plus the full running memo, so the
/// title reflects the session's main issue rather than the latest tangent, and
/// captures both the subject matter and the nature of the work.
pub async fn generate_title(
    first_prompt: Option<&str>,
    topics: &[String],
    api_key: &str,
) -> Result<String> {
    if api_key.is_empty() {
        return Err(anyhow!("ANTHROPIC_API_KEY not set"));
    }

    let mut context = String::new();
    if let Some(fp) = first_prompt {
        let mut p = fp.trim().to_string();
        if p.len() > 800 { p.truncate(800); p.push('…'); }
        if !p.is_empty() {
            context.push_str(&format!("ORIGINAL REQUEST (what started the session):\n{p}\n\n"));
        }
    }
    if !topics.is_empty() {
        // Cap how much memo we send; the earliest + latest entries matter most.
        let joined = topics.join("\n- ");
        context.push_str(&format!("RUNNING MEMO OF THE SESSION:\n- {joined}\n"));
    }
    if context.trim().is_empty() {
        return Err(anyhow!("no context for title"));
    }

    let system = "You write the title for a coding-chat session window. You are given the ORIGINAL REQUEST that \
                  opened the session and a RUNNING MEMO of everything that has happened since. Write a title that \
                  accurately recaps what the session is actually about.\n\n\
                  A good title:\n\
                  - Names the real subject specifically — the feature, bug, file, tool, or system in play — and, \
                  when it fits, the nature of the work (debugging, back-and-forth, setup, refactor, investigation).\n\
                  - Reflects the dominant, recurring thread across the WHOLE memo, not just the latest message or a \
                  minor tangent.\n\
                  - Reads the way a person skimming the chat would summarize it.\n\n\
                  Avoid:\n\
                  - Vague one-word tags like 'Storage', 'Bug', or 'Setup'.\n\
                  - Titles that describe only the last step when the session was mostly about something else.\n\
                  - The repo or project name (it is shown separately).\n\n\
                  Examples (request+memo → title):\n\
                  - Why the app wouldn't start, then installing the missing toolchain → Debugging why it won't launch\n\
                  - A long iterative discussion refining a PTY integration → PTY implementation back and forth\n\
                  - Chose SQLite over a JSON store and wired it up → Switching storage to SQLite\n\n\
                  Output ONLY the title: a single phrase of roughly 3 to 7 words, sentence case (capitalize the \
                  first word plus any acronyms or proper nouns), no quotes, no colon, no trailing punctuation.";

    let body = json!({
        "model": MODEL,
        "max_tokens": 40,
        "system": system,
        "messages": [{ "role": "user", "content": format!("{context}\nWrite the session title now:") }]
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
        .ok_or_else(|| anyhow!("no text in response"))?;

    let title = text
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '.')
        .trim()
        .to_string();
    if title.is_empty() {
        return Err(anyhow!("empty title"));
    }
    // Guard against a runaway response; keep it short.
    Ok(title.chars().take(60).collect())
}
