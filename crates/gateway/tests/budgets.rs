//! A signed-in caller's budget, reserve then settle: a call that could exceed it is refused
//! before it is sent, naming whose it is; a call that ends settles to what it cost; one that
//! fails spends nothing; a target the budget cannot afford is skipped; calls side by side never
//! overshoot; and a call for no one signed in counts in none.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use glave_event::{Currency, Money, PrincipalId};
use glave_gateway::{BudgetKey, Budgets, Gateway, Price, Routes, Target};
use glave_ports::{Failure, ModelProvider, ModelRequest, ProviderError, Purpose};
use jiff::Timestamp;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::support::{Fake, answer, block_on, request};

/// The most the test request could cost at a millionth of a dollar a token: 14 bytes of
/// input at 3 bytes a token, rounded up, and its 100 tokens of answer.
const WORST: u64 = 5 + 100;

/// A dollar a million tokens, in and out: a millionth of a dollar a token.
fn price(per_mtok: u64) -> Price {
    Price {
        list: "test".into(),
        currency: Currency::USD,
        input_per_mtok: per_mtok,
        cache_read_per_mtok: 0,
        output_per_mtok: per_mtok,
    }
}

fn dollars(micros: i64) -> Money {
    Money {
        micros,
        currency: Currency::USD,
    }
}

fn caller(n: u8) -> BudgetKey {
    BudgetKey {
        person: PrincipalId::new(Timestamp::from_second(i64::from(n)).unwrap(), [n; 10]).unwrap(),
        period: 1,
    }
}

/// A gateway over `targets`, each person held to `limit` a period.
fn gateway(targets: Vec<Target>, limit: i64) -> Gateway {
    Gateway::new(Routes::new(targets).unwrap()).with_budgets(Budgets {
        person: Some(dollars(limit)),
    })
}

fn complete_for(
    gateway: &Gateway,
    caller: BudgetKey,
    asked: &ModelRequest,
) -> Result<glave_ports::Completion, ProviderError> {
    block_on!(gateway.complete_for(caller, None, asked, &mut |_| {}))
}

#[test]
fn a_call_that_could_exceed_its_caller_s_budget_is_refused_before_it_is_sent() {
    let fake = Fake::new("p");
    let gateway = gateway(vec![fake.target("m").with_price(price(1_000_000))], 200);
    let (alice, bob) = (caller(1), caller(2));
    // The fake reports 100 tokens in and 10 out: 110 millionths.
    complete_for(&gateway, alice, &request(Purpose::Main)).unwrap();
    assert_eq!(gateway.spent(alice), 110);
    let refused = complete_for(&gateway, alice, &request(Purpose::Main)).unwrap_err();
    assert_eq!(refused.failure(), Failure::OverBudget);
    let ProviderError::OverBudget { budget, detail } = &refused else {
        panic!("{refused}")
    };
    assert_eq!(budget, &format!("person {}", alice.person));
    assert!(detail.contains("110 of 200 millionths of USD"), "{detail}");
    assert!(detail.contains(&format!("{WORST} more")), "{detail}");
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        1,
        "the second was not sent"
    );
    assert!(
        complete_for(&gateway, bob, &request(Purpose::Main)).is_ok(),
        "each caller has their own"
    );
    assert!(
        block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).is_ok(),
        "a call for no one signed in counts in none"
    );
}

#[test]
fn a_failed_call_spends_nothing_and_releases_its_hold() {
    let fake = Fake::failing("p", ProviderError::Protocol("garbled".into()));
    let target = fake.target("m").with_price(price(1_000_000));
    let gateway = gateway(vec![target], i64::try_from(WORST).unwrap());
    let alice = caller(1);
    assert!(complete_for(&gateway, alice, &request(Purpose::Main)).is_err());
    assert_eq!(gateway.spent(alice), 0);
    assert!(
        complete_for(&gateway, alice, &request(Purpose::Main)).is_ok(),
        "the whole budget is free again"
    );
}

#[test]
fn a_budget_skips_a_target_it_cannot_afford() {
    let fake = Fake::new("p");
    // At $10 a million tokens the worst case is 1,050 millionths; at $1, 105.
    let targets = vec![
        fake.target("dear").with_price(price(10_000_000)),
        fake.target("cheap").with_price(price(1_000_000)),
    ];
    let gateway = gateway(targets, 1_000);
    let alice = caller(1);
    let answer = complete_for(&gateway, alice, &request(Purpose::Main)).unwrap();
    assert_eq!(answer.served.unwrap().target, "p/cheap");
    assert_eq!(gateway.spent(alice), 110);
}

#[test]
fn calls_side_by_side_never_overshoot() {
    let gate = Arc::new(Semaphore::new(0));
    let fake = Fake::gated("p", Arc::clone(&gate));
    for _ in 0..3 {
        fake.script
            .lock()
            .unwrap()
            .push_back(Ok(answer("p", "m", 5, 50)));
    }
    let limit = 3 * WORST + WORST / 2;
    let target = fake.target("m").with_price(price(1_000_000));
    let gateway = Arc::new(gateway(vec![target], i64::try_from(limit).unwrap()));
    let alice = caller(1);
    block_on!(async {
        let mut calls = JoinSet::new();
        for _ in 0..8 {
            let gateway = Arc::clone(&gateway);
            calls.spawn(async move {
                let asked = request(Purpose::Main);
                gateway.complete_for(alice, None, &asked, &mut |_| {}).await
            });
        }
        let mut refused = 0;
        while refused < 5 {
            let done = calls.join_next().await.unwrap().unwrap();
            assert_eq!(done.unwrap_err().failure(), Failure::OverBudget);
            refused += 1;
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
    assert_eq!(gateway.spent(alice), 3 * 55);
    assert!(gateway.spent(alice) <= limit);
}
