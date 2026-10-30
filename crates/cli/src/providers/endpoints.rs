//! A router's endpoints (`specs/gateway.md`, "Catalogs"): OpenRouter serves most models from
//! several providers, each with its own precision, context, price, speed and data policy, and
//! says so in its public lists. When an OpenRouter account's catalog is listed, glave reads its
//! providers (`providers`: each one's slug, name and where it is based), the endpoints that keep
//! nothing (`endpoints/zdr`), and each model's endpoints (`models/{id}/endpoints`), a few at a
//! time within one limit, so a picker can say who serves a model and a person's filters can
//! choose among them. What OpenRouter does not say is left out, never guessed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use glave_event::{CatalogEndpoint, CatalogModel, CatalogProvider};
use glave_ports::ProviderError;
use serde_json::Value;
use tokio::task::JoinSet;

use super::card::{number, price};
use crate::model::Model;

/// How long reading OpenRouter's lists may take, all of them together.
const READING: Duration = Duration::from_mins(2);

/// Models whose endpoints are read at once.
const AT_ONCE: usize = 8;

/// The longest slug, name or country kept, in bytes.
const LONGEST: usize = 100;

/// The most providers kept, endpoints kept of one model, and datacenters of one provider.
const MOST: usize = 500;

/// Reads `model`'s router's providers and each of `models`' endpoints into them, within
/// [`READING`] in all: the providers, and whether every model's endpoints were read in time. A
/// model whose endpoints are not read keeps those `before` held. Refused, saying what, when the
/// providers or the endpoints that keep nothing cannot be read in time, as nothing of a model's
/// would be known.
pub(super) async fn read(
    model: Model,
    models: &mut [CatalogModel],
    before: &[CatalogModel],
) -> Result<(Vec<CatalogProvider>, bool), String> {
    let model = Arc::new(model);
    let ids: Vec<String> = models.iter().map(|card| card.id.clone()).collect();
    let mut known = None;
    let mut read = BTreeMap::new();
    let lists = async {
        let unread = |what: &'static str| move |e: ProviderError| format!("its {what}: {e}");
        let listed = model.get("providers").await.map_err(unread("providers"))?;
        let providers = providers(&listed);
        let zdr = model.get("endpoints/zdr").await;
        let zdr = keeping_nothing(&zdr.map_err(unread("endpoints that keep nothing"))?);
        let named: BTreeMap<String, String> = providers
            .iter()
            .map(|p| (p.name.clone(), p.slug.clone()))
            .collect();
        known = Some(providers);
        each(&model, &ids, Arc::new((named, zdr)), &mut read).await;
        Ok::<_, String>(())
    };
    let whole = if let Ok(read) = tokio::time::timeout(READING, lists).await {
        read.map(|()| true)?
    } else {
        let waited = glave_ports::spoken(READING);
        tracing::warn!("reading OpenRouter's endpoints took over {waited}: some wait");
        false
    };
    let providers = known.ok_or_else(|| {
        format!(
            "its providers were not read within {}",
            glave_ports::spoken(READING)
        )
    })?;
    for card in models.iter_mut() {
        card.endpoints = read
            .remove(&card.id)
            .unwrap_or_else(|| kept(before, &card.id));
    }
    Ok((providers, whole))
}

/// The endpoints `before` held for the model `id`.
fn kept(before: &[CatalogModel], id: &str) -> Vec<CatalogEndpoint> {
    let card = before.iter().find(|card| card.id == id);
    card.map(|card| card.endpoints.clone()).unwrap_or_default()
}

/// What reading a model's endpoints needs to know first: the providers' slugs by name, and the
/// endpoints that keep nothing, by model and tag.
type Known = (BTreeMap<String, String>, BTreeSet<(String, String)>);

/// Reads each of `ids`' endpoints into `read`, as each comes, [`AT_ONCE`] at a time, with
/// what is `known`; one that cannot be read is left out.
async fn each(
    model: &Arc<Model>,
    ids: &[String],
    known: Arc<Known>,
    read: &mut BTreeMap<String, Vec<CatalogEndpoint>>,
) {
    let mut asked = JoinSet::new();
    let mut waiting = ids.iter();
    loop {
        while asked.len() < AT_ONCE {
            let Some(id) = waiting.next() else { break };
            let (model, id, known) = (Arc::clone(model), id.clone(), Arc::clone(&known));
            asked.spawn(async move {
                let listed = model.get(&format!("models/{id}/endpoints")).await.ok()?;
                let (named, zdr) = &*known;
                let endpoints = endpoints(&id, &listed, named, zdr);
                Some((id, endpoints))
            });
        }
        let Some(done) = asked.join_next().await else {
            return;
        };
        if let Ok(Some((id, endpoints))) = done {
            read.insert(id, endpoints);
        }
    }
}

/// `text` when it is a short, plain word or name.
fn plain(text: &str) -> Option<String> {
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
fn providers(listed: &Value) -> Vec<CatalogProvider> {
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
fn keeping_nothing(listed: &Value) -> BTreeSet<(String, String)> {
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
fn endpoints(
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

#[cfg(test)]
mod tests;
