//! Signing in to a remote server as MCP's authorization says, against a fake authorization
//! server and the test server asking for a sign-in, on this computer: the 401 that asks, the
//! sign-in through the person's browser, the token on every request, asked for again each
//! time, and its renewal, each refresh token spent once.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::Arguments;
use glave_mcp::{Limits, McpError, McpServer, SignIn, SignInError, renew};
use glave_ports::auth::{Auth, AuthError, Header, Outgoing, RequestSigner};
use glave_ports::testing::{FAKE_CLIENT, FakeAuthServer, FakeHttp, Grant, fake_secret};
use glave_ports::{BoxFuture, NeverCancelled, ToolSource as _, block_on, conformance};

use crate::http::Served;

/// Where the browser would come back to; never visited here.
const REDIRECT: &str = "http://127.0.0.1:1/connect/callback";

/// The tokens the fake grants, and their refresh tokens: the sign-in's, then the renewal's.
struct Tokens {
    tokens: [String; 2],
    refreshes: [String; 2],
}

impl Tokens {
    fn new() -> Self {
        Self {
            tokens: [fake_secret("mcp_"), fake_secret("mcp_")],
            refreshes: [fake_secret("mcr_"), fake_secret("mcr_")],
        }
    }

    fn grants(&self) -> Vec<Grant> {
        self.tokens
            .iter()
            .zip(&self.refreshes)
            .map(|(token, refresh)| Grant {
                token: token.clone(),
                refresh: Some(refresh.clone()),
                lasts: Some(3600),
            })
            .collect()
    }

    /// The test server, letting in either token.
    fn served(&self, auth: &FakeAuthServer) -> Served {
        let [one, two] = &self.tokens;
        Served::with(&["--sign-in", &auth.url, "--token", one, "--token", two])
    }

    fn all(&self) -> impl Iterator<Item = &String> {
        self.tokens.iter().chain(&self.refreshes)
    }
}

/// Proves who is calling with whichever token it holds now.
struct Current(Mutex<String>);

impl Current {
    fn set(&self, token: &str) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = token.to_owned();
    }
}

impl RequestSigner for Current {
    fn authorize<'a>(
        &'a self,
        _request: &'a Outgoing<'a>,
    ) -> BoxFuture<'a, Result<Vec<Header>, AuthError>> {
        let token = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Box::pin(async move {
            Ok(vec![Header {
                name: "authorization".into(),
                value: glave_event::Secret::new(format!("Bearer {token}")),
            }])
        })
    }

    fn describe(&self) -> String {
        "the token held now".to_owned()
    }
}

fn remote(url: &str, auth: Auth) -> McpServer {
    McpServer::remote("docs", url, &BTreeMap::new(), auth, Limits::default())
}

/// What `server`'s `whose` says.
async fn whose(server: &McpServer) -> String {
    let none = Arguments::Json(serde_json::Map::new());
    let said = server
        .call("mcp__docs__whose", &none, &NeverCancelled)
        .await;
    match said.parts.as_slice() {
        [glave_event::Part::Text { text }] => text.clone(),
        other => panic!("not one text part: {other:?}"),
    }
}

/// Where the authorization server sends the person's browser back to from `page`, as the
/// person approving glave there.
async fn approved(page: &str) -> reqwest::Url {
    let browser = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let answer = browser.get(page).send().await.unwrap();
    assert_eq!(answer.status(), 302, "{page}");
    let location = answer.headers()["location"].to_str().unwrap();
    reqwest::Url::parse(location).unwrap()
}

