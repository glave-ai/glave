//! Connections through the session API: signing in to one sends the browser back to this server, on
//! its own address on this computer or where people reach it from elsewhere, and the browser,
//! signed in to nothing of glave's, finishes it there; then it is listed, never with a token,
//! and removed. Only the person on this computer may do this: a caller signed in with OIDC is
//! refused, a server whose sign-ins someone else sets up refuses everyone, and one no browser
//! can come back to says so. Each connection in the person's settings says how often it was
//! called this month; one glave suggests says what it is for.

use std::path::Path;
use std::sync::{Arc, Mutex};

use glave_ports::BoxFuture;
use glave_session::protocol::{
    ApiError, ConnectionInfo, ConnectionStatus, Reply, Request, SignInLink, code,
};
use glave_session::signin::{LocalToken, SignIn};
use glave_session::{Answering, Connections, Returned, Suggestion};
use serde_json::json;

use crate::conformance::{Client, OverHttp};
use crate::support::issuer::fixed_clock;
use crate::support::web::{Web, serve_with};
use crate::support::*;

/// The server the fake signs in to.
const SERVER: &str = "https://mcp.example.com/mcp";
/// An address none of the person's servers is at.
const NOT_MINE: &str = "https://plain.example.com/mcp";

/// Connections kept in memory: a sign-in's page names where the browser comes back to, and the
/// browser that comes back with its state finishes it.
#[derive(Default)]
pub(crate) struct InMemory {
    redirects: Mutex<Vec<Option<String>>>,
    kept: Mutex<Vec<ConnectionInfo>>,
}

impl Connections for InMemory {
    fn start(
        &self,
        server: String,
        redirect: Option<String>,
    ) -> BoxFuture<'_, Result<Option<SignInLink>, ApiError>> {
        Box::pin(async move {
            if server == NOT_MINE {
                return Err(ApiError::new(code::NO_SERVER, "not one of theirs"));
            }
            self.redirects.lock().unwrap().push(redirect.clone());
            let Some(redirect) = redirect else {
                return Ok(None);
            };
            let page =
                format!("https://auth.example.com/authorize?state=s1&redirect_uri={redirect}");
            Ok(Some(SignInLink {
                flow: "flow-1".into(),
                url: page,
            }))
        })
    }

    fn poll(&self, flow: String) -> BoxFuture<'_, Result<Option<ConnectionInfo>, ApiError>> {
        Box::pin(async move {
            if flow != "flow-1" {
                return Err(ApiError::new(code::NO_CREDENTIAL, "no such sign-in"));
            }
            Ok(self.kept.lock().unwrap().first().cloned())
        })
    }

    fn finish(&self, returned: Returned) -> BoxFuture<'_, String> {
        Box::pin(async move {
            match (returned.state.as_str(), returned.code, returned.error) {
                ("s1", Some(_), None) => {
                    self.kept.lock().unwrap().push(docs());
                    "Signed in to docs.".into()
                }
                (_, _, Some(error)) => format!("Not signed in: {error}."),
                _ => "glave is not waiting for that sign-in.".into(),
            }
        })
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<ConnectionInfo>, ApiError>> {
        Box::pin(async move { Ok(self.kept.lock().unwrap().clone()) })
    }

    fn add(&self, url: String) -> BoxFuture<'_, Result<ConnectionInfo, ApiError>> {
        Box::pin(async move {
            let suggestion = Suggestion::search(None);
            if url != suggestion.url {
                return Err(ApiError::new(
                    code::NO_SERVER,
                    "glave suggests no such server",
                ));
            }
            let on = suggestion.listed(ConnectionStatus::On);
            let mut kept = self.kept.lock().unwrap();
            kept.retain(|connection| connection.url != on.url);
            kept.push(on.clone());
            Ok(on)
        })
    }

    fn remove(&self, server: String) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let mut kept = self.kept.lock().unwrap();
            let before = kept.len();
            kept.retain(|connection| connection.url != server);
            if kept.len() == before {
                return Err(ApiError::new(code::NO_CREDENTIAL, "no such connection"));
            }
            Ok(())
        })
    }
}

/// The person's server `docs`, signed in to.
fn docs() -> ConnectionInfo {
    ConnectionInfo {
        url: SERVER.into(),
        name: Some("docs".into()),
        status: ConnectionStatus::SignedIn,
        suggested: None,
        usage: None,
    }
}

