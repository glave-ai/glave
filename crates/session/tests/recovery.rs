//! Getting back into a glave account by email (`specs/model.md`, "Identity"): someone who lost
//! their passkeys asks with their address; the link sent there makes a new passkey for the
//! account it has, once, which the address and the person's other browsers are told of. An
//! address with none is answered alike and sent a note, and the last passkey may go once the
//! account can get back in.

use std::sync::Arc;

use glave_identity::testing::Authenticator;
use glave_ports::testing::Outbox;
use glave_session::protocol::{LinkKind, Request, code};
use serde_json::{Value, json};

use crate::signups::{linked, serve_timed, signed_up};
use crate::support::web::{ORIGIN, Web, asked, elsewhere, make_passkey};
use crate::support::*;

/// What `web`'s person's passkeys are listed as.
async fn listed(web: &mut Web) -> Value {
    asked(web, &Request::PasskeyList {}).await
}

#[test]
fn a_link_to_get_back_in_makes_a_new_passkey_once_and_the_address_and_other_browsers_are_told() {
    block_on!(async {
        let dir = folder("recovery-back");
        let outbox = Arc::new(Outbox::default());
        let (served, clock) = serve_timed(&dir, true, &outbox).await;
        let mut first = signed_up(&served, &outbox, "ada@example.com").await;
        let mut anyone = elsewhere(&served, None);
        let told = asked(&mut anyone, &Request::Discover {}).await;
        assert!(
            told["signin"]
                .as_array()
                .unwrap()
                .contains(&json!("recovery")),
            "{told}"
        );

        // A minute after her sign-up's email, she asks for a link: one, whatever is asked.
        clock.advance_seconds(61);
        let start = Request::RecoveryStart {
            email: "Ada@Example.com".into(),
        };
        assert_eq!(asked(&mut anyone, &start).await, json!({}));
        assert_eq!(asked(&mut anyone, &start).await, json!({}));
        let sent = outbox.sent();
        assert_eq!(sent.len(), 2, "her sign-up's, then one link a minute");
        assert_eq!(sent[1].subject, "Get back into glave");
        let code = linked(&outbox, "recover").unwrap();
        // A sign-up's kind of passkey request opens nothing with it.
        let wrong = make_passkey(&served, code.clone(), LinkKind::Signup, &mut device()).await;
        assert_eq!(wrong, json!(code::NO_INVITE));

        let made = make_passkey(&served, code.clone(), LinkKind::Recovery, &mut device()).await;
        let session = made["session"].as_str().map(str::to_owned);
        assert!(session.is_some(), "{made}");
        let mut back = elsewhere(&served, session);
        let people = asked(
            &mut back,
            &Request::PeopleList {
                after: None,
                limit: None,
            },
        )
        .await;
        let only = &people["people"][0];
        assert_eq!(
            only["email"],
            json!("ada@example.com"),
            "her account: {people}"
        );
        assert_eq!(only["you"], json!(true));
        let sent = outbox.sent();
        assert_eq!(
            sent.last().unwrap().subject,
            "A new passkey for your glave account"
        );
        assert_eq!(sent.last().unwrap().to, "ada@example.com");
        // Once.
        let again = make_passkey(&served, code, LinkKind::Recovery, &mut device()).await;
        assert_eq!(again, json!(code::NO_INVITE));

        // Her first browser is told of it; the one it signed in is not, and knows it as its own.
        let there = listed(&mut first).await;
        assert!(there["notice"].is_string(), "{there}");
        assert_eq!(there["email"], json!("ada@example.com"));
        let emailed: Vec<_> = there["passkeys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.get("emailed").is_some())
            .collect();
        assert_eq!(emailed, [false, true]);
        let here = listed(&mut back).await;
        assert!(here.get("notice").is_none(), "{here}");
        assert_eq!(here["passkeys"][1]["current"], json!(true));
        // A week on, no one is told any more: a week past the passkey's making, which this
        // server's accounts time by a clock of their own, a few days ahead of the server's.
        clock.advance_seconds(15 * 24 * 3600);
        assert!(listed(&mut first).await.get("notice").is_none());
        served.close().await;
    });
}

#[test]
fn an_address_with_no_account_is_answered_alike_and_sent_no_link() {
    block_on!(async {
        let dir = folder("recovery-none");
        let outbox = Arc::new(Outbox::default());
        let (served, _) = serve_timed(&dir, false, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let start = Request::RecoveryStart {
            email: "nobody@example.com".into(),
        };
        assert_eq!(asked(&mut anyone, &start).await, json!({}));
        let sent = outbox.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].subject, "Getting back into glave");
        assert!(linked(&outbox, "recover").is_none(), "{}", sent[0].text);
        served.close().await;
    });
}

#[test]
fn the_last_passkey_may_go_once_the_account_can_get_back_in_by_email() {
    block_on!(async {
        let dir = folder("recovery-last");
        let outbox = Arc::new(Outbox::default());
        let (served, _) = serve_timed(&dir, true, &outbox).await;
        let mut ada = signed_up(&served, &outbox, "ada@example.com").await;
        let passkeys = listed(&mut ada).await;
        let id = passkeys["passkeys"][0]["id"].as_str().unwrap().to_owned();
        let removed = asked(&mut ada, &Request::PasskeyRemove { passkey: id }).await;
        assert_eq!(removed, json!({}), "her address gets her back in");
        served.close().await;
    });
}

/// A device elsewhere that makes passkeys for [`ORIGIN`].
fn device() -> Authenticator {
    Authenticator::new(ORIGIN).unwrap()
}
