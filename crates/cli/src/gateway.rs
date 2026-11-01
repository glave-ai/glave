//! The gateway every model call goes through, built from today's settings: `GLAVE_MODEL` is
//! the `main` route, and `GLAVE_JUDGE_MODEL`, when it names another model at the same
//! endpoint, a choice the judge's calls name. Every purpose uses `main` unless a call names a
//! choice, and there are no budgets, so nothing changes for anyone who sets nothing new. A policy's `[models]` rules are kept
//! here: a target they do not allow is refused before the gateway exists. With `[gateway]
//! url`, the endpoint is the organization's gateway, which keeps them itself, for each caller.
//! The judge model and each of `[[models]]` are choices a thread may make instead of the main
//! model (`model/list`), checked against the policy as the main model is, and so are the
//! models of the accounts the person connected (`providers`), leaving out those it refuses.
//! The gateway in use is replaced whole when those accounts change ([`GatewaySwitch`]).

pub(crate) mod remote;
pub(crate) mod serve;

use std::sync::{Arc, PoisonError, RwLock};

use glave_event::{Currency, Filters};
use glave_gateway::{Credits, Filtering, Gateway, Price, Router, Routes, Target};
use glave_ports::{
    Completion, Delta, Failure, ModelProvider, ModelRequest, ProviderError, Purpose,
};

use crate::config::Endpoint;
use crate::data::DataLog;
use crate::model::{Model, documented_reasoning};
use crate::providers::{identity, service_name};
use glave_config::policy::{Constraints, ModelEntry, RouterFloorFile, Source, check_model};
use glave_session::PersonFilters;
use glave_session::protocol::ModelInfo;

/// What a model on this computer costs, called straight rather than through a gateway
/// elsewhere: nothing, so its calls report $0.00 as any other's report their cost.
fn free_here() -> Price {
    Price {
        list: "this computer".to_owned(),
        currency: Currency::USD,
        input_per_mtok: 0,
        cache_read_per_mtok: 0,
        output_per_mtok: 0,
    }
}

/// `target` at `endpoint`, priced at nothing when it runs on this computer and nothing else
/// prices it.
pub(crate) fn free_if_here(target: Target, endpoint: &Endpoint) -> Target {
    if endpoint.local && endpoint.gateway.is_none() && target.price.is_none() {
        target.with_price(free_here())
    } else {
        target
    }
}

/// `target`, a router that keeps to a person's filters among the providers its catalog says
/// serve its model, `router`, when `provider` is OpenRouter.
pub(crate) fn routing_if_router(target: Target, provider: &str, router: Router) -> Target {
    if provider == "openrouter" {
        target.routing(router)
    } else {
        target
    }
}

/// Which models a policy allows, and where the main and judge models were chosen.
pub(crate) struct Allowed<'a> {
    pub constraints: &'a Constraints,
    /// Where the main model was chosen, when anyone chose it.
    pub main: Option<&'a Source>,
    pub judge: &'a Source,
    /// Where `[[models]]` was chosen.
    pub models: &'a Source,
}

