//! Budgets, reserve then settle: a call that could exceed a budget is refused before it is
//! sent, naming the budget; a call that ends settles to what it cost; a subagent spends from
//! its parent's budget too; and calls side by side never overshoot.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use glave_event::{Currency, Money, ThreadId};
use glave_gateway::{Budget, Budgets, Gateway, Limit, Price, Routes};
use glave_ports::{Failure, ModelProvider, ModelRequest, ProviderError, Purpose};
use jiff::Timestamp;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::support::{Fake, answer, block_on, request};

/// The most the test request could cost: 14 bytes of input at 3 bytes a token, rounded up,
/// and its 100 tokens of answer.
const WORST: u64 = 5 + 100;

fn thread(n: u8) -> ThreadId {
    ThreadId::new(Timestamp::from_second(i64::from(n)).unwrap(), [n; 10]).unwrap()
}

fn for_threads(threads: &[ThreadId]) -> ModelRequest {
    let mut asked = request(Purpose::Main);
    asked.threads = threads.to_vec();
    asked
}

fn gateway(fake: &Arc<Fake>, budgets: Budgets) -> Gateway {
    Gateway::new(Routes::new(vec![fake.target("m")]).unwrap()).with_budgets(budgets)
}

#[test]
fn a_call_that_could_exceed_a_budget_is_refused_before_it_is_sent() {
    let fake = Fake::new("p");
    let budgets = Budgets {
        thread: Some(Limit::Tokens(WORST - 1)),
        ..Budgets::default()
    };
    let gateway = gateway(&fake, budgets);
    let id = thread(1);
    let refused = block_on!(gateway.complete(&for_threads(&[id]), &mut |_| {})).unwrap_err();
    assert_eq!(refused.failure(), Failure::OverBudget);
    let ProviderError::OverBudget { budget, detail } = &refused else {
        panic!("{refused}")
    };
    assert_eq!(budget, &format!("thread {id}"));
    assert!(detail.contains("0 of 104 tokens committed"), "{detail}");
    assert!(detail.contains("105 more"), "{detail}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0, "nothing was sent");
    assert!(
        block_on!(gateway.complete(&for_threads(&[]), &mut |_| {})).is_ok(),
        "a call for no thread counts in no thread budget"
    );
}

#[test]
fn a_call_settles_to_what_it_cost_in_its_thread_and_every_thread_above() {
    let fake = Fake::new("p");
    let budgets = Budgets {
        organization: Some(Limit::Tokens(1_000_000)),
        person: Some(Limit::Tokens(1_000_000)),
        thread: Some(Limit::Tokens(1_000)),
    };
    let gateway = gateway(&fake, budgets);
    let (parent, child) = (thread(1), thread(2));
    block_on!(gateway.complete(&for_threads(&[parent]), &mut |_| {})).unwrap();
    block_on!(gateway.complete(&for_threads(&[child, parent]), &mut |_| {})).unwrap();
    // The fake reports 100 tokens in and 10 out for each.
    assert_eq!(gateway.spent(Budget::Thread(child)), 110);
    assert_eq!(gateway.spent(Budget::Thread(parent)), 220);
    assert_eq!(gateway.spent(Budget::Person), 220);
    assert_eq!(gateway.spent(Budget::Organization), 220);
    // The parent has 780 left, the child 890: the child's next call is held to its parent's.
    for _ in 0..7 {
        block_on!(gateway.complete(&for_threads(&[parent]), &mut |_| {})).unwrap();
    }
    let refused = block_on!(gateway.complete(&for_threads(&[child, parent]), &mut |_| {}));
    let Err(ProviderError::OverBudget { budget, .. }) = refused else {
        panic!("{refused:?}")
    };
    assert_eq!(budget, format!("thread {parent}"));
}

#[test]
fn a_failed_call_spends_nothing_and_releases_its_hold() {
    let fake = Fake::failing("p", ProviderError::Protocol("garbled".into()));
    let budgets = Budgets {
        person: Some(Limit::Tokens(WORST)),
        ..Budgets::default()
    };
    let gateway = gateway(&fake, budgets);
    assert!(block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).is_err());
    assert_eq!(gateway.spent(Budget::Person), 0);
    assert!(
        block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).is_ok(),
        "the whole budget is free again"
    );
}

#[test]
fn a_money_budget_skips_a_target_it_cannot_afford() {
    let fake = Fake::new("p");
    let price = |per_mtok: u64| Price {
        list: "test".into(),
        currency: Currency::USD,
        input_per_mtok: per_mtok,
        cache_read_per_mtok: 0,
        output_per_mtok: per_mtok,
    };
    // At $10 a million tokens the worst case is 1,050 millionths; at $1, 105.
    let routes = Routes::new(vec![
        fake.target("dear").with_price(price(10_000_000)),
        fake.target("cheap").with_price(price(1_000_000)),
    ])
    .unwrap();
    let limit = Money {
        micros: 1_000,
        currency: Currency::USD,
    };
    let gateway = Gateway::new(routes).with_budgets(Budgets {
        person: Some(Limit::Money(limit)),
        ..Budgets::default()
    });
    let answer = block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).unwrap();
    assert_eq!(answer.served.unwrap().target, "p/cheap");
    // 100 in and 10 out at $1 a million.
    assert_eq!(gateway.spent(Budget::Person), 110);
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
    let gateway = Arc::new(gateway(
        &fake,
        Budgets {
            person: Some(Limit::Tokens(limit)),
            ..Budgets::default()
        },
    ));
    block_on!(async {
        let mut calls = JoinSet::new();
        for _ in 0..8 {
            let gateway = Arc::clone(&gateway);
            calls
                .spawn(async move { gateway.complete(&request(Purpose::Main), &mut |_| {}).await });
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
    assert_eq!(gateway.spent(Budget::Person), 3 * 55);
    assert!(gateway.spent(Budget::Person) <= limit);
}
