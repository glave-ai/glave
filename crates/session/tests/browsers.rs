//! Signing browsers in on the person's own computer (`specs/one-server.md`, "Finding it"): a
//! link's code works once, for a session that outlives a restart and is shared by the
//! browser's tabs, is the local person, ends when the browser signs out, and is this server's
//! and this computer's alone; it is handed over in two halves, a cookie the page's scripts
//! cannot read and the answer's, and neither alone signs in; a foreign `Host` or `Origin` is
//! refused, cookie or not; and nobody an identity provider signs in gets a session, so their
//! sign-in ends when the provider stops renewing it (#365).

use std::path::Path;
use std::sync::Arc;

use glave_ports::testing::ManualClock;
use glave_session::Answering;
use glave_session::client::Remote;
use glave_session::protocol::{Reply, Request, code};
use glave_session::signin::{
    Browsers, Door, LocalToken, SESSION_PREFIX, SignIn, SignInError, Trust,
};
use serde_json::{Value, json};

use crate::support::issuer::{AUDIENCE, Issuer, NOW, Signer, fixed_clock};
use crate::support::web::{Served, Web, serve_with};
use crate::support::*;

/// The browsers of the glave whose folder is `dir`, as it opens them at each start: on the
/// store in that folder, the server named after the folder.
async fn browsers(dir: &Path, clock: &ManualClock) -> Arc<Browsers> {
    let store = sqlite(&dir.join("browsers.db")).await;
    let server = dir.file_name().unwrap().to_string_lossy().into_owned();
    let opened = Browsers::open(store.log(), scope(), server, None, Box::new(clock.clone()));
    Arc::new(opened.await.unwrap())
}

/// A server signing browsers in with `browsers`, and programs here with `token`.
async fn serve(dir: &Path, browsers: Arc<Browsers>, token: &LocalToken) -> Served {
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    setup.browsers = Some(browsers);
    let (server, _) = setup.build(dir).await;
    let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
    serve_with(server, signin, None).await
}

/// A link's code, asked for with the local token as `glave` asks for one.
async fn link(served: &Served, token: &LocalToken) -> String {
    let linked = Web::new(served, Some(token.reveal().to_owned()))
        .call(&Request::SigninLink {})
        .await;
    linked["result"]["code"].as_str().unwrap().to_owned()
}

fn redeem(code: &str) -> Request {
    Request::SigninRedeem {
        code: glave_event::Secret::new(code),
    }
}

/// A browser signed in with a link from `token`'s program, as the app is: the answer's half of
/// its session as its bearer, the cookie's half kept as a browser keeps cookies; and the
/// answer's `Set-Cookie`.
async fn signed_in(served: &Served, token: &LocalToken) -> (Web, String) {
    let code = link(served, token).await;
    let mut browser = Web::new(served, None);
    let (message, _) = browser.message(&redeem(&code));
    let answered = browser.post(&message).await;
    let redeemed = answered.messages.first().unwrap();
    browser.token = redeemed["result"]["session"].as_str().map(str::to_owned);
    assert!(browser.token.is_some(), "{redeemed}");
    (browser, answered.set_cookies.concat())
}

/// The whole session `browser` holds, its two halves joined, as the server joins them.
fn whole(browser: &Web) -> String {
    let second = browser.token.as_deref().unwrap();
    let cookie = browser.cookie();
    let first = cookie.split_once('=').unwrap().1;
    format!(
        "{SESSION_PREFIX}{first}{}",
        second.strip_prefix(SESSION_PREFIX).unwrap()
    )
}

fn projects() -> Request {
    Request::ProjectList { limit: None }
}

/// The code a browser's call of `request` is answered with: none when it is answered.
async fn refused(browser: &mut Web, request: &Request) -> Value {
    browser.call(request).await["error"]["code"].clone()
}

