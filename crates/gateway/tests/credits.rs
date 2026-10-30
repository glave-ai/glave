//! An organization's credits, reserve then settle: a call that says whose work it is holds its
//! worst case against its organization's credits and is charged what it cost; one that could
//! overdraw the balance or pass the month's limit is refused before it is sent, saying which;
//! a model with no price is refused, and passed over for the next target that has one; a call
//! that breaks off after its answer began, or is cancelled, is charged for what it read and
//! what came; a charge the store refuses is still counted and written with the next; calls side by side never overdraw; another server on the
//! store, or this one after a restart, reads the same balance; and a call no organization
//! asked for is charged to no one.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use glave_event::{Principal, PrincipalId};
use glave_gateway::{Credits, Gateway, Routes, Target};
use glave_ports::testing::{CountingEntropy, FaultyLog, ManualClock};
use glave_ports::{
    Failure, ModelProvider, ModelRequest, OrgId, Payer, ProviderError, Purpose, Unpaid,
};
use jiff::Timestamp;
use tokio::sync::{Semaphore, oneshot};
use tokio::task::JoinSet;

use crate::support::{Fake, block_on, dollars, price, request};

/// 2026-10-30T09:00:00Z.
const NOW: i64 = 1_793_350_800_000;

/// The most the test request could cost at a millionth of a dollar a token: its input, and
/// its 100 tokens of answer.
const WORST: i64 = 5 + 100;

/// What the fake's answer, 100 tokens in and 10 out, costs at that price.
const COST: i64 = 110;

/// What a call cut short after "Twelve bytes" came costs at that price: its 14 bytes read at
/// 4 bytes a token, and the 12 that came: 4 tokens and 3.
const SO_FAR: i64 = 4 + 3;

fn org(n: u8) -> OrgId {
    OrgId::from_uuid(person(n.saturating_add(100)).as_uuid())
}

fn person(n: u8) -> PrincipalId {
    PrincipalId::new(Timestamp::from_second(i64::from(n)).unwrap(), [n; 10]).unwrap()
}

/// The credits on `log`, as one server reads them, each organization limited to `limit` a
/// month.
fn credits(log: &FaultyLog, limit: Option<i64>) -> Arc<Credits<FaultyLog>> {
    Arc::new(Credits::new(
        log.clone(),
        Box::new(ManualClock::ticking(NOW)),
        Box::new(CountingEntropy::starting_at(1)),
        limit.map(dollars),
    ))
}

/// A gateway over `fake`'s model `m` at a millionth of a dollar a token, charging `credits`.
fn gateway(fake: &Arc<Fake>, credits: &Arc<Credits<FaultyLog>>) -> Gateway {
    over(vec![fake.target("m").with_price(price(1_000_000))], credits)
}

fn over(targets: Vec<Target>, credits: &Arc<Credits<FaultyLog>>) -> Gateway {
    Gateway::new(Routes::new(targets).unwrap()).with_credits(Arc::clone(credits))
}

/// A fake that says "Twelve bytes" as its answer begins.
fn saying(fake: &mut Arc<Fake>) {
    Arc::get_mut(fake).unwrap().says = Some("Twelve bytes".into());
}

/// The test request, as `person`'s work in `org`.
fn paid(org: OrgId, person: PrincipalId) -> ModelRequest {
    ModelRequest {
        payer: Some(Payer {
            org,
            person: Some(person),
        }),
        ..request(Purpose::Main)
    }
}

fn add(credits: &Credits<FaultyLog>, org: OrgId, micros: i64, reference: &str) {
    let by = Principal::person(person(9));
    block_on!(credits.add(org, dollars(micros), reference, by)).unwrap();
}

fn balance(credits: &Credits<FaultyLog>, org: OrgId) -> i64 {
    block_on!(credits.account(org)).unwrap().balance.micros
}

fn ask(gateway: &Gateway, asked: &ModelRequest) -> Result<glave_ports::Completion, ProviderError> {
    block_on!(gateway.complete(asked, &mut |_| {}))
}

