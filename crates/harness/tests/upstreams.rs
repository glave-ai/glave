//! A thread's calls say which provider a router last served its conversation with at each
//! target, as its `model.responded` events record it, so a gateway asks for that one first:
//! the provider that served the first call serves the next, one that served after another
//! failed serves from then on, and a thread read again from its log goes on with the latest.

use std::collections::BTreeMap;

use glave_event::StopReason;
use glave_harness::{Cancel, NoOneToAsk};
use glave_ports::{Completion, InMemoryLog, Served};

use crate::support::*;

const AT: &str = "openrouter/anthropic/claude-haiku-5.5";

/// An answer for `route`, served at the router's target by `upstream`.
fn served_by(route: &str, upstream: &str) -> Completion {
    let mut said = answer(Some("Done."), vec![], StopReason::EndTurn);
    said.served = Some(Served {
        route: route.into(),
        target: AT.into(),
        service: Some("OpenRouter".into()),
        effort: None,
        reasoning_stripped: false,
    });
    said.upstream = Some(upstream.into());
    said
}

/// What each call asked for first, by target.
fn asked(seen: &Seen) -> Vec<BTreeMap<String, String>> {
    let seen = seen.lock().unwrap();
    seen.iter()
        .map(|request| request.upstreams.clone())
        .collect()
}

fn at(upstream: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(AT.to_owned(), upstream.to_owned())])
}

#[test]
fn each_call_asks_for_the_provider_that_served_the_last_and_resuming_keeps_it() {
    let dir = folder("upstreams");
    block_on!(async {
        let answers = vec![
            served_by("main", "Google"),
            // Google failed, so the router passed the call to Bedrock.
            served_by("main", "Amazon Bedrock"),
            served_by("summary", "Amazon Bedrock"),
            served_by("main", "Amazon Bedrock"),
        ];
        let (harness, seen) =
            harness_seeing(&dir, answers, NoOneToAsk, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "hello").await;
        turn(&harness, &mut thread, "and again").await;
        harness
            .compact(&mut thread, &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let mut again = harness.resume_thread(scope(), thread.id).await.unwrap();
        turn(&harness, &mut again, "once more").await;
        assert_eq!(
            asked(&seen),
            [
                BTreeMap::new(),
                at("Google"),
                at("Amazon Bedrock"),
                at("Amazon Bedrock"),
            ]
        );
    });
}
