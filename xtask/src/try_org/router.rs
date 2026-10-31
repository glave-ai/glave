//! What try-org's stand-in says as OpenRouter: a few sample models, each served by a few
//! providers in different countries, at different precisions, prices and speeds, some keeping
//! nothing, so the model picker's filters can be tried with an OpenRouter account connected
//! with any key. Samples, not OpenRouter's catalog: its providers are made up, names and all,
//! and so are the prices and speeds.

use serde_json::{Value, json};

/// The sample providers: slug, name, the country each is based in, and its datacenters' where
/// said.
const PROVIDERS: [(&str, &str, &str, Option<&[&str]>); 7] = [
    ("yellowriver", "Yellow River Compute", "CN", Some(&["CN"])),
    ("northwind", "Northwind AI", "US", None),
    ("kestrel", "Kestrel Cloud", "US", Some(&["US"])),
    ("dunes", "Dunes GPU", "NL", Some(&["FI", "NL"])),
    ("bamboo", "Bamboo Inference", "CN", None),
    ("mesa", "Mesa Labs", "US", Some(&["US"])),
    ("harbor", "Harbor Cloud", "CN", Some(&["SG", "CN"])),
];

/// One provider's endpoint of a model: its slug, how its weights are stored, its price in
/// dollars per million tokens in and out, its speed, and whether it keeps nothing.
struct Serving {
    slug: &'static str,
    quantization: &'static str,
    input: f64,
    output: f64,
    speed: u32,
    keeps_nothing: bool,
}

/// The sample models: id, name, context, and who serves each.
fn models() -> Vec<(&'static str, &'static str, u64, Vec<Serving>)> {
    let serving = |slug, quantization, (input, output), speed, keeps_nothing| Serving {
        slug,
        quantization,
        input,
        output,
        speed,
        keeps_nothing,
    };
    vec![
        (
            "deepseek/deepseek-v4.1-flash",
            "DeepSeek: DeepSeek V4.1 Flash",
            131_072,
            vec![
                serving("yellowriver", "fp8", (0.10, 0.40), 90, false),
                serving("northwind", "fp8", (0.15, 0.60), 130, true),
                serving("kestrel", "fp4", (0.12, 0.50), 210, true),
                serving("dunes", "bf16", (0.20, 0.80), 80, false),
            ],
        ),
        (
            "moonshotai/kimi-k2.5",
            "MoonshotAI: Kimi K2.5",
            262_144,
            vec![
                serving("bamboo", "int4", (0.60, 2.50), 70, false),
                serving("mesa", "fp8", (0.45, 2.00), 110, true),
                serving("northwind", "fp4", (0.35, 1.60), 160, true),
            ],
        ),
        (
            "qwen/qwen3.6-27b",
            "Qwen: Qwen3.6 27B",
            131_072,
            vec![
                serving("harbor", "bf16", (0.25, 1.00), 90, false),
                serving("mesa", "fp8", (0.20, 0.80), 140, true),
                serving("dunes", "unknown", (0.18, 0.75), 120, false),
            ],
        ),
    ]
}

/// A price in dollars per million tokens as OpenRouter writes it: dollars a token.
fn per_token(dollars_per_million: f64) -> String {
    format!("{:.10}", dollars_per_million / 1_000_000.0)
}

/// The tag OpenRouter gives `serving`'s endpoint of a model.
fn tag(serving: &Serving) -> String {
    format!("{}/{}", serving.slug, serving.quantization)
}

