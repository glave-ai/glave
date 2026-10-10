//! The gateway every model call goes through, built from today's settings: `GLAVE_MODEL` is
//! the `main` route, and `GLAVE_JUDGE_MODEL`, when it names another model at the same
//! endpoint, the `judge` route. Every other purpose uses `main`, and there are no budgets, so
//! nothing changes for anyone who sets nothing new. A policy's `[models]` rules are kept
//! here: a target they do not allow is refused before the gateway exists.

use std::sync::Arc;

use glave_gateway::{Gateway, Routes, Target};
use glave_ports::Purpose;

use crate::config::Endpoint;
use crate::model::Model;
use crate::policy::{Constraints, Source, check_model};

/// Which models a policy allows, and where the main and judge models were chosen.
pub struct Allowed<'a> {
    pub constraints: &'a Constraints,
    /// Where the main model was chosen, when anyone chose it.
    pub main: Option<&'a Source>,
    pub judge: &'a Source,
}

/// The gateway over `provider` at `endpoint`, with a judge route when `judge` names a model
/// other than the main one. Fails when `allowed` refuses either model.
pub fn gateway(
    provider: Model,
    endpoint: &Endpoint,
    judge: Option<&str>,
    allowed: &Allowed<'_>,
) -> anyhow::Result<Gateway> {
    let handle: Arc<dyn glave_gateway::Model> = Arc::new(provider);
    let name = &endpoint.provider.provider;
    let main = Target::new(Arc::clone(&handle), name.clone(), endpoint.model.clone());
    let chosen = allowed.main.unwrap_or(&Source::Default);
    check_model(&main.name, chosen, allowed.constraints)?;
    let mut routes = Routes::new(vec![main])?;
    if let Some(model) = judge.filter(|model| *model != endpoint.model) {
        let target = Target::new(handle, name.clone(), model);
        check_model(&target.name, allowed.judge, allowed.constraints)?;
        routes = routes.with_route(Purpose::Judge, vec![target])?;
    }
    Ok(Gateway::new(routes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_provider_openai::Config;

    fn endpoint() -> Endpoint {
        let provider = Config::for_url("https://openrouter.ai/api/v1", None).unwrap();
        Endpoint {
            local: false,
            provider,
            model: "big".into(),
        }
    }

    fn built(judge: Option<&str>, constraints: &Constraints) -> anyhow::Result<Gateway> {
        let endpoint = endpoint();
        let provider = Model::for_endpoint(&endpoint.provider)?;
        let allowed = Allowed {
            constraints,
            main: Some(&Source::Flag("--model")),
            judge: &Source::Env("GLAVE_JUDGE_MODEL"),
        };
        gateway(provider, &endpoint, judge, &allowed)
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
        let policy = |text: &str| crate::policy::tests::constraints(text);
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
