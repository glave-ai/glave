//! A conversation stays with the provider that served it (`specs/gateway.md`, "Fallback"):
//! each provider behind a router keeps its own prompt cache, so a call passed on to another
//! than the last repays the whole prefix. The router is asked for the last one first, by the
//! name it gave it, never past what the call's filters and floor allow, and falls back as it
//! would when that one fails; whichever serves is the one the next call asks for.

use glave_ports::{ModelRequest, Route};

use crate::filtering::Router;
use crate::target::Target;

/// `route`, asking `target`'s router first for the provider that last served `request`'s
/// conversation there, where its catalog names that provider and the route allows it. As it
/// was where the target is not a router or nothing served the conversation there yet.
pub(crate) fn stay(route: Option<Route>, target: &Target, request: &ModelRequest) -> Option<Route> {
    let Some(router) = &target.router else {
        return route;
    };
    let Some(slug) = (request.upstreams.get(&target.name)).and_then(|name| slug_of(router, name))
    else {
        return route;
    };
    let allowed = route.as_ref().is_none_or(|route| {
        route
            .only
            .as_ref()
            .is_none_or(|only| only.iter().any(|s| s == slug))
            && !route.ignore.iter().any(|s| s == slug)
    });
    let listed = router.endpoints.is_empty() || router.endpoints.iter().any(|e| e.provider == slug);
    if !allowed || !listed {
        return route;
    }
    let mut route = route.unwrap_or_default();
    route.order = vec![slug.to_owned()];
    Some(route)
}

/// The slug of the provider `router`'s catalog names `name`, as the router names it in its
/// answers (`Amazon Bedrock` is `amazon-bedrock`); none when the catalog lists no such one.
fn slug_of<'r>(router: &'r Router, name: &str) -> Option<&'r str> {
    let name = name.trim();
    router
        .providers
        .iter()
        .find(|provider| provider.name.eq_ignore_ascii_case(name) || provider.slug == name)
        .map(|provider| provider.slug.as_str())
}
