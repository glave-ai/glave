//! Signing up (`specs/model.md`, "Identity"): where the server's configuration takes sign-ups,
//! keeps glave accounts at a public origin and sends email, someone gives their email, opens
//! the one-time link sent there and makes a passkey, which makes their account and an
//! organization of their own with them its Owner. An address is answered alike whatever it
//! is, a link opens once, one address gets one email a minute, and a server that takes no
//! sign-ups says so.

use std::path::Path;
use std::sync::Arc;

use glave_event::Secret;
use glave_identity::testing::Authenticator;
use glave_ports::host::PublicOrigin;
use glave_ports::testing::Outbox;
use glave_session::Answering;
use glave_session::protocol::{LinkKind, Request, code};
use glave_session::signin::{Accounts, Browsers, LocalToken, SignIn};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::issuer::fixed_clock;
use crate::support::web::{ORIGIN, Served, asked, elsewhere, make_passkey, serve_with};
use crate::support::*;

/// A server reached at [`ORIGIN`] keeping glave accounts, sending its email to `outbox`, and
/// taking sign-ups if `signup`.
pub(crate) async fn serve(dir: &Path, signup: bool, outbox: &Arc<Outbox>) -> Served {
    serve_timed(dir, signup, outbox).await.0
}

/// [`serve`], with the clock its server reads, to move on.
async fn serve_timed(
    dir: &Path,
    signup: bool,
    outbox: &Arc<Outbox>,
) -> (Served, glave_ports::testing::ManualClock) {
    let store = sqlite(&dir.join("sign-ups.db")).await;
    let server = "0192f5c6-0000-7000-8000-0000000005e9".to_owned();
    let origin = PublicOrigin::parse(ORIGIN).unwrap();
    let clock = fixed_clock();
    let browsers = Browsers::open(
        store.log(),
        scope(),
        server.clone(),
        Uuid::from_u128(0x693),
        Some(&origin),
        Box::new(clock.clone()),
    );
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    setup.browsers = Some(Arc::new(browsers.await.unwrap()));
    setup.mail = Some(Arc::clone(outbox) as Arc<dyn glave_ports::Mailer>);
    let directory = Arc::new(store.directory());
    let accounts = Accounts::new(store.log(), directory, server, &origin, Arc::new(clock));
    setup.accounts = Some(Arc::new(accounts.unwrap()));
    setup.origin = Some(origin);
    setup.signup = signup;
    let ticks = setup.clock.clone();
    let (server, _) = setup.build(dir).await;
    let signin = SignIn::local(LocalToken::new([21; 32]), Box::new(fixed_clock()));
    (serve_with(server, signin, None).await, ticks)
}

/// The code in the sign-up link `outbox` sent last.
fn emailed_code(outbox: &Outbox) -> Secret {
    linked(outbox, "signup").unwrap()
}

/// The code of the link of the kind `fragment` (`signup`, `recover`) in what `outbox` sent
/// last, if it holds one.
pub(crate) fn linked(outbox: &Outbox, fragment: &str) -> Option<Secret> {
    let sent = outbox.sent();
    let text = &sent.last()?.text;
    let marker = format!("#{fragment}=");
    let link = text.split_whitespace().find(|word| word.contains(&marker))?;
    Some(Secret::new(link.split_once(&marker)?.1))
}

/// What making a passkey on `device` with the sign-up `code` is answered with.
async fn make(served: &Served, code: Secret, device: &mut Authenticator) -> Value {
    make_passkey(served, code, LinkKind::Signup, device).await
}

