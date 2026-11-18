//! Getting back into a glave account by email (`specs/model.md`, "Identity"): someone who lost
//! their passkeys asks with their address, the one they signed up with or the one an emailed
//! invite went to alone; the link sent there makes a new passkey for the
//! account it has, once, which the address and the person's other browsers are told of. An
//! address with none is answered alike and sent a note, and the last passkey may go once the
//! account can get back in.

use std::sync::Arc;

use glave_event::{OrgRole, PrincipalId, RawEvent, Secret};
use glave_identity::signin::kept;
use glave_identity::testing::Authenticator;
use glave_ports::stream::Reserved;
use glave_ports::testing::Outbox;
use glave_ports::{EventLog, OrgId, Scope};
use glave_session::protocol::{LinkKind, Request, code};
use serde_json::{Value, json};

use crate::signups::{linked, organized, serve_sharing, serve_timed, signed_up};
use crate::support::web::{ORIGIN, Served, Web, asked, elsewhere, make_passkey};
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
        let mut bob = organized(&served, &outbox, "bob@example.com", "Bob's").await;
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

/// An invite naming an address is emailed there alone: whoever made it is never answered its
/// code, so she cannot open it herself and hold the account the address's own person would
/// then be led into (#1465, #1551).
#[test]
fn an_invite_naming_an_address_is_never_answered_to_whoever_made_it() {
    block_on!(async {
        let dir = folder("recovery-invited");
        let outbox = Arc::new(Outbox::default());
        let (served, clock) = serve_timed(&dir, true, &outbox).await;
        let theirs = "victim@example.com";
        // Mallory invites an address that is not hers: told it was sent there, never its code.
        let mut mallory = organized(&served, &outbox, "mallory@example.com", "Mallory's").await;
        let made = asked(&mut mallory, &invite_to(theirs)).await;
        assert_eq!(made["invite"]["email"], json!(theirs));
        assert!(made.get("code").is_none(), "her answer opens it: {made}");
        assert_eq!(outbox.sent().last().unwrap().to, theirs);

        // Asked to get its person back in, the address is sent no link: no account has it.
        clock.advance_seconds(61);
        assert!(back_in(&served, &outbox, theirs).await.is_none());
        // And its own person still signs up with it, to an account of their own.
        clock.advance_seconds(61);
        let mut victim = signed_up(&served, &outbox, theirs).await;
        assert_eq!(listed(&mut victim).await["email"], json!(theirs));
        served.close().await;
    });
}

/// Whoever opens an emailed invite's link holds its mailbox, since its code went there alone:
/// the account it makes gets back in with the address, unless another account has it already,
/// which nothing then says (#1551).
#[test]
fn an_emailed_invite_s_person_gets_back_in_with_its_address() {
    block_on!(async {
        let dir = folder("recovery-invited-back");
        let outbox = Arc::new(Outbox::default());
        let (served, clock) = serve_timed(&dir, true, &outbox).await;
        let theirs = "grace@example.com";
        let mut bob = organized(&served, &outbox, "bob@example.com", "Bob's").await;
        // Grace opens the link in her mail, and her account has the address.
        let mut grace = joined_by_mail(&served, &outbox, &mut bob, theirs).await;
        assert_eq!(listed(&mut grace).await["email"], json!(theirs));

        // Asking to get back in mails her a link into her own account.
        clock.advance_seconds(61);
        let mut back = back_in(&served, &outbox, theirs).await.unwrap();
        let passkeys = listed(&mut back).await;
        assert_eq!(passkeys["email"], json!(theirs));
        assert_eq!(made_by_email(&passkeys), [false, true], "hers: {passkeys}");

        // An address another account has stays its own: the account an invite to it makes has
        // none, and is answered as any.
        let mut ada = signed_up(&served, &outbox, "ada@example.com").await;
        let mut other = joined_by_mail(&served, &outbox, &mut bob, "ada@example.com").await;
        assert!(listed(&mut other).await.get("email").is_none());
        assert_eq!(listed(&mut ada).await["email"], json!("ada@example.com"));
        served.close().await;
    });
}