#[test]
fn a_call_is_charged_what_it_cost_and_one_that_could_overdraw_is_never_sent() {
    let log = FaultyLog::default();
    let credits = credits(&log, None);
    let fake = Fake::new("p");
    let gateway = gateway(&fake, &credits);
    let (acme, ada) = (org(1), person(1));
    let refused = ask(&gateway, &paid(acme, ada)).unwrap_err();
    assert_eq!(refused, ProviderError::Unpaid(Unpaid::Spent));
    assert_eq!(refused.failure(), Failure::OverBudget);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0, "nothing was sent");
    add(&credits, acme, WORST + COST - 1, "order-1");
    add(&credits, acme, WORST + COST - 1, "order-1");
    let by = Principal::person(person(9));
    let other = block_on!(credits.add(acme, dollars(5), "order-1", by));
    assert!(other.is_err(), "the same payment for another amount");
    ask(&gateway, &paid(acme, ada)).unwrap();
    let account = block_on!(credits.account(acme)).unwrap();
    assert_eq!(
        account.balance,
        dollars(WORST - 1),
        "each payment adds once"
    );
    assert_eq!(account.spent, dollars(COST));
    assert_eq!(account.people.get(&ada), Some(&dollars(COST)));
    assert_eq!(
        ask(&gateway, &paid(acme, ada)).unwrap_err(),
        ProviderError::Unpaid(Unpaid::Short {
            free: dollars(WORST - 1),
            needs: dollars(WORST)
        }),
        "something is left, but less than the call could cost"
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert!(
        ask(&gateway, &request(Purpose::Main)).is_ok(),
        "a call no organization asked for is charged to no one"
    );
    let plain = Gateway::new(Routes::new(vec![fake.target("m")]).unwrap());
    assert!(
        ask(&plain, &paid(acme, ada)).is_ok(),
        "a gateway that keeps no credits charges none"
    );
}

