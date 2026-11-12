//! Reading OpenRouter's image models: their providers by their slugs, and a price only where
//! everything is billed by the token and every provider can be read.

use glave_ports::testing::FakeHttp;
use serde_json::json;

use super::*;

fn per_million(dollars: i64) -> Nanos {
    Nanos::new(dollars * 1_000_000_000)
}

#[test]
fn a_provider_is_priced_by_the_token_at_its_dearest_lines_and_otherwise_not_at_all() {
    let listed = json!({"id": "openai/gpt-image-2.6-flare", "endpoints": [
        {"provider_slug": "openai", "pricing": [
            {"billable": "input_image", "unit": "token", "cost_usd": 8e-06},
            {"billable": "input_text", "unit": "token", "cost_usd": 5e-06},
            {"billable": "output_image", "unit": "token", "cost_usd": 3e-05},
            {"billable": "output_image", "unit": "token", "cost_usd": 4e-05, "variant": "4k"},
        ]},
        {"provider_slug": "seed", "pricing": [
            {"billable": "output_image", "unit": "image", "cost_usd": 0.045},
        ]},
    ]});
    let endpoints = endpoints("openai/gpt-image-2.6-flare", &listed);
    let slugs: Vec<&str> = endpoints.iter().map(|e| e.provider.as_str()).collect();
    assert_eq!(slugs, ["openai", "seed"]);
    assert_eq!(
        endpoints[0].price,
        Some(TokenPrice {
            input: per_million(8),
            output: per_million(40),
            cache_read: None,
        }),
        "the dearest line of each, a variant's among them"
    );
    assert_eq!(endpoints[1].price, None, "billed by the image");
    assert_eq!(
        dearest(&endpoints),
        None,
        "a model is priced only when every provider is"
    );
    assert_eq!(dearest(&endpoints[..1]), endpoints[0].price);
    assert_eq!(dearest(&[]), None);
    let cheaper = CatalogEndpoint {
        price: Some(TokenPrice {
            input: per_million(9),
            output: per_million(30),
            cache_read: None,
        }),
        ..endpoints[0].clone()
    };
    let both = dearest(&[endpoints[0].clone(), cheaper]).unwrap();
    assert_eq!((both.input, both.output), (per_million(9), per_million(40)));
    let nothing_out = json!([{"billable": "input_text", "unit": "token", "cost_usd": 5e-06}]);
    assert_eq!(by_the_token(&nothing_out), None, "nothing made is priced");
}

#[test]
fn a_provider_that_cant_be_read_leaves_its_model_unknown_and_unpriced() {
    let listed = json!({"endpoints": [
        {"provider_slug": "openai", "pricing": [
            {"billable": "output_image", "unit": "token", "cost_usd": 3e-05},
        ]},
        {"provider_slug": "", "pricing": []},
    ]});
    let endpoints = endpoints("openai/gpt-image-2.6-flare", &listed);
    assert!(endpoints.is_empty(), "{endpoints:?}");
    assert_eq!(dearest(&endpoints), None);
}

#[test]
fn an_image_model_not_read_keeps_what_it_had_and_the_read_is_not_whole() {
    let flare = json!({"endpoints": [{"provider_slug": "openai", "pricing": [
        {"billable": "output_image", "unit": "token", "cost_usd": 3e-05},
    ]}]});
    let fake = FakeHttp::answering(move |_, path| match path {
        "/api/v1/images/models/openai/gpt-image-2.6-flare/endpoints" => (200, flare.to_string()),
        _ => (404, "{}".to_owned()),
    })
    .unwrap();
    let at = glave_provider_openai::Config::for_url(format!("{}/api/v1", fake.url), None).unwrap();
    let router = Model::OpenAi(glave_provider_openai::OpenAiCompatible::new(at).unwrap());
    let mut had = CatalogModel::new("openai/gpt-image-2.6-sunburst");
    had.price = Some(TokenPrice {
        input: per_million(8),
        output: per_million(30),
        cache_read: None,
    });
    let ids = [
        "openai/gpt-image-2.6-flare",
        "openai/gpt-image-2.6-sunburst",
    ];
    let before = [had.clone()];
    let (models, whole) = glave_ports::block_on!(read(router, &ids, &before));
    assert!(!whole, "one was not found");
    assert_eq!(
        models[0].price.map(|price| price.output),
        Some(per_million(30))
    );
    assert_eq!(models[1], had, "kept as it was");
}
