//! Connections end to end: the real binary serves the person over HTTP in a fresh home, with a
//! remote MCP server in their `mcp.json` that asks them to sign in. They start signing in
//! through the session API, approve glave in the browser at a fake authorization server, and
//! the browser comes back to their server, which keeps the sign-in sealed. From the next turn
//! the server is signed in as them, its token renewed before it expires; removed, it is signed
//! out again. No token, refresh token or code shows in an answer or in what the server says.

use std::process::{Command, Stdio};

use glave_ports::testing::{FakeAuthServer, FakeGitHost, Grant, Running, fake_secret};

use glave_provider_openai::fake::{FakeServer, Reply};
use glave_session::Sent;
use glave_session::client::Remote;
use glave_session::protocol::Request;
use serde_json::{Value, json};

use crate::http::{only_the_server_answers, serve_http_with};
use crate::served::holds;
use crate::support::*;

/// The remote server, asking for a sign-in at `issuer` and letting in `tokens`, and its
/// address.
fn remote_server(issuer: &str, tokens: &[&str]) -> (Running, String) {
    let mut args = vec!["--sign-in", issuer];
    for token in tokens {
        args.extend(["--token", token]);
    }
    serving(&args)
}

/// The test server over HTTP with `args`, and its address.
fn serving(args: &[&str]) -> (Running, String) {
    let child = Command::new(crate::mcp::test_server())
        .arg("--http")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut running = Running::new(child);
    let mut url = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(running.stdout.take().unwrap()),
        &mut url,
    )
    .unwrap();
    (running, url.trim().to_owned())
}

/// The session API of the person's server at `url`, signed in with its local token.
struct Api {
    remote: Remote,
    runtime: tokio::runtime::Runtime,
}

impl Api {
    fn new(place: &Place, url: &str) -> Self {
        let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();
        Self {
            remote: Remote::new(url, token.trim().to_owned()).unwrap(),
            runtime: glave_ports::testing::runtime().unwrap(),
        }
    }

    /// `method` with `params`: its result, or its error, as JSON.
    fn call(&self, method: &str, params: Value) -> Value {
        let request = request(method, params);
        match self.runtime.block_on(self.remote.request(&request)) {
            Ok(reply) => json!({"result": reply}),
            Err(error) => json!({"error": error}),
        }
    }

    /// Runs a turn of `thread` to its end.
    fn turn(&self, thread: &Value, prompt: &str) {
        let request = request("turn/start", json!({"thread": thread, "prompt": prompt}));
        self.runtime.block_on(async {
            let Ok(Sent::Started(mut turn)) = self.remote.start(&request).await else {
                panic!("a turn of its own");
            };
            while turn.next().await.is_some() {}
        });
    }

    /// What the browser opening `page` is sent to next.
    fn followed(&self, page: &str) -> String {
        self.runtime.block_on(async {
            let browser = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let answer = browser.get(page).send().await.unwrap();
            let location = answer
                .headers()
                .get("location")
                .map(|at| at.to_str().unwrap());
            location.unwrap_or_default().to_owned()
        })
    }

    /// What the browser opening `page` is shown.
    fn shown(&self, page: &str) -> String {
        self.runtime
            .block_on(async { reqwest::get(page).await.unwrap().text().await.unwrap() })
    }
}

fn request(method: &str, params: Value) -> Request {
    let message =
        serde_json::Map::from_iter([("method".into(), method.into()), ("params".into(), params)]);
    serde_json::from_value(Value::Object(message)).unwrap()
}

/// Whether the model was offered the remote server's `whose` in `body`.
fn offered_whose(body: &Value) -> bool {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["function"]["name"] == "mcp__docs__whose")
}

/// The person's server, in a fresh home whose `mcp.json` names the remote server `docs`, which
/// asks for a sign-in at a fake authorization server granting `grants`, and lets in their
/// tokens; with a model answering `replies`.
///
/// Fields drop in the order they are declared, so the place goes last: the server is killed
/// and waited for before its folder is removed.
struct World {
    api: Api,
    _served: Running,
    said: crate::http::Said,
    url: String,
    model: FakeServer,
    _remote: Running,
    address: String,
    auth: FakeAuthServer,
    place: Place,
}

impl World {
    fn new(name: &str, grants: &[Grant], replies: Vec<Reply>) -> Self {
        let place = place(name);
        let auth = FakeAuthServer::start(grants.to_vec()).unwrap();
        let tokens: Vec<&str> = grants.iter().map(|grant| grant.token.as_str()).collect();
        let (remote, address) = remote_server(&auth.url, &tokens);
        std::fs::create_dir_all(&place.home).unwrap();
        let servers = json!({"mcpServers": {"docs": {"url": address}}});
        std::fs::write(place.home.join("mcp.json"), servers.to_string()).unwrap();
        choose(&place, "memory", "\"off\"");
        choose(&place, "permissions", "\"full-access\"");
        let model = FakeServer::start(replies).unwrap();
        let (served, url, said) = serve_http_with(&place, &model, &["--model", "m"], &[]);
        let api = Api::new(&place, &url);
        Self {
            api,
            _served: served,
            said,
            url,
            model,
            _remote: remote,
            address,
            auth,
            place,
        }
    }

