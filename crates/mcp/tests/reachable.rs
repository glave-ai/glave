//! Which addresses a remote server may be at: a person on their own computer keeps loopback,
//! where people sign in from elsewhere only public addresses are reached, by name or as written,
//! in sign-in's discovery too, and cloud metadata never is. A server on this computer signs in
//! on it. A server's redirect goes nowhere.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use glave_mcp::{Limits, McpError, McpServer, Reachable, SignIn, SignInError, offers_sign_in};
use glave_ports::auth::Auth;
use glave_ports::block_on;
use glave_ports::testing::FakeHttp;
use serde_json::json;

use crate::http::Served;

const REDIRECT: &str = "http://127.0.0.1:9/connect/callback";

fn remote(url: &str, reachable: Reachable) -> McpServer {
    let limits = Limits {
        reachable,
        ..Limits::default()
    };
    McpServer::remote("docs", url, &BTreeMap::new(), Auth::None, limits)
}

/// Why `server` did not start.
async fn why(server: &McpServer) -> String {
    match server.started().await {
        Err(McpError::Start { message, .. }) => message,
        Ok(()) => panic!("it started"),
    }
}

/// `url` with its loopback address in place named as `host`.
fn as_host(url: &str, host: &str) -> String {
    url.replacen("127.0.0.1", host, 1)
}

#[test]
fn a_person_on_their_own_computer_keeps_loopback_by_address_and_by_name() {
    let listening = Served::with(&[]);
    block_on!(async {
        for url in [listening.url.clone(), as_host(&listening.url, "localhost")] {
            let server = remote(&url, Reachable::AlsoPrivate);
            assert_eq!(server.started().await, Ok(()), "{url}");
            server.close().await;
        }
    });
}

#[test]
fn where_people_sign_in_from_elsewhere_a_private_address_is_refused_however_it_is_named() {
    let listening = Served::with(&[]);
    block_on!(async {
        for host in ["127.0.0.1", "[::ffff:127.0.0.1]", "2130706433", "localhost"] {
            let url = as_host(&listening.url, host);
            let said = why(&remote(&url, Reachable::OnlyPublic)).await;
            assert!(
                said.contains("where people sign in from elsewhere"),
                "{url}: {said}"
            );
        }
    });
}

#[test]
fn cloud_metadata_is_refused_everywhere_before_anything_is_sent() {
    block_on!(async {
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://[::ffff:169.254.169.254]/latest/meta-data/",
            "http://2852039166/latest/meta-data/",
            "http://[fd00:ec2::254]/latest/meta-data/",
        ] {
            for reachable in [Reachable::AlsoPrivate, Reachable::OnlyPublic] {
                let said = why(&remote(url, reachable)).await;
                assert!(!said.starts_with("could not reach"), "{url}: {said}");
                assert!(
                    said.contains("metadata") || said.contains("credentials"),
                    "{said}"
                );
            }
        }
    });
}

#[test]
fn a_server_s_redirect_is_not_followed_to_what_it_names() {
    let there = FakeHttp::always(200, "{}").unwrap();
    let to_there = there.url.clone();
    let here = FakeHttp::always(307, &to_there).unwrap();
    block_on!(async {
        let server = remote(&format!("{}/mcp", here.url), Reachable::AlsoPrivate);
        assert!(server.started().await.is_err());
    });
    assert!(!here.received().is_empty(), "asked");
    assert!(there.received().is_empty(), "never reached");
}

#[test]
fn sign_in_s_discovery_keeps_to_the_same_rule() {
    let here = FakeHttp::always(404, "{}").unwrap();
    let by_name = format!("{}/mcp", as_host(&here.url, "localhost"));
    let by_address = format!("{}/mcp", here.url);
    block_on!(async {
        for url in [&by_name, &by_address] {
            let reachable = Reachable::OnlyPublic;
            assert_eq!(offers_sign_in(url, reachable).await, Ok(false), "{url}");
            let started = SignIn::start(url, "docs", REDIRECT, &[], reachable).await;
            assert!(started.is_err(), "{url}");
        }
    });
    assert!(here.received().is_empty(), "nothing reached it");
}

