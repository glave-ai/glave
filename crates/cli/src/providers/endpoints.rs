//! A router's endpoints (`specs/gateway.md`, "Catalogs"): OpenRouter serves most models from
//! several providers, each with its own precision, context, price, speed and data policy, and
//! says so in its public lists. When an OpenRouter account's catalog is listed, glave reads its
//! providers (`providers`: each one's slug, name and where it is based), the endpoints that keep
//! nothing (`endpoints/zdr`), and each model's endpoints (`models/{id}/endpoints`), a few at a
//! time within one limit, so a picker can say who serves a model and a person's filters can
//! choose among them. What OpenRouter does not say is left out, never guessed.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use glave_event::{CatalogEndpoint, CatalogModel, CatalogProvider};
use glave_ports::ProviderError;
use glave_provider_openai::lists::{endpoints, keeping_nothing, providers};
use serde_json::Value;
use tokio::task::JoinSet;

use crate::model::Model;

/// How long reading OpenRouter's lists may take, all of them together.
pub(super) const READING: Duration = Duration::from_mins(2);

/// Models whose endpoints are read at once.
const AT_ONCE: usize = 8;

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
        let parse = move |id: &str, listed: &Value| endpoints(id, listed, &named, &zdr);
        let path = |id: &str| format!("models/{id}/endpoints");
        // A model not found keeps what it had: chat models come and go between the two reads.
        each(&model, &ids, (path, Arc::new(parse)), &mut read).await;
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
pub(super) fn kept(before: &[CatalogModel], id: &str) -> Vec<CatalogEndpoint> {
    let card = before.iter().find(|card| card.id == id);
    card.map(|card| card.endpoints.clone()).unwrap_or_default()
}

/// Where a model's endpoints are listed, by its id, and how its list is read.
pub(super) type Listing<F> = (fn(&str) -> String, Arc<F>);

/// Reads each of `ids`' endpoints into `read`, as each comes, [`AT_ONCE`] at a time: asked at
/// the path `listing` gives, read by its parser; one that cannot be read is left out. Whether
/// every one was read.
pub(super) async fn each<F>(
    model: &Arc<Model>,
    ids: &[String],
    (path, parse): Listing<F>,
    read: &mut BTreeMap<String, Vec<CatalogEndpoint>>,
) -> bool
where
    F: Fn(&str, &Value) -> Vec<CatalogEndpoint> + Send + Sync + 'static,
{
    let mut missed = false;
    let mut asked = JoinSet::new();
    let mut waiting = ids.iter();
    loop {
        while asked.len() < AT_ONCE {
            let Some(id) = waiting.next() else { break };
            let (model, id, parse) = (Arc::clone(model), id.clone(), Arc::clone(&parse));
            asked.spawn(async move {
                let listed = model.get(&path(&id)).await.ok()?;
                let endpoints = parse(&id, &listed);
                Some((id, endpoints))
            });
        }
        let Some(done) = asked.join_next().await else {
            return !missed;
        };
        match done {
            Ok(Some((id, endpoints))) => {
                read.insert(id, endpoints);
            }
            Ok(None) | Err(_) => missed = true,
        }
    }
}

#[cfg(test)]
mod tests;
