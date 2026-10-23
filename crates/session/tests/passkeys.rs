//! Passkeys over HTTP (`specs/session.md`, "Signing in"): where the server is reached from
//! elsewhere, glave on its computer makes a setup link, a browser elsewhere trades its code for
//! a passkey and a session, and the passkey signs browsers in from then on, through the public
//! origin as a tunnel on this computer forwards it. The local token and a link's sessions and
//! codes sign in only from this computer, never through the tunnel; the person signed in with
//! a passkey adds, lists and removes passkeys, and nobody signed in does.

use std::path::Path;
use std::sync::Arc;

use glave_identity::testing::Authenticator;
use glave_ports::host::PublicOrigin;
use glave_ports::testing::ManualClock;
use glave_session::Answering;
use glave_session::protocol::{Request, code};
use glave_session::signin::{Browsers, LocalToken, SignIn};
use serde_json::{Value, json};

use crate::support::issuer::fixed_clock;
use crate::support::web::{Served, Web, serve_here, serve_with};
use crate::support::*;

/// Where people reach the server from elsewhere.
const ORIGIN: &str = "https://glave.test";

/// The browsers of the glave in `dir`, reached from elsewhere at [`ORIGIN`] if `reached`.
async fn browsers(dir: &Path, clock: &ManualClock, reached: bool) -> Arc<Browsers> {
    let store = sqlite(&dir.join("browsers.db")).await;
    let server = "0192f5c6-0000-7000-8000-0000000005e7".to_owned();
    let origin = PublicOrigin::parse(ORIGIN).unwrap();
    let origin = reached.then_some(&origin);
    let opened = Browsers::open(
        store.log(),
        scope(),
        server,
        origin,
        Box::new(clock.clone()),
    );
    Arc::new(opened.await.unwrap())
}

/// A server reached at [`ORIGIN`] if `reached`, signing browsers in with `browsers` and
/// programs here with `token`.
async fn serve(dir: &Path, reached: bool, token: &LocalToken) -> Served {
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    setup.browsers = Some(browsers(dir, &fixed_clock(), reached).await);
    setup.origin = reached.then(|| PublicOrigin::parse(ORIGIN).unwrap());
    let (server, _) = setup.build(dir).await;
    let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
    serve_with(server, signin, None).await
}

/// A client through the tunnel on this computer: its requests come to [`ORIGIN`]'s name.
fn elsewhere(served: &Served, token: Option<String>) -> Web {
    Web::new(served, token).reaching(ORIGIN)
}

/// What `request` is answered with: its result, or its error's code.
async fn asked(web: &mut Web, request: &Request) -> Value {
    let answered = web.call(request).await;
    match answered.get("result") {
        Some(result) => result.clone(),
        None => answered["error"]["code"].clone(),
    }
}

/// A passkey made on `phone` through a setup link glave here made, and its session.
async fn set_up(served: &Served, token: &LocalToken, phone: &mut Authenticator) -> String {
    let mut here = Web::new(served, Some(token.reveal().to_owned()));
    let setup = asked(&mut here, &Request::SigninSetup {}).await;
    let code = glave_event::Secret::new(setup["code"].as_str().unwrap());
    let mut anyone = elsewhere(served, None);
    let options = asked(&mut anyone, &Request::PasskeyCreateOptions { code }).await;
    let credential = phone.create(&options["options"]).unwrap();
    let created = Request::PasskeyCreate {
        ceremony: options["ceremony"].as_str().unwrap().to_owned(),
        credential,
        name: Some("Phone".into()),
    };
    let signed_in = asked(&mut anyone, &created).await;
    signed_in["session"].as_str().unwrap().to_owned()
}

/// A browser elsewhere signed in with `phone`'s passkey: its session.
async fn sign_in(served: &Served, phone: &mut Authenticator) -> Value {
    let mut anyone = elsewhere(served, None);
    let options = asked(&mut anyone, &Request::PasskeyGetOptions {}).await;
    let credential = phone.get(&options["options"]).unwrap();
    let ceremony = options["ceremony"].as_str().unwrap().to_owned();
    asked(
        &mut anyone,
        &Request::PasskeyGet {
            ceremony,
            credential,
        },
    )
    .await
}

