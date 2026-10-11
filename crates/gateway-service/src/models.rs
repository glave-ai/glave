//! Which targets a request's model name means, and which of them the caller may use. A name
//! is a route (`main`, `judge`, `summary`, `subagent`: its targets, in order); a target the
//! routes have, by its name or its model; or any other model at the main route's first
//! provider, as `provider/model` or the model alone. The caller's policy then keeps only the
//! targets it allows.

use glave_gateway::{Gateway, Target};
use glave_identity::Caller;
use glave_ports::Purpose;

use crate::errors::{Kind, Refusal};
use crate::service::Policy;

/// Every purpose, in the order routes are searched for a target.
const PURPOSES: [Purpose; 4] = [
    Purpose::Main,
    Purpose::Judge,
    Purpose::Summary,
    Purpose::Subagent,
];

/// The purpose and the targets `name` means for `caller`, or why they may use none.
pub(crate) fn resolve(
    gateway: &Gateway,
    policy: &dyn Policy,
    caller: &Caller,
    name: &str,
) -> Result<(Purpose, Vec<Target>), Refusal> {
    let (purpose, targets) = meant(gateway, name)?;
    let mut refused = None;
    let allowed: Vec<Target> = targets
        .into_iter()
        .filter(|target| match policy.refuses_model(caller, &target.name) {
            Some(rule) => {
                refused.get_or_insert(format!("model {} is not allowed: {rule}", target.name));
                false
            }
            None => true,
        })
        .collect();
    match (allowed.is_empty(), refused) {
        (true, Some(refused)) => Err(Refusal::new(Kind::Forbidden, refused)),
        (true, None) => Err(Refusal::new(
            Kind::NotFound,
            format!("no model serves {name:?}"),
        )),
        (false, _) => Ok((purpose, allowed)),
    }
}

fn meant(gateway: &Gateway, name: &str) -> Result<(Purpose, Vec<Target>), Refusal> {
    let routes = gateway.routes();
    if let Some(purpose) = PURPOSES.into_iter().find(|purpose| purpose.id() == name) {
        return Ok((purpose, routes.targets(purpose).to_vec()));
    }
    for purpose in PURPOSES {
        if let Some(target) = routes
            .targets(purpose)
            .iter()
            .find(|target| target.name == name || target.model == name)
        {
            return Ok((Purpose::Main, vec![target.clone()]));
        }
    }
    let first = routes
        .targets(Purpose::Main)
        .first()
        .ok_or_else(|| Refusal::new(Kind::NotFound, "the gateway has no models"))?;
    if name.trim().is_empty() {
        return Err(Refusal::invalid("model is empty"));
    }
    let model = name
        .strip_prefix(&first.provider)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(name);
    Ok((Purpose::Main, vec![first.sibling(model)]))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glave_gateway::{Model, Routes};
    use glave_ports::{BoxFuture, Completion, Delta, ModelRequest, ProviderError};

    use super::*;

    struct Never;

    impl Model for Never {
        fn send<'a>(
            &'a self,
            _: &'a ModelRequest,
            _: &'a mut (dyn FnMut(Delta) + Send),
        ) -> BoxFuture<'a, Result<Completion, ProviderError>> {
            Box::pin(std::future::ready(Err(ProviderError::Transport(
                "x".into(),
            ))))
        }
    }

    struct Only(&'static str);

    impl Policy for Only {
        fn refuses_model(&self, _: &Caller, target: &str) -> Option<String> {
            (target != self.0).then(|| format!("only {}", self.0))
        }
        fn refuses_tool(&self, _: &Caller, _: &str, _: &str) -> Option<String> {
            None
        }
    }

    fn gateway() -> Gateway {
        let handle: Arc<dyn Model> = Arc::new(Never);
        let main = vec![
            Target::new(Arc::clone(&handle), "acme", "big"),
            Target::new(Arc::clone(&handle), "acme", "backup"),
        ];
        let judge = vec![Target::new(handle, "acme", "small")];
        let routes = Routes::new(main)
            .unwrap()
            .with_route(Purpose::Judge, judge)
            .unwrap();
        Gateway::new(routes)
    }

    fn names(name: &str, policy: &dyn Policy) -> Result<(Purpose, Vec<String>), Refusal> {
        resolve(&gateway(), policy, &Caller::default(), name)
            .map(|(purpose, targets)| (purpose, targets.into_iter().map(|t| t.name).collect()))
    }

    struct Any;

    impl Policy for Any {
        fn refuses_model(&self, _: &Caller, _: &str) -> Option<String> {
            None
        }
        fn refuses_tool(&self, _: &Caller, _: &str, _: &str) -> Option<String> {
            None
        }
    }

    #[test]
    fn names_mean_routes_targets_or_other_models_at_the_main_provider() {
        let main = (Purpose::Main, vec!["acme/big".into(), "acme/backup".into()]);
        assert_eq!(names("main", &Any).unwrap(), main);
        let judge = (Purpose::Judge, vec!["acme/small".to_owned()]);
        assert_eq!(names("judge", &Any).unwrap(), judge);
        assert_eq!(names("small", &Any).unwrap().1, ["acme/small"]);
        assert_eq!(names("acme/backup", &Any).unwrap().1, ["acme/backup"]);
        assert_eq!(names("acme/other", &Any).unwrap().1, ["acme/other"]);
        assert_eq!(
            names("vendor/other", &Any).unwrap().1,
            ["acme/vendor/other"]
        );
    }

    #[test]
    fn the_policy_keeps_only_the_targets_it_allows() {
        let backup = Only("acme/backup");
        assert_eq!(names("main", &backup).unwrap().1, ["acme/backup"]);
        let refused = names("judge", &backup).unwrap_err();
        assert_eq!(refused.kind, Kind::Forbidden);
        assert_eq!(
            refused.message,
            "model acme/small is not allowed: only acme/backup"
        );
    }
}
