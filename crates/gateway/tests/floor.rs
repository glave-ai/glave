//! Glave Cloud's floor (providers based in the US that keep nothing, weights at 8 bits or
//! better) on models as a router's catalog says who serves them: closed models, whose makers
//! say no precision, still route; an open model served only below the floor is refused with a
//! sentence and never sent.

use std::sync::Arc;

use glave_event::{CatalogEndpoint, CatalogProvider, Filters, Precision};
use glave_gateway::{Gateway, Router, Routes};
use glave_ports::{ModelProvider, Purpose, Route};

use crate::support::{Fake, block_on, request};

/// Glave Cloud's floor.
fn cloud() -> Filters {
    Filters {
        based_in: vec!["US".into()],
        keeps_nothing: true,
        precision: Some(Precision::EightBit),
        ..Filters::default()
    }
}

/// OpenRouter's providers, each by slug and where it is based.
fn providers() -> Arc<[CatalogProvider]> {
    let provider = |slug: &str, home: &str| CatalogProvider {
        slug: slug.into(),
        name: slug.into(),
        headquarters: Some(home.into()),
        datacenters: None,
    };
    [
        ("anthropic", "US"),
        ("amazon-bedrock", "US"),
        ("google-vertex", "US"),
        ("openai", "US"),
        ("azure", "US"),
        ("deepseek", "CN"),
        ("fireworks", "US"),
        ("groq", "US"),
        ("together", "US"),
        ("siliconflow", "CN"),
    ]
    .into_iter()
    .map(|(slug, home)| provider(slug, home))
    .collect()
}

/// An endpoint of `slug`'s, its weights stored as `quantization` (none where the router says
/// `unknown`, as it does for a closed model's), keeping nothing when `zdr`.
fn endpoint(slug: &str, quantization: Option<&str>, zdr: bool) -> CatalogEndpoint {
    CatalogEndpoint {
        provider: slug.into(),
        quantization: quantization.map(str::to_owned),
        context: None,
        max_output: None,
        price: None,
        zdr,
        speed: None,
    }
}

/// `model` at a router whose catalog lists `endpoints` of it, under Cloud's floor: whom the
/// call went to, or the sentence it was refused with.
fn served(model: &str, endpoints: Vec<CatalogEndpoint>) -> Result<Route, String> {
    let openrouter = Fake::new("openrouter");
    let catalog = Router {
        providers: providers(),
        endpoints,
    };
    let targets = vec![openrouter.target(model).routing(catalog)];
    let routes = Routes::new(targets).map_err(|err| err.to_string())?;
    let gateway = Gateway::new(routes).with_floor(Some(cloud()));
    let answer = block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {}));
    let sent = openrouter.seen();
    match answer {
        Ok(_) => (sent.first().and_then(|sent| sent.route.clone()))
            .ok_or_else(|| "sent asking nothing of the router".to_owned()),
        Err(refused) => {
            assert!(sent.is_empty(), "a refused call is never sent");
            Err(refused.to_string())
        }
    }
}

fn only(route: &Route) -> Vec<&str> {
    route.only.iter().flatten().map(String::as_str).collect()
}

#[test]
fn a_closed_model_whose_maker_says_no_precision_still_routes() {
    let haiku = served(
        "anthropic/claude-haiku-5.5",
        vec![
            endpoint("anthropic", None, true),
            endpoint("amazon-bedrock", None, true),
            endpoint("google-vertex", None, false),
        ],
    )
    .unwrap();
    assert_eq!(only(&haiku), ["amazon-bedrock", "anthropic"]);
    assert!(haiku.zdr);
    assert!(
        haiku.quantizations.iter().any(|named| named == "unknown"),
        "the router keeps endpoints that don't say: {:?}",
        haiku.quantizations
    );
    let luna = served(
        "openai/gpt-6-luna",
        vec![
            endpoint("openai", None, true),
            endpoint("azure", None, true),
        ],
    )
    .unwrap();
    assert_eq!(only(&luna), ["azure", "openai"]);
}

#[test]
fn an_open_model_routes_only_to_providers_at_8_bits_or_better() {
    let flash = served(
        "deepseek/deepseek-v4.1-flash-1017",
        vec![
            endpoint("deepseek", Some("fp8"), false),
            endpoint("fireworks", Some("fp8"), true),
            endpoint("groq", Some("fp4"), true),
            endpoint("together", Some("bf16"), true),
            endpoint("siliconflow", Some("fp8"), true),
        ],
    )
    .unwrap();
    assert_eq!(only(&flash), ["fireworks", "together"]);
    assert_eq!(
        flash.quantizations,
        ["int8", "fp8", "mxfp8", "fp16", "bf16", "fp32", "unknown"]
    );
}

#[test]
fn a_model_served_only_below_the_floor_s_precision_is_refused_with_a_sentence() {
    let refused = served(
        "some/open-model-q4",
        vec![
            endpoint("groq", Some("fp4"), true),
            endpoint("together", Some("int4"), true),
        ],
    )
    .unwrap_err();
    assert!(
        refused.contains("this server allows no provider of asked-for"),
        "{refused}"
    );
}

#[test]
fn a_person_s_own_precision_holds_providers_that_don_t_say_theirs() {
    // The floor keeps a closed model; a person who chose a precision chose the providers that
    // say theirs, so the same model is refused for them with the sentence that says both.
    let openrouter = Fake::new("openrouter");
    let catalog = Router {
        providers: providers(),
        endpoints: vec![endpoint("anthropic", None, true)],
    };
    let targets = vec![
        openrouter
            .target("anthropic/claude-haiku-5.5")
            .routing(catalog),
    ];
    let full = Filters {
        precision: Some(Precision::Full),
        ..Filters::default()
    };
    let filters = crate::filters::Kept::answering(Ok(Some(full)));
    let gateway = Gateway::new(Routes::new(targets).unwrap())
        .with_filters(filters)
        .with_floor(Some(cloud()));
    let refused = block_on!(gateway.complete(&crate::filters::theirs(), &mut |_| {}));
    let refused = refused.unwrap_err().to_string();
    assert!(openrouter.seen().is_empty(), "never sent");
    assert!(
        refused.contains("both your filters and what this server allows"),
        "{refused}"
    );
}