#[test]
fn a_model_with_no_price_is_refused_since_no_one_can_say_what_it_could_cost() {
    let log = FaultyLog::default();
    let credits = credits(&log, None);
    let fake = Fake::new("p");
    let gateway = over(vec![fake.target("m")], &credits);
    add(&credits, org(1), 1_000_000, "order-1");
    assert_eq!(
        ask(&gateway, &paid(org(1), person(1))).unwrap_err(),
        ProviderError::Unpaid(Unpaid::Unpriced { model: "m".into() })
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_target_with_no_price_is_passed_over_for_the_next_that_has_one() {
    let log = FaultyLog::default();
    let credits = credits(&log, None);
    let fake = Fake::new("p");
    let targets = vec![
        fake.target("free"),
        fake.target("m").with_price(price(1_000_000)),
    ];
    let gateway = over(targets, &credits);
    add(&credits, org(1), 1_000_000, "order-1");
    let served = ask(&gateway, &paid(org(1), person(1)))
        .unwrap()
        .served
        .unwrap();
    assert_eq!(served.target, "p/m");
    assert_eq!(balance(&credits, org(1)), 1_000_000 - COST);
}

#[test]
fn a_call_past_the_month_s_limit_is_refused_and_the_limit_set_counts() {
    let log = FaultyLog::default();
    let credits = credits(&log, Some(WORST));
    let fake = Fake::new("p");
    let gateway = gateway(&fake, &credits);
    let (acme, ada) = (org(1), person(1));
    add(&credits, acme, 1_000_000, "order-1");
    ask(&gateway, &paid(acme, ada)).unwrap();
    assert_eq!(
        ask(&gateway, &paid(acme, ada)).unwrap_err(),
        ProviderError::Unpaid(Unpaid::MonthLimit {
            limit: dollars(WORST)
        })
    );
    let raised = dollars(1_000);
    let by = Principal::person(person(9));
    let account = block_on!(credits.set_limit(acme, raised, by)).unwrap();
    assert_eq!(account.limit, Some(raised));
    ask(&gateway, &paid(acme, ada)).unwrap();
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_call_that_breaks_off_is_charged_for_what_it_read_and_what_came() {
    let log = FaultyLog::default();
    let credits = credits(&log, None);
    let broken = ProviderError::AfterOutput(Box::new(ProviderError::Protocol("cut".into())));
    let mut fake = Fake::new("p");
    saying(&mut fake);
    fake.script.lock().unwrap().push_back(Err(broken));
    let (acme, ada) = (org(1), person(1));
    add(&credits, acme, 1_000, "order-1");
    assert!(ask(&gateway(&fake, &credits), &paid(acme, ada)).is_err());
    assert_eq!(balance(&credits, acme), 1_000 - SO_FAR);
    let failing = Fake::failing("p", ProviderError::Protocol("garbled".into()));
    assert!(ask(&gateway(&failing, &credits), &paid(acme, ada)).is_err());
    assert_eq!(
        balance(&credits, acme),
        1_000 - SO_FAR,
        "one that failed before any answer came is charged nothing"
    );
}

#[test]
fn a_call_cancelled_after_its_answer_began_is_charged_for_what_came() {
    let log = FaultyLog::default();
    let credits = credits(&log, None);
    let gate = Arc::new(Semaphore::new(0));
    let mut fake = Fake::gated("p", gate);
    saying(&mut fake);
    let gateway = Arc::new(gateway(&fake, &credits));
    let (acme, ada) = (org(1), person(1));
    add(&credits, acme, 1_000, "order-1");
    block_on!(async {
        let (began, beginning) = oneshot::channel();
        let gateway = Arc::clone(&gateway);
        // The fake says its words, then waits at its gate, where the call is cancelled.
        let call = tokio::spawn(async move {
            let mut began = Some(began);
            let asked = paid(acme, ada);
            let mut heard = |_| {
                if let Some(began) = began.take() {
                    let _ = began.send(());
                }
            };
            gateway.complete(&asked, &mut heard).await
        });
        beginning.await.unwrap();
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
    });
    assert_eq!(balance(&credits, acme), 1_000 - SO_FAR, "counted at once");
    assert_eq!(
        balance(&credits_on(&log), acme),
        1_000,
        "not yet in the store: it is written with the next call"
    );
}

#[test]
fn a_charge_the_store_refuses_is_counted_and_written_with_the_next() {
    let log = FaultyLog::default();
    let credits = credits(&log, None);
    let gateway = gateway(&Fake::new("p"), &credits);
    let (acme, ada) = (org(1), person(1));
    add(&credits, acme, 1_000_000, "order-1");
    log.fail_with("credits.charged");
    ask(&gateway, &paid(acme, ada)).unwrap();
    assert_eq!(balance(&credits, acme), 1_000_000 - COST, "counted here");
    let later = credits_on(&log);
    assert_eq!(balance(&later, acme), 1_000_000, "the store refused it");
    ask(&gateway, &paid(acme, ada)).unwrap();
    assert_eq!(
        balance(&later, acme),
        1_000_000 - 2 * COST,
        "both written with the next call"
    );
}

/// The credits on `log` as another server, or this one after a restart, reads them.
fn credits_on(log: &FaultyLog) -> Arc<Credits<FaultyLog>> {
    credits(log, None)
}

#[test]
fn calls_side_by_side_never_overdraw() {
    let log = FaultyLog::default();
    let credits = credits(&log, None);
    let gate = Arc::new(Semaphore::new(0));
    let fake = Fake::gated_answering("p", Arc::clone(&gate), 3);
    let gateway = Arc::new(gateway(&fake, &credits));
    let (acme, ada) = (org(1), person(1));
    add(&credits, acme, 3 * WORST + WORST / 2, "order-1");
    block_on!(async {
        let mut calls = JoinSet::new();
        for _ in 0..8 {
            let gateway = Arc::clone(&gateway);
            calls.spawn(async move { gateway.complete(&paid(acme, ada), &mut |_| {}).await });
        }
        for _ in 0..5 {
            let done = calls.join_next().await.unwrap().unwrap();
            assert_eq!(done.unwrap_err().failure(), Failure::OverBudget);
        }
        assert_eq!(
            fake.calls.load(Ordering::SeqCst),
            3,
            "only three could be held"
        );
        gate.add_permits(3);
        while let Some(done) = calls.join_next().await {
            assert!(done.unwrap().is_ok());
        }
    });
    assert_eq!(balance(&credits, acme), 3 * WORST + WORST / 2 - 3 * 55);
}

#[test]
fn another_server_on_the_store_reads_the_same_credits_and_holds_after_its_charges() {
    let log = FaultyLog::default();
    let (here, there) = (credits(&log, None), credits_on(&log));
    let fake = Fake::new("p");
    let (acme, ada) = (org(1), person(1));
    add(&here, acme, WORST + COST, "order-1");
    add(&there, acme, WORST + COST, "order-1");
    ask(&gateway(&fake, &here), &paid(acme, ada)).unwrap();
    let read = block_on!(there.account(acme)).unwrap();
    assert_eq!(read.balance, dollars(WORST), "a restart reads it back");
    assert_eq!(read.people.get(&ada), Some(&dollars(COST)));
    ask(&gateway(&fake, &there), &paid(acme, ada)).unwrap();
    assert!(
        ask(&gateway(&fake, &here), &paid(acme, ada)).is_err(),
        "the charge there is read here before it holds"
    );
    assert_eq!(balance(&here, org(2)), 0, "each organization has its own");
}
