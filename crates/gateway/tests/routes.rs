//! Routes by purpose: each purpose goes to its own route, or `main`'s when it has none, under
//! the target's model name, and every answer says which route and target served it.

use glave_event::{Currency, Money, Nanos};
use glave_gateway::{Gateway, Price, RouteError, Routes};
use glave_ports::{ModelProvider, Purpose, Served};

use crate::support::{Fake, block_on, request};

#[test]
fn a_purpose_without_a_route_uses_main() {
    let main = Fake::new("openrouter");
    let gateway = Gateway::new(Routes::new(vec![main.target("big")]).unwrap());
    for purpose in [
        Purpose::Main,
        Purpose::Judge,
        Purpose::Summary,
        Purpose::Subagent,
    ] {
        let answer = block_on!(gateway.complete(&request(purpose), &mut |_| {})).unwrap();
        assert_eq!(
            answer.served,
            Some(Served {
                route: purpose.id().into(),
                target: "openrouter/big".into(),
                service: None,
                effort: None,
                reasoning_stripped: false,
            })
        );
    }
    let models: Vec<String> = main.seen().into_iter().map(|r| r.model).collect();
    assert_eq!(
        models, ["big"; 4],
        "the target's model, not the one asked for"
    );
}

#[test]
fn a_purpose_with_a_route_uses_its_own_targets() {
    let provider = Fake::new("openrouter");
    let routes = Routes::new(vec![provider.target("big")])
        .unwrap()
        .with_route(
            Purpose::Judge,
            vec![provider.target("small").named("cheap judge")],
        )
        .unwrap();
    let gateway = Gateway::new(routes);
    let judged = block_on!(gateway.complete(&request(Purpose::Judge), &mut |_| {})).unwrap();
    let main = block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).unwrap();
    assert_eq!(judged.served.unwrap().target, "cheap judge");
    assert_eq!(main.served.unwrap().target, "openrouter/big");
    let models: Vec<String> = provider.seen().into_iter().map(|r| r.model).collect();
    assert_eq!(models, ["small", "big"]);
}

#[test]
fn a_chosen_model_replaces_main_but_never_a_purpose_with_its_own_route() {
    let provider = Fake::new("openrouter");
    let routes = Routes::new(vec![provider.target("big"), provider.target("backup")])
        .unwrap()
        .with_route(Purpose::Judge, vec![provider.target("small")])
        .unwrap()
        .with_choices(vec![provider.target("other")]);
    let gateway = Gateway::new(routes);
    let asking = |purpose, model: &str| {
        let mut asked = request(purpose);
        asked.model = model.into();
        block_on!(gateway.complete(&asked, &mut |_| {}))
            .unwrap()
            .served
            .unwrap()
            .target
    };
    assert_eq!(asking(Purpose::Main, "other"), "openrouter/other");
    assert_eq!(
        asking(Purpose::Subagent, "openrouter/other"),
        "openrouter/other"
    );
    assert_eq!(asking(Purpose::Summary, "other"), "openrouter/other");
    assert_eq!(asking(Purpose::Judge, "other"), "openrouter/small");
    // The default, and a model that is not a choice, keep the route as it is.
    assert_eq!(asking(Purpose::Main, "big"), "openrouter/big");
    assert_eq!(asking(Purpose::Main, "unknown"), "openrouter/big");
    let models: Vec<String> = provider.seen().into_iter().map(|r| r.model).collect();
    assert_eq!(models, ["other", "other", "other", "small", "big", "big"]);
}

#[test]
fn a_chosen_model_served_at_several_providers_falls_back_among_them_in_order() {
    let main = Fake::new("openrouter");
    let overloaded = glave_ports::ProviderError::Http {
        status: 529,
        body: "overloaded".into(),
    };
    let maker = Fake::failing("anthropic", overloaded.clone());
    maker.script.lock().unwrap().push_back(Err(overloaded));
    let routes = Routes::new(vec![main.target("big")])
        .unwrap()
        .with_choices(vec![
            maker.target("claude-haiku-5-5").serving("claude-haiku-5-5"),
            main.target("other"),
            main.target("anthropic/claude-haiku-5.5")
                .serving("claude-haiku-5-5"),
        ]);
    let gateway = Gateway::new(routes);
    for named in ["claude-haiku-5-5", "anthropic/claude-haiku-5.5"] {
        let mut asked = request(Purpose::Main);
        asked.model = named.into();
        let answer = block_on!(gateway.complete(&asked, &mut |_| {})).unwrap();
        assert_eq!(
            answer.served.unwrap().target,
            "openrouter/anthropic/claude-haiku-5.5",
            "the maker's own API first, then the aggregator: {named}"
        );
    }
    assert_eq!(maker.seen().len(), 2);
}