fn field(url: &reqwest::Url, name: &str) -> String {
    url.query_pairs()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

#[test]
fn a_person_signs_in_and_their_token_goes_with_every_request_renewed() {
    let tokens = Tokens::new();
    let auth = FakeAuthServer::start(tokens.grants()).unwrap();
    let listening = tokens.served(&auth);
    let [one, two] = &tokens.tokens;
    let [first_refresh, second_refresh] = &tokens.refreshes;
    block_on!(async {
        let unsigned = remote(&listening.url, Auth::None);
        let Err(McpError::Start { message, .. }) = unsigned.started().await else {
            panic!("it started without a sign-in");
        };
        assert_eq!(
            message,
            "it asks you to sign in to it: connect it, then try again"
        );

        let (signin, page) = SignIn::start(&listening.url, "docs", REDIRECT)
            .await
            .unwrap();
        assert!(
            page.starts_with(&format!("{}/authorize?", auth.url)),
            "{page}"
        );
        for part in [
            "code_challenge_method=S256",
            "client_id=fake-client",
            "resource=",
        ] {
            assert!(page.contains(part), "{part} in {page}");
        }
        let back = approved(&page).await;
        assert!(back.as_str().starts_with(REDIRECT), "{back}");
        assert_eq!(field(&back, "state"), signin.state());
        let granted = signin.finish(&field(&back, "code"), None).await.unwrap();
        assert_eq!(granted.token.expose(), one.as_str());
        let refresh = granted.refresh.as_ref().unwrap();
        assert_eq!(refresh.expose(), first_refresh.as_str());
        assert_eq!(granted.lasts, Some(Duration::from_secs(3600)));
        assert_eq!(granted.client.client_id, FAKE_CLIENT);
        assert_eq!(granted.client.issuer.as_deref(), Some(auth.url.as_str()));

        let current = Arc::new(Current(Mutex::new(one.clone())));
        let server = remote(&listening.url, Auth::Changing(Arc::clone(&current) as _));
        assert_eq!(conformance::tool_source(&server).await, Ok(()));
        assert_eq!(whose(&server).await, "signed in with token 1");
        let renewed = renew(&listening.url, &granted.client, refresh)
            .await
            .unwrap();
        assert_eq!(renewed.token.expose(), two.as_str());
        let next = renewed.refresh.as_ref().unwrap();
        assert_eq!(next.expose(), second_refresh.as_str());
        current.set(two);
        assert_eq!(
            whose(&server).await,
            "signed in with token 2",
            "asked again for the next request"
        );
        let spent = renew(&listening.url, &granted.client, refresh).await;
        let Err(SignInError::Refused { message, .. }) = spent else {
            panic!("a spent refresh token renewed");
        };
        let shown = format!("{message} {granted:?} {renewed:?}");
        for secret in tokens.all() {
            assert!(!shown.contains(secret.as_str()), "no secret in {shown}");
        }
        server.close().await;
    });
    let asked = auth.received();
    let traded = asked
        .iter()
        .find(|(_, path, body)| path == "/token" && body.contains("grant_type=authorization_code"));
    let (_, _, body) = traded.unwrap();
    assert!(
        body.contains("code_verifier=") && body.contains("resource="),
        "{body}"
    );
}

#[test]
fn a_token_granted_for_the_address_above_the_server_is_renewed_for_it() {
    let tokens = Tokens::new();
    let auth = FakeAuthServer::start(tokens.grants()).unwrap();
    let [one, two] = &tokens.tokens;
    let above = Served::with(&[
        "--sign-in",
        &auth.url,
        "--resource-above",
        "--token",
        one,
        "--token",
        two,
    ]);
    block_on!(async {
        let (signin, page) = SignIn::start(&above.url, "docs", REDIRECT).await.unwrap();
        let back = approved(&page).await;
        let granted = signin.finish(&field(&back, "code"), None).await.unwrap();
        let refresh = granted.refresh.as_ref().unwrap();
        let renewed = renew(&above.url, &granted.client, refresh).await;
        let renewed = renewed.unwrap_or_else(|error| panic!("for the same resource: {error}"));
        assert_eq!(renewed.token.expose(), two.as_str());
    });
    let registered = auth.received();
    let (_, _, asked) = registered
        .iter()
        .find(|(_, path, _)| path == "/register")
        .unwrap();
    assert!(
        asked.contains("glave (docs)"),
        "named for its server: {asked}"
    );
}

#[test]
fn a_server_that_no_longer_says_where_its_sign_in_is_is_sent_no_refresh_token() {
    let tokens = Tokens::new();
    let auth = FakeAuthServer::start(tokens.grants()).unwrap();
    let listening = tokens.served(&auth);
    let gone = FakeHttp::always(404, "{}").unwrap();
    block_on!(async {
        let (signin, page) = SignIn::start(&listening.url, "docs", REDIRECT)
            .await
            .unwrap();
        let back = approved(&page).await;
        let granted = signin.finish(&field(&back, "code"), None).await.unwrap();
        let refresh = granted.refresh.as_ref().unwrap();
        // As if the server at that address now answers nothing it used to publish.
        let moved = format!("{}/mcp", gone.url);
        let renewed = renew(&moved, &granted.client, refresh).await;
        assert!(
            matches!(renewed, Err(SignInError::Unreachable { .. })),
            "asked again later: {renewed:?}"
        );
    });
    let asked = gone.received();
    assert!(
        asked.iter().all(|asked| asked.method != "POST"),
        "nothing was sent there: {asked:?}"
    );
    let renewals = auth.received();
    let spent = renewals
        .iter()
        .any(|(_, _, body)| body.contains("refresh_token="));
    assert!(!spent, "the refresh token is unspent");
}

#[test]
fn a_code_the_server_never_gave_is_refused() {
    let tokens = Tokens::new();
    let auth = FakeAuthServer::start(tokens.grants()).unwrap();
    let served = tokens.served(&auth);
    block_on!(async {
        let (signin, _) = SignIn::start(&served.url, "docs", REDIRECT).await.unwrap();
        let refused = signin.finish("not-a-code", None).await;
        assert!(
            matches!(refused, Err(SignInError::Refused { .. })),
            "{refused:?}"
        );
    });
}

#[test]
fn an_authorization_server_that_takes_no_registration_is_asked_nothing_more() {
    let tokens = Tokens::new();
    let auth = FakeAuthServer::start(tokens.grants()).unwrap();
    auth.close_registration();
    let served = tokens.served(&auth);
    block_on!(async {
        let started = SignIn::start(&served.url, "docs", REDIRECT).await;
        assert!(
            matches!(started, Err(SignInError::Unregistered { .. })),
            "{:?}",
            started.err()
        );
        let asked = auth.received();
        assert!(
            asked.iter().all(|(_, path, _)| path != "/register"),
            "{asked:?}"
        );
    });
}

#[test]
fn a_server_that_asks_no_one_to_sign_in_offers_no_sign_in() {
    let served = Served::with(&[]);
    block_on!(async {
        let offered = SignIn::start(&served.url, "docs", REDIRECT).await;
        assert_eq!(
            offered.err(),
            Some(SignInError::NotOffered {
                url: served.url.clone()
            })
        );
    });
}
