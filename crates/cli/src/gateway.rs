//! The gateway every model call goes through, built from today's settings: `GLAVE_MODEL` is
//! the `main` route, and `GLAVE_JUDGE_MODEL`, when it names another model at the same
//! endpoint, the `judge` route. Every other purpose uses `main`, and there are no budgets, so
//! nothing changes for anyone who sets nothing new.

use std::sync::Arc;

use glave_gateway::{Gateway, Routes, Target};
use glave_ports::Purpose;

use crate::config::Endpoint;
use crate::model::Model;

/// The gateway over `provider` at `endpoint`, with a judge route when `judge` names a model
/// other than the main one.
pub fn gateway(
    provider: Model,
    endpoint: &Endpoint,
    judge: Option<&str>,
) -> anyhow::Result<Gateway> {
    let handle: Arc<dyn glave_gateway::Model> = Arc::new(provider);
    let name = &endpoint.provider.provider;
    let main = Target::new(Arc::clone(&handle), name.clone(), endpoint.model.clone());
    let mut routes = Routes::new(vec![main])?;
    if let Some(model) = judge.filter(|model| *model != endpoint.model) {
        let target = Target::new(handle, name.clone(), model);
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

    fn judge_target(judge: Option<&str>) -> String {
        let endpoint = endpoint();
        let provider = Model::for_endpoint(&endpoint.provider).unwrap();
        let gateway = gateway(provider, &endpoint, judge).unwrap();
        let targets = gateway.routes().targets(Purpose::Judge);
        assert_eq!(targets.len(), 1);
        targets[0].name.clone()
    }

    #[test]
    fn the_judge_has_a_route_only_when_it_names_another_model() {
        assert_eq!(judge_target(None), "openrouter/big");
        assert_eq!(judge_target(Some("big")), "openrouter/big");
        assert_eq!(judge_target(Some("small")), "openrouter/small");
        let endpoint = endpoint();
        let provider = Model::for_endpoint(&endpoint.provider).unwrap();
        let gateway = gateway(provider, &endpoint, Some("small")).unwrap();
        assert_eq!(
            gateway.routes().targets(Purpose::Main)[0].name,
            "openrouter/big"
        );
    }
}
