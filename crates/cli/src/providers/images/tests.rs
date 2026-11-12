//! Reading OpenRouter's image models: each by its id and name, its providers by their slugs,
//! and a price only where everything is billed by the token.

use serde_json::json;

use super::*;

fn per_million(dollars: i64) -> Nanos {
    Nanos::new(dollars * 1_000_000_000)
}

#[test]
fn image_models_are_listed_by_id_and_name_once_each() {
    let listed = json!({"data": [
        {"id": "openai/gpt-image-2.6-flare", "name": "OpenAI: GPT Image 2.6 Flare",
         "architecture": {"input_modalities": ["text", "image"], "output_modalities": ["image"]}},
        {"id": "black-forest-labs/flux-3-image", "name": "",
         "architecture": {"input_modalities": ["text"]}},
        {"id": "openai/gpt-image-2.6-flare"},
        {"id": "has space"},
        {"name": "no id"},
    ]});
    let cards = cards(&listed);
    let ids: Vec<&str> = cards.iter().map(|card| card.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "openai/gpt-image-2.6-flare",
            "black-forest-labs/flux-3-image"
        ]
    );
    assert_eq!(
        cards[0].name.as_deref(),
        Some("OpenAI: GPT Image 2.6 Flare")
    );
    assert_eq!(cards[0].images, Some(true));
    assert_eq!(
        (cards[1].name.as_deref(), cards[1].images),
        (None, Some(false))
    );
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
        {"provider_slug": "", "pricing": []},
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
