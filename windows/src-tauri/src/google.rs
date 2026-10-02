// Google AI (Gemini) chat client — the OpenAI-compatible endpoint, so the same
// request/response shape works for both "other" chat providers (ClaudeService's
// chatOpenAICompatible on macOS). Shares Chat's history with claude.rs: whatever
// provider answers, the conversation carries on where the other left off.

use serde_json::{json, Value};

use crate::claude::{Chat, ChatContext, ChatReply, SYSTEM_PROMPT};
use crate::secrets;

const CHAT_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";
const MODELS_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/openai/models";
pub const KEY_NAME: &str = "google-ai-api-key";
const MAX_TOKENS: u32 = 4096;

pub async fn send(
    chat: &Chat,
    model: &str,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let key = secrets::get(KEY_NAME)
        .ok_or_else(|| "Google AI API key missing. Open settings.".to_string())?;

    let mut msgs: Vec<Value> = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
    for m in chat.snapshot() {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
        msgs.push(json!({ "role": role, "content": flatten(m.get("content")) }));
    }

    // File / window context rides along with the first message only, exactly
    // like claude::send — and like ClaudeService.chatOpenAICompatible on macOS.
    let mut user_text = query.clone();
    if chat.is_empty() {
        match &context {
            Some(ChatContext::File { name, .. }) => user_text = format!("File: {name}\n\n{query}"),
            Some(ChatContext::Window { app_name, title, url }) => {
                let mut prefix = format!("Context — App: {app_name}, Window: {title}");
                if let Some(url) = url {
                    prefix.push_str(&format!(", URL: {url}"));
                }
                user_text = format!("{prefix}\n\n{query}");
            }
            None => {}
        }
    }
    msgs.push(json!({ "role": "user", "content": user_text }));

    // Stored as a plain string: Anthropic's API accepts a bare string for
    // `content` as readily as a block array, so this stays compatible if the
    // model is switched back to Claude mid-conversation.
    chat.push(json!({ "role": "user", "content": user_text }));

    let body = json!({ "model": model, "max_tokens": MAX_TOKENS, "messages": msgs });

    let reply = match call(&key, &body).await {
        Ok(v) => v,
        Err(err) => {
            chat.pop();
            return Err(err);
        }
    };

    let text = reply
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    if text.is_empty() {
        chat.pop();
        return Err("No response text.".into());
    }
    chat.push(json!({ "role": "assistant", "content": text }));
    Ok(ChatReply { text })
}

/// An Anthropic-shaped `content` (string, or an array of `{type, text}` blocks)
/// reduced to the plain string the OpenAI-compatible endpoint wants. Mirrors
/// ClaudeService.chatOpenAICompatible's own flattening on macOS.
fn flatten(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .and_then(|b| b.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

async fn call(key: &str, body: &Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .post(CHAT_ENDPOINT)
        .header("Authorization", format!("Bearer {key}"))
        .header("content-type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("error")?.get("message")?.as_str().map(str::to_string))
            .unwrap_or_else(|| text.chars().take(200).collect());
        return Err(format!("Google AI {status}: {detail}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("Bad API response: {e}"))
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
}

/// Substrings of a model id that mean "not a chat model" — embeddings, image
/// and video generators, audio/TTS, live sessions. Same filter as
/// ClaudeService.fetchGoogleModels on macOS.
const EXCLUDED: &[&str] = &["embed", "imagen", "veo", "aqa", "tts", "audio", "live"];

/// Roughly cheapest-first, so the dropdown's first entries are the ones worth
/// reaching for by default: flash-lite, then flash, then pro, then anything else.
fn cost_rank(id: &str) -> u8 {
    let id = id.to_lowercase();
    if id.contains("flash-lite") {
        0
    } else if id.contains("flash") {
        1
    } else if id.contains("pro") {
        2
    } else if id.contains("ultra") {
        3
    } else {
        4
    }
}

/// Every Gemini chat model the key can see, cheapest first. An empty result on
/// any error — the caller falls back to whatever is already selected.
pub async fn fetch_models(key: &str) -> Vec<ModelInfo> {
    let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build() else {
        return Vec::new();
    };
    let Ok(response) = client.get(MODELS_ENDPOINT).bearer_auth(key).send().await else {
        return Vec::new();
    };
    if !response.status().is_success() {
        return Vec::new();
    }
    let Ok(json) = response.json::<Value>().await else { return Vec::new() };
    let Some(items) = json.get("data").and_then(Value::as_array) else { return Vec::new() };

    let mut models: Vec<ModelInfo> = items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(|raw| raw.strip_prefix("models/").unwrap_or(raw).to_string())
        .filter(|id| {
            let lower = id.to_lowercase();
            !EXCLUDED.iter().any(|bad| lower.contains(bad))
        })
        .map(|id| ModelInfo { label: id.clone(), id })
        .collect();

    models.sort_by(|a, b| cost_rank(&a.id).cmp(&cost_rank(&b.id)).then_with(|| b.id.cmp(&a.id)));
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_block_shaped_content_both_flatten_to_text() {
        assert_eq!(flatten(Some(&json!("hi"))), "hi");
        assert_eq!(flatten(Some(&json!([{ "type": "text", "text": "hi" }]))), "hi");
        // A tool_use block with no text block present flattens to empty, not a crash.
        assert_eq!(flatten(Some(&json!([{ "type": "tool_use", "id": "x" }]))), "");
        assert_eq!(flatten(None), "");
    }

    #[test]
    fn cheap_models_rank_before_expensive_ones() {
        assert!(cost_rank("gemini-2.5-flash-lite") < cost_rank("gemini-2.5-flash"));
        assert!(cost_rank("gemini-2.5-flash") < cost_rank("gemini-2.5-pro"));
        assert!(cost_rank("gemini-2.5-pro") < cost_rank("gemini-ultra"));
    }
}
