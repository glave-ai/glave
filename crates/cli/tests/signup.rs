//! `glave signup link`, the real binary through the person's server at a public origin, with no
//! sign-up or email configured: whoever runs it makes a link, which someone opens in a browser
//! of their own, gives their name and makes a passkey, landing in their own personal space; a
//! second time it opens nothing. A server no one else reaches says it makes none, and so does
//! a person's own server, its owner's alone, which signs up and invites no one.

use glave_identity::testing::Authenticator;
use glave_ports::block_on;
use glave_provider_openai::fake::{FakeServer, Reply};
use glave_session::client::Remote;
use serde_json::json;

use crate::connected::request;
use crate::http::{serve_http, serve_http_with};
use crate::invited::from_browser;
use crate::support::*;

#[test]
fn whoever_runs_the_server_makes_a_link_that_signs_someone_up_once() {
    let place = place("signup-link");
    let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    choose(&place, "serve", "{ origin = \"http://localhost\" }");
    let (_served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &[]);
    let (_, port) = url.rsplit_once(':').unwrap();
    let origin = format!("http://localhost:{port}");
    let output = glave(&place, &model, &["signup", "link"], "");
    let said = text(&output);
    assert!(output.status.success(), "{said}");
    assert!(
        said.starts_with("A sign-up link, good once until "),
        "{said}"
    );
    let marker = format!("{origin}/#signup=");
    let code = said
        .split_whitespace()
        .find_map(|word| word.strip_prefix(&marker))
        .unwrap_or_else(|| panic!("{said}"))
        .to_owned();
    block_on!(async {
        let params = json!({"code": code, "link": "signup"});
        let asked = from_browser(&url, &origin, "passkey/create_options", params.clone()).await;
        let credential = Authenticator::new(&origin)
            .unwrap()
            .create(&asked["options"])
            .unwrap();
        let made = json!({"ceremony": asked["ceremony"], "credential": credential,
            "person_name": "Ada"});
        let made = from_browser(&url, &origin, "passkey/create", made).await;
        let ada = Remote::new(&url, made["session"].as_str().unwrap().to_owned()).unwrap();
        let listed = ada.request(&request("organization/list", json!({}))).await;
        let listed = serde_json::to_value(listed.unwrap()).unwrap();
        let orgs = listed["organizations"].as_array().unwrap();
        assert_eq!(orgs.len(), 1, "{listed}");
        assert_eq!(orgs[0]["role"], json!("owner"), "{listed}");
        let again = from_browser(&url, &origin, "passkey/create_options", params).await;
        assert!(again.is_null(), "used once: {again}");
    });
}

#[test]
fn a_server_no_one_else_reaches_makes_no_sign_up_links() {
    let place = place("signup-link-none");
    let model = FakeServer::start(vec![]).unwrap();
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    let output = glave(&place, &model, &["signup", "link"], "");
    assert!(!output.status.success());
    let said = text(&output);
    assert!(
        said.contains("doesn't make organizations by sign-up"),
        "{said}"
    );
}

/// A person's own server, reached at its public origin, is its owner's alone (#1553): it keeps
/// no glave accounts for anyone else, so it makes no sign-up link, takes no sign-ups though its
/// settings ask for them and it sends email, and invites no one, saying so.
#[test]
fn a_person_s_own_server_signs_up_and_invites_no_one_but_its_owner() {
    let place = place("signup-own-server");
    let model = FakeServer::start(vec![]).unwrap();
    let serve = "{ origin = \"https://glave.example.com\", signup = true }";
    choose(&place, "serve", serve);
    let email = "{ host = \"127.0.0.1\", from = \"glave <glave@example.com>\" }";
    choose(&place, "email", email);
    let (_served, url) = serve_http(&place, &model, &["--model", "m"]);
    let output = glave(&place, &model, &["signup", "link"], "");
    assert!(!output.status.success());
    let said = text(&output);
    assert!(
        said.contains("doesn't make organizations by sign-up"),
        "{said}"
    );
    let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();
    block_on!(async {
        let owner = Remote::new(&url, token.trim().to_owned()).unwrap();
        let found = owner.request(&request("server/discover", json!({}))).await;
        let found = serde_json::to_value(found.unwrap()).unwrap();
        assert!(found.get("invites").is_none(), "{found}");
        assert!(found.get("invite_only").is_none(), "{found}");
        assert!(!found["signin"].to_string().contains("signup"), "{found}");
        let invite = request("invite/create", json!({"role": "member"}));
        let refused = owner.request(&invite).await.unwrap_err();
        let refused = serde_json::to_string(&refused).unwrap();
        assert!(refused.contains("this glave is yours alone"), "{refused}");
    });
}