/// An invite from before its code could go to its address alone (`invite.created` v1) was
/// answered to whoever made it, emailed or not: one still waiting as glave is updated never
/// makes its address the way back into the account it makes (#1465).
#[test]
fn an_invite_its_maker_was_shown_never_makes_its_address_a_way_back_in() {
    block_on!(async {
        let dir = folder("recovery-shown");
        let outbox = Arc::new(Outbox::default());
        let log = Shared::default();
        let (served, _) = serve_sharing(&dir, true, &outbox, log.clone()).await;
        let theirs = "victim@example.com";
        let mut mallory = organized(&served, &outbox, "mallory@example.com", "Mallory's").await;
        let code = shown_invite(&log, &mut mallory, theirs).await;
        // Mallory opens it herself.
        let made = make_passkey(&served, code, LinkKind::Invite, &mut device()).await;
        let mut hers = elsewhere(&served, made["session"].as_str().map(str::to_owned));
        let passkeys = listed(&mut hers).await;
        assert!(
            passkeys.get("email").is_none(),
            "the address gets her back in: {passkeys}"
        );
        assert!(back_in(&served, &outbox, theirs).await.is_none());
        served.close().await;
    });
}

/// Taken out of the last organization they were in, someone's account never signs in again, so
/// its address goes with it, though that organization knew them by another: the account a new
/// invite to the address makes gets it, and gets back in with it.
#[test]
fn a_removed_person_s_address_goes_to_the_account_a_new_invite_makes() {
    block_on!(async {
        let dir = folder("recovery-removed");
        let outbox = Arc::new(Outbox::default());
        let (served, clock) = serve_timed(&dir, true, &outbox).await;
        let theirs = "grace@example.com";
        let mut bob = organized(&served, &outbox, "bob@example.com", "Bob's").await;
        let mut carol = organized(&served, &outbox, "carol@example.com", "Carol's").await;
        // Grace joins Bob's by the address, then, signed in, Carol's by an invite to another.
        let mut first = joined_by_mail(&served, &outbox, &mut bob, theirs).await;
        assert_eq!(listed(&mut first).await["email"], json!(theirs));
        let other = "grace@work.example.com";
        asked(&mut carol, &invite_to(other)).await;
        let code = linked(&outbox, outbox.sent().len(), "invite")
            .await
            .unwrap();
        let accepted = asked(&mut first, &Request::InviteAccept { code }).await;
        assert_eq!(accepted["person"]["email"], json!(other), "{accepted}");
        // Taken out of Bob's, then Carol's, which knew her by the other address.
        let grace = picked(&mut bob, |it| it["email"] == json!(theirs)).await;
        let known = picked(&mut carol, |it| it["email"] == json!(other)).await;
        assert_eq!(known, grace);
        let out = Request::PersonRemove { person: grace };
        assert_eq!(asked(&mut bob, &out).await, json!({}));
        assert_eq!(asked(&mut carol, &out).await, json!({}));

        // Invited again, she makes a new account, which has the address.
        clock.advance_seconds(61);
        let mut second = joined_by_mail(&served, &outbox, &mut bob, theirs).await;
        let again = picked(&mut bob, |it| it["email"] == json!(theirs)).await;
        assert_ne!(again, grace, "a new account");
        assert_eq!(listed(&mut second).await["email"], json!(theirs));
        // And it is the one the address gets her back into.
        clock.advance_seconds(61);
        let mut back = back_in(&served, &outbox, theirs).await.unwrap();
        let passkeys = listed(&mut back).await;
        assert_eq!(passkeys["email"], json!(theirs));
        assert_eq!(
            made_by_email(&passkeys),
            [false, true],
            "the new one: {passkeys}"
        );
        served.close().await;
    });
}

