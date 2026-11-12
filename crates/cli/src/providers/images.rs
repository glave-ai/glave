//! A router's image models (`specs/gateway.md`, "Catalogs" and "Media tools"): OpenRouter's
//! image API lists them apart from its chat models (`images/models`), and each one's providers
//! with what they bill (`images/models/{id}/endpoints`: a line for each thing billed, text and
//! images in and images out, by the token or by the image, some by a variant such as a larger
//! size). When an OpenRouter catalog is listed, glave reads them, a few at a time within one
//! limit, so a server that keeps credits holds an image call at its price and filters by
//! country choose among its providers. A provider is priced by the token, at the dearest of
//! its lines for each; one that bills anything otherwise (by the image or the megapixel) has no
//! price, and a model has one only when every provider of it does, at the dearest of theirs.

use std::collections::BTreeMap;
use std::sync::Arc;

use glave_event::{CatalogEndpoint, CatalogModel, Nanos, TokenPrice};
use serde_json::Value;

use super::endpoints::{MOST, READING, each, kept, plain};
use crate::model::Model;

/// The longest model id kept, in bytes, as the harness allows.
const LONGEST_ID: usize = 255;

/// The longest model name kept, in bytes.
const LONGEST_NAME: usize = 200;

/// Reads OpenRouter's image models and each one's providers through `model`, within
/// [`READING`]: the models, and whether every one's providers were read in time. A model whose
/// providers are not read keeps those `before` held. Refused, saying what, when the models
/// themselves cannot be listed.
pub(super) async fn read(
    model: Model,
    before: &[CatalogModel],
) -> Result<(Vec<CatalogModel>, bool), String> {
    let model = Arc::new(model);
    let mut listed = None;
    let mut read = BTreeMap::new();
    let lists = async {
        let models = model.get("images/models").await;
        let models = cards(&models.map_err(|error| format!("its image models: {error}"))?);
        let ids: Vec<String> = models.iter().map(|card| card.id.clone()).collect();
        listed = Some(models);
        let path = |id: &str| format!("images/models/{id}/endpoints");
        each(&model, &ids, (path, Arc::new(endpoints)), &mut read).await;
        Ok::<_, String>(())
    };
    let whole = if let Ok(read) = tokio::time::timeout(READING, lists).await {
        read.map(|()| true)?
    } else {
        let waited = glave_ports::spoken(READING);
        tracing::warn!("reading OpenRouter's image models took over {waited}: some wait");
        false
    };
    let mut models = listed.ok_or_else(|| {
        let waited = glave_ports::spoken(READING);
        format!("its image models were not listed within {waited}")
    })?;
    for card in &mut models {
        card.endpoints = read
            .remove(&card.id)
            .unwrap_or_else(|| kept(before, &card.id));
        card.price = dearest(&card.endpoints);
    }
    Ok((models, whole))
}

/// The image models of `listed`, an `images/models` answer, in its order: each by its id and
/// name, and whether it takes images in.
fn cards(listed: &Value) -> Vec<CatalogModel> {
    let entries = listed.get("data").and_then(Value::as_array);
    let mut cards: Vec<CatalogModel> = Vec::new();
    for entry in entries.into_iter().flatten() {
        let Some(id) = entry.get("id").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let usable = !id.is_empty()
            && id.len() <= LONGEST_ID
            && !id.chars().any(|c| c.is_control() || c.is_whitespace());
        if !usable || cards.len() >= MOST || cards.iter().any(|card| card.id == id) {
            continue;
        }
        let mut card = CatalogModel::new(id);
        card.name = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= LONGEST_NAME)
            .map(str::to_owned);
        let inputs = entry.pointer("/architecture/input_modalities");
        card.images = inputs
            .and_then(Value::as_array)
            .map(|inputs| inputs.iter().any(|input| input == "image"));
        cards.push(card);
    }
    cards
}

/// The providers of an image model's endpoint list `listed`, each by its slug, with its price.
fn endpoints(_: &str, listed: &Value) -> Vec<CatalogEndpoint> {
    let entries = listed.get("endpoints").and_then(Value::as_array);
    let mut endpoints = Vec::new();
    for entry in entries.into_iter().flatten().take(MOST) {
        let slug = entry.get("provider_slug").and_then(Value::as_str);
        let Some(provider) = slug.and_then(plain) else {
            continue;
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
/// none unless every one has a price.
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