    /// Starts signing in to `server`, and opens its page as the person: the flow, and where
    /// the browser is sent back to.
    fn signing_in(&self, server: &str) -> (Value, String) {
        let started = self.api.call("connection/start", json!({"server": server}));
        let link = started["result"]["sign_in"].clone();
        let page = link["url"].as_str().unwrap_or_else(|| panic!("{started}"));
        assert!(
            page.starts_with(&format!("{}/authorize?", self.auth.url)),
            "{page}"
        );
        let back = self.api.followed(page);
        let callback = format!("{}/connect/callback?", self.url);
        assert!(back.starts_with(&callback), "{back}");
        (link["flow"].clone(), back)
    }

    /// The renewals the authorization server was asked for.
    fn renewals(&self) -> Vec<String> {
        let asked = self.auth.received().into_iter();
        let renewing = |(_, path, body): &(String, String, String)| {
            path == "/token" && body.contains("grant_type=refresh_token")
        };
        asked.filter(renewing).map(|(_, _, body)| body).collect()
    }
}

/// The sign-in's grants: a token that lasts a minute, so it is renewed before its first use,
/// and the one renewing it gives.
fn grants() -> Vec<Grant> {
    [60, 3600]
        .map(|lasts| Grant {
            token: fake_secret("mcp_"),
            refresh: Some(fake_secret("mcr_")),
            lasts: Some(lasts),
        })
        .into()
}

#[test]
fn a_connection_signed_in_in_the_browser_signs_the_server_in_renewed_until_removed() {
    let replies = vec![
        Reply::text("Not signed in yet."),
        Reply::tool_call("call_1", "mcp__docs__whose", &json!({})),
        Reply::text("Signed in."),
        Reply::text("Signed out again."),
    ];
    let grants = grants();
    let world = World::new("conn", &grants, replies);
    let api = &world.api;
    let thread = api.call("thread/start", json!({}))["result"]["thread"]["id"].clone();
    api.turn(&thread, "who am I there?");
    assert!(!offered_whose(&world.model.received()[0].body));

    let asking = api.call("connection/list", json!({}));
    let signed_out = json!([{"url": world.address, "name": "docs", "status": "signed_out"}]);
    assert_eq!(asking["result"]["connections"], signed_out, "{asking}");

    let elsewhere = json!({"server": "https://elsewhere.example.com/mcp"});
    let refused = api.call("connection/start", elsewhere);
    assert_eq!(
        refused["error"]["code"], -32032,
        "not one of theirs: {refused}"
    );
    let (flow, back) = world.signing_in("docs");
    let shown = api.shown(&back);
    assert_eq!(
        shown,
        "Signed in to docs. You can close this tab and go back to glave."
    );
    let connected = api.call("connection/poll", json!({"flow": flow}));
    let connection = &connected["result"]["connection"];
    assert_eq!(
        (&connection["name"], &connection["url"]),
        (&json!("docs"), &json!(world.address))
    );
    assert_eq!(
        api.shown(&back),
        shown,
        "the page loaded again trades nothing"
    );
    let listed = api.call("connection/list", json!({}));
    assert_eq!(
        listed["result"]["connections"][0]["status"], "signed_in",
        "{listed}"
    );

    api.turn(&thread, "and now?");
    let asked = world.model.received();
    assert!(offered_whose(&asked[1].body), "it started again, signed in");
    let results = said(&asked[2].body, "tool").concat();
    assert!(
        results.contains("signed in with token 2"),
        "renewed first: {results}"
    );
    let renewals = world.renewals();
    assert_eq!(
        renewals.len(),
        1,
        "a refresh token is spent once: {renewals:?}"
    );
    let spent = grants[0].refresh.as_deref().unwrap();
    assert!(renewals[0].contains(spent));

    let removed = api.call("connection/remove", json!({"server": world.address}));
    assert!(removed["error"].is_null(), "{removed}");
    let again = api.call("connection/list", json!({}));
    assert_eq!(again["result"]["connections"], signed_out, "{again}");
    api.turn(&thread, "still there?");
    assert!(!offered_whose(&world.model.received()[3].body));

    let told: Vec<String> = world.said.later.try_iter().collect();
    shows_no_secret(
        &format!("{connected} {listed} {removed} {shown} {told:?}"),
        &grants,
    );
    assert!(
        !holds(&world.place.home, spent),
        "sealed at rest, and in no log"
    );
}

/// Asserts `everything` shows none of the tokens and refresh tokens of `grants`, nor a code.
fn shows_no_secret(everything: &str, grants: &[Grant]) {
    let secrets = grants
        .iter()
        .flat_map(|grant| [Some(&grant.token), grant.refresh.as_ref()]);
    for secret in secrets.flatten() {
        assert!(
            !everything.contains(secret.as_str()),
            "a secret shows: {everything}"
        );
    }
    assert!(
        !everything.contains("code="),
        "the code shows: {everything}"
    );
}

