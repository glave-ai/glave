//! Every attempt's calls go through glave's gateway, as a server's do, held to Glave Cloud's
//! floor among a router's providers unless `--no-floor` says otherwise
//! (`specs/gateway.md`, "A server's floor"), so a model is measured as people on Glave Cloud
//! get it. Who serves each model is read from OpenRouter's lists once, at the start, as glave's
//! catalog reads them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use glave_event::{CatalogEndpoint, CatalogProvider, Filters, Precision};
use glave_gateway::{Gateway, Router, Routes, Target};
use glave_provider_openai::{Config, OpenAiCompatible, lists};

use crate::error::EvalError;

/// How long reading OpenRouter's lists may take, all of them together.
const READING: Duration = Duration::from_mins(2);

/// Glave Cloud's floor, in words, as a report says it.
pub(crate) const SAID: &str = "based in the US, keeping nothing, weights at fp8 or better";

/// Glave Cloud's floor: providers based in the US that keep nothing of what they are sent,
/// serving weights at 8 bits or better (a closed model, whose providers say none, is kept).
fn cloud() -> Filters {
    Filters {
        based_in: vec!["US".to_owned()],
        keeps_nothing: true,
        precision: Some(Precision::EightBit),
        ..Filters::default()
    }
}

/// The floor an eval's calls keep to, and who its router says serves each model.
#[derive(Debug, Clone)]
pub struct Floor {
    pub(crate) filters: Filters,
    /// What the router's lists say serves each model, by the model's id.
    pub(crate) routers: BTreeMap<String, Router>,
}

impl Floor {
    /// Glave Cloud's floor for `models` at `endpoint`, with who serves each as OpenRouter's
    /// lists say: refused at once where the endpoint isn't OpenRouter, since a floor sends
    /// nothing to a service it can't check, or where its lists can't be read in time.
    pub async fn cloud(endpoint: &Config, models: &[String]) -> Result<Self, String> {
        if endpoint.provider != "openrouter" {
            return Err(format!(
                "the floor keeps to providers OpenRouter vouches for, and the endpoint is {}: \
                 point GLAVE_BASE_URL at OpenRouter, or run with --no-floor",
                endpoint.provider
            ));
        }
        let router = OpenAiCompatible::new(endpoint.clone()).map_err(|err| err.to_string())?;
        let read = tokio::time::timeout(READING, listed(&router, models)).await;
        let routers = read.map_err(|_| {
            format!(
                "OpenRouter's lists were not read within {}",
                glave_ports::spoken(READING)
            )
        })??;
        Ok(Self {
            filters: cloud(),
            routers,
        })
    }
}

/// Who `router`'s lists say serves each of `models`: its providers, shared, and each model's
/// endpoints.
async fn listed(
    router: &OpenAiCompatible,
    models: &[String],
) -> Result<BTreeMap<String, Router>, String> {
    let unread = |what: &str| {
        let what = what.to_owned();
        move |err| format!("OpenRouter's {what}: {err}")
    };
    let providers = router.get("providers").await.map_err(unread("providers"))?;
    let providers: Arc<[CatalogProvider]> = lists::providers(&providers).into();
    let zdr = router.get("endpoints/zdr").await;
    let zdr = lists::keeping_nothing(&zdr.map_err(unread("endpoints that keep nothing"))?);
    let named: BTreeMap<String, String> = (providers.iter())
        .map(|provider| (provider.name.clone(), provider.slug.clone()))
        .collect();
    let mut routers = BTreeMap::new();
    for model in models {
        let path = format!("models/{model}/endpoints");
        let listed = router.get(&path).await.map_err(unread(&path))?;
        let endpoints: Vec<CatalogEndpoint> = lists::endpoints(model, &listed, &named, &zdr);
        let providers = Arc::clone(&providers);
        routers.insert(
            model.clone(),
            Router {
                providers,
                endpoints,
            },
        );
    }
    Ok(routers)
}

/// The gateway an attempt's calls of `model` at `endpoint` go through: held to `floor` when
/// there is one, as a router that keeps to it.
pub(crate) fn gateway(
    endpoint: &Config,
    model: &str,
    floor: Option<&Floor>,
) -> Result<Gateway, EvalError> {
    let provider =
        OpenAiCompatible::new(endpoint.clone()).map_err(|err| EvalError::Setup(err.to_string()))?;
    let handle: Arc<dyn glave_gateway::Model> = Arc::new(provider);
    let target = Target::new(handle, endpoint.provider.clone(), model);
    let target = match floor.and_then(|floor| floor.routers.get(model)) {
        Some(router) => {
            let mut target = target.routing(router.clone());
            // What the lists say of the model, so a turn asks for no longer an answer than
            // its providers give nor than its window holds: the most any of them serves.
            let endpoints = &router.endpoints;
            target.can.context_tokens = endpoints.iter().filter_map(|e| e.context).max();
            target.can.max_output_tokens = (endpoints.iter().filter_map(|e| e.max_output))
                .max()
                .map(|tokens| u32::try_from(tokens).unwrap_or(u32::MAX));
            target
        }
        None => target,
    };
    let routes = Routes::new(vec![target]).map_err(|err| EvalError::Setup(err.to_string()))?;
    Ok(Gateway::new(routes).with_floor(floor.map(|floor| floor.filters.clone())))
}
