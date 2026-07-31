use crate::Turn;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::json;

const MODEL: &str = "claude-sonnet-5";

#[derive(Debug, Deserialize)]
pub struct HistoryReview {
    pub title: String,
    pub exclude_prompt_indexes: Vec<usize>,
}

pub async fn review_history(
    current_title: &str,
    user_prompts: &[String],
    history_turns: &[Turn],
    force_new_title: bool,
    provider: &str,
    api_key: &str,
) -> Result<HistoryReview> {
    let prompts_text = user_prompts
        .iter()
        .enumerate()
        .map(|(index, prompt)| format!("{index}: {}", truncate(prompt, 800)))
        .collect::<Vec<_>>()
        .join("\n\n");
    let transcript_text = history_turns
        .iter()
        .map(|turn| format!("[{}] {}", turn.role, truncate(&turn.text, 2_000)))
        .collect::<Vec<_>>()
        .join("\n\n");

    let system = "\
You review the history of a live AI-assisted work conversation. Do not inspect files, run commands, browse, or use tools.

Return one strict JSON object with exactly these fields:
{\"title\":\"...\",\"exclude_prompt_indexes\":[0]}

FILTER RULES
- The numbered USER PROMPTS are copied verbatim from the source history.
- Exclude only standalone continuation signals, acknowledgements, or permission grants that add no new goal, constraint, detail, correction, or substantive question.
- Typical exclusions include \"proceed\", \"continue\", \"yes\", \"okay\", \"do it\", and \"go ahead\" when they stand alone.
- Keep short messages when they add meaningful intent, such as \"why?\", \"use Rust\", a correction, or a new constraint.
- Return zero-based indexes exactly as numbered. Never rewrite, summarize, merge, or reorder prompts.

TITLE RULES
- Answer this exact question in the title field: \"What is this chat about?\"
- Read the supplied chat content before answering.
- Write a comprehensible, specific description no longer than 180 characters.
- Do not include a folder or repository name; the application displays that separately.
- When MUST GENERATE NEW TITLE is yes, ignore the current title and write a fresh one.
- Otherwise keep the current title only when it remains a clear answer to the question.
- Replace filenames, raw IDs, directory names, and generic placeholders.
- Never include the product name \"Dooni\"; describe the actual objective instead.
- Avoid generic titles such as \"Coding session\", \"Project work\", or \"Conversation\".

Output only the JSON object. Do not use markdown or code fences.";

    let user = format!(
        "MUST GENERATE NEW TITLE: {}\n\nCURRENT TITLE:\n{current_title}\n\nQUESTION: What is this chat about?\n\nUSER PROMPTS:\n{prompts_text}\n\nCHAT CONTENT:\n{transcript_text}\n\nReturn the review object.",
        if force_new_title { "yes" } else { "no" }
    );

    let response_text = if provider == "codex" {
        crate::codex_runtime::complete(&format!("{system}\n\n{user}")).await?
    } else {
        if provider != "anthropic" {
            return Err(anyhow!("unsupported runtime provider: {provider}"));
        }
        if api_key.is_empty() {
            return Err(anyhow!("ANTHROPIC_API_KEY not set"));
        }
        let body = json!({
            "model": MODEL,
            "max_tokens": 1024,
            "temperature": 0,
            "system": system,
            "messages": [{ "role": "user", "content": user }]
        });
        let response = reqwest::Client::new()
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("Anthropic API {status}: {text}"));
        }
        let value: serde_json::Value = response.json().await?;
        value
            .get("content")
            .and_then(|content| content.as_array())
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find_map(|block| block.get("text").and_then(|text| text.as_str()))
            })
            .ok_or_else(|| anyhow!("no text in response"))?
            .to_string()
    };

    parse_history_review(&response_text, current_title, user_prompts.len())
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn parse_history_review(
    text: &str,
    current_title: &str,
    prompt_count: usize,
) -> Result<HistoryReview> {
    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let mut review: HistoryReview = serde_json::from_str(cleaned)
        .map_err(|error| anyhow!("failed to parse history review: {error}; raw: {cleaned}"))?;

    let unbranded_title = review
        .title
        .split_whitespace()
        .filter(|word| {
            !word
                .trim_matches(|character: char| !character.is_alphanumeric())
                .eq_ignore_ascii_case("dooni")
        })
        .collect::<Vec<_>>()
        .join(" ");
    review.title = crate::sessions::limit_title(&unbranded_title);
    if review.title.is_empty() {
        review.title = current_title.to_string();
    }
    review
        .exclude_prompt_indexes
        .retain(|index| *index < prompt_count);
    review.exclude_prompt_indexes.sort_unstable();
    review.exclude_prompt_indexes.dedup();
    Ok(review)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_bounds_excluded_indexes() {
        let raw = r#"{"title":"  Durable prompt history  ","exclude_prompt_indexes":[3,1,1,99]}"#;
        let review = parse_history_review(raw, "fallback", 4).unwrap();
        assert_eq!(review.title, "Durable prompt history");
        assert_eq!(review.exclude_prompt_indexes, vec![1, 3]);
    }

    #[test]
    fn preserves_current_title_when_model_returns_empty_title() {
        let raw = r#"{"title":"","exclude_prompt_indexes":[]}"#;
        let review = parse_history_review(raw, "Existing title", 0).unwrap();
        assert_eq!(review.title, "Existing title");
    }

    #[test]
    fn removes_product_brand_from_generated_titles() {
        let raw = r#"{"title":"Dooni Desktop App Interface Redesign","exclude_prompt_indexes":[]}"#;
        let review = parse_history_review(raw, "Existing title", 0).unwrap();
        assert_eq!(review.title, "Desktop App Interface Redesign");
    }

    #[test]
    fn truncates_on_character_boundaries() {
        assert_eq!(truncate("a🦀bc", 2), "a🦀…");
    }
}