#[test]
fn a_route_needs_a_target() {
    assert_eq!(
        Routes::new(Vec::new()).unwrap_err(),
        RouteError::Empty("main")
    );
    let main = Fake::new("p");
    let empty = Routes::new(vec![main.target("m")])
        .unwrap()
        .with_route(Purpose::Summary, Vec::new());
    assert_eq!(empty.unwrap_err(), RouteError::Empty("summary"));
}

#[test]
fn a_configured_price_costs_an_answer_the_provider_did_not() {
    let provider = Fake::new("vllm");
    let price = Price {
        list: "ours-2026-10".into(),
        currency: Currency::USD,
        input_per_mtok: Nanos::new(1_000_000_000),
        cache_read_per_mtok: Nanos::ZERO,
        output_per_mtok: Nanos::new(2_000_000_000),
    };
    let gateway =
        Gateway::new(Routes::new(vec![provider.target("hope").with_price(price)]).unwrap());
    let answer = block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).unwrap();
    let cost = answer.cost.unwrap();
    // 100 in at $1 and 10 out at $2, per million.
    assert_eq!(
        cost.amount,
        Money {
            nanos: Nanos::new(120_000),
            currency: Currency::USD
        }
    );
    assert_eq!(cost.price_list, "ours-2026-10");
}

#[test]
fn an_answer_is_never_asked_longer_than_its_target_gives() {
    let provider = Fake::new("openrouter");
    let mut short = provider.target("short");
    short.can.max_output_tokens = Some(64);
    let mut long = provider.target("long");
    long.can.max_output_tokens = Some(393_000);
    let routes = Routes::new(vec![short])
        .unwrap()
        .with_route(Purpose::Judge, vec![long])
        .unwrap();
    let gateway = Gateway::new(routes);
    let mut unlimited = request(Purpose::Main);
    unlimited.max_output_tokens = None;
    for asked in [request(Purpose::Main), request(Purpose::Judge), unlimited] {
        block_on!(gateway.complete(&asked, &mut |_| {})).unwrap();
    }
    let sent: Vec<Option<u32>> = (provider.seen().into_iter())
        .map(|sent| sent.max_output_tokens)
        .collect();
    assert_eq!(
        sent,
        [Some(64), Some(100), None],
        "capped at what the catalog says the model gives; never raised, nor set unasked"
    );
}

/// What a turn asking for 32,000 tokens, and a judge asking for 100, are sent asking for at a
/// target that knows `context` of its model, and nothing else.
fn sent_with_window(context: Option<u64>) -> Vec<Option<u32>> {
    let provider = Fake::new("vllm");
    let mut target = provider.target("local");
    target.can.context_tokens = context;
    let gateway = Gateway::new(Routes::new(vec![target]).unwrap());
    let mut turn = request(Purpose::Main);
    turn.max_output_tokens = Some(32_000);
    for asked in [turn, request(Purpose::Judge)] {
        block_on!(gateway.complete(&asked, &mut |_| {})).unwrap();
    }
    (provider.seen().into_iter())
        .map(|sent| sent.max_output_tokens)
        .collect()
}

#[test]
fn an_answer_fits_the_window_left_after_its_input() {
    // A 32k local model: the turn asks for what is left after its input ("Be brief." and
    // "hello", 14 bytes, about 5 tokens), so a server such as vLLM does not refuse it.
    assert_eq!(sent_with_window(Some(32_768)), [Some(32_000), Some(100)]);
    assert_eq!(sent_with_window(Some(8_192)), [Some(8_187), Some(100)]);
    // A window with less room than the least answer, estimated cautiously, still asks for it.
    assert_eq!(sent_with_window(Some(1_028)), [Some(1_024), Some(100)]);
    // Nothing known: a turn's own cap is not sent, as it may not fit; a judge's is.
    assert_eq!(sent_with_window(None), [None, Some(100)]);
}