#[test]
fn sign_in_goes_nowhere_a_server_s_metadata_names_that_may_not_be_reached() {
    let address = Arc::new(OnceLock::new());
    let its: Arc<OnceLock<String>> = Arc::clone(&address);
    let here = FakeHttp::answering(move |_, path| {
        if path.contains("oauth-protected-resource") {
            let names = json!({
                "resource": its.get(),
                "authorization_servers": ["http://169.254.169.254/"],
            });
            (200, names.to_string())
        } else {
            (404, "{}".to_owned())
        }
    })
    .unwrap();
    address.set(format!("{}/mcp", here.url)).unwrap();
    let url = format!("{}/mcp", here.url);
    block_on!(async {
        let asked = offers_sign_in(&url, Reachable::AlsoPrivate).await;
        assert!(
            matches!(asked, Err(SignInError::Unreachable { .. })),
            "{asked:?}"
        );
    });
    assert!(!here.received().is_empty(), "the server was asked");
}

/// A server on this computer that is its own authorization server at `127.0.0.1`, whose own
/// address names it as `host`, and whose metadata names `register` as where glave registers.
fn naming_registration(host: &'static str, register: &'static str) -> FakeHttp {
    let address = Arc::new(OnceLock::new());
    let its: Arc<OnceLock<String>> = Arc::clone(&address);
    let here = FakeHttp::answering(move |_, path| {
        let here = its.get().cloned().unwrap_or_default();
        let answer = if path.contains("oauth-protected-resource") {
            let resource = format!("{}/mcp", as_host(&here, host));
            json!({"resource": resource, "authorization_servers": [here]})
        } else if path.contains("oauth-authorization-server") {
            json!({
                "issuer": here,
                "authorization_endpoint": format!("{here}/authorize"),
                "registration_endpoint": register,
                "token_endpoint": format!("{here}/token"),
                "response_types_supported": ["code"],
                "code_challenge_methods_supported": ["S256"],
            })
        } else {
            return (404, "{}".to_owned());
        };
        (200, answer.to_string())
    })
    .unwrap();
    address.set(here.url.clone()).unwrap();
    here
}

#[test]
fn sign_in_registers_nowhere_an_authorization_server_names_that_may_not_be_reached() {
    // rmcp checks the addresses discovery finds as they are written; the endpoints an
    // authorization server names are reached through glave's client alone (RUSTSEC-2026-0457).
    block_on!(async {
        for register in [
            "http://[::ffff:169.254.169.254]/register",
            "http://[fd00:ec2::254]/register",
        ] {
            let here = naming_registration("127.0.0.1", register);
            let url = format!("{}/mcp", here.url);
            let started = SignIn::start(&url, "docs", REDIRECT, &[], Reachable::AlsoPrivate).await;
            let Err(SignInError::Refused { message, .. }) = started else {
                panic!("{register}: it registered");
            };
            assert!(message.contains("credentials"), "{register}: {message}");
            assert!(!here.received().is_empty(), "the server was asked");
        }
    });
}

#[test]
fn a_server_on_this_computer_signs_in_on_it_under_another_name() {
    // Its own address is `localhost`, its authorization server's `127.0.0.1`: both this
    // computer, which a server on the internet's sign-in never reaches.
    let here = naming_registration("localhost", "http://127.0.0.1:9/register");
    let url = format!("{}/mcp", as_host(&here.url, "localhost"));
    block_on!(async {
        assert_eq!(offers_sign_in(&url, Reachable::AlsoPrivate).await, Ok(true));
    });
    let asked: Vec<String> = here.received().into_iter().map(|got| got.path).collect();
    assert!(
        asked
            .iter()
            .any(|path| path.contains("oauth-authorization-server")),
        "its authorization server was asked: {asked:?}"
    );
}