/// The gateway over `provider` at `endpoint`, with the judge's model, when `judge` names one
/// other than the main one, and each of `offered` as choices. Fails when `allowed` refuses any
/// of them. Then each of `accounts` the policy allows: after the main model, when it serves
/// the same one; in the place of the main model or a choice that is the very same; else a
/// choice, unless a model of the same id already is.
pub(crate) fn gateway(
    provider: Model,
    endpoint: &Endpoint,
    judge: Option<&str>,
    offered: &[ModelEntry],
    accounts: Vec<Target>,
    allowed: &Allowed<'_>,
) -> anyhow::Result<Gateway> {
    let handle: Arc<dyn glave_gateway::Model> = Arc::new(provider);
    let name = &endpoint.provider.provider;
    let service = service_name(name);
    // No catalog describes these: Claude among them reasons as Anthropic documents.
    let at = |model: &str| {
        let target = Target::new(Arc::clone(&handle), name.clone(), model).at_service(&service);
        // No catalog says who serves a configured model: the router finds out each time.
        let target = routing_if_router(target, name, Router::default());
        let target = free_if_here(target, endpoint);
        match documented_reasoning(endpoint, model) {
            Some(reasoning) => target.with_reasoning(&reasoning),
            None => target,
        }
    };
    let mut main = at(&endpoint.model);
    let chosen = allowed.main.unwrap_or(&Source::Default);
    // A gateway elsewhere keeps the policy's models itself, for each caller.
    let checks = endpoint.gateway.is_none();
    if checks {
        check_model(&main.name, chosen, allowed.constraints)?;
    }
    let same = identity(&endpoint.model);
    let mut fallbacks = Vec::new();
    let mut choices: Vec<Target> = Vec::new();
    // The judge's calls name its model, so a project's judge is found among the choices too.
    if let Some(model) = judge.filter(|model| *model != endpoint.model) {
        let target = at(model);
        if checks {
            check_model(&target.name, allowed.judge, allowed.constraints)?;
        }
        choices.push(target);
    }
    for entry in offered {
        let id = entry.id.trim();
        if id.is_empty() || id == endpoint.model || choices.iter().any(|c| c.model == id) {
            continue;
        }
        let target = at(id);
        if checks {
            check_model(&target.name, allowed.models, allowed.constraints)?;
        }
        choices.push(target);
    }
    for target in accounts {
        if check_model(&target.name, &Source::Default, allowed.constraints).is_err() {
            continue;
        }
        if target.name == main.name {
            main = target;
        } else if target.same_model.as_ref() == Some(&same) || target.model == endpoint.model {
            fallbacks.push(target);
        } else if let Some(choice) = choices.iter_mut().find(|c| c.name == target.name) {
            // Its own service's description, levels too, wins over the settings' bare name.
            *choice = target;
        } else if !choices.iter().any(|c| c.model == target.model) {
            choices.push(target);
        }
    }
    let routes = Routes::new([main].into_iter().chain(fallbacks).collect())?;
    Ok(Gateway::new(routes.with_choices(choices)))
}

/// What the server's own settings allow among a router's providers for everyone it serves, as
/// the filters it holds every call to; none when `[router_floor]` asks nothing.
pub(crate) fn router_floor(floor: Option<&RouterFloorFile>) -> Option<Filters> {
    let floor = floor?;
    (!floor.based_in.is_empty() || floor.keeps_nothing).then(|| Filters {
        based_in: floor.based_in.clone(),
        keeps_nothing: floor.keeps_nothing,
        ..Filters::default()
    })
}

/// The gateway in use, or none while no model is set up: replaced whole when the person
/// connects or removes a model account, so each call goes to the models set up when it
/// starts. Every gateway it is given charges the organizations' credits, where the server
/// keeps them. Clones share it.
#[derive(Clone, Default)]
pub(crate) struct GatewaySwitch {
    current: Arc<RwLock<Option<Arc<Gateway>>>>,
    credits: Option<Arc<Credits<DataLog>>>,
    /// What the server allows among a router's providers for everyone it serves.
    floor: Option<Filters>,
    /// Each person's filters among a router's providers, which the session server answers
    /// once it is made.
    filters: Arc<PersonFilters>,
}

impl GatewaySwitch {
    pub(crate) fn new(
        gateway: Option<Gateway>,
        credits: Option<Arc<Credits<DataLog>>>,
        floor: Option<Filters>,
    ) -> Self {
        let switch = Self {
            current: Arc::default(),
            credits,
            floor,
            filters: Arc::default(),
        };
        switch.set(gateway);
        switch
    }

    /// Sends calls to `gateway` from now on; calls already sent finish where they are.
    pub(crate) fn set(&self, gateway: Option<Gateway>) {
        let gateway = gateway.map(|gateway| {
            let gateway = gateway
                .with_filters(Arc::clone(&self.filters) as Arc<dyn Filtering>)
                .with_floor(self.floor.clone());
            match &self.credits {
                Some(credits) => gateway.with_credits(Arc::clone(credits)),
                None => gateway,
            }
        });
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = gateway.map(Arc::new);
    }

    /// What its gateways ask for each person's filters, for the session server to answer.
    pub(crate) fn filters(&self) -> Arc<PersonFilters> {
        Arc::clone(&self.filters)
    }

