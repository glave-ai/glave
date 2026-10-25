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
    ApiError, ConnectionInfo, ConnectionStatus, Reply, Request, SearchInfo, SignInLink, code,
};
use glave_session::signin::{LocalToken, SignIn};
use glave_session::{Answering, Connections, Returned, Searching};
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
    /// Web search, the person's to turn on or off once it is set; none, not theirs.
    searching: Mutex<Option<Searching>>,
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

    fn search(&self) -> BoxFuture<'_, Result<Option<Searching>, ApiError>> {
        Box::pin(async move { Ok(self.searching.lock().unwrap().clone()) })
    }

    fn set_search(&self, on: bool) -> BoxFuture<'_, Result<Option<Searching>, ApiError>> {
        Box::pin(async move {
            let mut searching = self.searching.lock().unwrap();
            if let Some(searching) = searching.as_mut() {
                searching.on = on;
            }
            Ok(searching.clone())
        })
    }
}

/// The person's server `docs`, signed in to.
fn docs() -> ConnectionInfo {
    ConnectionInfo {
        url: SERVER.into(),
        name: Some("docs".into()),
        status: ConnectionStatus::SignedIn,
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
        let on = Request::SearchSet { on: true };
        for request in [start(SERVER), Request::ConnectionList {}, on.clone()] {
            let refused = client.call(request).await.unwrap_err();
            assert_eq!(refused.code, code::NOT_ALLOWED, "{refused}");
        }
        client.close().await;

        let tested = server(&folder("conn-none"), Vec::new(), Answering::Clients).await;
        for request in [start(SERVER), on] {
            let refused = tested.request(request).await.unwrap_err();
            assert_eq!(refused.code, code::NOT_AVAILABLE);
        }
        let never = tested.request(Request::SearchGet {}).await.unwrap();
        assert!(
            matches!(
                never,
                Reply::Search {
                    search: SearchInfo::Never
                }
            ),
            "not theirs to choose, so not shown: {never:?}"
        );
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
        let call = |tool: &str| answer(None, Some((tool, json!({"query": "rust"}))));
        let mut setup = Setup::new(vec![
            call("mcp__docs__echo"),
            call("mcp__docs__echo"),
            call("mcp__search__web_search_exa"),
            answer(Some("done"), None),
        ]);
        setup.connections = Some(Arc::clone(&connections) as Arc<dyn Connections>);
        let dir = folder("conn-usage");
        let (tested, _) = setup.build(&dir).await;
        let docs_used = |listed: &[ConnectionInfo]| listed.first().and_then(|it| it.usage.clone());
        let none = docs_used(&listed(&tested).await).unwrap();
        assert_eq!((none.calls, &*none.sentence), (0, "No calls this month"));

        let thread = crate::support::start(&tested, false).await;
        turn(&tested, thread, "look it up").await.unwrap();
        let used = docs_used(&listed(&tested).await).unwrap();
        assert_eq!((used.calls, &*used.sentence), (2, "2 calls this month"));
        tested.shutdown().await;
    });
}

/// Web search as the server answers it.
async fn search(tested: &Tested, request: Request) -> SearchInfo {
    match tested.request(request).await.unwrap() {
        Reply::Search { search } => search,
        other => panic!("search, not {other:?}"),
    }
}

#[test]
fn web_search_is_the_person_s_to_turn_on_and_says_how_often_it_searched_this_month() {
    block_on!(async {
        let connections = Arc::new(InMemory::default());
        *connections.searching.lock().unwrap() = Some(Searching {
            on: false,
            name: "search".into(),
        });
        let searched = || {
            answer(
                None,
                Some(("mcp__search__web_search_exa", json!({"query": "x"}))),
            )
        };
        let mut setup = Setup::new(vec![searched(), searched(), answer(Some("done"), None)]);
        setup.connections = Some(Arc::clone(&connections) as Arc<dyn Connections>);
        let dir = folder("search-usage");
        let (tested, _) = setup.build(&dir).await;
        let off = search(&tested, Request::SearchGet {}).await;
        let SearchInfo::Chosen { on, detail, usage } = off else {
            panic!("the person's to choose, not {off:?}");
        };
        assert_eq!(
            (on, &*detail, usage),
            (false, "Uses Exa, free. Your searches go to Exa.", None)
        );
        assert!(
            listed(&tested)
                .await
                .iter()
                .all(|it| it.name.as_deref() != Some("search"))
        );

        let on = search(&tested, Request::SearchSet { on: true }).await;
        let SearchInfo::Chosen {
            on: true,
            usage: Some(usage),
            ..
        } = on
        else {
            panic!("on, with its searches, not {on:?}");
        };
        assert_eq!(
            (usage.calls, &*usage.sentence),
            (0, "No searches this month")
        );
        let thread = crate::support::start(&tested, false).await;
        turn(&tested, thread, "search twice").await.unwrap();
        let counted = search(&tested, Request::SearchGet {}).await;
        let SearchInfo::Chosen {
            usage: Some(usage), ..
        } = counted
        else {
            panic!("counted, not {counted:?}");
        };
        assert_eq!(
            (usage.calls, &*usage.sentence),
            (2, "2 searches this month")
        );
        let off = search(&tested, Request::SearchSet { on: false }).await;
        assert!(
            matches!(
                off,
                SearchInfo::Chosen {
                    on: false,
                    usage: None,
                    ..
                }
            ),
            "{off:?}"
        );
        tested.shutdown().await;
    });
}
