//! A conversation stays with the provider that served it: a call that says which provider a
//! router last served it with at a target asks that router for it first (`order`), by the slug
//! its catalog has for the name the router gave, never outside what the floor and the person's
//! filters allow, and with the prompt sent as it came.

use std::sync::Arc;

use glave_event::{CatalogEndpoint, CatalogProvider, Filters, PrincipalId};
use glave_gateway::{Gateway, Router, Routes};
use glave_ports::{ModelProvider, ModelRequest, OrgId, Payer, Purpose, Route};
use jiff::Timestamp;

use crate::filters::Kept;
use crate::support::{Fake, block_on, request, with_tool};

const HAIKU: &str = "anthropic/claude-haiku-5.5";

/// The gateway's name for the router's target serving it.
const AT: &str = "openrouter/anthropic/claude-haiku-5.5";

/// OpenRouter's providers of Claude Haiku 5.5, as its list names them, all based in the US.
fn providers() -> Arc<[CatalogProvider]> {
    let provider = |slug: &str, name: &str| CatalogProvider {
        slug: slug.into(),
        name: name.into(),
        headquarters: Some("US".into()),
        datacenters: None,
    };
    [
        ("anthropic", "Anthropic"),
        ("claude-on-aws", "Claude Platform on AWS"),
        ("amazon-bedrock", "Amazon Bedrock"),
        ("google-vertex", "Google"),
        ("google-ai-studio", "Google AI Studio"),
    ]
    .into_iter()
    .map(|(slug, name)| provider(slug, name))
    .collect()
}

/// An endpoint of `slug`'s, keeping nothing when `zdr`, as OpenRouter listed Haiku's on
/// 2026-11-15: its maker's own are not on its list of those that keep nothing, the clouds' are.
fn endpoint(slug: &str, zdr: bool) -> CatalogEndpoint {
    CatalogEndpoint {
        provider: slug.into(),
        quantization: None,
        context: None,
        max_output: None,
        price: None,
        zdr,
        speed: None,
    }
}

fn catalog() -> Router {
    Router {
        providers: providers(),
        endpoints: vec![
            endpoint("anthropic", false),
            endpoint("claude-on-aws", false),
            endpoint("amazon-bedrock", true),
            endpoint("google-vertex", true),
        ],
    }
}

fn payer() -> Payer {
    let person = PrincipalId::new(Timestamp::from_second(1).unwrap(), [1; 10]).unwrap();
    Payer {
        org: OrgId::from_uuid(person.as_uuid()),
        person: Some(person),
    }
}

/// A conversation's call, with a tool on offer, that `served` says a router last served at
/// each target, by the target's name.
fn call(served: &[(&str, &str)]) -> ModelRequest {
    let mut request = with_tool(request(Purpose::Main));
    request.model = HAIKU.into();
    request.payer = Some(payer());
    request.upstreams = served
        .iter()
        .map(|(target, upstream)| ((*target).to_owned(), (*upstream).to_owned()))
        .collect();
    request
}

/// What `request` was sent with at OpenRouter, under `floor` and the person's `filters`.
fn sent(request: &ModelRequest, floor: Option<Filters>, filters: Option<Filters>) -> ModelRequest {
    let openrouter = Fake::new("openrouter");
    let targets = vec![openrouter.target(HAIKU).routing(catalog())];
    let gateway = Gateway::new(Routes::new(targets).unwrap())
        .with_floor(floor)
        .with_filters(Kept::answering(Ok(filters)));
    block_on!(gateway.complete(request, &mut |_| {})).unwrap();
    openrouter.seen().pop().unwrap()
}

fn cloud() -> Filters {
    Filters {
        based_in: vec!["US".into()],
        keeps_nothing: true,
        ..Filters::default()
    }
}

fn the_clouds() -> Vec<String> {
    vec!["amazon-bedrock".into(), "google-vertex".into()]
}

#[test]
fn a_conversation_s_first_call_asks_for_no_provider_first() {
    assert_eq!(sent(&call(&[]), None, None).route, None);
}

#[test]
fn a_later_call_asks_first_for_the_provider_that_served_it_by_its_slug() {
    let bedrock = sent(&call(&[(AT, "Amazon Bedrock")]), None, None);
    assert_eq!(
        bedrock.route,
        Some(Route {
            order: vec!["amazon-bedrock".into()],
            ..Route::default()
        }),
        "only the order: the router still falls back as it would"
    );
    let google = sent(&call(&[(AT, "Google")]), None, None);
    let order = google.route.map(|route| route.order);
    assert_eq!(
        order,
        Some(vec!["google-vertex".into()]),
        "Google is Vertex, not AI Studio"
    );
}

#[test]
fn the_order_comes_with_the_floor_and_never_widens_it() {
    let bedrock = sent(&call(&[(AT, "Amazon Bedrock")]), Some(cloud()), None);
    let route = bedrock.route.unwrap();
    assert_eq!(route.order, ["amazon-bedrock"]);
    assert_eq!(route.only, Some(the_clouds()));
    assert!(route.zdr);
    // A provider the floor rules out, though it served the conversation before the floor
    // came, is never asked for.
    let anthropic = sent(&call(&[(AT, "Anthropic")]), Some(cloud()), None);
    let route = anthropic.route.unwrap();
    assert_eq!(route.order, Vec::<String>::new());
    assert_eq!(route.only, Some(the_clouds()));
}

#[test]
fn a_provider_the_person_avoids_or_one_the_catalog_does_not_name_is_never_asked_for() {
    let avoid = Filters {
        avoid: vec!["amazon-bedrock".into()],
        ..Filters::default()
    };
    let avoided = sent(&call(&[(AT, "Amazon Bedrock")]), None, Some(avoid));
    let route = avoided.route.unwrap();
    assert_eq!(route.order, Vec::<String>::new());
    assert_eq!(route.ignore, ["amazon-bedrock"]);
    let unknown = sent(&call(&[(AT, "Someone New")]), None, None);
    assert_eq!(unknown.route, None);
}

#[test]
fn only_the_target_that_served_it_is_asked() {
    let elsewhere = sent(
        &call(&[("openai/gpt-6-luna", "Amazon Bedrock")]),
        None,
        None,
    );
    assert_eq!(elsewhere.route, None);
}

#[test]
fn the_prompt_is_sent_as_it_came() {
    let request = call(&[(AT, "Amazon Bedrock")]);
    let sent = sent(&request, Some(cloud()), None);
    assert_eq!(sent.system, request.system);
    assert_eq!(sent.messages, request.messages);
    assert_eq!(sent.tools, request.tools);
    assert_eq!(sent.tool_choice, request.tool_choice);
    assert_eq!(sent.max_output_tokens, request.max_output_tokens);
    assert_eq!(sent.effort, request.effort);
}