#[test]
fn a_setup_link_makes_a_passkey_that_signs_browsers_in_from_elsewhere() {
    block_on!(async {
        let dir = folder("passkeys-from-elsewhere");
        let token = LocalToken::new([5; 32]);
        let served = serve(&dir, true, &token).await;
        let discovered = asked(&mut elsewhere(&served, None), &Request::Discover {}).await;
        assert_eq!(discovered["origin"], ORIGIN);
        assert_eq!(discovered["signin"], json!(["link", "passkey"]));

        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let first = set_up(&served, &token, &mut phone).await;
        let mut person = elsewhere(&served, Some(first));
        let listed = asked(&mut person, &Request::PasskeyList {}).await;
        assert_eq!(listed["passkeys"][0]["name"], "Phone", "{listed}");

        let again = sign_in(&served, &mut phone).await;
        assert_eq!(again["server"], discovered["id"]);
        let mut later = elsewhere(&served, Some(again["session"].as_str().unwrap().into()));
        assert!(asked(&mut later, &Request::PasskeyList {}).await["passkeys"].is_array());
        served.close().await;
    });
}

#[test]
fn only_a_passkey_signs_in_through_the_public_origin() {
    block_on!(async {
        let dir = folder("passkeys-doors");
        let token = LocalToken::new([6; 32]);
        let served = serve(&dir, true, &token).await;
        let no_sign_in = json!(code::NO_SIGN_IN);
        let mut tunnelled = elsewhere(&served, Some(token.reveal().to_owned()));
        assert_eq!(
            asked(&mut tunnelled, &Request::PasskeyList {}).await,
            no_sign_in
        );

        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let link = asked(&mut here, &Request::SigninLink {}).await;
        let code = glave_event::Secret::new(link["code"].as_str().unwrap());
        let redeem = Request::SigninRedeem { code };
        assert_eq!(
            asked(&mut elsewhere(&served, None), &redeem).await,
            no_sign_in
        );
        let session = asked(&mut Web::new(&served, None), &redeem).await;
        let by_link = session["session"].as_str().unwrap().to_owned();
        let mut linked = elsewhere(&served, Some(by_link));
        assert_eq!(
            asked(&mut linked, &Request::PasskeyList {}).await,
            no_sign_in
        );

        let mut anyone = elsewhere(&served, None);
        for signed_in_only in [Request::PasskeyAddOptions {}, Request::PasskeyList {}] {
            assert_eq!(asked(&mut anyone, &signed_in_only).await, no_sign_in);
        }
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let by_passkey = set_up(&served, &token, &mut phone).await;
        let mut person = elsewhere(&served, Some(by_passkey));
        let setup = asked(&mut person, &Request::SigninSetup {}).await;
        assert_eq!(
            setup,
            json!(code::NOT_ALLOWED),
            "setup links come from a program here"
        );
        served.close().await;
    });
}

#[test]
fn the_person_adds_and_removes_passkeys_and_removing_one_signs_its_browsers_out() {
    block_on!(async {
        let dir = folder("passkeys-add-remove");
        let token = LocalToken::new([7; 32]);
        let served = serve(&dir, true, &token).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let session = set_up(&served, &token, &mut phone).await;
        let mut person = elsewhere(&served, Some(session));
        let options = asked(&mut person, &Request::PasskeyAddOptions {}).await;
        let mut laptop = Authenticator::new(ORIGIN).unwrap();
        let added = Request::PasskeyAdd {
            ceremony: options["ceremony"].as_str().unwrap().to_owned(),
            credential: laptop.create(&options["options"]).unwrap(),
            name: Some("Laptop".into()),
        };
        let laptop_s = asked(&mut person, &added).await["passkey"]["id"].clone();
        let listed = asked(&mut person, &Request::PasskeyList {}).await;
        assert_eq!(listed["passkeys"].as_array().unwrap().len(), 2);

        let phone_s = listed["passkeys"][0]["id"].as_str().unwrap().to_owned();
        let mut on_laptop = elsewhere(&served, None);
        let laptop_session = sign_in(&served, &mut laptop).await["session"].clone();
        on_laptop.token = laptop_session.as_str().map(str::to_owned);
        let removed = Request::PasskeyRemove { passkey: phone_s };
        assert_eq!(asked(&mut on_laptop, &removed).await, json!({}));
        let gone = asked(&mut person, &Request::PasskeyList {}).await;
        assert_eq!(
            gone,
            json!(code::NO_SIGN_IN),
            "the phone's browser is signed out"
        );
        let left = asked(&mut on_laptop, &Request::PasskeyList {}).await;
        assert_eq!(left["passkeys"][0]["id"], laptop_s);
        served.close().await;
    });
}