#[test]
fn a_link_signs_one_browser_in_once_for_a_session_that_outlives_a_restart() {
    block_on!(async {
        let dir = folder("browsers-link");
        let clock = fixed_clock();
        let token = LocalToken::new([3; 32]);
        let served = serve(&dir, browsers(&dir, &clock).await, &token).await;
        let mut anyone = Web::new(&served, None);
        let discovered = anyone.call(&Request::Discover {}).await;
        let id = discovered["result"]["id"].clone();
        assert!(id.is_string(), "discover needs no sign-in: {discovered}");

        let code = link(&served, &token).await;
        let mut browser = Web::new(&served, None);
        let redeemed = browser.call(&redeem(&code)).await;
        assert_eq!(redeemed["result"]["server"], id, "{redeemed}");
        browser.token = redeemed["result"]["session"].as_str().map(str::to_owned);
        let again = anyone.call(&redeem(&code)).await;
        assert_eq!(again["error"]["code"], json!(code::NO_SIGN_IN), "once");
        assert_eq!(
            refused(&mut anyone, &projects()).await,
            json!(code::NO_SIGN_IN),
            "nothing else without a sign-in"
        );

        let listed = browser.call(&projects()).await;
        assert!(listed["result"]["projects"].is_array(), "{listed}");
        assert_eq!(
            refused(&mut browser, &Request::SigninLink {}).await,
            json!(code::NOT_ALLOWED),
            "a browser makes no links"
        );
        served.close().await;

        // A restart: a new token, the same folder, the same server and session, in every tab.
        let token = LocalToken::new([4; 32]);
        let served = serve(&dir, browsers(&dir, &clock).await, &token).await;
        let rediscovered = Web::new(&served, None).call(&Request::Discover {}).await;
        assert_eq!(rediscovered["result"]["id"], id);
        let mut browser = browser.at(&served);
        let mut tab = browser.tab();
        assert!(browser.call(&projects()).await["result"].is_object());
        assert!(
            tab.call(&projects()).await["result"].is_object(),
            "a new tab"
        );
        let session = whole(&browser);
        let second = browser.token.clone().unwrap();
        let cookie = browser.cookie();
        let first = cookie.split_once('=').unwrap().1.to_owned();
        let ended = browser.call(&Request::SigninEnd {}).await;
        assert_eq!(ended["result"], json!({}), "{ended}");
        assert_eq!(browser.cookie(), "", "signing out clears the cookie");
        assert_eq!(
            refused(&mut tab, &projects()).await,
            json!(code::NO_SIGN_IN),
            "signed out in every tab"
        );
        let kept = std::fs::read(dir.join("browsers.db")).unwrap();
        for kept_not in [&session, &first, &second] {
            assert!(
                !kept
                    .windows(kept_not.len())
                    .any(|window| window == kept_not.as_bytes()),
                "only its hash is kept"
            );
        }
        served.close().await;
    });
}

#[test]
fn a_session_comes_in_halves_neither_of_which_signs_in_alone() {
    block_on!(async {
        let dir = folder("browsers-halves");
        let token = LocalToken::new([8; 32]);
        let served = serve(&dir, browsers(&dir, &fixed_clock()).await, &token).await;
        let (mut browser, set) = signed_in(&served, &token).await;
        let id = Web::new(&served, None).call(&Request::Discover {}).await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            set.starts_with(&format!("glave-{id}=")),
            "named after its server: {set}"
        );
        assert!(
            set.ends_with("; Path=/; Max-Age=2592000; HttpOnly; SameSite=Strict"),
            "no script reads it, no other site sends it, and over plain HTTP: {set}"
        );
        let second = browser.token.clone().unwrap();
        assert!(!second.contains(set.split_once(';').unwrap().0.split_once('=').unwrap().1));

        // The answer's half alone, as a page of this origin's could take away.
        let mut page_only = Web::new(&served, Some(second.clone()));
        assert_eq!(
            refused(&mut page_only, &projects()).await,
            json!(code::NO_SIGN_IN)
        );
        // The cookie alone, as every page on this computer's loopback is sent.
        let mut cookie_only = browser.tab();
        cookie_only.token = None;
        assert_eq!(
            refused(&mut cookie_only, &projects()).await,
            json!(code::NO_SIGN_IN)
        );
        // The whole token in one piece, as nothing over HTTP is ever handed.
        let mut whole_bearer = Web::new(&served, Some(whole(&browser)));
        assert_eq!(
            refused(&mut whole_bearer, &projects()).await,
            json!(code::NO_SIGN_IN)
        );

        // Both, as the app's own page presents them; each answer renews the cookie.
        let (message, _) = browser.message(&projects());
        let answered = browser.post(&message).await;
        assert!(answered.messages[0]["result"].is_object());
        assert_eq!(
            answered.set_cookies.concat(),
            set,
            "the cookie lasts 30 more days"
        );
        served.close().await;
    });
}

#[test]
fn signing_other_browsers_out_keeps_this_one() {
    block_on!(async {
        let dir = folder("browsers-others");
        let token = LocalToken::new([5; 32]);
        let served = serve(&dir, browsers(&dir, &fixed_clock()).await, &token).await;
        let (mut this, _) = signed_in(&served, &token).await;
        let (mut other, _) = signed_in(&served, &token).await;
        let done = this.call(&Request::SigninEndOthers {}).await;
        assert_eq!(done["result"], json!({}), "{done}");
        assert!(this.call(&projects()).await["result"].is_object());
        assert_eq!(
            refused(&mut other, &projects()).await,
            json!(code::NO_SIGN_IN)
        );
        served.close().await;
    });
}

