use glave_event::{MaxPrice, Sort};

use super::*;

const FLASH: &str = "deepseek/deepseek-v4.1-flash";

/// A provider of `slug`'s, based in `home`.
fn provider(slug: &str, home: Option<&str>) -> CatalogProvider {
    CatalogProvider {
        slug: slug.to_owned(),
        name: slug.to_uppercase(),
        headquarters: home.map(str::to_owned),
        datacenters: None,
    }
}

/// An endpoint of `slug`'s, its weights stored as `quantization`, keeping nothing when `zdr`.
fn endpoint(slug: &str, quantization: &str, zdr: bool) -> CatalogEndpoint {
    CatalogEndpoint {
        provider: slug.to_owned(),
        quantization: Some(quantization.to_owned()),
        context: None,
        max_output: None,
        price: Some(TokenPrice {
            input: 100_000,
            output: 400_000,
            cache_read: None,
        }),
        zdr,
        speed: None,
    }
}

/// DeepSeek V4.1 Flash, as a router's catalog says who serves it: from China, the US twice,
/// Germany, and a provider whose home it does not say.
fn flash() -> Router {
    Router {
        providers: vec![
            provider("deepseek", Some("CN")),
            provider("fireworks", Some("US")),
            provider("groq", Some("US")),
            provider("nebius", Some("DE")),
            provider("unsaid", None),
        ]
        .into(),
        endpoints: vec![
            endpoint("deepseek", "fp8", false),
            endpoint("fireworks", "fp8", true),
            endpoint("groq", "fp4", true),
            endpoint("nebius", "bf16", true),
            endpoint("unsaid", "bf16", true),
        ],
    }
}

fn based_in(places: &[&str]) -> Filters {
    Filters {
        based_in: places.iter().map(|place| (*place).to_owned()).collect(),
        ..Filters::default()
    }
}

fn only(route: Result<Option<Route>, Kept>) -> Vec<String> {
    route.unwrap().unwrap().only.unwrap()
}

#[test]
fn no_filters_ask_nothing_and_only_an_order_asks_a_router_alone() {
    assert_eq!(
        route_at(&Filters::default(), [FLASH, FLASH], Some(&flash()), true),
        Ok(None)
    );
    let fastest = Filters {
        sort: Some(Sort::Speed),
        ..Filters::default()
    };
    let asked = route_at(&fastest, [FLASH, FLASH], Some(&flash()), true)
        .unwrap()
        .unwrap();
    assert_eq!(asked.sort, Some(Sort::Speed));
    assert_eq!(asked.only, None);
    assert_eq!(
        route_at(&fastest, [FLASH, FLASH], None, true),
        Ok(None),
        "not a router: nothing to ask"
    );
}

#[test]
fn where_no_router_serves_the_model_filters_leave_it_alone() {
    assert_eq!(
        route_at(&based_in(&["US"]), [FLASH, FLASH], None, false),
        Ok(None)
    );
}

#[test]
fn a_provider_chosen_under_the_router_s_own_name_still_counts() {
    let mut chosen = Filters::default();
    chosen
        .serves
        .insert(FLASH.into(), Serve::Provider("groq".into()));
    let asked = route_at(
        &chosen,
        ["deepseek-v4-1-flash", FLASH],
        Some(&flash()),
        true,
    );
    assert_eq!(asked.unwrap().unwrap().only, Some(vec!["groq".to_owned()]));
    let mut avoided = chosen.clone();
    avoided.avoid = vec!["groq".into()];
    assert_eq!(
        route_at(&avoided, [FLASH, FLASH], Some(&Router::default()), true),
        Err(Kept::NoneFits),
        "chosen and avoided at once: none"
    );
}

#[test]
fn filters_that_narrow_keep_a_call_from_any_target_but_a_router() {
    assert_eq!(
        route_at(&based_in(&["US"]), [FLASH, FLASH], None, true),
        Err(Kept::NotARouter)
    );
    let keeps = Filters {
        keeps_nothing: true,
        ..Filters::default()
    };
    assert_eq!(
        route_at(&keeps, [FLASH, FLASH], None, true),
        Err(Kept::NotARouter)
    );
}

#[test]
fn based_in_keeps_providers_whose_home_and_datacenters_are_there() {
    let us = based_in(&["US"]);
    assert_eq!(
        only(route_at(&us, [FLASH, FLASH], Some(&flash()), true)),
        ["fireworks", "groq"]
    );
    let mut abroad = flash();
    let mut providers = abroad.providers.to_vec();
    if let Some(groq) = providers.get_mut(2) {
        groq.datacenters = Some(vec!["US".into(), "SG".into()]);
    }
    abroad.providers = providers.into();
    assert_eq!(
        only(route_at(&us, [FLASH, FLASH], Some(&abroad), true)),
        ["fireworks"],
        "a datacenter abroad"
    );
    assert_eq!(
        only(route_at(
            &based_in(&["US", "DE"]),
            [FLASH, FLASH],
            Some(&flash()),
            true
        )),
        ["fireworks", "groq", "nebius"],
        "a provider whose home isn't said is never shown to be anywhere"
    );
    assert_eq!(
        route_at(&based_in(&["FR"]), [FLASH, FLASH], Some(&flash()), true),
        Err(Kept::NoneFits)
    );
}