/// An invite to be a Member, emailed to `email`.
fn invite_to(email: &str) -> Request {
    Request::InviteCreate {
        role: OrgRole::Member,
        email: Some(email.into()),
    }
}

/// Whoever holds `email`'s mailbox, joining by the invite `maker` emails there: their browser.
async fn joined_by_mail(served: &Served, outbox: &Outbox, maker: &mut Web, email: &str) -> Web {
    asked(maker, &invite_to(email)).await;
    let code = linked(outbox, outbox.sent().len(), "invite").await.unwrap();
    let made = make_passkey(served, code, LinkKind::Invite, &mut device()).await;
    elsewhere(served, made["session"].as_str().map(str::to_owned))
}

/// A browser signed in by the link to get back in that `email` is sent, if it is sent one.
async fn back_in(served: &Served, outbox: &Outbox, email: &str) -> Option<Web> {
    let mut anyone = elsewhere(served, None);
    let before = outbox.sent().len();
    let start = Request::RecoveryStart {
        email: email.into(),
    };
    assert_eq!(asked(&mut anyone, &start).await, json!({}));
    let code = linked(outbox, before + 1, "recover").await?;
    let made = make_passkey(served, code, LinkKind::Recovery, &mut device()).await;
    Some(elsewhere(
        served,
        made["session"].as_str().map(str::to_owned),
    ))
}

/// Whether each of the passkeys `passkey/list` answered was made by email.
fn made_by_email(passkeys: &Value) -> Vec<bool> {
    let listed = passkeys["passkeys"].as_array().unwrap();
    listed.iter().map(|p| p.get("emailed").is_some()).collect()
}

/// The first of the people `admin` lists whom `which` picks.
async fn picked(admin: &mut Web, which: impl Fn(&Value) -> bool) -> PrincipalId {
    let listed = Request::PeopleList {
        after: None,
        limit: None,
    };
    let people = asked(admin, &listed).await;
    let found = people["people"].as_array().unwrap().iter();
    let theirs = found.into_iter().find(|it| which(it)).unwrap();
    serde_json::from_value(theirs["id"].clone()).unwrap()
}

/// An invite to `email` in the organization `maker` works in, written as glave wrote one
/// before v2 (`invite.created` v1), straight into its people ledger: its code, which its maker
/// was shown.
async fn shown_invite(log: &Shared, maker: &mut Web, email: &str) -> Secret {
    let listed = asked(maker, &Request::OrganizationList {}).await;
    let orgs = listed["organizations"].as_array().unwrap().iter();
    let current = orgs.into_iter().find(|it| it.get("current").is_some());
    let org: OrgId = serde_json::from_value(current.unwrap()["id"].clone()).unwrap();
    let by = picked(maker, |it| it.get("you").is_some()).await;
    let code = Secret::new(format!("{}.{}", org.as_uuid().simple(), "c0de".repeat(11)));
    let scope = Scope::of_organization(org);
    let ledger = Reserved::People.id();
    let seq = EventLog::read(log, &scope, ledger, None, usize::MAX)
        .await
        .unwrap()
        .len()
        + 1;
    let data = json!({
        "invite": "0192f5c7-0000-7000-8000-0000000001a2",
        "hash": kept(code.expose()),
        "role": "member",
        "by": {"kind": "person", "id": by},
        "expires": "2099-01-01T00:00:00.000Z",
        "email": email,
    });
    let line = format!(
        r#"{{"glave":1,"id":"0192f5c2-7000-7000-8000-0000000001a1","thread":"{ledger}","seq":{seq},"at":"2026-10-29T09:10:00.000Z","type":"invite.created","v":1,"data":{data}}}"#
    );
    let raw = RawEvent::from_json(&line).unwrap();
    EventLog::append(log, &scope, &[raw]).await.unwrap();
    code
}

/// A device elsewhere that makes passkeys for [`ORIGIN`].
fn device() -> Authenticator {
    Authenticator::new(ORIGIN).unwrap()
}
