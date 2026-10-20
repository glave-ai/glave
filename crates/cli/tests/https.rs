//! HTTPS of glave's own (`[serve] domain`), against a real ACME authority: Pebble, Let's
//! Encrypt's test server, where one runs (CI's Linux job starts it). glave gets a certificate
//! for `glave.test`, answering the authority's TLS-ALPN-01 check on the port it serves HTTPS
//! on, serves the session API over HTTPS with it, and after a restart serves the certificate it
//! kept at once. Runs when `GLAVE_TEST_ACME_DIRECTORY` names the authority's directory, with
//! `GLAVE_TEST_ACME_ROOT` the file of the root its directory is served under,
//! `GLAVE_TEST_ACME_ROOTS` where it publishes the root it issues under, and
//! `GLAVE_TEST_ACME_PORT` the port it checks names on.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use glave_ports::block_on;
use glave_provider_openai::fake::FakeServer;

use crate::http::{Said, serve_http_with};
use crate::support::*;

/// The name glave serves HTTPS for, which the authority reaches this computer at.
const NAME: &str = "glave.test";

/// How long getting a certificate may take: a guard against a hang, not a speed limit.
const PATIENCE: Duration = Duration::from_mins(2);

/// The test authority, where one runs.
struct Authority {
    directory: String,
    /// The file of the root its directory is served under.
    root: String,
    /// Where it publishes the root it issues certificates under.
    roots: String,
    /// The port it checks names on.
    port: u16,
}

fn authority() -> Option<Authority> {
    let var = |name: &str| std::env::var(name).ok();
    Some(Authority {
        directory: var("GLAVE_TEST_ACME_DIRECTORY")?,
        root: var("GLAVE_TEST_ACME_ROOT")?,
        roots: var("GLAVE_TEST_ACME_ROOTS")?,
        port: var("GLAVE_TEST_ACME_PORT")?.parse().unwrap(),
    })
}

/// A client of glave's HTTPS that reaches `NAME` on this computer and trusts only the root the
/// authority issues under.
async fn client(authority: &Authority) -> reqwest::Client {
    let pem = std::fs::read(&authority.root).unwrap();
    let fetching = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(&pem).unwrap()])
        .build()
        .unwrap();
    let issuing = fetching.get(&authority.roots).send().await.unwrap();
    let issuing = issuing.bytes().await.unwrap();
    let here = SocketAddr::from(([127, 0, 0, 1], authority.port));
    reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(&issuing).unwrap()])
        .resolve(NAME, here)
        .build()
        .unwrap()
}

/// The status of `server/discover` over HTTPS at `NAME`, from a page of its own.
async fn discovered(client: &reqwest::Client, port: u16) -> u16 {
    client
        .post(format!("https://{NAME}:{port}/"))
        .header("content-type", "application/json")
        .header("origin", format!("https://{NAME}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"server/discover"}"#)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// Waits until the server says it has its certificate; fails on what it says instead.
fn certified(said: &Said) {
    let asked = Instant::now();
    loop {
        let left = PATIENCE.saturating_sub(asked.elapsed());
        let line = said
            .later
            .recv_timeout(left)
            .expect("no certificate in time");
        assert!(!line.contains("could not get a certificate"), "{line}");
        if line.contains("its certificate good until") {
            return;
        }
    }
}

#[test]
fn glave_gets_a_certificate_serves_https_with_it_and_keeps_it_across_a_restart() {
    let Some(authority) = authority() else {
        eprintln!("skipped: set GLAVE_TEST_ACME_DIRECTORY (and the rest) to run the ACME test");
        return;
    };
    let place = place("https");
    let serve = format!(
        "{{ domain = \"{NAME}\", acme = \"{}\" }}",
        authority.directory
    );
    choose(&place, "serve", &serve);
    choose(&place, "titles", "false");
    let model = FakeServer::start(vec![]).unwrap();
    let https = format!("0.0.0.0:{}", authority.port);
    let args = ["--https", https.as_str()];
    // The authority's directory is served under a root of its own, which glave's HTTP client
    // trusts here alone.
    let env = [("SSL_CERT_FILE", authority.root.as_str())];
    let (first, _, said) = serve_http_with(&place, &model, &args, &env);
    let start = said.start.join("\n");
    assert!(start.contains("once its certificate is in hand"), "{start}");
    certified(&said);
    block_on!(async {
        let client = client(&authority).await;
        assert_eq!(discovered(&client, authority.port).await, 200);
    });
    drop(first);

    let (_again, _, said) = serve_http_with(&place, &model, &args, &env);
    let start = said.start.join("\n");
    assert!(
        start.contains(&format!("serving HTTPS at https://{NAME} on")),
        "{start}"
    );
    assert!(
        !start.contains("once its certificate"),
        "the one kept: {start}"
    );
    block_on!(async {
        let client = client(&authority).await;
        assert_eq!(discovered(&client, authority.port).await, 200);
    });
}