#[test]
fn a_session_is_one_servers_and_this_computers_alone() {
    block_on!(async {
        let token = LocalToken::new([6; 32]);
        let (ours, theirs) = (folder("browsers-ours"), folder("browsers-theirs"));
        let served = serve(&ours, browsers(&ours, &fixed_clock()).await, &token).await;
        let other = serve(&theirs, browsers(&theirs, &fixed_clock()).await, &token).await;
        let (browser, _) = signed_in(&served, &token).await;
        let ours_id =
            Web::new(&served, None).call(&Request::Discover {}).await["result"]["id"].clone();
        let theirs_id =
            Web::new(&other, None).call(&Request::Discover {}).await["result"]["id"].clone();
        assert_ne!(
            ours_id, theirs_id,
            "each glave folder is a server of its own"
        );
        // Another glave on this computer is sent the same cookie, which it does not read.
        let mut elsewhere = browser.at(&other);
        assert_eq!(
            refused(&mut elsewhere, &projects()).await,
            json!(code::NO_SIGN_IN)
        );

        let session = whole(&browser);
        let browsers = browsers(&ours, &fixed_clock()).await;
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock())).with_browsers(browsers);
        for there in [Door::Public, Door::Elsewhere] {
            assert!(matches!(
                signin.verify_from(&session, there).await,
                Err(SignInError::OnlyThere)
            ));
        }
        assert!(signin.verify_from(&session, Door::Here).await.is_ok());
        served.close().await;
        other.close().await;
    });
}

#[test]
fn a_foreign_host_or_origin_is_refused_cookie_or_not() {
    block_on!(async {
        let dir = folder("browsers-hosts");
        let token = LocalToken::new([7; 32]);
        let served = serve(&dir, browsers(&dir, &fixed_clock()).await, &token).await;
        let (browser, _) = signed_in(&served, &token).await;
        let http = reqwest::Client::new();
        let listing = json!({"jsonrpc": "2.0", "id": 1, "method": "project/list", "params": {}});
        let port = served.address.port();
        let bearer = format!("Bearer {}", browser.token.as_deref().unwrap());
        let cookie = browser.cookie();
        let status = |host: String, origin: Option<String>| {
            let mut request = http
                .post(&served.url)
                .header("Host", host)
                .header("Authorization", &bearer)
                .header("Cookie", &cookie)
                .body(listing.to_string());
            if let Some(origin) = origin {
                request = request.header("Origin", origin);
            }
            async move { request.send().await.unwrap().status().as_u16() }
        };
        let own = format!("127.0.0.1:{port}");
        assert_eq!(status(own.clone(), None).await, 200);
        assert_eq!(
            status(own.clone(), Some(format!("http://{own}"))).await,
            200
        );
        // A name rebound to 127.0.0.1, and a page of another site, a page of another port of
        // this computer (which the browser sends the cookie) and one at another name of it.
        assert_eq!(status(format!("rebound.example:{port}"), None).await, 403);
        for page in [
            "https://evil.example".to_owned(),
            format!("http://127.0.0.1:{}", port.wrapping_add(1)),
            format!("http://localhost:{port}"),
            "null".to_owned(),
        ] {
            assert_eq!(status(own.clone(), Some(page.clone())).await, 403, "{page}");
        }
        let page = http
            .get(&served.url)
            .header("Host", format!("rebound.example:{port}"))
            .send()
            .await
            .unwrap();
        assert_eq!(page.status().as_u16(), 403, "not even a page");

        // `glave` alone asks for its link as a program here does, and is let through.
        let glave = Remote::new(&served.url, token.reveal().to_owned()).unwrap();
        let linked = glave.request(&Request::SigninLink {}).await.unwrap();
        assert!(matches!(linked, Reply::Link { .. }), "{linked:?}");
        served.close().await;
    });
}

#[test]
fn nobody_a_provider_signs_in_gets_a_session_so_theirs_ends_with_the_provider() {
    block_on!(async {
        let issuer = Issuer::start().await;
        let signer = Signer::es256("key-1");
        issuer.publish(&signer);
        let dir = folder("browsers-oidc");
        let clock = fixed_clock();
        let mut setup = Setup::asking(Vec::new(), Answering::Clients);
        setup.browsers = Some(browsers(&dir, &clock).await);
        let (built, _) = setup.build(&dir).await;
        let trust = Trust {
            issuers: vec![issuer.url.clone()],
            audience: AUDIENCE.into(),
        };
        let signin = SignIn::new(trust, Box::new(clock.clone())).unwrap();
        let served = serve_with(built, signin, None).await;
        // The provider signs ada in for ten minutes at a time.
        let mut ada = Web::new(&served, Some(signer.sign(&issuer.claims("ada"))));
        assert!(ada.call(&projects()).await["result"].is_object());
        let refused: Value = ada.call(&Request::SigninLink {}).await;
        assert_eq!(
            refused["error"]["code"],
            json!(code::NOT_ALLOWED),
            "{refused}"
        );
        assert_eq!(
            ada.call(&Request::SigninEnd {}).await["error"]["code"],
            json!(code::INVALID_PARAMS),
            "she holds no session of glave's"
        );

        // The provider refuses her from now on: it renews nothing, and what it gave her ends.
        clock.set_millis((NOW + 600 + 61) * 1_000);
        let ended = ada.call(&projects()).await;
        assert_eq!(ended["error"]["code"], json!(code::NO_SIGN_IN), "{ended}");
        served.close().await;
    });
}