#[test]
fn a_sign_in_the_person_declines_keeps_nothing() {
    let world = World::new("conn-no", &grants(), Vec::new());
    world.auth.refuse();
    let (flow, back) = world.signing_in(&world.address);
    assert!(back.contains("error=access_denied"), "{back}");
    let shown = world.api.shown(&back);
    assert_eq!(
        shown,
        "glave was not signed in to docs: the sign-in was declined. You can close this tab."
    );
    let polled = world.api.call("connection/poll", json!({"flow": flow}));
    assert_eq!(polled["error"]["code"], -32025, "{polled}");
    let none = world.api.call("connection/list", json!({}));
    let status = &none["result"]["connections"][0]["status"];
    assert_eq!(status, "signed_out", "{none}");
    let again = world.api.shown(&back);
    assert_eq!(again, shown, "the page loaded again says the same");
    let never = world.api.shown(&back.replace("state=", "state=x"));
    assert!(never.starts_with("glave is no longer waiting"), "{never}");
}

#[test]
fn glave_connect_signs_in_to_a_server_and_connections_lists_and_removes_it() {
    let world = World::new("conn-cli", &grants(), Vec::new());
    let (place, model) = (&world.place, &world.model);
    only_the_server_answers(place);
    let mut connecting = start(place, model, &["connect", "docs"]);
    let mut out = std::io::BufReader::new(connecting.stdout.take().unwrap());
    let mut line = String::new();
    std::io::BufRead::read_line(&mut out, &mut line).unwrap();
    let page = line
        .trim()
        .strip_prefix("Open ")
        .unwrap_or_else(|| panic!("{line}"));
    let page = page.strip_suffix(" to sign in").unwrap();
    let back = world.api.followed(page);
    assert!(world.api.shown(&back).starts_with("Signed in to docs."));
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut out, &mut rest).unwrap();
    assert!(connecting.wait().unwrap().success(), "{rest}");
    let connected = format!("Connected docs ({}), signed in", world.address);
    assert!(rest.contains(&connected), "{rest}");

    let listed = text(&glave(place, model, &["connections"], ""));
    assert!(
        listed.contains(&format!("docs ({}), signed in", world.address)),
        "{listed}"
    );
    let removed = glave(place, model, &["connections", "remove", "docs"], "");
    let said = text(&removed);
    assert!(
        removed.status.success() && said.contains("Removed docs"),
        "{said}"
    );
    let listed = text(&glave(place, model, &["connections"], ""));
    let asking = format!("docs ({}), not signed in", world.address);
    assert!(
        listed.contains(&asking) && listed.contains("Sign in with glave connect docs"),
        "it asks again: {listed}"
    );

    let host = glave(place, model, &["connect", "https://git.example.com"], "");
    let said = text(&host);
    assert!(
        !host.status.success() && said.contains("no token given"),
        "a git host's: {said}"
    );
}

#[test]
fn with_no_server_running_a_git_host_still_takes_its_token_and_a_server_says_what_it_needs() {
    let place = place("conn-alone");
    let token = fake_secret("gta_");
    let host = FakeGitHost::start("ana", &token).unwrap();
    std::fs::create_dir_all(&place.home).unwrap();
    let servers = json!({"mcpServers": {"docs": {"url": "https://mcp.example.com/mcp"}}});
    std::fs::write(place.home.join("mcp.json"), servers.to_string()).unwrap();
    let model = FakeServer::start(Vec::new()).unwrap();
    // No one is at the terminal and no server runs: each answers in its own process.
    let piped = glave(
        &place,
        &model,
        &["connect", &host.url],
        &format!("{token}\n"),
    );
    let said = text(&piped);
    assert!(
        piped.status.success() && said.contains("Connected"),
        "{said}"
    );
    let asked = glave(&place, &model, &["connect", "docs"], "");
    let said = text(&asked);
    assert!(!asked.status.success(), "{said}");
    assert!(
        said.contains("needs glave running in the background"),
        "{said}"
    );
    assert!(!said.contains(&token), "{said}");
}

#[test]
fn only_the_servers_that_ask_for_a_sign_in_are_listed_before_one() {
    let world = World::new("conn-list", &grants(), Vec::new());
    let (_open, open) = serving(&[]);
    // Asking for a sign-in too, but given a key of the person's own.
    let (_keyed, keyed) = remote_server(&world.auth.url, &[]);
    let gone = "http://127.0.0.1:1/mcp";
    let key = format!("Bearer {}", fake_secret("key_"));
    let servers = json!({"mcpServers": {
        "docs": {"url": world.address},
        "open": {"url": open},
        "keyed": {"url": keyed, "headers": {"authorization": key}},
        "gone": {"url": gone},
        "local": {"command": crate::mcp::test_server()},
    }});
    std::fs::write(world.place.home.join("mcp.json"), servers.to_string()).unwrap();
    let listed = world.api.call("connection/list", json!({}));
    assert_eq!(
        listed["result"]["connections"],
        json!([{"url": world.address, "name": "docs", "status": "signed_out"}]),
        "not one that asks no one, has a key, can't be reached or runs here: {listed}"
    );
}
