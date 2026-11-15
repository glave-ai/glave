//! What the public lists of OpenAI-compatible services say, read the same way wherever glave
//! reads them (`specs/gateway.md`, "Catalogs"): numbers and prices in a model list, and
//! OpenRouter's providers (`providers`: each one's slug, name and where it is based), the
//! endpoints that keep nothing (`endpoints/zdr`) and each model's endpoints
//! (`models/{id}/endpoints`). What a list does not say is left out, never guessed.

use std::collections::{BTreeMap, BTreeSet};

use glave_event::{CatalogEndpoint, CatalogProvider, Nanos, TokenPrice};
use serde_json::Value;

/// The longest slug, name or country kept, in bytes.
const LONGEST: usize = 100;

/// The most providers kept, endpoints kept of one model, and datacenters of one provider.
pub const MOST: usize = 500;

/// The whole number at `path` in `value`, rounded when the listing writes it with decimals.
pub fn number(value: &Value, path: &[&str]) -> Option<u64> {
    let found = path_in(value, path)?;
    found.as_u64().or_else(|| {
        let rounded = format!("{:.0}", found.as_f64()?);
        rounded.parse().ok()
    })
}

/// The value at `path` in `value`.
pub fn path_in<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, key| value.get(key))
}

/// What the listing says the model's tokens cost: OpenRouter's `pricing`, in dollars per token
/// as decimal text. None when it says no price, or one below zero, as a router's is.
pub fn price(pricing: Option<&Value>) -> Option<TokenPrice> {
    let pricing = pricing?;
    let per = |key: &str| {
        let text = pricing.get(key).and_then(Value::as_str)?;
        Nanos::per_million_tokens(text.trim())
    };
    Some(TokenPrice {
        input: per("prompt")?,
        output: per("completion")?,
        cache_read: per("input_cache_read"),
    })
}

/// `text` when it is a short, plain word or name.
pub fn plain(text: &str) -> Option<String> {
    let text = text.trim();
    let fits = !text.is_empty() && text.len() <= LONGEST;
    (fits && !text.chars().any(char::is_control)).then(|| text.to_owned())
}

/// A country as OpenRouter writes it, `US`: two letters.
fn country(value: &Value) -> Option<String> {
    let code = value.as_str()?.trim();
    let letters = code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic());
    letters.then(|| code.to_ascii_uppercase())
}

/// OpenRouter's providers, from its `providers` list. Datacenters it lists as none are as
/// unknown as ones it does not list.
pub fn providers(listed: &Value) -> Vec<CatalogProvider> {
    let entries = listed.get("data").and_then(Value::as_array);
    let mut providers: Vec<CatalogProvider> = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in entries.into_iter().flatten() {
        let text = |key: &str| entry.get(key).and_then(Value::as_str).and_then(plain);
        let (Some(slug), Some(name)) = (text("slug"), text("name")) else {
            continue;
        };
        if providers.len() >= MOST || !seen.insert(slug.clone()) {
            continue;
        }
        let mut datacenters: Vec<String> = entry
            .get("datacenters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(country)
            .take(MOST)
            .collect();
        datacenters.sort_unstable();
        datacenters.dedup();
        providers.push(CatalogProvider {
            slug,
            name,
            headquarters: entry.get("headquarters").and_then(country),
            datacenters: (!datacenters.is_empty()).then_some(datacenters),
        });
    }
    providers
}

/// The endpoints OpenRouter lists as keeping nothing (`endpoints/zdr`), by their model's id and
/// their tag.
pub fn keeping_nothing(listed: &Value) -> BTreeSet<(String, String)> {
    let entries = listed.get("data").and_then(Value::as_array);
    entries
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let model = entry.get("model_id")?.as_str()?;
            let tag = entry.get("tag")?.as_str()?;
            Some((model.to_owned(), tag.to_owned()))
        })
        .collect()
}

/// The endpoints of model `id` in its endpoint list `listed` that can call tools, as every
/// call glave makes may, each by its provider's slug: the provider OpenRouter names (`named`,
/// slugs by name), else its tag's first part.
pub fn endpoints(
    id: &str,
    listed: &Value,
    named: &BTreeMap<String, String>,
    zdr: &BTreeSet<(String, String)>,
) -> Vec<CatalogEndpoint> {
    let entries = listed.get("data").and_then(|data| data.get("endpoints"));
    let entries = entries.and_then(Value::as_array);
    let mut endpoints = Vec::new();
    for entry in entries.into_iter().flatten().take(MOST) {
        let tag = entry.get("tag").and_then(Value::as_str).unwrap_or_default();
        let by_name = entry
            .get("provider_name")
            .and_then(Value::as_str)
            .and_then(|name| named.get(name).cloned());
        let by_tag = tag.split('/').next().and_then(plain);
        let Some(provider) = by_name.or(by_tag) else {
            continue;
        };
        if !calls_tools(entry) {
            continue;
        }
        let quantization = entry
            .get("quantization")
            .and_then(Value::as_str)
            .and_then(plain)
            .filter(|quantization| quantization != "unknown");
        endpoints.push(CatalogEndpoint {
            provider,
            quantization,
            context: number(entry, &["context_length"]).filter(|tokens| *tokens > 0),
            max_output: number(entry, &["max_completion_tokens"]).filter(|tokens| *tokens > 0),
            price: price(entry.get("pricing")),
            zdr: zdr.contains(&(id.to_owned(), tag.to_owned())),
            speed: number(entry, &["throughput_last_30m", "p50"]).filter(|speed| *speed > 0),
        });
    }
    endpoints
}

/// Whether the endpoint `entry` takes tools, as OpenRouter lists what it takes: one that
/// lists nothing may.
fn calls_tools(entry: &Value) -> bool {
    entry
        .get("supported_parameters")
        .and_then(Value::as_array)
        .is_none_or(|taken| taken.iter().any(|each| each.as_str() == Some("tools")))
}
