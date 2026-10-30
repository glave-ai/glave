//! What glave emails when someone asks over the network (`specs/model.md`, "Identity"): one
//! network may ask for twenty an hour, counted alike by every server on the store, and a
//! link goes after the answer, so the answer takes as long whether or not the address has an
//! account.

use std::sync::Arc;

use glave_ports::testing::Outbox;
use glave_session::protocol::{Request, code};
use serde_json::{Value, json};

use crate::signups::{serve_behind, serve_sharing, signed_up};
use crate::support::web::{Web, asked, elsewhere};
use crate::support::*;

/// What a sign-up for `email`, asked through `web`, is answered with.
async fn sign_up(web: &mut Web, email: &str) -> Value {
    let start = Request::SignupStart {
        email: email.into(),
    };
    asked(web, &start).await
}

#[test]
fn a_network_asks_for_twenty_emails_an_hour_from_any_server_on_the_store() {
    block_on!(async {
        let (one, two) = (folder("mailing-one"), folder("mailing-two"));
        let outbox = Arc::new(Outbox::default());
        let log = Shared::default();
        let (first, first_clock) = serve_sharing(&one, true, &outbox, log.clone()).await;
        let (second, second_clock) = serve_sharing(&two, true, &outbox, log).await;
        let network = "203.0.113.7";
        let mut here = elsewhere(&first, None).from(network);
        let mut there = elsewhere(&second, None).from(network);
        for asking in 0..20 {
            let web = if asking % 2 == 0 {
                &mut here
            } else {
                &mut there
            };
            let email = format!("person{asking}@example.com");
            assert_eq!(sign_up(web, &email).await, json!({}), "{asking}");
        }
        assert_eq!(
            sign_up(&mut there, "one-more@example.com").await,
            json!(code::LIMIT),
            "the other server counted the first's"
        );
        assert_eq!(
            sign_up(&mut here, "one-more@example.com").await,
            json!(code::LIMIT)
        );
        // Another network asks as ever; so does this one an hour later.
        let mut neighbour = elsewhere(&first, None).from("198.51.100.9");
        assert_eq!(sign_up(&mut neighbour, "bea@example.com").await, json!({}));
        for clock in [&first_clock, &second_clock] {
            clock.advance_seconds(3600);
        }
        assert_eq!(sign_up(&mut there, "later@example.com").await, json!({}));
        assert_eq!(outbox.sent_by(22).await.len(), 22);
        first.close().await;
        second.close().await;
    });
}

#[test]
fn a_link_goes_after_the_answer_whether_or_not_the_address_has_an_account() {
    block_on!(async {
        let dir = folder("mailing-after");
        let outbox = Arc::new(Outbox::default());
        let (served, clock) = serve_sharing(&dir, true, &outbox, Shared::default()).await;
        signed_up(&served, &outbox, "ada@example.com").await;
        clock.advance_seconds(61);
        // The mail server holds every message: both are answered before either goes.
        outbox.hold();
        let mut anyone = elsewhere(&served, None);
        assert_eq!(sign_up(&mut anyone, "ada@example.com").await, json!({}));
        assert_eq!(sign_up(&mut anyone, "bea@example.com").await, json!({}));
        assert_eq!(outbox.sent().len(), 1, "her sign-up's alone, so far");
        outbox.release();
        let sent = outbox.sent_by(3).await;
        let mut told: Vec<_> = sent[1..].iter().map(|mail| mail.to.as_str()).collect();
        told.sort_unstable();
        assert_eq!(told, ["ada@example.com", "bea@example.com"]);
        served.close().await;
    });
}

#[test]
fn behind_google_s_load_balancer_a_forged_address_counts_as_the_visitor_s_own() {
    block_on!(async {
        let dir = folder("mailing-behind");
        let outbox = Arc::new(Outbox::default());
        let behind = (true, std::num::NonZeroU8::new(2));
        let (served, _) = serve_behind(&dir, behind, &outbox, Shared::default()).await;
        // Each time a neighbour's address first, before the load balancer's `<client>,<itself>`.
        for asking in 0..20 {
            let forged = "198.51.100.9, 203.0.113.7, 34.120.1.1";
            let mut visitor = elsewhere(&served, None).from(forged);
            let email = format!("person{asking}@example.com");
            assert_eq!(sign_up(&mut visitor, &email).await, json!({}), "{asking}");
        }
        let forged = "192.0.2.1, 203.0.113.7, 34.120.1.1";
        let mut visitor = elsewhere(&served, None).from(forged);
        assert_eq!(
            sign_up(&mut visitor, "one-more@example.com").await,
            json!(code::LIMIT),
            "what the visitor put first changes nothing"
        );
        let mut neighbour = elsewhere(&served, None).from("198.51.100.9, 34.120.1.1");
        assert_eq!(
            sign_up(&mut neighbour, "bea@example.com").await,
            json!({}),
            "the neighbour named twenty times was charged none of them"
        );
        served.close().await;
    });
}
