//! glave as the installer sets it up on a server (`install/server.sh`), signed in to from
//! elsewhere (`specs/one-server.md`, "On a server"): the link the installer printed makes a
//! passkey, once, and the passkey signs a browser in again, over the HTTPS glave serves with
//! the certificate it got itself, or through the tunnel on the server. CI's install jobs run it
//! on the server they installed glave on, naming in `GLAVE_TEST_INSTALLED` the file of what the
//! installer printed and, where glave serves HTTPS itself, in `GLAVE_TEST_INSTALLED_ROOT` the
//! root its certificate chains to; it skips without them.

use std::net::SocketAddr;

use glave_identity::testing::Authenticator;
use glave_ports::block_on;
use serde_json::{Value, json};

/// Where the tunnel on the server forwards requests to: glave's loopback listener.
const LOOPBACK: &str = "http://127.0.0.1:4180";

/// A browser elsewhere, on a page at glave's public origin.
struct Browser {
    http: reqwest::Client,
    origin: String,
    /// Where its requests go: the origin, or the loopback listener a tunnel forwards them to.
    url: String,
    session: Option<String>,
}

impl Browser {
    /// It reaches glave's name on this server over the HTTPS glave serves itself, trusting
    /// only `root`; without one, through the tunnel on this server, which forwards each request
    /// to the loopback listener with the name it came to, as Tailscale does.
    fn new(origin: &str, root: Option<&[u8]>) -> Self {
        let name = origin.strip_prefix("https://").unwrap();
        let (http, url) = match root {
            Some(root) => (
                reqwest::Client::builder()
                    .tls_certs_only([reqwest::Certificate::from_pem(root).unwrap()])
                    .resolve(name, SocketAddr::from(([127, 0, 0, 1], 443)))
                    .build()
                    .unwrap(),
                origin.to_owned(),
            ),
            None => (reqwest::Client::new(), LOOPBACK.to_owned()),
        };
        Self {
            http,
            origin: origin.to_owned(),
            url,
            session: None,
        }
    }

    /// What `method` with `params` is answered with: its result, or its error's code.
    async fn ask(&self, method: &str, params: Value) -> Value {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let name = self.origin.strip_prefix("https://").unwrap();
        let mut post = self
            .http
            .post(format!("{}/", self.url))
            .header("host", name)
            .header("origin", &self.origin)
            .header("glave-method", method)
            .json(&message);
        if let Some(session) = &self.session {
            post = post.bearer_auth(session);
        }
        let answered: Value = post.send().await.unwrap().json().await.unwrap();
        match answered.get("result") {
            Some(result) => result.clone(),
            None => answered["error"]["code"].clone(),
        }
    }

    /// Answers the request `asked` began with `credential`, made or used on a device, through
    /// `answered`, and keeps the session that began.
    async fn signed_in(&mut self, asked: &Value, answered: &str, credential: Value) {
        let ceremony = asked["ceremony"].clone();
        let params = json!({"ceremony": ceremony, "credential": credential});
        let began = self.ask(answered, params).await;
        let session = began["session"]
            .as_str()
            .unwrap_or_else(|| panic!("{began}"));
        self.session = Some(session.to_owned());
    }
}

/// The setup link in what the installer printed: its origin and its code.
fn setup_link(printed: &str) -> (String, String) {
    let link = printed
        .split_whitespace()
        .find(|word| word.contains("/#setup="))
        .unwrap_or_else(|| panic!("no setup link in {printed}"));
    let (origin, code) = link.split_once("/#setup=").unwrap();
    (origin.to_owned(), code.to_owned())
}

#[test]
fn the_installers_link_makes_a_passkey_once_and_the_passkey_signs_in_again() {
    let Ok(printed) = std::env::var("GLAVE_TEST_INSTALLED") else {
        eprintln!("skipped: set GLAVE_TEST_INSTALLED (and GLAVE_TEST_INSTALLED_ROOT) to run it");
        return;
    };
    let printed = std::fs::read_to_string(printed).unwrap();
    let (origin, code) = setup_link(&printed);
    let root = std::env::var("GLAVE_TEST_INSTALLED_ROOT")
        .ok()
        .map(|root| std::fs::read(root).unwrap());
    block_on!(async {
        let mut phone = Browser::new(&origin, root.as_deref());
        let discovered = phone.ask("server/discover", json!({})).await;
        assert_eq!(discovered["origin"], origin.as_str());
        assert_eq!(discovered["signin"], json!(["link", "passkey"]));

        let mut device = Authenticator::new(&origin).unwrap();
        let asked = phone
            .ask("passkey/create_options", json!({"code": code}))
            .await;
        let credential = device.create(&asked["options"]).unwrap();
        phone.signed_in(&asked, "passkey/create", credential).await;
        let listed = phone.ask("passkey/list", json!({})).await;
        assert_eq!(listed["passkeys"].as_array().unwrap().len(), 1, "{listed}");
        let again = phone
            .ask("passkey/create_options", json!({"code": code}))
            .await;
        assert!(again.is_number(), "the link works once: {again}");

        let mut laptop = Browser::new(&origin, root.as_deref());
        let asked = laptop.ask("passkey/get_options", json!({})).await;
        let credential = device.get(&asked["options"]).unwrap();
        laptop.signed_in(&asked, "passkey/get", credential).await;
        let listed = laptop.ask("passkey/list", json!({})).await;
        assert!(listed["passkeys"][0]["used"].is_string(), "{listed}");
    });
}