#[test]
fn where_the_catalog_lists_no_endpoints_its_providers_still_say_where_they_are() {
    let providers_only = Router {
        endpoints: Vec::new(),
        ..flash()
    };
    assert_eq!(
        only(route_at(
            &based_in(&["US"]),
            [FLASH, FLASH],
            Some(&providers_only),
            true
        )),
        ["fireworks", "groq"]
    );
    let unknown = Router::default();
    assert_eq!(
        route_at(&based_in(&["US"]), [FLASH, FLASH], Some(&unknown), true),
        Err(Kept::Unknown),
        "never sent past a place glave can't check"
    );
    let keeps = Filters {
        keeps_nothing: true,
        ..Filters::default()
    };
    let asked = route_at(&keeps, [FLASH, FLASH], Some(&unknown), true)
        .unwrap()
        .unwrap();
    assert!(
        asked.zdr && asked.only.is_none(),
        "the router keeps to it itself"
    );
}

#[test]
fn precision_keeping_nothing_price_and_avoiding_go_as_the_router_takes_them() {
    let strict = Filters {
        precision: Some(Precision::EightBit),
        keeps_nothing: true,
        max_price: Some(MaxPrice {
            input: Some(100_000),
            output: None,
        }),
        avoid: vec!["nebius".into()],
        ..Filters::default()
    };
    let asked = route_at(&strict, [FLASH, FLASH], Some(&flash()), true)
        .unwrap()
        .unwrap();
    assert_eq!(asked.only, None, "the router checks these itself");
    assert_eq!(asked.ignore, ["nebius"]);
    assert_eq!(
        asked.quantizations,
        ["int8", "fp8", "mxfp8", "fp16", "bf16", "fp32"]
    );
    assert!(asked.zdr);
    let dearer = Filters {
        max_price: Some(MaxPrice {
            input: Some(99_999),
            output: None,
        }),
        ..Filters::default()
    };
    assert_eq!(
        route_at(&dearer, [FLASH, FLASH], Some(&flash()), true),
        Err(Kept::NoneFits),
        "refused before it is sent where the catalog shows none fits"
    );
}

#[test]
fn a_provider_chosen_for_a_model_serves_it_alone_and_only_within_the_filters() {
    let mut chosen = Filters {
        sort: Some(Sort::Speed),
        ..Filters::default()
    };
    chosen
        .serves
        .insert(FLASH.into(), Serve::Provider("deepseek".into()));
    let asked = route_at(&chosen, [FLASH, FLASH], Some(&flash()), true)
        .unwrap()
        .unwrap();
    assert_eq!(asked.only, Some(vec!["deepseek".to_owned()]));
    assert_eq!(asked.sort, None, "the one chosen, not the fastest");
    assert_eq!(
        only(route_at(
            &chosen,
            [FLASH, FLASH],
            Some(&Router::default()),
            true
        )),
        ["deepseek"],
        "one the catalog doesn't list yet is the router's to find"
    );
    chosen.based_in = vec!["US".into()];
    assert_eq!(
        route_at(&chosen, [FLASH, FLASH], Some(&flash()), true),
        Err(Kept::NoneFits),
        "hidden by the filters: refused, never sent elsewhere"
    );
    let mut fastest_here = Filters::default();
    fastest_here
        .serves
        .insert(FLASH.into(), Serve::Sort(Sort::Speed));
    let asked = route_at(&fastest_here, [FLASH, FLASH], Some(&flash()), true)
        .unwrap()
        .unwrap();
    assert_eq!(asked.sort, Some(Sort::Speed));
}

#[test]
fn each_filter_that_hides_a_provider_is_named() {
    let strict = Filters {
        based_in: vec!["US".into()],
        precision: Some(Precision::Full),
        keeps_nothing: true,
        max_price: Some(MaxPrice {
            input: None,
            output: Some(1),
        }),
        avoid: vec!["deepseek".into()],
        ..Filters::default()
    };
    let serving = flash();
    let hides = |index: usize| {
        let endpoint = serving.endpoints.get(index).unwrap();
        hidden_by(&facts(endpoint, &serving), &strict)
    };
    assert_eq!(
        hides(0),
        [
            Hides::BasedIn,
            Hides::Precision,
            Hides::KeepsSome,
            Hides::Price,
            Hides::Avoided
        ]
    );
    assert_eq!(hides(1), [Hides::Precision, Hides::Price]);
    let endpoint = serving.endpoints.get(3).unwrap();
    assert_eq!(
        hidden_by(&facts(endpoint, &serving), &Filters::default()),
        []
    );
}