#[test]
fn signing_up_makes_an_account_and_an_organization_of_ones_own() {
    block_on!(async {
        let dir = folder("signups-made");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let told = asked(&mut anyone, &Request::Discover {}).await;
        assert!(
            told["signin"]
                .as_array()
                .unwrap()
                .contains(&json!("signup")),
            "{told}"
        );

        let start = Request::SignupStart {
            email: " Ada@Example.com ".into(),
        };
        assert_eq!(asked(&mut anyone, &start).await, json!({}));
        let sent = outbox.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].to, "ada@example.com",
            "the address as glave reads it"
        );
        assert!(
            sent[0].text.contains(&format!("{ORIGIN}/#signup=")),
            "{}",
            sent[0].text
        );
        // Asked again at once, it is answered alike, and nothing more is sent.
        assert_eq!(asked(&mut anyone, &start).await, json!({}));
        assert_eq!(outbox.sent().len(), 1, "one email a minute to an address");

        let code = emailed_code(&outbox);
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let made = make(&served, code.clone(), &mut phone).await;
        let token = made["session"].as_str().map(str::to_owned);
        assert!(token.is_some(), "{made}");
        let mut ada = elsewhere(&served, token);
        // Alone in it, she is shown no organization, and may invite the first.
        let told = asked(&mut ada, &Request::Discover {}).await;
        assert_eq!(told["invites"], json!(true), "{told}");
        assert!(told.get("exists").is_none(), "{told}");
        let listed = Request::PeopleList {
            after: None,
            limit: None,
        };
        let people = asked(&mut ada, &listed).await;
        let only = people["people"].as_array().unwrap();
        assert_eq!(only.len(), 1, "an organization of her own: {people}");
        assert_eq!(only[0]["role"], json!("owner"));
        assert_eq!(only[0]["you"], json!(true));
        assert_eq!(only[0]["email"], json!("ada@example.com"));
        let start_thread: Request =
            serde_json::from_value(json!({"method": "thread/start", "params": {}})).unwrap();
        assert!(asked(&mut ada, &start_thread).await["thread"]["id"].is_string());

        // The link works once.
        let mut laptop = Authenticator::new(ORIGIN).unwrap();
        let again = make(&served, code, &mut laptop).await;
        assert_eq!(again, json!(code::NO_INVITE));
        served.close().await;
    });
}

#[test]
fn an_address_with_an_account_is_answered_alike_and_told_to_sign_in() {
    block_on!(async {
        let dir = folder("signups-had");
        let outbox = Arc::new(Outbox::default());
        let (served, clock) = serve_timed(&dir, true, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let start = |email: &str| Request::SignupStart {
            email: email.into(),
        };
        asked(&mut anyone, &start("grace@example.com")).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let made = make(&served, emailed_code(&outbox), &mut phone).await;
        assert!(made["session"].is_string(), "{made}");
        // A name around it is no address.
        for written in ["Grace <grace@example.com>", "<grace@example.com>"] {
            let refused = asked(&mut anyone, &start(written)).await;
            assert_eq!(refused, json!(code::INVALID_PARAMS));
        }
        // Asked again a minute later: the same answer, and a note to sign in, with no link.
        clock.advance_seconds(61);
        assert_eq!(
            asked(&mut anyone, &start("GRACE@example.com")).await,
            json!({})
        );
        let sent = outbox.sent();
        assert_eq!(sent.len(), 2);
        assert!(!sent[1].text.contains("#signup="), "{}", sent[1].text);
        assert!(sent[1].subject.contains("already"), "{}", sent[1].subject);
        served.close().await;
    });
}

#[test]
fn a_link_carries_its_address_and_no_one_can_change_it() {
    block_on!(async {
        let dir = folder("signups-forged");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let start = Request::SignupStart {
            email: "ada@example.com".into(),
        };
        asked(&mut anyone, &start).await;
        let code = emailed_code(&outbox);
        let (secret, _) = code.expose().split_once('.').unwrap();
        let forged = Secret::new(format!("{secret}.{}", hex_of("bea@example.com")));
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        assert_eq!(
            make(&served, forged, &mut phone).await,
            json!(code::NO_INVITE)
        );
        served.close().await;
    });
}

/// `text` in lowercase hex, as a sign-up's code carries its address.
fn hex_of(text: &str) -> String {
    text.bytes()
        .flat_map(|byte| [byte >> 4, byte & 0xf])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

#[test]
fn a_server_that_takes_no_sign_ups_says_so() {
    block_on!(async {
        let dir = folder("signups-off");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, false, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let told = asked(&mut anyone, &Request::Discover {}).await;
        let ways = told["signin"].as_array().cloned().unwrap_or_default();
        assert!(!ways.contains(&json!("signup")), "{told}");
        let start = Request::SignupStart {
            email: "ada@example.com".into(),
        };
        assert_eq!(asked(&mut anyone, &start).await, json!(code::NOT_AVAILABLE));
        let guessed = Secret::new("00".repeat(32));
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let made = make(&served, guessed, &mut phone).await;
        assert_eq!(made, json!(code::NO_INVITE));
        assert_eq!(outbox.sent(), Vec::new());
        served.close().await;
    });
}
