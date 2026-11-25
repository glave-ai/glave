//! A media tool's calls (`specs/gateway.md`, "Media tools"): served only by a route of their
//! own, never `main`'s; sent to that route's target as what it makes; and, where credits are
//! kept, held at the tokens the call allows and charged as `media`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use glave_event::{Body, CostKind, Principal};
use glave_gateway::{Credits, Gateway, Routes};
use glave_ports::testing::{CountingEntropy, FaultyLog, ManualClock};
use glave_ports::{EventLog as _, MediaAsk, ModelProvider, Payer, ProviderError, Purpose, Scope};

use crate::support::{
    Fake, answer, balance, block_on, cancelled_once_sent, dollars, funded, org, person, price,
    request,
};

#[test]
fn a_media_call_goes_only_to_its_own_route_and_comes_back_with_what_it_made() {
    let chat = Fake::new("chat");
    let images = Fake::new("images");
    let routes = Routes::new(vec![chat.target("m")]).unwrap();
    let without = Gateway::new(routes.clone());
    assert!(without.serves(Purpose::Main) && without.serves(Purpose::Title));
    assert!(
        !without.serves(Purpose::ImageGenerate),
        "never main's route"
    );
    let refused = block_on!(without.make(&request(Purpose::ImageGenerate), &MediaAsk::default()));
    assert!(
        matches!(&refused, Err(ProviderError::Unserved { reason, .. }) if reason.contains("image_generate")),
        "{refused:?}"
    );
    assert_eq!(chat.calls.load(Ordering::SeqCst), 0);

    let routes = routes
        .with_route(Purpose::ImageGenerate, vec![images.target("painter")])
        .unwrap();
    let with = Gateway::new(routes);
    assert!(with.serves(Purpose::ImageGenerate) && !with.serves(Purpose::ImageEdit));
    let made = block_on!(with.make(&request(Purpose::ImageGenerate), &MediaAsk::default()));
    let made = made.unwrap();
    assert_eq!(made.files.len(), 1);
    let served = made.completion.served.unwrap();
    assert_eq!(
        (served.route.as_str(), served.target.as_str()),
        ("image_generate", "images/painter")
    );
    assert_eq!(
        images.seen()[0].model,
        "painter",
        "sent as its target names it"
    );
    assert_eq!(chat.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_media_call_is_held_at_what_it_allows_and_charged_as_media() {
    let log = FaultyLog::default();
    let credits = Arc::new(Credits::new(
        log.clone(),
        Box::new(ManualClock::ticking(1_793_350_800_000)),
        Box::new(CountingEntropy::starting_at(1)),
        None,
    ));
    let images = Fake::new("images");
    let painter = images.target("painter").with_price(price(1_000_000));
    let routes = Routes::new(vec![Fake::new("chat").target("m")])
        .unwrap()
        .with_route(Purpose::ImageGenerate, vec![painter])
        .unwrap();
    let gateway = Gateway::new(routes).with_credits(Arc::clone(&credits));
    let by = Principal::person(person());
    block_on!(credits.add(org(), dollars(104), "order-1", by)).unwrap();
    let mut asked = request(Purpose::ImageGenerate);
    asked.payer = Some(Payer {
        org: org(),
        person: Some(person()),
    });
    let short = block_on!(gateway.make(&asked, &MediaAsk::default())).unwrap_err();
    assert!(
        matches!(short, ProviderError::Unpaid(_)),
        "held at its 100 tokens of answer and what it reads, more than is free: {short:?}"
    );
    block_on!(credits.add(
        org(),
        dollars(1_000),
        "order-2",
        Principal::person(person())
    ))
    .unwrap();
    block_on!(gateway.make(&asked, &MediaAsk::default())).unwrap();
    let scope = Scope::of_organization(org());
    let ledger = glave_ports::stream::Reserved::Credits.id();
    let events = block_on!(log.read(&scope, ledger, None, 100)).unwrap();
    let kinds: Vec<CostKind> = events
        .into_iter()
        .filter_map(|raw| match glave_event::decode(raw).unwrap().body {
            Body::CreditsCharged(charged) => Some(charged.kind),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, [CostKind::Media]);
}

/// A gateway making images at `fake`'s `painter`, a billionth of a dollar a token, charging
/// `credits`.
fn painting(fake: &Arc<Fake>, credits: &Arc<Credits<FaultyLog>>) -> Gateway {
    let painter = fake.target("painter").with_price(price(1_000_000));
    let routes = Routes::new(vec![Fake::new("chat").target("m")])
        .unwrap()
        .with_route(Purpose::ImageGenerate, vec![painter])
        .unwrap();
    Gateway::new(routes).with_credits(Arc::clone(credits))
}

fn paid() -> glave_ports::ModelRequest {
    let mut asked = request(Purpose::ImageGenerate);
    asked.payer = Some(Payer {
        org: org(),
        person: Some(person()),
    });
    asked
}

/// What the test request holds at a billionth of a dollar a token: what it reads and its 100
/// tokens of images.
const HELD: i64 = 5 + 100;

#[test]
fn a_media_call_left_unanswered_is_charged_its_hold_and_one_refused_nothing() {
    let credits = funded(1_000);
    let silent = ProviderError::Timeout { seconds: 300 };
    let gateway = painting(&Fake::failing("images", silent), &credits);
    assert!(block_on!(gateway.make(&paid(), &MediaAsk::default())).is_err());
    assert_eq!(
        balance(&credits),
        1_000 - HELD,
        "billed whether or not it was read"
    );
    let refused = ProviderError::Http {
        status: 502,
        body: "Generation failed".into(),
    };
    let gateway = painting(&Fake::failing("images", refused), &credits);
    assert!(block_on!(gateway.make(&paid(), &MediaAsk::default())).is_err());
    assert_eq!(
        balance(&credits),
        1_000 - HELD,
        "no failed generation is billed"
    );
}

#[test]
fn a_media_answer_that_says_nothing_of_what_it_used_is_charged_its_hold() {
    let credits = funded(1_000);
    let fake = Fake::new("images");
    fake.script
        .lock()
        .unwrap()
        .push_back(Ok(answer("images", "painter", 0, 0)));
    block_on!(painting(&fake, &credits).make(&paid(), &MediaAsk::default())).unwrap();
    assert_eq!(balance(&credits), 1_000 - HELD, "never nothing");
}

#[test]
fn a_media_call_that_never_reached_its_provider_is_charged_nothing_and_its_fallback_once() {
    let credits = funded(1_000);
    let unreached = Fake::failing(
        "down",
        ProviderError::Unserved {
            failure: glave_ports::Failure::Unreachable,
            reason: "connection refused".into(),
        },
    );
    let painters = vec![
        unreached.target("painter").with_price(price(1_000_000)),
        Fake::new("up")
            .target("painter")
            .with_price(price(1_000_000)),
    ];
    let routes = Routes::new(vec![Fake::new("chat").target("m")])
        .unwrap()
        .with_route(Purpose::ImageGenerate, painters)
        .unwrap();
    let gateway = Gateway::new(routes).with_credits(Arc::clone(&credits));
    block_on!(gateway.make(&paid(), &MediaAsk::default())).unwrap();
    assert_eq!(
        balance(&credits),
        1_000 - 110,
        "the fake's 110 tokens, once"
    );
}

#[test]
fn a_media_call_dropped_once_sent_is_charged_its_hold() {
    let credits = funded(1_000);
    let fake = Fake::gated("images", Arc::new(tokio::sync::Semaphore::new(0)));
    let gateway = Arc::new(painting(&fake, &credits));
    block_on!(async {
        let making = Arc::clone(&gateway);
        let call = tokio::spawn(async move { making.make(&paid(), &MediaAsk::default()).await });
        cancelled_once_sent(&fake, call).await;
    });
    assert_eq!(balance(&credits), 1_000 - HELD);
}

#[test]
fn where_credits_are_kept_a_media_tool_is_served_only_when_a_model_of_it_is_priced() {
    let credits = funded(1_000);
    let unpriced = Routes::new(vec![Fake::new("chat").target("m")])
        .unwrap()
        .with_route(
            Purpose::ImageGenerate,
            vec![Fake::new("images").target("painter")],
        )
        .unwrap();
    assert!(Gateway::new(unpriced.clone()).serves(Purpose::ImageGenerate));
    let paying = Gateway::new(unpriced).with_credits(Arc::clone(&credits));
    assert!(
        !paying.serves(Purpose::ImageGenerate),
        "it would refuse every call"
    );
    assert!(paying.serves(Purpose::Main));
    let priced = painting(&Fake::new("images"), &credits);
    assert!(priced.serves(Purpose::ImageGenerate));
}
