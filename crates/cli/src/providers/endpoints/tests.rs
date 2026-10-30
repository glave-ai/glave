use glave_ports::testing::FakeHttp;
use serde_json::json;

use super::*;

/// OpenRouter's lists, cut to what decides: three providers, two of them serving a model.
fn listed() -> (Value, Value, Value) {
    let providers = json!({"data": [
        {"slug": "deepinfra", "name": "DeepInfra", "headquarters": "US", "datacenters": null},
        {"slug": "deepseek", "name": "DeepSeek", "headquarters": "CN", "datacenters": []},
        {"slug": "alibaba", "name": "Alibaba", "headquarters": "SG", "datacenters": ["SG", "cn", "SG"]},
        {"slug": "", "name": "Nameless"},
        {"slug": "deepinfra", "name": "Again"},
    ]});
    let zdr = json!({"data": [
        {"model_id": "deepseek/deepseek-chat", "tag": "deepinfra/fp8"},
        {"model_id": "other/model", "tag": "deepseek"},
    ]});
    let endpoints = json!({"data": {"id": "deepseek/deepseek-chat", "endpoints": [
        {"provider_name": "DeepInfra", "tag": "deepinfra/fp8", "quantization": "fp8",
         "context_length": 163_840, "max_completion_tokens": 147_456,
         "pricing": {"prompt": "0.000000256", "completion": "0.000000712"},
         "throughput_last_30m": {"p50": 84.6, "p90": 120}},
        {"provider_name": "DeepSeek", "tag": "deepseek", "quantization": "unknown",
         "context_length": 64000, "pricing": {"prompt": "0.00000027", "completion": "0.0000011"},
         "throughput_last_30m": null},
        {"provider_name": "Somebody New", "tag": "newcomer/turbo", "quantization": null},
    ]}});
    (providers, zdr, endpoints)
}

#[test]
fn providers_say_where_they_are_based_and_datacenters_never_listed_stay_unknown() {
    let (listed, _, _) = listed();
    let providers = providers(&listed);
    let slugs: Vec<&str> = providers.iter().map(|p| p.slug.as_str()).collect();
    assert_eq!(
        slugs,
        ["deepinfra", "deepseek", "alibaba"],
        "each once, none unnamed"
    );
    assert_eq!(providers[0].headquarters.as_deref(), Some("US"));
    assert_eq!(providers[0].datacenters, None, "listed as null: unknown");
    assert_eq!(
        providers[1].datacenters, None,
        "listed as none: unknown too"
    );
    assert_eq!(
        providers[2].datacenters,
        Some(vec!["CN".to_owned(), "SG".to_owned()]),
        "each once, in order"
    );
}

#[test]
fn a_model_s_endpoints_say_their_provider_precision_price_speed_and_whether_they_keep_nothing() {
    let (providers, zdr, listed) = listed();
    let named: BTreeMap<String, String> = super::providers(&providers)
        .into_iter()
        .map(|p| (p.name, p.slug))
        .collect();
    let zdr = keeping_nothing(&zdr);
    let found = endpoints("deepseek/deepseek-chat", &listed, &named, &zdr);
    assert_eq!(found.len(), 3);
    let infra = &found[0];
    assert_eq!(infra.provider, "deepinfra");
    assert_eq!(infra.quantization.as_deref(), Some("fp8"));
    assert_eq!(
        (infra.context, infra.max_output),
        (Some(163_840), Some(147_456))
    );
    assert_eq!(
        infra.price.map(|p| (p.input, p.output)),
        Some((256_000, 712_000))
    );
    assert!(infra.zdr, "listed as keeping nothing");
    assert_eq!(infra.speed, Some(85), "the median, rounded");
    let own = &found[1];
    assert_eq!(own.quantization, None, "unknown says nothing");
    assert!(!own.zdr && own.speed.is_none());
    assert_eq!(
        found[2].provider, "newcomer",
        "a provider not listed: its tag's first part"
    );
}

/// A model reached at `fake`'s address, as an OpenRouter account's is.
fn router(fake: &FakeHttp) -> Model {
    let config =
        glave_provider_openai::Config::for_url(format!("{}/api/v1", fake.url), None).unwrap();
    Model::OpenAi(glave_provider_openai::OpenAiCompatible::new(config).unwrap())
}

/// A stand-in for OpenRouter's lists: `listed()`'s, the model `deepseek/deepseek-chat`'s
/// endpoints, and none for any other model, or no providers at all when `broken`.
fn stand_in(broken: bool) -> FakeHttp {
    let (providers, zdr, endpoints) = listed();
    FakeHttp::answering(move |_, path| match path {
        "/api/v1/providers" if broken => (500, "{}".to_owned()),
        "/api/v1/providers" => (200, providers.to_string()),
        "/api/v1/endpoints/zdr" => (200, zdr.to_string()),
        "/api/v1/models/deepseek/deepseek-chat/endpoints" => (200, endpoints.to_string()),
        _ => (404, "{}".to_owned()),
    })
    .unwrap()
}

#[test]
fn a_model_whose_endpoints_are_not_read_keeps_those_it_had() {
    let fake = stand_in(false);
    let mut models = vec![
        CatalogModel::new("deepseek/deepseek-chat"),
        CatalogModel::new("other/model"),
    ];
    let mut had = CatalogModel::new("other/model");
    had.endpoints = endpoints("x", &listed().2, &BTreeMap::new(), &BTreeSet::new());
    let before = [had.clone()];
    let read = glave_ports::block_on!(read(router(&fake), &mut models, &before)).unwrap();
    let (providers, whole) = read;
    assert!(whole, "every model's endpoints were asked for in time");
    assert_eq!(providers.len(), 3);
    assert_eq!(models[0].endpoints.len(), 3, "read");
    assert_eq!(
        models[1].endpoints, had.endpoints,
        "not found: kept as it was"
    );
}

#[test]
fn nothing_is_read_when_the_providers_cannot_be() {
    let fake = stand_in(true);
    let mut models = vec![CatalogModel::new("deepseek/deepseek-chat")];
    let refused = glave_ports::block_on!(read(router(&fake), &mut models, &[])).unwrap_err();
    assert!(refused.starts_with("its providers: "), "{refused}");
    assert_eq!(models[0].endpoints, Vec::new());
}

#[test]
fn endpoints_that_cannot_call_tools_are_left_out() {
    let listed = json!({"data": {"endpoints": [
        {"provider_name": "A", "tag": "a", "supported_parameters": ["tools", "max_tokens"]},
        {"provider_name": "B", "tag": "b", "supported_parameters": ["max_tokens"]},
        {"provider_name": "C", "tag": "c"},
    ]}});
    let found = endpoints("m", &listed, &BTreeMap::new(), &BTreeSet::new());
    let slugs: Vec<&str> = found.iter().map(|e| e.provider.as_str()).collect();
    assert_eq!(slugs, ["a", "c"], "one that says nothing may");
}
