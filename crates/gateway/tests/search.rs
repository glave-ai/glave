//! Web search through the gateway (`specs/gateway.md`, "Web search"): served only where the
//! server has a search service, and, where credits are kept, each search held at the service's
//! price for one and charged as `search`; refused with a sentence when the balance can't pay.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use glave_event::{Body, CostKind, Principal};
use glave_gateway::{Credits, Gateway, Routes, WebSearch};
use glave_ports::testing::{CountingEntropy, FaultyLog, ManualClock};
use glave_ports::{
    EventLog as _, ModelProvider, Payer, ProviderError, Purpose, Scope, SearchAsk, Unpaid,
};

use crate::support::{Fake, block_on, dollars, org, person, request};

/// What one search costs in the tests: half a cent.
const PRICE: i64 = 5_000_000;

fn ask() -> SearchAsk {
    SearchAsk {
        query: "rust async runtimes".into(),
        count: 5,
    }
}

/// A search request whose work is the test person's.
fn paid() -> glave_ports::ModelRequest {
    let mut asked = request(Purpose::WebSearch);
    asked.payer = Some(Payer {
        org: org(),
        person: Some(person()),
    });
    asked
}

/// A gateway searching through `fake` at [`PRICE`] a search.
fn searching(fake: &Arc<Fake>) -> Gateway {
    let routes = Routes::new(vec![Fake::new("chat").target("m")]).unwrap();
    Gateway::new(routes).with_search(Some(WebSearch {
        handle: Arc::clone(fake) as Arc<dyn glave_gateway::Model>,
        price: dollars(PRICE),
    }))
}

/// Credits on `log`, with `nanos` added for the test's organization.
fn funded(log: &FaultyLog, nanos: i64) -> Arc<Credits<FaultyLog>> {
    let credits = Arc::new(Credits::new(
        log.clone(),
        Box::new(ManualClock::ticking(1_793_350_800_000)),
        Box::new(CountingEntropy::starting_at(1)),
        None,
    ));
    if nanos > 0 {
        let by = Principal::person(person());
        block_on!(credits.add(org(), dollars(nanos), "order-1", by)).unwrap();
    }
    credits
}

fn balance(credits: &Credits<FaultyLog>) -> i64 {
    block_on!(credits.account(org()))
        .unwrap()
        .balance
        .nanos
        .get()
}

/// Each charge's kind and amount in `log`'s credits ledger, in order.
fn charges(log: &FaultyLog) -> Vec<(CostKind, i64)> {
    let scope = Scope::of_organization(org());
    let ledger = glave_ports::stream::Reserved::Credits.id();
    let events = block_on!(log.read(&scope, ledger, None, 100)).unwrap();
    events
        .into_iter()
        .filter_map(|raw| match glave_event::decode(raw).unwrap().body {
            Body::CreditsCharged(charged) => Some((charged.kind, charged.amount.nanos.get())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_search_is_served_only_where_the_server_has_a_search_service() {
    let chat = Fake::new("chat");
    let without = Gateway::new(Routes::new(vec![chat.target("m")]).unwrap());
    assert!(!without.serves(Purpose::WebSearch), "never main's route");
    let refused = block_on!(without.search(&request(Purpose::WebSearch), &ask())).unwrap_err();
    assert!(
        matches!(&refused, ProviderError::Unserved { reason, .. } if reason.contains("web search")),
        "{refused:?}"
    );
    assert_eq!(chat.calls.load(Ordering::SeqCst), 0);

    let service = Fake::new("search");
    let with = searching(&service);
    assert!(with.serves(Purpose::WebSearch) && with.serves(Purpose::Main));
    let found = block_on!(with.search(&request(Purpose::WebSearch), &ask())).unwrap();
    assert_eq!(found.hits[0].title, "rust async runtimes");
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_search_is_held_at_its_price_charged_as_search_and_refused_when_nothing_is_left() {
    let log = FaultyLog::default();
    let credits = funded(&log, 0);
    let service = Fake::new("search");
    let gateway = searching(&service).with_credits(Arc::clone(&credits));
    let out = block_on!(gateway.search(&paid(), &ask())).unwrap_err();
    assert!(
        matches!(out, ProviderError::Unpaid(Unpaid::Spent)),
        "out of credits: {out:?}"
    );
    assert!(out.to_string().contains("balance is used up"), "{out}");
    assert_eq!(service.calls.load(Ordering::SeqCst), 0, "never sent");

    let by = Principal::person(person());
    block_on!(credits.add(org(), dollars(PRICE * 3), "order-2", by)).unwrap();
    block_on!(gateway.search(&paid(), &ask())).unwrap();
    assert_eq!(balance(&credits), PRICE * 2);
    assert_eq!(charges(&log), [(CostKind::Search, PRICE)]);

    // Where no one says whose work it is, nothing is held or charged.
    block_on!(gateway.search(&request(Purpose::WebSearch), &ask())).unwrap();
    assert_eq!(balance(&credits), PRICE * 2);
}

#[test]
fn a_search_refused_is_charged_nothing_and_one_left_unanswered_its_price() {
    let log = FaultyLog::default();
    let credits = funded(&log, PRICE * 4);
    let limited = ProviderError::Http {
        status: 429,
        body: "rate limited".into(),
    };
    let gateway = searching(&Fake::failing("search", limited)).with_credits(Arc::clone(&credits));
    assert!(block_on!(gateway.search(&paid(), &ask())).is_err());
    assert_eq!(
        balance(&credits),
        PRICE * 4,
        "a refused search is not billed"
    );
    let silent = ProviderError::Timeout { seconds: 30 };
    let gateway = searching(&Fake::failing("search", silent)).with_credits(Arc::clone(&credits));
    assert!(block_on!(gateway.search(&paid(), &ask())).is_err());
    assert_eq!(balance(&credits), PRICE * 3, "it may have been billed");
}
