//! People who share one computer (`specs/session.md`, "Doors"): a server whose origin is set to
//! this computer's `http://localhost:<port>` keeps glave accounts there, each person in a
//! browser of their own, while the person on the computer signs in at its address, the door
//! that is theirs alone. A glave account there is never taken for the local person, and a
//! sign-in link's code, or the local token, works at that door alone.

use std::sync::Arc;

use glave_event::{OrgRole, Secret};
use glave_identity::testing::Authenticator;
use glave_ports::host::PublicOrigin;
use glave_session::Answering;
use glave_session::protocol::{LinkKind, Request, code};
use glave_session::signin::{Accounts, Browsers, LocalToken, SignIn};
use serde_json::json;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::support::issuer::fixed_clock;
use crate::support::web::{Served, Web, asked, serve_bound};
use crate::support::*;

/// A server whose origin is `http://localhost:<port>`, at the port it serves at, keeping glave
/// accounts there, its programs here signing in with `token`: it, and that origin.
async fn shared(dir: &std::path::Path, token: &LocalToken) -> (Served, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let at = format!("http://localhost:{port}");
    let origin = PublicOrigin::parse(&at).unwrap();
    let store = sqlite(&dir.join("shared.db")).await;
    let server = "0192f5c6-0000-7000-8000-0000000005ea".to_owned();
    let clock = fixed_clock();
    let browsers = Browsers::open(
        store.log(),
        scope(),
        server.clone(),
        Uuid::from_u128(0x694),
        Some(&origin),
        Box::new(clock.clone()),
    );
    let browsers = Arc::new(browsers.await.unwrap());
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    setup.browsers = Some(Arc::clone(&browsers));
    let directory = Arc::new(store.directory());
    let accounts = Accounts::new(store.log(), directory, server, &origin, Arc::new(clock));
    let accounts = Arc::new(accounts.unwrap());
    setup.accounts = Some(Arc::clone(&accounts));
    setup.origin = Some(origin);
    let (server, _) = setup.build(dir).await;
    let signin = SignIn::local(token.clone(), Box::new(fixed_clock()))
        .with_browsers(browsers)
        .with_accounts(accounts);
    (serve_bound(server, signin, listener).await, at)
}

#[test]
fn on_a_shared_computer_an_account_is_never_the_local_person_and_links_are_theirs_alone() {
    block_on!(async {
        let dir = folder("sharing");
        let token = LocalToken::new([24; 32]);
        let (served, origin) = shared(&dir, &token).await;
        // The person on the computer, at its address.
        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let told = asked(&mut here, &Request::Discover {}).await;
        assert_eq!(told["origin"], json!(origin), "{told}");
        assert_eq!(
            told["invites"],
            json!(true),
            "alone, they may invite: {told}"
        );

        // A sign-in link's code works at the address alone, never at the shared origin.
        let link = asked(&mut here, &Request::SigninLink {}).await;
        let code = Secret::new(link["code"].as_str().unwrap());
        let mut at_origin = Web::new(&served, None).reaching(&origin);
        let refused = asked(
            &mut at_origin,
            &Request::SigninRedeem { code: code.clone() },
        )
        .await;
        assert_eq!(refused, json!(code::NO_SIGN_IN), "{refused}");
        let mut at_address = Web::new(&served, None);
        let redeemed = asked(&mut at_address, &Request::SigninRedeem { code }).await;
        let link_session = redeemed["session"].as_str().map(str::to_owned);
        assert!(link_session.is_some(), "{redeemed}");
        // Nor does the session it began, or the local token, sign anyone in at the shared origin.
        for (key, kept) in [
            ("a link's session", link_session),
            ("the local token", Some(token.reveal().to_owned())),
        ] {
            let mut there = Web::new(&served, kept).reaching(&origin);
            let refused = asked(&mut there, &Request::SigninLink {}).await;
            assert_eq!(refused, json!(code::NO_SIGN_IN), "{key}");
        }

        // Someone invited makes a passkey at the shared origin: an account, never the owner.
        let invited = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let invited = asked(&mut here, &invited).await;
        let code = Secret::new(invited["code"].as_str().unwrap());
        let mut device = Authenticator::new(&origin).unwrap();
        let made = make_passkey_at(&served, &origin, code, &mut device).await;
        let session = made["session"].as_str().map(str::to_owned);
        assert!(session.is_some(), "{made}");
        for (door, reaching) in [("the shared origin", Some(&origin)), ("the address", None)] {
            let web = Web::new(&served, session.clone());
            let mut theirs = match reaching {
                Some(origin) => web.reaching(origin),
                None => web,
            };
            let told = asked(&mut theirs, &Request::Discover {}).await;
            assert_eq!(told["role"], json!("member"), "at {door}: {told}");
            let refused = asked(&mut theirs, &Request::SigninLink {}).await;
            assert!(
                refused.is_number(),
                "at {door}, no link for them: {refused}"
            );
        }
        served.close().await;
    });
}

/// What making a passkey with an invite's `code`, as a browser at the shared `origin` does, is
/// answered with.
async fn make_passkey_at(
    served: &Served,
    origin: &str,
    code: Secret,
    device: &mut Authenticator,
) -> serde_json::Value {
    let mut anyone = Web::new(served, None).reaching(origin);
    let create = Request::PasskeyCreateOptions {
        code,
        link: LinkKind::Invite,
    };
    let options = asked(&mut anyone, &create).await;
    let made = Request::PasskeyCreate {
        ceremony: options["ceremony"].as_str().unwrap().to_owned(),
        credential: device.create(&options["options"]).unwrap(),
        name: Some("Laptop".into()),
        person_name: None,
    };
    asked(&mut anyone, &made).await
}
