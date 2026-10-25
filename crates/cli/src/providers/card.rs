//! One model as a service's model list describes it: whatever the listing says of its context,
//! output limit, images, price and reasoning, read the same way from every service, so nothing
//! about a model is written into glave. OpenRouter says all of it; vLLM says its context
//! (`max_model_len`); most services say little beyond the id.

use glave_event::{CatalogModel, Reasoning, TokenPrice};
use serde_json::Value;

/// The longest name kept, in bytes.
const LONGEST_NAME: usize = 200;

/// The most reasoning levels kept, and the longest name of one, in bytes.
const MOST_EFFORTS: usize = 12;
const LONGEST_EFFORT: usize = 32;

/// Decimal places of a price per token: a millionth of a dollar per million tokens.
const PLACES: u32 = 12;

/// `id`, as the listing's `model` entry describes it.
pub(super) fn described(id: &str, model: &Value) -> CatalogModel {
    let number = |path: &[&str]| path_in(model, path).and_then(Value::as_u64);
    CatalogModel {
        id: id.to_owned(),
        name: model
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= LONGEST_NAME && name != &id)
            .map(str::to_owned),
        context: number(&["context_length"])
            .or_else(|| number(&["max_model_len"]))
            .filter(|tokens| *tokens > 0),
        max_output: number(&["top_provider", "max_completion_tokens"]).filter(|t| *t > 0),
        images: modalities(model, "input_modalities")
            .map(|input| input.iter().any(|modality| modality == "image")),
        price: price(model.get("pricing")),
        reasoning: reasoning(model.get("reasoning")),
    }
}

/// What the listing says the model takes in (`input_modalities`) or gives out
/// (`output_modalities`): `text`, `image`, `audio` and the like, as OpenRouter's `architecture`
/// says; none when it does not say, or says an empty list.
pub(super) fn modalities<'a>(model: &'a Value, key: &str) -> Option<&'a [Value]> {
    path_in(model, &["architecture", key])
        .and_then(Value::as_array)
        .filter(|listed| !listed.is_empty())
        .map(Vec::as_slice)
}

/// The value at `path` in `value`.
fn path_in<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, key| value.get(key))
}

/// What the listing says the model's tokens cost: OpenRouter's `pricing`, in dollars per token
/// as decimal text. None when it says no price, or one below zero, as a router's is.
fn price(pricing: Option<&Value>) -> Option<TokenPrice> {
    let pricing = pricing?;
    let per = |key: &str| pricing.get(key).and_then(Value::as_str).and_then(micros);
    Some(TokenPrice {
        input: per("prompt")?,
        output: per("completion")?,
        cache_read: per("input_cache_read"),
    })
}

/// Dollars per token, such as `0.000003`, in millionths of a dollar per million tokens:
/// `3000000`. Exact to the twelfth decimal place; anything that is not a plain decimal at or
/// above zero is none.
fn micros(text: &str) -> Option<u64> {
    let (whole, fraction) = text.trim().split_once('.').unwrap_or((text.trim(), ""));
    let digits = |part: &str| part.chars().all(|c| c.is_ascii_digit());
    if whole.is_empty() || !digits(whole) || !digits(fraction) {
        return None;
    }
    let fraction = fraction.get(..fraction.len().min(PLACES as usize))?;
    let scale = 10u64.checked_pow(PLACES - u32::try_from(fraction.len()).ok()?)?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().ok()?
    };
    whole
        .parse::<u64>()
        .ok()?
        .checked_mul(10u64.pow(PLACES))?
        .checked_add(fraction.checked_mul(scale)?)
}