/// The answer to a request for `path` under OpenRouter's API, as OpenRouter's would be:
/// the key's account (any key works), its models, providers, endpoints that keep nothing, or
/// one model's endpoints.
pub(super) fn answer(path: &str) -> Option<Value> {
    let path = path.split('?').next().unwrap_or(path);
    let listed = models();
    if path.ends_with("/key") {
        return Some(json!({"data": {"label": "try-org", "limit": null, "usage": 0}}));
    }
    if path.ends_with("/providers") {
        return Some(
            json!({"data": PROVIDERS.iter().map(|(slug, name, home, places)| json!({
            "slug": slug, "name": name, "headquarters": home, "datacenters": places,
        })).collect::<Vec<_>>()}),
        );
    }
    if path.ends_with("/endpoints/zdr") {
        let zdr = listed.iter().flat_map(|(id, _, _, servings)| {
            let kept = servings.iter().filter(|serving| serving.keeps_nothing);
            kept.map(move |serving| json!({"model_id": id, "tag": tag(serving)}))
        });
        return Some(json!({"data": zdr.collect::<Vec<_>>()}));
    }
    if let Some(id) = path
        .split_once("/models/")
        .and_then(|(_, rest)| rest.strip_suffix("/endpoints"))
    {
        let (_, _, context, servings) = listed.iter().find(|(known, ..)| *known == id)?;
        let endpoints = servings.iter().map(|serving| {
            let name = PROVIDERS
                .iter()
                .find(|(slug, ..)| *slug == serving.slug)
                .map_or(serving.slug, |(_, name, ..)| *name);
            json!({
                "provider_name": name, "tag": tag(serving), "quantization": serving.quantization,
                "context_length": context, "max_completion_tokens": 32_768,
                "pricing": {"prompt": per_token(serving.input),
                            "completion": per_token(serving.output)},
                "throughput_last_30m": {"p50": serving.speed},
            })
        });
        return Some(json!({"data": {"id": id, "endpoints": endpoints.collect::<Vec<_>>()}}));
    }
    if path.ends_with("/models") {
        let models = listed.iter().map(|(id, name, context, servings)| {
            let cheapest = servings
                .iter()
                .map(|serving| serving.input)
                .fold(f64::MAX, f64::min);
            let output = servings
                .iter()
                .map(|serving| serving.output)
                .fold(f64::MAX, f64::min);
            json!({
                "id": id, "name": name, "created": 1, "context_length": context,
                "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]},
                "supported_parameters": ["tools", "tool_choice", "max_tokens"],
                "pricing": {"prompt": per_token(cheapest), "completion": per_token(output)},
                "top_provider": {"context_length": context, "max_completion_tokens": 32_768},
            })
        });
        return Some(json!({"data": models.collect::<Vec<_>>()}));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_a_glave_keeping_credits_uses_is_one_the_lists_price() {
        let listed = answer("/api/v1/models").unwrap();
        let known = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["id"] == crate::try_org::model::PRICED_MODEL);
        assert!(known.is_some_and(|model| model["pricing"]["prompt"].is_string()));
    }

    #[test]
    fn it_answers_as_openrouter_for_each_list_glave_reads() {
        let models = answer("/api/v1/models").unwrap();
        assert_eq!(models["data"].as_array().unwrap().len(), 3);
        let providers = answer("/api/v1/providers").unwrap();
        assert_eq!(providers["data"][0]["headquarters"], "CN");
        assert_eq!(providers["data"][1]["datacenters"], Value::Null, "not said");
        let flash = answer("/api/v1/models/deepseek/deepseek-v4.1-flash/endpoints").unwrap();
        let endpoints = flash["data"]["endpoints"].as_array().unwrap();
        assert_eq!(endpoints.len(), 4);
        assert_eq!(endpoints[1]["provider_name"], "Northwind AI");
        assert_eq!(endpoints[1]["pricing"]["prompt"], "0.0000001500");
        let zdr = answer("/api/v1/endpoints/zdr").unwrap();
        assert!(
            zdr["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|kept| kept["tag"] == "kestrel/fp4")
        );
        assert!(answer("/api/v1/key").is_some(), "any key works");
        assert_eq!(answer("/api/v1/models/no/such/endpoints"), None);
        assert_eq!(answer("/v1/elsewhere"), None);
    }
}
