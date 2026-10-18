//! Signing browsers in on the person's own computer (`specs/one-server.md`, "Finding it"): a
//! link's code works once, for a session that outlives a restart, is the local person, ends
//! when the browser signs out, and is this server's and this computer's alone.

use std::path::Path;
use std::sync::Arc;

use glave_ports::testing::ManualClock;
use glave_session::Answering;
use glave_session::protocol::{Request, code};
use glave_session::signin::{Browsers, LocalToken, SignIn};
use serde_json::json;

use crate::support::issuer::fixed_clock;
use crate::support::web::{Served, Web, serve_with};
use crate::support::*;

/// The browsers of the glave whose folder is `dir`, as it opens them at each start.
fn browsers(dir: &Path, clock: &ManualClock) -> Arc<Browsers> {
    Arc::new(Browsers::open(dir.join("browsers.json"), Box::new(clock.clone())).unwrap())
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

fn spaces() -> Request {
    Request::SpaceList { limit: None }
}

#[test]
fn a_link_signs_one_browser_in_once_for_a_session_that_outlives_a_restart() {
    block_on!(async {
        let dir = folder("browsers-link");
        let clock = fixed_clock();
        let token = LocalToken::new([3; 32]);
        let served = serve(&dir, browsers(&dir, &clock), &token).await;
        let mut anyone = Web::new(&served, None);
        let discovered = anyone.call(&Request::Discover {}).await;
        let id = discovered["result"]["id"].clone();
        assert!(id.is_string(), "discover needs no sign-in: {discovered}");

        let code = link(&served, &token).await;
        let redeemed = anyone.call(&redeem(&code)).await;
        assert_eq!(redeemed["result"]["server"], id, "{redeemed}");
        let session = redeemed["result"]["session"].as_str().unwrap().to_owned();
        let again = anyone.call(&redeem(&code)).await;
        assert_eq!(again["error"]["code"], json!(code::NO_SIGN_IN), "once");
        assert_eq!(
            anyone.call(&spaces()).await["error"]["code"],
            json!(code::NO_SIGN_IN),
            "nothing else without a sign-in"
        );

        let mut browser = Web::new(&served, Some(session.clone()));
        let listed = browser.call(&spaces()).await;
        assert!(listed["result"]["spaces"].is_array(), "{listed}");
        let refused = browser.call(&Request::SigninLink {}).await;
        assert_eq!(
            refused["error"]["code"],
            json!(code::NOT_ALLOWED),
            "a browser makes no links"
        );
        served.close().await;

        // A restart: a new token, the same folder, the same server and session.
        let token = LocalToken::new([4; 32]);
        let served = serve(&dir, browsers(&dir, &clock), &token).await;
        let rediscovered = Web::new(&served, None).call(&Request::Discover {}).await;
        assert_eq!(rediscovered["result"]["id"], id);
        let mut browser = Web::new(&served, Some(session.clone()));
        assert!(browser.call(&spaces()).await["result"].is_object());
        let ended = browser.call(&Request::SigninEnd {}).await;
        assert_eq!(ended["result"], json!({}), "{ended}");
        let after = browser.call(&spaces()).await;
        assert_eq!(
            after["error"]["code"],
            json!(code::NO_SIGN_IN),
            "signed out"
        );
        assert!(
            !std::fs::read_to_string(dir.join("browsers.json"))
                .unwrap()
                .contains(&session),
            "only its hash is kept"
        );
        served.close().await;
    });
}

#[test]
fn signing_other_browsers_out_keeps_this_one() {
    block_on!(async {
        let dir = folder("browsers-others");
        let token = LocalToken::new([5; 32]);
        let served = serve(&dir, browsers(&dir, &fixed_clock()), &token).await;
        let mut sessions = Vec::new();
        for _ in 0..2 {
            let code = link(&served, &token).await;
            let redeemed = Web::new(&served, None).call(&redeem(&code)).await;
            sessions.push(redeemed["result"]["session"].as_str().unwrap().to_owned());
        }
        let mut this = Web::new(&served, Some(sessions[0].clone()));
        let mut other = Web::new(&served, Some(sessions[1].clone()));
        let done = this.call(&Request::SigninEndOthers {}).await;
        assert_eq!(done["result"], json!({}), "{done}");
        assert!(this.call(&spaces()).await["result"].is_object());
        assert_eq!(
            other.call(&spaces()).await["error"]["code"],
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
        let served = serve(&ours, browsers(&ours, &fixed_clock()), &token).await;
        let other = serve(&theirs, browsers(&theirs, &fixed_clock()), &token).await;
        let code = link(&served, &token).await;
        let session = Web::new(&served, None).call(&redeem(&code)).await["result"]["session"]
            .as_str()
            .unwrap()
            .to_owned();
        let ours_id =
            Web::new(&served, None).call(&Request::Discover {}).await["result"]["id"].clone();
        let theirs_id =
            Web::new(&other, None).call(&Request::Discover {}).await["result"]["id"].clone();
        assert_ne!(
            ours_id, theirs_id,
            "each glave folder is a server of its own"
        );
        let elsewhere = Web::new(&other, Some(session.clone()))
            .call(&spaces())
            .await;
        assert_eq!(elsewhere["error"]["code"], json!(code::NO_SIGN_IN));

        let browsers = browsers(&ours, &fixed_clock());
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock())).with_browsers(browsers);
        let there = "192.168.1.20".parse().unwrap();
        assert!(signin.verify_from(&session, there).await.is_err());
        assert!(
            signin
                .verify_from(&session, "127.0.0.1".parse().unwrap())
                .await
                .is_ok()
        );
        served.close().await;
        other.close().await;
    });
}
