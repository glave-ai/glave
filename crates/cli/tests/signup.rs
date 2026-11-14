//! `glave signup link`, the real binary through the person's server at a public origin, with no
//! sign-up or email configured: whoever runs it makes a link, which someone opens in a browser
//! of their own, gives their name and makes a passkey, landing in their own personal space; a
//! second time it opens nothing. A server no one else reaches says it makes none.

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
