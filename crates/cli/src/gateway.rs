//! The gateway every model call goes through, built from today's settings: `GLAVE_MODEL` is
//! the `main` route, and `GLAVE_JUDGE_MODEL`, when it names another model at the same
//! endpoint, the `judge` route. Every other purpose uses `main`, and there are no budgets, so
//! nothing changes for anyone who sets nothing new. A policy's `[models]` rules are kept
//! here: a target they do not allow is refused before the gateway exists. With `[gateway]
//! url`, the endpoint is the organization's gateway, which keeps them itself, for each caller.
//! The judge model and each of `[[models]]` are choices a thread may make instead of the main
//! model (`model/list`), checked against the policy as the main model is.

pub mod remote;
pub mod serve;

use std::sync::Arc;

use glave_gateway::{Gateway, Routes, Target};
use glave_ports::Purpose;

use crate::config::Endpoint;
use crate::model::Model;
use glave_config::policy::{Constraints, ModelEntry, Source, check_model};
use glave_session::protocol::ModelInfo;

/// Which models a policy allows, and where the main and judge models were chosen.
pub struct Allowed<'a> {
    pub constraints: &'a Constraints,
    /// Where the main model was chosen, when anyone chose it.
    pub main: Option<&'a Source>,
    pub judge: &'a Source,
    /// Where `[[models]]` was chosen.
    pub models: &'a Source,
}

/// The gateway over `provider` at `endpoint`, with a judge route when `judge` names a model
/// other than the main one, and the judge's model and each of `offered` as choices. Fails
/// when `allowed` refuses any of them.
pub fn gateway(
    provider: Model,
    endpoint: &Endpoint,
    judge: Option<&str>,
    offered: &[ModelEntry],
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
    let mut routes = Routes::new(vec![main])?;
    let mut choices: Vec<Target> = Vec::new();
    if let Some(model) = judge.filter(|model| *model != endpoint.model) {
        let target = Target::new(Arc::clone(&handle), name.clone(), model);
        if checks {
            check_model(&target.name, allowed.judge, allowed.constraints)?;
        }
        choices.push(target.clone());
        routes = routes.with_route(Purpose::Judge, vec![target])?;
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
    Ok(Gateway::new(routes.with_choices(choices)))
}

/// What `model/list` offers besides the main model, which the server adds itself: the
/// gateway's choices, each named and marked as `[[models]]` says, and the main model's own
/// entry, for its name and mark.
pub fn offered(gateway: &Gateway, entries: &[ModelEntry]) -> Vec<ModelInfo> {
    let info = |id: &str| {
        let entry = entries.iter().find(|entry| entry.id.trim() == id);
        ModelInfo {
            id: id.to_owned(),
            name: entry.and_then(|entry| entry.name.clone()),
            default: false,
            hope: entry.is_some_and(|entry| entry.hope),
        }
    };
    let routes = gateway.routes();
    routes
        .targets(Purpose::Main)
        .first()
        .into_iter()
        .chain(routes.choices())
        .map(|target| info(&target.model))
        .collect()
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

    fn endpoint() -> Endpoint {
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
        gateway(provider, &endpoint, judge, offered, &allowed)
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

    fn judge_target(judge: Option<&str>) -> String {
        let gateway = built(judge, &Constraints::default()).unwrap();
        let targets = gateway.routes().targets(Purpose::Judge);
        assert_eq!(targets.len(), 1);
        targets[0].name.clone()
    }

    #[test]
    fn the_judge_has_a_route_only_when_it_names_another_model() {
        assert_eq!(judge_target(None), "openrouter/big");
        assert_eq!(judge_target(Some("big")), "openrouter/big");
        assert_eq!(judge_target(Some("small")), "openrouter/small");
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