/// A server whose connections the fake keeps.
async fn fresh(dir: &Path) -> (Tested, Arc<InMemory>) {
    let connections = Arc::new(InMemory::default());
    let mut setup = Setup::new(Vec::new());
    setup.connections = Some(Arc::clone(&connections) as Arc<dyn Connections>);
    (setup.build(dir).await.0, connections)
}

fn start(url: &str) -> Request {
    Request::ConnectionStart { server: url.into() }
}

/// The browser coming back to `served` from the authorization server, with `query`.
async fn back(base: &str, query: &str) -> (u16, String, String) {
    let answer = reqwest::get(format!("{base}connect/callback?{query}"))
        .await
        .unwrap();
    let status = answer.status().as_u16();
    assert_eq!(answer.headers()["x-content-type-options"], "nosniff");
    assert_eq!(answer.headers()["cache-control"], "no-store");
    let kind = answer.headers()["content-type"]
        .to_str()
        .unwrap()
        .to_owned();
    (status, kind, answer.text().await.unwrap())
}

#[test]
fn the_browser_comes_back_to_this_server_and_finishes_the_sign_in() {
    block_on!(async {
        let token = LocalToken::new([7; 32]);
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let dir = folder("conn-http");
        let (tested, connections) = fresh(&dir).await;
        let served = serve_with(tested, signin, None).await;
        let mut web = Web::new(&served, Some(token.reveal().to_owned()));

        let refused = web.call(&start(NOT_MINE)).await;
        assert_eq!(refused["error"]["code"], code::NO_SERVER, "{refused}");
        let started = web.call(&start(SERVER)).await;
        let link = &started["result"]["sign_in"];
        assert_eq!(link["flow"], "flow-1", "{started}");
        let callback = format!("http://{}/connect/callback", served.address);
        assert_eq!(*connections.redirects.lock().unwrap(), [Some(callback)]);
        let poll = Request::ConnectionPoll {
            flow: "flow-1".into(),
        };
        let waiting = web.call(&poll).await;
        assert_eq!(waiting["result"]["flow"], "flow-1", "{waiting}");

        let (status, kind, said) = back(&served.url, "error=access_denied&state=s1").await;
        assert_eq!(
            (status, said.as_str()),
            (200, "Not signed in: access_denied.")
        );
        assert_eq!(kind, "text/plain; charset=utf-8");
        // A prefetch's HEAD finishes nothing.
        let prefetch = format!("{}connect/callback?code=c1&state=s1", served.url);
        let peeked = reqwest::Client::new().head(prefetch).send().await.unwrap();
        assert_ne!(peeked.status(), 200);
        assert!(
            connections.kept.lock().unwrap().is_empty(),
            "nothing finished"
        );
        let (_, _, said) = back(&served.url, "code=c1&state=s1").await;
        assert_eq!(said, "Signed in to docs.");
        let connected = web.call(&poll).await;
        let connection = &connected["result"]["connection"];
        assert_eq!(connection["url"], SERVER, "{connected}");
        assert_eq!(connection["status"], "signed_in");
        let listed = web.call(&Request::ConnectionList {}).await;
        assert_eq!(
            listed["result"]["connections"][0]["name"], "docs",
            "{listed}"
        );

        let remove = Request::ConnectionRemove {
            server: SERVER.into(),
        };
        assert_eq!(web.call(&remove).await["result"], serde_json::json!({}));
        let again = web.call(&remove).await;
        assert_eq!(again["error"]["code"], code::NO_CREDENTIAL, "{again}");
        served.close().await;
    });
}

#[test]
fn a_sign_in_comes_back_where_people_reach_the_server_from_elsewhere() {
    block_on!(async {
        let connections = Arc::new(InMemory::default());
        let mut setup = Setup::new(Vec::new());
        setup.connections = Some(Arc::clone(&connections) as Arc<dyn Connections>);
        setup.origin = glave_ports::host::PublicOrigin::parse("https://glave.example.com");
        let tested = setup.build(&folder("conn-origin")).await.0;
        // Not listening over HTTP, it is not where the browser comes back to.
        let refused = tested.request(start(SERVER)).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE, "{refused:?}");
        let token = LocalToken::new([7; 32]);
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let served = serve_with(tested, signin, None).await;
        let mut web = Web::new(&served, Some(token.reveal().to_owned()));
        let started = web.call(&start(SERVER)).await;
        assert!(started["error"].is_null(), "{started}");
        let back = "https://glave.example.com/connect/callback".to_owned();
        assert_eq!(*connections.redirects.lock().unwrap(), [None, Some(back)]);
        served.close().await;
    });
}