/// How the listing says the model reasons: OpenRouter's `reasoning`, with the levels it
/// offers (`supported_efforts`), whether it always reasons (`mandatory`), and the level it
/// uses unasked, known only where the listing says whether it reasons unasked: the level
/// reasoning starts at (`default_effort`) when it always reasons or does by default
/// (`default_enabled`), and `none` when it does not by default.
fn reasoning(reasoning: Option<&Value>) -> Option<Reasoning> {
    let reasoning = reasoning.filter(|value| value.is_object())?;
    let level = |value: &Value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|level| {
                !level.is_empty()
                    && level.len() <= LONGEST_EFFORT
                    && level.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .map(str::to_owned)
    };
    let mut efforts: Vec<String> = Vec::new();
    for effort in reasoning
        .get("supported_efforts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(level)
    {
        if efforts.len() < MOST_EFFORTS && !efforts.contains(&effort) {
            efforts.push(effort);
        }
    }
    let mandatory = reasoning
        .get("mandatory")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let unasked = match reasoning.get("default_enabled").and_then(Value::as_bool) {
        Some(true) => reasoning.get("default_effort").and_then(level),
        _ if mandatory => reasoning.get("default_effort").and_then(level),
        Some(false) => Some("none".to_owned()),
        None => None,
    };
    Some(Reasoning {
        default: unasked.filter(|default| efforts.contains(default)),
        efforts,
        mandatory,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openrouter_says_everything_and_it_is_kept() {
        let model = json!({
            "id": "anthropic/claude-haiku-5.5",
            "name": "Anthropic: Claude Haiku 5.5",
            "context_length": 1_000_000,
            "architecture": {"input_modalities": ["text", "image", "file"]},
            "pricing": {
                "prompt": "0.0000001", "completion": "0.0000005",
                "input_cache_read": "0.00000001", "web_search": "0.01",
            },
            "top_provider": {"context_length": 1_000_000, "max_completion_tokens": 128_000},
            "reasoning": {
                "mandatory": false,
                "default_enabled": true,
                "supported_efforts": ["max", "xhigh", "high", "medium", "low", "none"],
                "default_effort": "medium",
            },
        });
        let card = described("anthropic/claude-haiku-5.5", &model);
        assert_eq!(card.name.as_deref(), Some("Anthropic: Claude Haiku 5.5"));
        assert_eq!(card.context, Some(1_000_000));
        assert_eq!(card.max_output, Some(128_000));
        assert_eq!(card.images, Some(true));
        assert_eq!(
            card.price,
            Some(TokenPrice {
                input: 100_000,
                output: 500_000,
                cache_read: Some(10_000),
            })
        );
        let reasoning = card.reasoning.unwrap();
        assert_eq!(
            reasoning.efforts,
            ["max", "xhigh", "high", "medium", "low", "none"]
        );
        assert_eq!(reasoning.default.as_deref(), Some("medium"));
        assert!(!reasoning.mandatory);
    }

    #[test]
    fn a_listing_that_says_little_describes_little() {
        let bare = described("gpt-6", &json!({"id": "gpt-6", "created": 1}));
        assert_eq!(bare, CatalogModel::new("gpt-6"));
        let vllm = described("qwen", &json!({"id": "qwen", "max_model_len": 32_768}));
        assert_eq!(vllm.context, Some(32_768));
        let text_only = described(
            "m",
            &json!({"architecture": {"input_modalities": ["text"]}}),
        );
        assert_eq!(text_only.images, Some(false));
    }

    #[test]
    fn prices_are_exact_and_a_router_s_unknown_price_is_none() {
        assert_eq!(micros("0.000003"), Some(3_000_000));
        assert_eq!(micros("0.000000037476"), Some(37_476));
        assert_eq!(micros("0"), Some(0));
        assert_eq!(micros("2"), Some(2_000_000_000_000));
        assert_eq!(micros("0.0000000000001"), Some(0), "past the twelfth place");
        for bad in ["-1", "", ".5", "1e-7", "0.00x", "18446744073709551615"] {
            assert_eq!(micros(bad), None, "{bad}");
        }
        let router = json!({"pricing": {"prompt": "-1", "completion": "-1"}});
        assert_eq!(described("openrouter/auto", &router).price, None);
    }

    #[test]
    fn reasoning_keeps_only_plain_levels_and_a_default_among_them() {
        let model = json!({"reasoning": {
            "mandatory": true,
            "supported_efforts": ["high", "high", "", "two words", 3, "low"],
            "default_effort": "medium",
        }});
        let reasoning = described("m", &model).reasoning.unwrap();
        assert_eq!(reasoning.efforts, ["high", "low"]);
        assert_eq!(reasoning.default, None, "medium is not offered");
        assert!(reasoning.mandatory);
        let on_off = described("m", &json!({"reasoning": {"mandatory": false}}));
        assert_eq!(on_off.reasoning.map(|r| r.efforts.len()), Some(0));
        assert_eq!(described("m", &json!({"reasoning": null})).reasoning, None);
    }

    #[test]
    fn the_level_used_unasked_is_known_only_where_the_listing_says_whether_it_reasons_unasked() {
        let unasked = |mandatory: bool, enabled: Option<bool>| {
            let mut reasoning = json!({
                "mandatory": mandatory,
                "supported_efforts": ["high", "medium", "low", "none"],
                "default_effort": "medium",
            });
            if let (Some(fields), Some(enabled)) = (reasoning.as_object_mut(), enabled) {
                fields.insert("default_enabled".into(), json!(enabled));
            }
            described("m", &json!({ "reasoning": reasoning }))
                .reasoning
                .and_then(|reasoning| reasoning.default)
        };
        assert_eq!(unasked(false, Some(true)).as_deref(), Some("medium"));
        assert_eq!(unasked(true, None).as_deref(), Some("medium"));
        assert_eq!(unasked(false, Some(false)).as_deref(), Some("none"));
        assert_eq!(
            unasked(false, None),
            None,
            "medium is where reasoning starts"
        );
        let cannot_turn_off = json!({"reasoning": {
            "default_enabled": false,
            "supported_efforts": ["high", "low"],
            "default_effort": "low",
        }});
        let reasoning = described("m", &cannot_turn_off).reasoning.unwrap();
        assert_eq!(reasoning.default, None, "none is not offered");
    }
}