    fn current(&self) -> Option<Arc<Gateway>> {
        self.current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether a thread may be sent to `model` now: one `model/list` offers.
    pub(crate) fn offers(&self, model: &str) -> bool {
        self.current().is_some_and(|gateway| {
            models(&gateway).any(|group| group.first().is_some_and(|t| t.model == model))
        })
    }
}

/// The models `gateway` offers threads: the main one, then its choices, each model once with
/// every target that serves it, in the order they are tried.
fn models(gateway: &Gateway) -> impl Iterator<Item = Vec<&Target>> {
    let routes = gateway.routes();
    let choices = routes.choices();
    let main = Some(routes.targets(Purpose::Main).iter().collect::<Vec<_>>());
    let grouped = choices.iter().enumerate().filter_map(move |(at, target)| {
        let Some(same) = &target.same_model else {
            return Some(vec![target]);
        };
        let earlier = choices.iter().take(at);
        if earlier.clone().any(|t| t.same_model.as_ref() == Some(same)) {
            return None;
        }
        Some(
            choices
                .iter()
                .skip(at)
                .filter(|t| t.same_model.as_ref() == Some(same))
                .collect(),
        )
    });
    main.into_iter().chain(grouped)
}

impl ModelProvider for GatewaySwitch {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        match self.current() {
            Some(gateway) => gateway.complete(request, on_delta).await,
            None => Err(ProviderError::Unserved {
                failure: Failure::UnknownModel,
                reason: "no model is set up yet: connect a model account first".to_owned(),
            }),
        }
    }
}

