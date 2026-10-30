//! Getting back into a glave account by email (`specs/model.md`, "Identity"): someone who lost
//! their passkeys asks with their address; the link sent there makes a new passkey for the
//! account it has, once, which the address and the person's other browsers are told of. An
//! address with none is answered alike and sent a note, and the last passkey may go once the
//! account can get back in.

use std::sync::Arc;

use glave_event::{OrgRole, Secret};
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
        let sent = outbox.sent_by(2).await;
        assert_eq!(sent.len(), 2, "her sign-up's, then one link a minute");
        assert_eq!(sent[1].subject, "Get back into glave");
        let code = linked(&outbox, 2, "recover").await.unwrap();
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
        // For a week, and then no one is told any more.
        clock.advance_seconds(7 * 24 * 3600 - 60);
        assert!(listed(&mut first).await["notice"].is_string());
        clock.advance_seconds(120);
        assert!(listed(&mut first).await.get("notice").is_none());
        served.close().await;
    });
}

#[test]
fn an_address_with_no_account_is_answered_alike_and_sent_no_link() {
    block_on!(async {
        // Where anyone may sign up, a note; where no one may, nothing, so the server mails no
        // stranger.
        for (signup, notes) in [(true, 1), (false, 0)] {
            let dir = folder("recovery-none");
            let outbox = Arc::new(Outbox::default());
            let (served, _) = serve_timed(&dir, signup, &outbox).await;
            let mut anyone = elsewhere(&served, None);
            let start = Request::RecoveryStart {
                email: "nobody@example.com".into(),
            };
            assert_eq!(asked(&mut anyone, &start).await, json!({}));
            let sent = outbox.sent_by(notes).await;
            assert_eq!(sent.len(), notes, "taking sign-ups: {signup}");
            assert!(
                sent.iter()
                    .all(|mail| mail.subject == "Getting back into glave")
            );
            assert!(linked(&outbox, notes, "recover").await.is_none());
            served.close().await;
        }
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
        // Someone she invites by a link alone has no address to get back in with: theirs stays.
        let mut bob = signed_up(&served, &outbox, "bob@example.com").await;
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let code = Secret::new(asked(&mut bob, &invite).await["code"].as_str().unwrap());
        let made = make_passkey(&served, code, LinkKind::Invite, &mut device()).await;
        let mut grace = elsewhere(&served, made["session"].as_str().map(str::to_owned));
        let passkeys = listed(&mut grace).await;
        assert!(passkeys.get("email").is_none(), "{passkeys}");
        let id = passkeys["passkeys"][0]["id"].as_str().unwrap().to_owned();
        let kept = asked(&mut grace, &Request::PasskeyRemove { passkey: id }).await;
        assert_eq!(kept, json!(code::NOT_ALLOWED));
        served.close().await;
    });
}

/// A device elsewhere that makes passkeys for [`ORIGIN`].
fn device() -> Authenticator {
    Authenticator::new(ORIGIN).unwrap()
}
