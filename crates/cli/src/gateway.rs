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

use glave_gateway::{Gateway, Routes, Target};
use glave_ports::{
    Completion, Delta, Failure, ModelProvider, ModelRequest, ProviderError, Purpose,
};

use crate::config::Endpoint;
use crate::model::Model;
use glave_config::policy::{Constraints, ModelEntry, Source, check_model};
use glave_session::protocol::ModelInfo;

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
/// other than the main one, and each of `offered` as choices. Fails
/// when `allowed` refuses any of them. Then each of `accounts` the policy allows is a choice
/// too, unless a model of the same id already is.
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
    let main = Target::new(Arc::clone(&handle), name.clone(), endpoint.model.clone());
    let chosen = allowed.main.unwrap_or(&Source::Default);
    // A gateway elsewhere keeps the policy's models itself, for each caller.
    let checks = endpoint.gateway.is_none();
    if checks {
        check_model(&main.name, chosen, allowed.constraints)?;
    }
    let routes = Routes::new(vec![main])?;
    let mut choices: Vec<Target> = Vec::new();
    // The judge's calls name its model, so a project's judge is found among the choices too.
    if let Some(model) = judge.filter(|model| *model != endpoint.model) {
        let target = Target::new(Arc::clone(&handle), name.clone(), model);
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
        let target = Target::new(Arc::clone(&handle), name.clone(), id);
        if checks {
            check_model(&target.name, allowed.models, allowed.constraints)?;
        }
        choices.push(target);
    }
    for target in accounts {
        let taken =
            target.model == endpoint.model || choices.iter().any(|c| c.model == target.model);
        if !taken && check_model(&target.name, &Source::Default, allowed.constraints).is_ok() {
            choices.push(target);
        }
    }
    Ok(Gateway::new(routes.with_choices(choices)))
}

/// The gateway in use, or none while no model is set up: replaced whole when the person
/// connects or removes a model account, so each call goes to the models set up when it
/// starts. Clones share it.
#[derive(Clone, Default)]
pub(crate) struct GatewaySwitch(Arc<RwLock<Option<Arc<Gateway>>>>);

impl GatewaySwitch {
    pub(crate) fn new(gateway: Option<Gateway>) -> Self {
        Self(Arc::new(RwLock::new(gateway.map(Arc::new))))
    }

    /// Sends calls to `gateway` from now on; calls already sent finish where they are.
    pub(crate) fn set(&self, gateway: Option<Gateway>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = gateway.map(Arc::new);
    }

    fn current(&self) -> Option<Arc<Gateway>> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether a thread may be sent to `model` now: one `model/list` offers.
    pub(crate) fn offers(&self, model: &str) -> bool {
        self.current()
            .is_some_and(|gateway| models(&gateway).any(|target| target.model == model))
    }
}

/// The models `gateway` offers threads: the main one, then its choices.
fn models(gateway: &Gateway) -> impl Iterator<Item = &Target> {
    let routes = gateway.routes();
    routes
        .targets(Purpose::Main)
        .first()
        .into_iter()
        .chain(routes.choices())
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

/// What `model/list` offers besides the main model, which the server adds itself: the
/// gateway's choices, each named and marked as `[[models]]` says, and the main model's own
/// entry, for its name and mark.
pub(crate) fn offered(gateway: &Gateway, entries: &[ModelEntry]) -> Vec<ModelInfo> {
    let info = |id: &str| {
        let entry = entries.iter().find(|entry| entry.id.trim() == id);
        ModelInfo {
            id: id.to_owned(),
            name: entry.and_then(|entry| entry.name.clone()),
            default: false,
            hope: entry.is_some_and(|entry| entry.hope),
        }
    };
    models(gateway).map(|target| info(&target.model)).collect()
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

    fn entry(id: &str, name: Option<&str>, hope: bool) -> ModelEntry {
        ModelEntry {
            id: id.into(),
            name: name.map(str::to_owned),
            hope,
        }
    }

    #[test]
    fn the_judge_and_each_model_offered_are_choices_once_and_the_main_one_is_not() {
        let offered = [
            entry("hope-12b", Some("Hope"), true),
            entry("big", Some("Big"), false),
            entry("small", None, false),
            entry("hope-12b", None, false),
        ];
        let gateway = offering(Some("small"), &offered, &Constraints::default()).unwrap();
        let choices: Vec<&str> = gateway
            .routes()
            .choices()
            .iter()
            .map(|target| target.name.as_str())
            .collect();
        assert_eq!(choices, ["openrouter/small", "openrouter/hope-12b"]);
        let listed = offered_models(&gateway, &offered);
        let ids: Vec<(&str, Option<&str>, bool)> = listed
            .iter()
            .map(|m| (m.id.as_str(), m.name.as_deref(), m.hope))
            .collect();
        assert_eq!(
            ids,
            [
                ("big", Some("Big"), false),
                ("small", None, false),
                ("hope-12b", Some("Hope"), true)
            ]
        );
    }

    #[test]
    fn a_model_offered_that_the_policy_does_not_allow_is_refused_naming_the_rule() {
        let policy = constraints("version = 1\n[models]\ndeny = [\"*/secret\"]\n");
        let refused = offering(None, &[entry("secret", None, false)], &policy)
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