/// What `model/list` offers: the main model's entry, then one for each of the gateway's
/// choices, as `describe` says of the targets that serve it.
pub(crate) fn offered(
    gateway: &Gateway,
    describe: impl Fn(&[&Target]) -> ModelInfo,
) -> Vec<ModelInfo> {
    models(gateway).map(|group| describe(&group)).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The rules of one policy file at the organization's place.
    pub(crate) fn constraints(text: &str) -> Constraints {
        glave_config::policy::constraints(
            text,
            std::path::Path::new(glave_config::policy::ORGANIZATION_POLICY),
        )
        .unwrap()
    }
    use glave_provider_openai::Config;

    use super::offered as offered_models;

    pub(crate) fn endpoint() -> Endpoint {
        let provider = Config::for_url("https://openrouter.ai/api/v1", None).unwrap();
        Endpoint {
            local: false,
            provider,
            claude: None,
            model: "big".into(),
            gateway: None,
            at: "GLAVE_BASE_URL".into(),
            key_from: "GLAVE_API_KEY",
        }
    }

    fn built(judge: Option<&str>, constraints: &Constraints) -> anyhow::Result<Gateway> {
        offering(judge, &[], constraints)
    }

    fn offering(
        judge: Option<&str>,
        offered: &[ModelEntry],
        constraints: &Constraints,
    ) -> anyhow::Result<Gateway> {
        let endpoint = endpoint();
        let provider = Model::for_endpoint(&endpoint)?;
        let file = Source::File("/home/ada/.glave/config.toml".into());
        let allowed = Allowed {
            constraints,
            main: Some(&Source::Flag("--model")),
            judge: &Source::Env("GLAVE_JUDGE_MODEL"),
            models: &file,
        };
        gateway(provider, &endpoint, judge, offered, Vec::new(), &allowed)
    }

    fn entry(id: &str, name: Option<&str>) -> ModelEntry {
        ModelEntry {
            id: id.into(),
            name: name.map(str::to_owned),
        }
    }

    #[test]
    fn the_judge_and_each_model_offered_are_choices_once_and_the_main_one_is_not() {
        let offered = [
            entry("hope-12b", Some("Hope")),
            entry("big", Some("Big")),
            entry("small", None),
            entry("hope-12b", None),
        ];
        let gateway = offering(Some("small"), &offered, &Constraints::default()).unwrap();
        let choices: Vec<&str> = gateway
            .routes()
            .choices()
            .iter()
            .map(|target| target.name.as_str())
            .collect();
        assert_eq!(choices, ["openrouter/small", "openrouter/hope-12b"]);
        let listed = offered_models(&gateway, |group| ModelInfo::new(&group[0].model));
        let ids: Vec<&str> = listed.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["big", "small", "hope-12b"]);
    }

    #[test]
    fn claude_the_settings_choose_takes_the_documented_levels_unless_its_account_lists_some() {
        let mut claude = endpoint();
        claude.provider = Config::for_url("https://api.anthropic.com", None).unwrap();
        "anthropic".clone_into(&mut claude.provider.provider);
        claude.model = "claude-opus-5-5".into();
        let constraints = Constraints::default();
        let allowed = Allowed {
            constraints: &constraints,
            main: None,
            judge: &Source::Default,
            models: &Source::Default,
        };
        let listed = glave_event::Reasoning {
            efforts: vec!["high".into()],
            default: None,
            mandatory: false,
        };
        let scripted = Arc::new(glave_ports::testing::ScriptedModel::new(Vec::new()));
        let account =
            Target::new(scripted, "anthropic", "claude-haiku-4-5").with_reasoning(&listed);
        let offered = [entry("claude-haiku-4-5", None)];
        let provider = Model::for_endpoint(&claude).unwrap();
        let gateway = gateway(provider, &claude, None, &offered, vec![account], &allowed).unwrap();
        let main = &gateway.routes().targets(Purpose::Main)[0];
        let documented = main.reasoning().map(|reasoning| reasoning.efforts.len());
        assert_eq!(
            documented,
            Some(5),
            "no listing: Anthropic's documented levels"
        );
        let haiku = &gateway.routes().choices()[0];
        assert_eq!(haiku.reasoning(), Some(&listed), "its account's listing");
        let other = built(None, &constraints).unwrap();
        assert_eq!(other.routes().targets(Purpose::Main)[0].reasoning(), None);
    }

    #[test]
    fn a_model_on_this_computer_costs_nothing_and_one_elsewhere_its_price() {
        let mut here = endpoint();
        here.provider = Config::for_url("http://localhost:11434/v1", None).unwrap();
        here.local = here.provider.is_local();
        let constraints = Constraints::default();
        let allowed = Allowed {
            constraints: &constraints,
            main: None,
            judge: &Source::Default,
            models: &Source::Default,
        };
        let provider = Model::for_endpoint(&here).unwrap();
        let gateway = gateway(provider, &here, None, &[], Vec::new(), &allowed).unwrap();
        let price = gateway.routes().targets(Purpose::Main)[0].price.as_ref();
        assert_eq!(price, Some(&free_here()), "its calls report $0.00");
        let elsewhere = built(None, &constraints).unwrap();
        assert_eq!(elsewhere.routes().targets(Purpose::Main)[0].price, None);
    }

    #[test]
    fn a_model_offered_that_the_policy_does_not_allow_is_refused_naming_the_rule() {
        let policy = constraints("version = 1\n[models]\ndeny = [\"*/secret\"]\n");
        let refused = offering(None, &[entry("secret", None)], &policy)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("openrouter/secret (from /home/ada/.glave/config.toml)"),
            "{refused}"
        );
    }

    /// Where a judge's call naming `named` goes, on the gateway whose judge is `judge`.
    fn judge_target(judge: Option<&str>, named: &str) -> String {
        let gateway = built(judge, &Constraints::default()).unwrap();
        let targets = gateway.routes().for_model(Purpose::Judge, named);
        assert_eq!(targets.len(), 1);
        targets[0].name.clone()
    }

    #[test]
    fn the_judge_s_calls_go_to_the_model_they_name_whoever_chose_it() {
        assert_eq!(judge_target(None, "big"), "openrouter/big");
        assert_eq!(judge_target(Some("big"), "big"), "openrouter/big");
        assert_eq!(judge_target(Some("small"), "small"), "openrouter/small");
        assert_eq!(
            judge_target(Some("small"), "big"),
            "openrouter/big",
            "a project whose judge is the main model"
        );
        let gateway = built(Some("small"), &Constraints::default()).unwrap();
        assert_eq!(
            gateway.routes().targets(Purpose::Main)[0].name,
            "openrouter/big"
        );
    }

    #[test]
    fn a_model_the_policy_does_not_allow_is_refused_naming_the_rule() {
        let policy = constraints;
        let only_small = policy("version = 1\n[models]\nallow = [\"openrouter/small\"]\n");
        let refused = built(None, &only_small).unwrap_err().to_string();
        assert_eq!(
            refused,
            "model openrouter/big (from --model) is not allowed: [models] allow = \
             [\"openrouter/small\"] in /etc/glave/policy.toml"
        );
        let both =
            policy("version = 1\n[models]\nallow = [\"openrouter/*\"]\ndeny = [\"*/small\"]\n");
        assert!(built(Some("big"), &both).is_ok());
        let judge = built(Some("small"), &both).unwrap_err().to_string();
        assert!(
            judge.contains("openrouter/small (from GLAVE_JUDGE_MODEL)"),
            "{judge}"
        );
        assert!(judge.contains("[models] deny"), "{judge}");
    }
}
