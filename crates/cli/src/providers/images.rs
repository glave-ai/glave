//! A router's image models (`specs/gateway.md`, "Catalogs" and "Media tools"): the ones glave's
//! image tiers name, each with its providers and what they bill, from OpenRouter's image API
//! (`images/models/{id}/endpoints`: a line for each thing billed, text and images in and images
//! out, by the token or by the image, some by a variant such as a larger size). When an
//! OpenRouter catalog is listed, glave reads them within one limit, so a server that keeps
//! credits holds an image call at its price and filters by country choose among its providers.
//! A provider is priced by the token, at the dearest of its lines for each part; one that bills
//! anything otherwise (by the image or the megapixel), or can't be read, leaves its model with
//! no price, and a model has one only when every provider of it does, at the dearest of theirs.

use std::collections::BTreeMap;
use std::sync::Arc;

use glave_event::{CatalogEndpoint, CatalogModel, Nanos, TokenPrice};
use serde_json::Value;

use super::endpoints::{MOST, READING, each, kept, plain};
use crate::model::Model;

/// Reads the image models `ids` and each one's providers through `model`, within [`READING`]
/// of its own: the models, and whether every one was read. A model not read keeps what
/// `before` held of it, until the listing that is due again an hour later reads it.
pub(super) async fn read(
    model: Model,
    ids: &[&str],
    before: &[CatalogModel],
) -> (Vec<CatalogModel>, bool) {
    let model = Arc::new(model);
    let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    let mut read = BTreeMap::new();
    let path = |id: &str| format!("images/models/{id}/endpoints");
    let reading = each(&model, &ids, (path, Arc::new(endpoints)), &mut read);
    let in_time = tokio::time::timeout(READING, reading).await.ok();
    if in_time.is_none() {
        let waited = glave_ports::spoken(READING);
        tracing::warn!("reading OpenRouter's image models took over {waited}: some wait");
    }
    let whole = in_time.unwrap_or(false);
    let models = ids
        .iter()
        .map(|id| match read.remove(id) {
            Some(endpoints) => {
                let mut card = CatalogModel::new(id.clone());
                card.price = dearest(&endpoints);
                card.endpoints = endpoints;
                card
            }
            None => before
                .iter()
                .find(|card| card.id == *id)
                .cloned()
                .unwrap_or_else(|| {
                    let mut card = CatalogModel::new(id.clone());
                    card.endpoints = kept(before, id);
                    card
                }),
        })
        .collect();
    (models, whole)
}

/// The providers of an image model's endpoint list `listed`, each by its slug, with its price;
/// none when any entry can't be read, so a model none of whose providers is known has no price.
fn endpoints(_: &str, listed: &Value) -> Vec<CatalogEndpoint> {
    let entries = listed.get("endpoints").and_then(Value::as_array);
    let mut endpoints = Vec::new();
    for entry in entries.into_iter().flatten().take(MOST) {
        let slug = entry.get("provider_slug").and_then(Value::as_str);
        let Some(provider) = slug.and_then(plain) else {
            return Vec::new();
        };
        endpoints.push(CatalogEndpoint {
            provider,
            quantization: None,
            context: None,
            max_output: None,
            price: entry.get("pricing").and_then(by_the_token),
            zdr: false,
            speed: None,
        });
    }
    endpoints
}

/// What an endpoint's pricing `lines` charge a million tokens, the dearest line for each of
/// what goes in (text, images) and what comes out (images); none when any line bills otherwise,
/// or nothing that comes out is priced.
fn by_the_token(lines: &Value) -> Option<TokenPrice> {
    let mut input = Nanos::ZERO;
    let mut output = None;
    for line in lines.as_array()? {
        let text = |key: &str| line.get(key).and_then(Value::as_str);
        if text("unit") != Some("token") {
            return None;
        }
        let cost = line.get("cost_usd")?;
        let per = Nanos::per_million_tokens(&cost.to_string())?;
        match text("billable")? {
            "output_image" => output = Some(output.map_or(per, |was: Nanos| was.max(per))),
            "input_image" | "input_text" | "input_reference" => input = input.max(per),
            _ => return None,
        }
    }
    Some(TokenPrice {
        input,
        output: output?,
        cache_read: None,
    })
}

/// The dearest of `endpoints`' prices, each part on its own, as a call's worst case holds it;
/// none unless there is one and every one has a price.
fn dearest(endpoints: &[CatalogEndpoint]) -> Option<TokenPrice> {
    let mut prices = endpoints.iter().map(|endpoint| endpoint.price);
    let first = prices.next()??;
    prices.try_fold(first, |dearest, price| {
        let price = price?;
        Some(TokenPrice {
            input: dearest.input.max(price.input),
            output: dearest.output.max(price.output),
            cache_read: None,
        })
    })
}

#[cfg(test)]
mod tests;
