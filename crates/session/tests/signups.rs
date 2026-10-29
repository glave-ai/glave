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
use crate::support::web::{ORIGIN, Served, asked, elsewhere, serve_with};
use crate::support::*;

/// A server reached at [`ORIGIN`] keeping glave accounts, sending its email to `outbox`, and
/// taking sign-ups if `signup`.
async fn serve(dir: &Path, signup: bool, outbox: &Arc<Outbox>) -> Served {
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
    let (server, _) = setup.build(dir).await;
    let signin = SignIn::local(LocalToken::new([21; 32]), Box::new(fixed_clock()));
    serve_with(server, signin, None).await
}

/// The code in the sign-up link `outbox` sent last.
fn emailed_code(outbox: &Outbox) -> Secret {
    let sent = outbox.sent();
    let text = &sent.last().unwrap().text;
    let link = text
        .split_whitespace()
        .find(|word| word.contains("#signup="))
        .unwrap();
    Secret::new(link.split_once("#signup=").unwrap().1)
}

/// What making a passkey on `device` with the sign-up `code` is answered with.
async fn make_passkey(served: &Served, code: Secret, device: &mut Authenticator) -> Value {
    let mut anyone = elsewhere(served, None);
    let create = Request::PasskeyCreateOptions {
        code,
        link: LinkKind::Signup,
    };
    let options = asked(&mut anyone, &create).await;
    if !options.is_object() {
        return options;
    }
    let made = Request::PasskeyCreate {
        ceremony: options["ceremony"].as_str().unwrap().to_owned(),
        credential: device.create(&options["options"]).unwrap(),
        name: Some("Phone".into()),
    };
    asked(&mut anyone, &made).await
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
        assert_eq!(sent[0].to, "Ada@Example.com");
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
        let made = make_passkey(&served, code.clone(), &mut phone).await;
        let token = made["session"].as_str().map(str::to_owned);
        assert!(token.is_some(), "{made}");
        let mut ada = elsewhere(&served, token);
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
        let again = make_passkey(&served, code, &mut laptop).await;
        assert_eq!(again, json!(code::NO_INVITE));
        served.close().await;
    });
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
        let made = make_passkey(&served, guessed, &mut phone).await;
        assert_eq!(made, json!(code::NO_INVITE));
        assert!(outbox.sent().is_empty());
        served.close().await;
    });
}
