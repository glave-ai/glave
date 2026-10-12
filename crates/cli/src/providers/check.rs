//! Checking a key when the person adds it: one small call that lists the account's models, so
//! a key that does not work is refused at once with what to do, and the models glave offers
//! come from the answer: newest first, only those it can talk with, at most 100.

use std::cmp::Reverse;
use std::time::Duration;

use glave_event::ProviderKind;
use glave_ports::ProviderError;
use serde_json::Value;

use crate::config::Endpoint;
use crate::model::Model;

/// How long the check may take.
const CHECK: Duration = Duration::from_secs(20);

/// The most models one account offers.
const MOST: usize = 100;

/// The longest model id kept, in bytes, as the harness allows.
const LONGEST_ID: usize = 255;

/// Words in the ids of models that do not chat: speech, images, embeddings and the like.
const NOT_CHAT: &[&str] = &[
    "audio",
    "babbage",
    "dall-e",
    "davinci",
    "embed",
    "image",
    "moderation",
    "realtime",
    "search",
    "transcribe",
    "tts",
    "whisper",
];

/// The models the account at `endpoint` offers, asked with its key: an error when the key
/// does not work there, or the service cannot be reached.
pub async fn models(kind: ProviderKind, endpoint: &Endpoint) -> Result<Vec<String>, ProviderError> {
    let model = Model::for_endpoint(endpoint)?;
    let asked = async {
        // OpenRouter lists its models to anyone, so the key is checked on its own.
        if kind == ProviderKind::Openrouter {
            model.get("key").await?;
        }
        let path = match kind {
            ProviderKind::Anthropic => "models?limit=1000",
            _ => "models",
        };
        model.get(path).await
    };
    let listing =
        tokio::time::timeout(CHECK, asked)
            .await
            .map_err(|_| ProviderError::Timeout {
                seconds: CHECK.as_secs(),
            })??;
    Ok(offered(kind, &listing))
}

/// The models of `listing`, a `/models` answer, that glave offers for an account at `kind`.
pub fn offered(kind: ProviderKind, listing: &Value) -> Vec<String> {
    let Some(data) = listing.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut listed: Vec<(i64, &str)> = data
        .iter()
        .filter_map(|model| {
            let id = model.get("id")?.as_str()?.trim();
            let usable = !id.is_empty() && id.len() <= LONGEST_ID && chats(kind, id, model);
            usable.then(|| (created(model), id))
        })
        .collect();
    // Stable: models the service gives no date keep its order.
    listed.sort_by_key(|(created, _)| Reverse(*created));
    let mut models: Vec<String> = Vec::new();
    for (_, id) in listed {
        if models.len() == MOST {
            break;
        }
        if !models.iter().any(|kept| kept == id) {
            models.push(id.to_owned());
        }
    }
    models
}

/// Whether glave can talk with `id` at `kind`, as far as the listing tells: OpenRouter says
/// which models take tools; OpenAI lists speech, image and embedding models beside chat ones.
fn chats(kind: ProviderKind, id: &str, model: &Value) -> bool {
    let lower = id.to_ascii_lowercase();
    if NOT_CHAT.iter().any(|word| lower.contains(word)) {
        return false;
    }
    match kind {
        ProviderKind::Openai => {
            let reasoning =
                lower.starts_with('o') && lower.chars().nth(1).is_some_and(|c| c.is_ascii_digit());
            lower.starts_with("gpt-") || lower.starts_with("chatgpt-") || reasoning
        }
        ProviderKind::Openrouter => model
            .get("supported_parameters")
            .and_then(Value::as_array)
            .is_some_and(|parameters| parameters.iter().any(|p| p == "tools")),
        ProviderKind::Anthropic | ProviderKind::Compatible => true,
    }
}

/// When the model was released, in seconds, as the listing says: `created` (OpenAI and
/// OpenRouter) or `created_at` (Anthropic); 0 when it does not.
fn created(model: &Value) -> i64 {
    if let Some(seconds) = model.get("created").and_then(Value::as_i64) {
        return seconds;
    }
    model
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        .map_or(0, jiff::Timestamp::as_second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_offers_its_chat_models_newest_first() {
        let listing = json!({"data": [
            {"id": "gpt-old", "created": 1},
            {"id": "text-embedding-3-large", "created": 9},
            {"id": "gpt-new", "created": 5},
            {"id": "o4-mini", "created": 3},
            {"id": "gpt-realtime", "created": 8},
            {"id": "whisper-1", "created": 7},
            {"id": "omni-moderation-latest", "created": 6},
        ]});
        assert_eq!(
            offered(ProviderKind::Openai, &listing),
            ["gpt-new", "o4-mini", "gpt-old"]
        );
    }

    #[test]
    fn openrouter_offers_the_models_that_take_tools() {
        let listing = json!({"data": [
            {"id": "a/talks", "created": 2, "supported_parameters": ["tools", "temperature"]},
            {"id": "b/silent", "created": 3, "supported_parameters": ["temperature"]},
            {"id": "c/unknown", "created": 4},
        ]});
        assert_eq!(offered(ProviderKind::Openrouter, &listing), ["a/talks"]);
    }

    #[test]
    fn anthropic_and_other_servers_offer_every_model_once_in_order() {
        let listing = json!({"data": [
            {"id": "claude-new", "created_at": "2026-09-01T00:00:00Z"},
            {"id": "claude-old", "created_at": "2025-01-01T00:00:00Z"},
        ]});
        assert_eq!(
            offered(ProviderKind::Anthropic, &listing),
            ["claude-new", "claude-old"]
        );
        let ollama = json!({"data": [
            {"id": "gemma4:12b"}, {"id": "nomic-embed-text"}, {"id": "qwen3:8b"},
            {"id": "gemma4:12b"}, {"id": " "},
        ]});
        assert_eq!(
            offered(ProviderKind::Compatible, &ollama),
            ["gemma4:12b", "qwen3:8b"]
        );
        let unlisted = offered(ProviderKind::Compatible, &json!({"models": []}));
        assert!(unlisted.is_empty(), "{unlisted:?}");
    }

    #[test]
    fn an_account_offers_at_most_a_hundred_models() {
        let many: Vec<Value> = (0..150).map(|n| json!({"id": format!("m{n}")})).collect();
        let listing = json!({ "data": many });
        assert_eq!(offered(ProviderKind::Compatible, &listing).len(), MOST);
    }
}
