//! Which models glave runs: the model the flags, the environment or `config.toml` choose, at
//! its endpoint, first and the default, as before; then the models of the accounts the person
//! connected, the first account's first model the default when nothing else chooses one.
//! Built again whenever the accounts change.

use std::sync::Arc;

use glave_config::policy::{Choices, Constraints, ModelEntry, Source, check_model};
use glave_event::{Catalog, ProviderConnected};
use glave_gateway::{Gateway, Target};
use glave_ports::settings::Judge;
use glave_session::Models;

use crate::config::Endpoint;
use crate::gateway::remote::Remote;
use crate::gateway::{Allowed, gateway, offered};
use crate::model::Model;
use crate::setup::Setup;

/// What the models are built from, kept to build them again.
#[derive(Clone)]
pub(crate) struct Lineup {
    /// The endpoint the settings choose, when they choose a model.
    pub configured: Option<Endpoint>,
    judge: Judge,
    judge_from: Source,
    main_from: Option<Source>,
    entries: Vec<ModelEntry>,
    entries_from: Source,
    constraints: Constraints,
}

/// The models of one lineup: the gateway to them, what `model/list` offers, and the endpoint
/// of the default; none of it while no model is set up.
#[derive(Default)]
pub(crate) struct Built {
    pub gateway: Option<Gateway>,
    pub models: Models,
    pub endpoint: Option<Endpoint>,
}

impl Lineup {
    /// The lineup of `setup`'s choices, with `configured` the endpoint they choose, if any.
    pub(crate) fn new(setup: &Setup, configured: Option<Endpoint>) -> Self {
        let choices = &setup.choices;
        Self {
            configured,
            judge: choices.judge.value.clone(),
            judge_from: choices.judge.from.clone(),
            main_from: choices.model.as_ref().map(|model| model.from.clone()),
            entries: choices
                .models
                .as_ref()
                .map_or_else(Vec::new, |models| models.value.clone()),
            entries_from: choices
                .models
                .as_ref()
                .map_or(Source::Default, |models| models.from.clone()),
            constraints: setup.loaded.constraints.clone(),
        }
    }

    /// The models of the configured endpoint and of `accounts`, as their services' `catalogs`
    /// list them, those the policy allows.
    pub(crate) fn build(
        &self,
        accounts: &[ProviderConnected],
        catalogs: &[Catalog],
    ) -> anyhow::Result<Built> {
        let mut targets = Vec::new();
        let mut first = None;
        for account in accounts {
            let allowed: Vec<&str> = super::offered_by(account, catalogs)
                .into_iter()
                .filter(|model| {
                    super::endpoint_of(account, model).is_some_and(|endpoint| {
                        let name = format!("{}/{model}", endpoint.provider.provider);
                        check_model(&name, &Source::Default, &self.constraints).is_ok()
                    })
                })
                .collect();
            let Some(endpoint) = allowed
                .first()
                .and_then(|model| super::endpoint_of(account, model))
            else {
                continue;
            };
            let handle: Arc<dyn glave_gateway::Model> = Arc::new(Model::for_endpoint(&endpoint)?);
            let provider = &endpoint.provider.provider;
            targets.extend(
                allowed
                    .into_iter()
                    .map(|model| Target::new(Arc::clone(&handle), provider.clone(), model)),
            );
            first.get_or_insert(endpoint);
        }
        let Some(endpoint) = self.configured.clone().or(first) else {
            return Ok(Built::default());
        };
        let allowed = Allowed {
            constraints: &self.constraints,
            main: self.main_from.as_ref(),
            judge: &self.judge_from,
            models: &self.entries_from,
        };
        let judge = self.judge.model(&endpoint.model);
        let gateway = gateway(
            Model::for_endpoint(&endpoint)?,
            &endpoint,
            judge.as_deref(),
            &self.entries,
            targets,
            &allowed,
        )?;
        let models = Models {
            default: Some((endpoint.model.clone(), endpoint.provider.provider.clone())),
            offered: offered(&gateway, &self.entries),
        };
        Ok(Built {
            gateway: Some(gateway),
            models,
            endpoint: Some(endpoint),
        })
    }
}

/// The gateway elsewhere, when `[gateway] url` names one, and the endpoint model calls go to:
/// that gateway, or the model chosen, if any is.
pub(crate) fn endpoint(choices: &Choices) -> anyhow::Result<(Option<Remote>, Option<Endpoint>)> {
    let remote = choices
        .gateway
        .as_ref()
        .map(Remote::from_choice)
        .transpose()?;
    let model = choices.model.as_ref().map(|m| m.value.clone());
    let endpoint = match (&remote, model) {
        (Some(remote), model) => Some(remote.endpoint(model)?),
        (None, Some(model)) => Some(crate::config::endpoint(
            Some(model),
            choices.provider.as_ref(),
        )?),
        (None, None) => None,
    };
    let idle = crate::config::idle_timeout()?;
    Ok((
        remote,
        endpoint.map(|mut endpoint| {
            if let Some(idle) = idle {
                endpoint.provider.idle_timeout = idle;
            }
            endpoint
        }),
    ))
}