#[test]
fn a_server_reached_only_here_offers_links_alone() {
    block_on!(async {
        let dir = folder("passkeys-here");
        let token = LocalToken::new([8; 32]);
        let served = serve(&dir, false, &token).await;
        let mut anyone = Web::new(&served, None);
        let discovered = asked(&mut anyone, &Request::Discover {}).await;
        assert_eq!(discovered["signin"], json!(["link"]));
        assert_eq!(discovered.get("origin"), None);
        let options = asked(&mut anyone, &Request::PasskeyGetOptions {}).await;
        assert_eq!(options, json!(code::NOT_AVAILABLE));
        served.close().await;
    });
}

/// A browser on this computer, on a page at `http://localhost:<port>`, its requests reaching
/// the listener at [::1] beside 127.0.0.1's, as a browser trying that address first does.
fn browser_here(served: &Served, token: Option<String>) -> Web {
    let port = served.address.port();
    Web::new(served, token)
        .via(&format!("http://[::1]:{port}/"))
        .reaching(&format!("http://localhost:{port}"))
}

/// A browser on this computer signed in with `mac`'s passkey: what signing in answered.
async fn sign_in_here(served: &Served, mac: &mut Authenticator) -> Value {
    let mut anyone = browser_here(served, None);
    let options = asked(&mut anyone, &Request::PasskeyGetOptions {}).await;
    let credential = mac.get(&options["options"]).unwrap();
    let ceremony = options["ceremony"].as_str().unwrap().to_owned();
    asked(
        &mut anyone,
        &Request::PasskeyGet {
            ceremony,
            credential,
        },
    )
    .await
}

#[test]
fn a_server_here_makes_a_passkey_at_localhost_that_signs_another_browser_in() {
    block_on!(async {
        let dir = folder("passkeys-localhost");
        let token = LocalToken::new([9; 32]);
        let mut setup = Setup::asking(Vec::new(), Answering::Clients);
        setup.browsers = Some(browsers(&dir, &fixed_clock(), false).await);
        let (built, _) = setup.build(&dir).await;
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let served = serve_here(built, signin).await;
        let here = format!("http://localhost:{}", served.address.port());
        let mut anyone = Web::new(&served, None);
        let discovered = asked(&mut anyone, &Request::Discover {}).await;
        assert_eq!(discovered["here"], json!(here));
        assert_eq!(
            discovered["signin"],
            json!(["link"]),
            "no passkey to sign in with yet"
        );

        // Signed in by glave's link, a browser here makes one.
        let mut linked = Web::new(&served, Some(token.reveal().to_owned()));
        let code = asked(&mut linked, &Request::SigninLink {}).await["code"].clone();
        let code = glave_event::Secret::new(code.as_str().unwrap());
        let session = asked(&mut anyone, &Request::SigninRedeem { code }).await;
        let mut first = Web::new(&served, session["session"].as_str().map(str::to_owned));
        let options = asked(&mut first, &Request::PasskeyAddOptions {}).await;
        assert_eq!(options["options"]["publicKey"]["rp"]["id"], "localhost");
        let mut mac = Authenticator::new(&here).unwrap();
        let added = Request::PasskeyAdd {
            ceremony: options["ceremony"].as_str().unwrap().to_owned(),
            credential: mac.create(&options["options"]).unwrap(),
            name: Some("Mac".into()),
        };
        assert_eq!(asked(&mut first, &added).await["passkey"]["name"], "Mac");
        let discovered = asked(&mut anyone, &Request::Discover {}).await;
        assert_eq!(discovered["signin"], json!(["link", "passkey"]));

        // Another browser here signs in with it at once.
        let signed_in = sign_in_here(&served, &mut mac).await;
        let mut second = browser_here(&served, signed_in["session"].as_str().map(str::to_owned));
        let listed = asked(&mut second, &Request::ProjectList { limit: None }).await;
        assert!(listed["projects"].is_array(), "{listed}");
        // A page another program serves at another port here gets nothing for the same passkey.
        mac.on_page(&format!(
            "http://localhost:{}",
            served.address.port().wrapping_add(1)
        ));
        let relayed = sign_in_here(&served, &mut mac).await;
        assert_eq!(relayed, json!(code::NO_SIGN_IN), "{relayed}");
        mac.on_page(&here);
        let again = sign_in_here(&served, &mut mac).await;
        assert!(
            again["session"].is_string(),
            "the page's origin was refused: {again}"
        );
        served.close().await;
    });
}
