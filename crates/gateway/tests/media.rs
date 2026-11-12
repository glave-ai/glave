//! A media tool's calls (`specs/gateway.md`, "Media tools"): served only by a route of their
//! own, never `main`'s; sent to that route's target as what it makes; and, where credits are
//! kept, held at the tokens the call allows and charged as `media`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use glave_event::{Body, CostKind, Principal, PrincipalId};
use glave_gateway::{Credits, Gateway, Routes};
use glave_ports::testing::{CountingEntropy, FaultyLog, ManualClock};
use glave_ports::{
    EventLog as _, MediaAsk, ModelProvider, OrgId, Payer, ProviderError, Purpose, Scope,
};
use jiff::Timestamp;

use crate::support::{Fake, block_on, dollars, price, request};

fn person() -> PrincipalId {
    PrincipalId::new(Timestamp::from_second(1).unwrap(), [1; 10]).unwrap()
}

fn org() -> OrgId {
    OrgId::from_uuid(
        PrincipalId::new(Timestamp::from_second(2).unwrap(), [2; 10])
            .unwrap()
            .as_uuid(),
    )
}

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
