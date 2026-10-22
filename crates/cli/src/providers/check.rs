//! Listing an account's models: when the person adds it, so a key that does not work is
//! refused at once with what to do, and again whenever its catalog is refreshed. The models
//! glave offers come from the answer, each as the listing describes it: newest first, only
//! the language models it can talk with, at most a thousand.

use std::cmp::Reverse;
use std::time::Duration;

use glave_event::{CatalogModel, ProviderKind};
use glave_ports::ProviderError;
use serde_json::Value;

use super::card::modalities;
use crate::config::Endpoint;
use crate::model::Model;

/// How long the check may take.
const CHECK: Duration = Duration::from_secs(20);

/// The most models one account offers.
const MOST: usize = 1_000;

/// The longest model id kept, in bytes, as the harness allows.
const LONGEST_ID: usize = 255;

/// Words in the ids of models that do not chat: speech, images, embeddings and the like. Only
/// for a listing that says nothing of what a model gives out, as OpenAI's own `/models` and
/// Ollama's: where a listing says (`language`), it is surer than a name, which can mislead.
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
pub(super) async fn models(
    kind: ProviderKind,
    endpoint: &Endpoint,
) -> Result<Vec<CatalogModel>, ProviderError> {
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

/// The models of `listing`, a `/models` answer, that glave offers for an account at `kind`,
/// each as the listing describes it.
pub(super) fn offered(kind: ProviderKind, listing: &Value) -> Vec<CatalogModel> {
    let Some(data) = listing.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut listed: Vec<(i64, &str, &Value)> = data
        .iter()
        .filter_map(|model| {
            let id = model.get("id")?.as_str()?.trim();
            let printable = !id.chars().any(|c| c.is_control() || c.is_whitespace());
            let usable =
                !id.is_empty() && id.len() <= LONGEST_ID && printable && chats(kind, id, model);
            usable.then(|| (created(model), id, model))
        })
        .collect();
    // Stable: models the service gives no date keep its order.
    listed.sort_by_key(|(created, _, _)| Reverse(*created));
    let mut models: Vec<CatalogModel> = Vec::new();
    for (_, id, model) in listed {
        if models.len() == MOST {
            break;
        }
        if !models.iter().any(|kept| kept.id == id) {
            models.push(super::card::described(id, model));
        }
    }
    models
}

/// Whether glave can talk with `id` at `kind`, as far as the listing tells: a language model
/// (`language`), which at OpenRouter must say it takes tools, and at OpenAI is one of its chat
/// models by its name's beginning.
fn chats(kind: ProviderKind, id: &str, model: &Value) -> bool {
    let lower = id.to_ascii_lowercase();
    if !language(&lower, model) {
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

/// Whether the model is a language model, by what the listing says it takes in and gives out:
/// text in (images in too, which is how glave shows a model the screen), and text out and
/// nothing else, no image, speech or embedding beside it. A listing that says nothing of what
/// it gives out leaves it to the model's lowercased id, `lower` (`NOT_CHAT`).
fn language(lower: &str, model: &Value) -> bool {
    let takes_text = modalities(model, "input_modalities")
        .is_none_or(|input| input.iter().any(|modality| modality == "text"));
    let gives_text = match modalities(model, "output_modalities") {
        Some(output) => output.iter().all(|modality| modality == "text"),
        None => !NOT_CHAT.iter().any(|word| lower.contains(word)),
    };
    takes_text && gives_text
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

    /// The ids `offered` gives.
    fn ids(kind: ProviderKind, listing: &Value) -> Vec<String> {
        offered(kind, listing).into_iter().map(|m| m.id).collect()
    }

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
            ids(ProviderKind::Openai, &listing),
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
        assert_eq!(ids(ProviderKind::Openrouter, &listing), ["a/talks"]);
    }

    #[test]
    fn openrouter_offers_language_models_by_what_each_takes_in_and_gives_out() {
        // Listed as OpenRouter's `/models` lists models, cut to what decides.
        let model = |id: &str, input: &[&str], output: &[&str]| {
            json!({
                "id": id,
                "created": 1,
                "architecture": {"input_modalities": input, "output_modalities": output},
                "supported_parameters": ["tools", "tool_choice", "max_tokens"],
            })
        };
        let listing = json!({"data": [
            // Makes images, though it gives text too and takes tools: left out, whatever its
            // name says.
            model("google/gemini-2.5-flash-image", &["image", "text"], &["image", "text"]),
            model("acme/painter", &["text"], &["text", "image"]),
            // Speaks, or gives embeddings: left out.
            model("openai/gpt-4o-audio-preview", &["text", "audio"], &["text", "audio"]),
            model("openai/text-embedding-3-small", &["text"], &["embeddings"]),
            // Reads images and files and writes text: kept, even when its name says image.
            model("anthropic/claude-sonnet-4.5", &["text", "image", "file"], &["text"]),
            model("acme/image-describer", &["image", "text"], &["text"]),
            // Takes no text in.
            model("acme/listener", &["audio"], &["text"]),
        ]});
        assert_eq!(
            ids(ProviderKind::Openrouter, &listing),
            ["anthropic/claude-sonnet-4.5", "acme/image-describer"]
        );
        // A listing that says nothing of what a model gives out, or says it with an empty
        // list, leaves it to the name; what it says it takes in still counts.
        let unsaid = json!({"data": [
            {"id": "a/embed-small", "supported_parameters": ["tools"]},
            {"id": "a/chat", "supported_parameters": ["tools"]},
            {"id": "a/blank", "architecture": {"output_modalities": []}, "supported_parameters": ["tools"]},
            {"id": "a/hears", "architecture": {"input_modalities": ["audio"]}, "supported_parameters": ["tools"]},
        ]});
        assert_eq!(
            ids(ProviderKind::Openrouter, &unsaid),
            ["a/chat", "a/blank"]
        );
    }

    #[test]
    fn anthropic_and_other_servers_offer_every_model_once_in_order() {
        let listing = json!({"data": [
            {"id": "claude-new", "created_at": "2026-09-01T00:00:00Z"},
            {"id": "claude-old", "created_at": "2025-01-01T00:00:00Z"},
        ]});
        assert_eq!(
            ids(ProviderKind::Anthropic, &listing),
            ["claude-new", "claude-old"]
        );
        let ollama = json!({"data": [
            {"id": "gemma4:12b"}, {"id": "nomic-embed-text"}, {"id": "qwen3:8b"},
            {"id": "gemma4:12b"}, {"id": " "}, {"id": "two\nlines"},
        ]});
        assert_eq!(
            ids(ProviderKind::Compatible, &ollama),
            ["gemma4:12b", "qwen3:8b"]
        );
        let unlisted = offered(ProviderKind::Compatible, &json!({"models": []}));
        assert!(unlisted.is_empty(), "{unlisted:?}");
    }

    #[test]
    fn an_account_offers_at_most_a_thousand_models() {
        let many: Vec<Value> = (0..1_500).map(|n| json!({"id": format!("m{n}")})).collect();
        let listing = json!({ "data": many });
        assert_eq!(offered(ProviderKind::Compatible, &listing).len(), MOST);
    }
}