#[test]
fn a_server_no_browser_can_come_back_to_gives_its_host_no_way_back_and_says_why() {
    block_on!(async {
        let (tested, connections) = fresh(&folder("conn-in-process")).await;
        let refused = tested.request(start(SERVER)).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        let said = refused.message.to_string();
        assert!(
            said.contains("needs glave running in the background"),
            "{said}"
        );
        assert_eq!(*connections.redirects.lock().unwrap(), [None]);
        let mine = tested.request(start(NOT_MINE)).await.unwrap_err();
        assert_eq!(mine.code, code::NO_SERVER, "not theirs comes first");
        tested.shutdown().await;
    });
}

#[test]
fn only_the_person_on_this_computer_signs_in_to_connections() {
    block_on!(async {
        let (tested, _) = fresh(&folder("conn-oidc")).await;
        let mut client = OverHttp::new(tested).await;
        let add = Request::ConnectionAdd {
            url: Suggestion::search(None).url,
        };
        for request in [start(SERVER), Request::ConnectionList {}, add.clone()] {
            let refused = client.call(request).await.unwrap_err();
            assert_eq!(refused.code, code::NOT_ALLOWED, "{refused}");
        }
        client.close().await;

        let tested = server(&folder("conn-none"), Vec::new(), Answering::Clients).await;
        for request in [start(SERVER), add] {
            let refused = tested.request(request).await.unwrap_err();
            assert_eq!(refused.code, code::NOT_AVAILABLE);
        }
        tested.shutdown().await;
    });
}

/// The connections the server lists.
async fn listed(tested: &Tested) -> Vec<ConnectionInfo> {
    match tested.request(Request::ConnectionList {}).await.unwrap() {
        Reply::Connections { connections } => connections,
        other => panic!("connections, not {other:?}"),
    }
}

#[test]
fn each_connection_in_the_person_s_settings_says_how_often_it_was_called_this_month() {
    block_on!(async {
        let connections = Arc::new(InMemory::default());
        connections.kept.lock().unwrap().push(docs());
        let search = Suggestion::search(None);
        let off = search.listed(ConnectionStatus::Off);
        connections.kept.lock().unwrap().push(off);
        let call = |text: &str| answer(None, Some(("mcp__docs__echo", json!({"text": text}))));
        let mut setup = Setup::new(vec![call("one"), call("two"), answer(Some("done"), None)]);
        setup.connections = Some(Arc::clone(&connections) as Arc<dyn Connections>);
        let dir = folder("conn-usage");
        let (tested, _) = setup.build(&dir).await;
        let before = listed(&tested).await;
        let usage = |connections: &[ConnectionInfo], name: &str| {
            let found = connections
                .iter()
                .find(|it| it.name.as_deref() == Some(name));
            found.and_then(|it| it.usage.clone())
        };
        let none = usage(&before, "docs").unwrap();
        assert_eq!((none.calls, &*none.sentence), (0, "No calls this month"));
        let suggested = before.iter().find(|it| it.suggested.is_some()).unwrap();
        let what = suggested.suggested.as_ref().unwrap();
        assert_eq!(&*what.title, "Search the web");
        assert!(what.detail.contains("Exa"), "{}", &*what.detail);
        assert_eq!(suggested.usage, None, "off, it was never used");

        let thread = crate::support::start(&tested, false).await;
        turn(&tested, thread, "look in the docs twice")
            .await
            .unwrap();
        let after = listed(&tested).await;
        let used = usage(&after, "docs").unwrap();
        assert_eq!((used.calls, &*used.sentence), (2, "2 calls this month"));

        let added = tested
            .request(Request::ConnectionAdd { url: search.url })
            .await
            .unwrap();
        let Reply::Connection { connection } = added else {
            panic!("the connection, not {added:?}");
        };
        assert_eq!(connection.status, ConnectionStatus::On);
        let on = connection.usage.unwrap();
        assert_eq!((on.calls, &*on.sentence), (0, "No calls this month"));
        tested.shutdown().await;
    });
}
