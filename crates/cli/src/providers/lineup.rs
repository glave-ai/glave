//! Which models glave runs: the model the flags, the environment or `config.toml` choose, at
//! its endpoint, first and the default, as before; then the models of the accounts the person
//! connected, the first account's first model the default when nothing else chooses one.
//! Built again whenever the accounts change.

use std::collections::HashMap;
use std::sync::Arc;

use glave_config::policy::{Choices, Constraints, ModelEntry, Source, check_model};
use glave_event::{
    Catalog, CatalogProvider, Principal, ProviderConnected, ProviderId, ProviderKind,
};
use glave_gateway::{Gateway, Router, Target};
use glave_ports::settings::Judge;
use glave_session::Models;
use glave_session::protocol::ModelInfo;
use jiff::Timestamp;

use crate::config::Endpoint;
use crate::gateway::remote::Remote;
use crate::gateway::{Allowed, free_if_here, gateway, offered, routing_if_router};
use crate::model::Model;
use crate::setup::Setup;

use super::describe::{self, cards_for, rank, service_name};

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
    /// Whether the server keeps credits (`[credits]`).
    credits: bool,
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
            credits: choices.credits.is_some(),
        }
    }

    /// The account of the server's own that the endpoint its settings choose is, when that is
    /// OpenRouter itself: read and offered as a person's is, with its catalog kept beside
    /// theirs, so its models have prices for credits and providers for filters. No one
    /// connected it, so it is never kept and never listed as anyone's.
    pub(crate) fn own_account(&self) -> Option<ProviderConnected> {
        let endpoint = self
            .configured
            .as_ref()
            .filter(|endpoint| endpoint.gateway.is_none())?;
        let provider = &endpoint.provider;
        if provider.provider != "openrouter" || !super::at_openrouter(&provider.base_url) {
            return None;
        }
        Some(ProviderConnected {
            // Never kept or shown: it only has to be one.
            provider: ProviderId::new(Timestamp::UNIX_EPOCH, [0; 10]).ok()?,
            kind: ProviderKind::Openrouter,
            url: None,
            key: provider.auth.fixed_key()?.clone(),
            models: vec![endpoint.model.clone()],
            by: Principal::glave(),
        })
    }

    /// Whether the server's own account offers `model`, `main` being the model its settings
    /// choose: every model its catalog lists on a server whose people pay for what they use and
    /// whose settings name no models, otherwise only those it names, `main` and the judge's. So
    /// choosing one model in the settings never offers everyone the rest on the server's key.
    fn offers(&self, model: &str, main: &str) -> bool {
        (self.credits && self.entries.is_empty())
            || model == main
            || self.judge.model(main).as_deref() == Some(model)
            || self.entries.iter().any(|entry| entry.id.trim() == model)
    }

    /// The models of the configured endpoint, of the server's own account when it is one, and
    /// of `accounts`, as their services' `catalogs` describe them, those the policy allows: one
    /// entry per model, whichever accounts run it, in the order of their services' [`rank`].
    pub(crate) fn build(
        &self,
        accounts: &[ProviderConnected],
        catalogs: &[Catalog],
    ) -> anyhow::Result<Built> {
        let mut targets: Vec<(u8, Target)> = Vec::new();
        let mut about = HashMap::new();
        let mut first = None;
        let own = self.own_account();
        let main = self
            .configured
            .as_ref()
            .map(|endpoint| endpoint.model.as_str());
        let accounts = own
            .iter()
            .map(|account| (account, main))
            .chain(accounts.iter().map(|account| (account, None)));
        for (account, own_main) in accounts {
            let allowed: Vec<&str> = super::offered_by(account, catalogs)
                .into_iter()
                .filter(|model| own_main.is_none_or(|main| self.offers(model, main)))
                .filter(|model| {
                    super::endpoint_of(account, model).is_some_and(|endpoint| {
                        let name = format!("{}/{model}", endpoint.provider.provider);
                        check_model(&name, &Source::Default, &self.constraints).is_ok()
                    })
                })
                .collect();
            let Some(mut endpoint) = allowed
                .first()
                .and_then(|model| super::endpoint_of(account, model))
            else {
                continue;
            };
            if let Some(configured) = self.configured.as_ref().filter(|_| own_main.is_some()) {
                endpoint.provider.idle_timeout = configured.provider.idle_timeout;
            }
            let handle: Arc<dyn glave_gateway::Model> = Arc::new(Model::for_endpoint(&endpoint)?);
            let provider = &endpoint.provider.provider;
            let catalog = super::catalog_of(account, catalogs);
            let providers: Arc<[CatalogProvider]> =
                catalog.map_or_else(Arc::default, |catalog| catalog.providers.clone().into());
            for model in allowed {
                let target = target_at(&handle, provider, model, catalog, &providers);
                about.insert(target.name.clone(), (catalog, service_name(provider)));
                targets.push((rank(account.kind), free_if_here(target, &endpoint)));
            }
            first.get_or_insert(endpoint);
        }
        // Stable: each service's models keep their order.
        targets.sort_by_key(|(rank, _)| *rank);
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
            targets.into_iter().map(|(_, target)| target).collect(),
            &allowed,
        )?;
        let describe = |group: &[&Target]| self.described(group, &about, catalogs);
        let models = Models {
            default: Some((endpoint.model.clone(), endpoint.provider.provider.clone())),
            offered: offered(&gateway, describe),
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

impl Lineup {
    /// The entry for the model `group` serves, each of its targets described by its own
    /// service's card, `about` says which, or another's of the same model, and with who serves
    /// it through the first that is a router.
    fn described(
        &self,
        group: &[&Target],
        about: &HashMap<String, (Option<&Catalog>, String)>,
        catalogs: &[Catalog],
    ) -> ModelInfo {
        let (mut cards, mut providers, mut routed) = (Vec::new(), Vec::new(), None);
        for target in group {
            let (own, service) = about
                .get(&target.name)
                .cloned()
                .unwrap_or_else(|| (None, service_name(&target.provider)));
            cards.extend(cards_for(&target.model, own, catalogs));
            providers.push(service);
            let openrouter = own.filter(|own| own.kind == ProviderKind::Openrouter);
            routed = routed.or_else(|| Some((openrouter?, target.model.as_str())));
        }
        let first = group.first();
        let id = first.map_or("", |target| target.model.as_str());
        let entry = self.entries.iter().find(|entry| entry.id.trim() == id);
        let name = entry.and_then(|e| e.name.as_deref());
        describe::info(id, &cards, providers, name, first.copied(), routed)
    }
}

/// `model` at `provider` through `handle`, as its `catalog`'s card describes it, and a router
/// that keeps to people's filters where `provider` is OpenRouter, with who the catalog says
/// serves the model (`providers` shared by its models).
fn target_at(
    handle: &Arc<dyn glave_gateway::Model>,
    provider: &str,
    model: &str,
    catalog: Option<&Catalog>,
    providers: &Arc<[CatalogProvider]>,
) -> Target {
    let card = catalog.and_then(|catalog| {
        let card = catalog.models.iter().find(|card| card.id == model)?;
        Some((card, catalog))
    });
    let router = Router {
        providers: Arc::clone(providers),
        endpoints: card
            .map(|(card, _)| card.endpoints.clone())
            .unwrap_or_default(),
    };
    let target =
        Target::new(Arc::clone(handle), provider, model).at_service(service_name(provider));
    describe::described(routing_if_router(target, provider, router), card)
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::fake_secret;

    use super::*;

    /// A lineup whose settings choose `model` at `url`, named `provider`.
    fn lineup(url: &str, provider: &str, model: &str) -> Lineup {
        let mut config =
            glave_provider_openai::Config::for_url(url, Some(fake_secret("sk-or-"))).unwrap();
        provider.clone_into(&mut config.provider);
        Lineup {
            configured: Some(Endpoint {
                local: false,
                provider: config,
                claude: None,
                model: model.to_owned(),
                gateway: None,
                at: String::new(),
                key_from: "",
            }),
            judge: Judge::Main,
            judge_from: Source::Default,
            main_from: None,
            entries: Vec::new(),
            entries_from: Source::Default,
            constraints: Constraints::default(),
            credits: false,
        }
    }

    const OPENROUTER: &str = "https://openrouter.ai/api/v1";

    #[test]
    fn only_openrouter_itself_is_the_servers_own_account() {
        let own = lineup(OPENROUTER, "openrouter", "m").own_account().unwrap();
        assert!(own.key.expose().starts_with("sk-or-"));
        assert_eq!(own.models, ["m"]);
        assert!(
            lineup("https://keys.example.com/v1", "openrouter", "m")
                .own_account()
                .is_none(),
            "a key named for OpenRouter never goes elsewhere for a catalog"
        );
        assert!(lineup(OPENROUTER, "other", "m").own_account().is_none());
    }

    #[test]
    fn the_whole_catalog_is_offered_only_where_people_pay_and_no_models_are_named() {
        let mut plain = lineup(OPENROUTER, "openrouter", "m");
        assert!(plain.offers("m", "m") && !plain.offers("n", "m"));
        plain.credits = true;
        assert!(plain.offers("n", "m"));
        plain.entries = vec![ModelEntry {
            id: "n".to_owned(),
            name: None,
        }];
        assert!(plain.offers("n", "m") && plain.offers("m", "m") && !plain.offers("o", "m"));
    }
}
