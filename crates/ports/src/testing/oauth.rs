//! A fake OAuth authorization server for tests of signing in to remote MCP servers, on
//! 127.0.0.1 with a thread of its own until dropped: its metadata (RFC 8414), dynamic client
//! registration (RFC 7591), an authorization page that approves at once or refuses, as the
//! person would in their browser, and a token endpoint that checks PKCE (S256) and hands out
//! its grants in order, the first for a code and each next for a refresh token, every code and
//! refresh token good once, and each only for the resource (RFC 8707) it was authorized for,
//! as a strict server holds them. It records what it was asked.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::http::{Received, read};
use crate::computer::bytes::encode;

/// The client id the fake registers every client as.
pub const FAKE_CLIENT: &str = "fake-client";

/// What the fake grants once: a token, the refresh token that renews it, and how many seconds
/// it lasts.
#[derive(Debug, Clone)]
pub struct Grant {
    pub token: String,
    pub refresh: Option<String>,
    pub lasts: Option<u64>,
}

/// A fake authorization server.
#[derive(Debug)]
pub struct FakeAuthServer {
    /// `http://` and the address: its issuer.
    pub url: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Default)]
struct State {
    grants: Vec<Grant>,
    /// Codes not yet traded, each with its PKCE challenge and the resource it is for.
    codes: BTreeMap<String, (String, String)>,
    /// Refresh tokens not yet spent, each with the resource it is for.
    refreshes: Vec<(String, String)>,
    refusing: bool,
    /// Whether its metadata leaves out registration, as a server that takes none does.
    closed: bool,
    received: Vec<Received>,
}

impl FakeAuthServer {
    /// Serves `grants`, the first for the sign-in and each next for a renewal.
    pub fn start(grants: Vec<Grant>) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(Mutex::new(State {
            grants,
            ..State::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (serving, stopping, issuer) = (Arc::clone(&state), Arc::clone(&stop), url.clone());
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(stream) = stream {
                    let _ = answer(stream, &issuer, &serving);
                }
            }
        });
        Ok(Self {
            url,
            state,
            stop,
            thread: Some(thread),
        })
    }

    /// From now on, the person refuses on the authorization page.
    pub fn refuse(&self) {
        self.lock().refusing = true;
    }

    /// From now on, it takes no registration and says so in its metadata.
    pub fn close_registration(&self) {
        self.lock().closed = true;
    }

    /// Every request so far.
    pub fn received(&self) -> Vec<(String, String, String)> {
        self.lock()
            .received
            .iter()
            .map(|asked| (asked.method.clone(), asked.path.clone(), asked.body.clone()))
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for FakeAuthServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the thread blocked in accept, so it sees the stop.
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Answers one request to the server at `issuer`.
fn answer(mut stream: TcpStream, issuer: &str, state: &Mutex<State>) -> io::Result<()> {
    let asked = read(&stream)?;
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    let (path, query) = asked.path.split_once('?').unwrap_or((&asked.path, ""));
    let answer = match (asked.method.as_str(), path) {
        ("GET", "/.well-known/oauth-authorization-server") => {
            Answer::Json(200, metadata(issuer, state.closed))
        }
        ("POST", "/register") => Answer::Json(201, registered(&asked.body)),
        ("GET", "/authorize") => authorized(&form(query), &mut state),
        ("POST", "/token") => token(&form(&asked.body), &mut state),
        _ => Answer::Json(404, json!({"error": "not_found"})),
    };
    state.received.push(asked);
    drop(state);
    match answer {
        Answer::Json(status, body) => {
            let body = body.to_string();
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            )
        }
        Answer::Redirect(location) => write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\
             Connection: close\r\n\r\n"
        ),
    }
}

enum Answer {
    Json(u16, Value),
    Redirect(String),
}

fn metadata(issuer: &str, closed: bool) -> Value {
    let mut metadata = json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    });
    if let Some(fields) = metadata.as_object_mut().filter(|_| !closed) {
        fields.insert(
            "registration_endpoint".into(),
            json!(format!("{issuer}/register")),
        );
    }
    metadata
}

fn registered(body: &str) -> Value {
    let asked: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    json!({"client_id": FAKE_CLIENT, "redirect_uris": asked.get("redirect_uris")})
}

/// The person's answer on the authorization page: back to the client with a code, or with
/// `access_denied`.
fn authorized(asked: &BTreeMap<String, String>, state: &mut State) -> Answer {
    let field = |name: &str| asked.get(name).cloned().unwrap_or_default();
    let (redirect, back) = (field("redirect_uri"), field("state"));
    if state.refusing {
        return Answer::Redirect(format!("{redirect}?error=access_denied&state={back}"));
    }
    let code = format!("code{}", state.codes.len() + state.received.len());
    let held = (field("code_challenge"), field("resource"));
    state.codes.insert(code.clone(), held);
    Answer::Redirect(format!("{redirect}?code={code}&state={back}"))
}

/// The token endpoint's answer: a grant; `invalid_grant` for a code or refresh token it did
/// not give or that was spent; `invalid_target` for one asked for another resource.
fn token(asked: &BTreeMap<String, String>, state: &mut State) -> Answer {
    let field = |name: &str| asked.get(name).map(String::as_str).unwrap_or_default();
    let held = match field("grant_type") {
        "authorization_code" => state
            .codes
            .remove(field("code"))
            .filter(|(challenge, _)| *challenge == s256(field("code_verifier")))
            .map(|(_, resource)| resource),
        "refresh_token" => {
            let spent = field("refresh_token");
            let at = state
                .refreshes
                .iter()
                .position(|(refresh, _)| refresh == spent);
            at.map(|at| state.refreshes.remove(at).1)
        }
        _ => None,
    };
    let Some(resource) = held.filter(|_| !state.grants.is_empty()) else {
        return Answer::Json(400, json!({"error": "invalid_grant"}));
    };
    if resource != field("resource") {
        return Answer::Json(400, json!({"error": "invalid_target"}));
    }
    let grant = state.grants.remove(0);
    state
        .refreshes
        .extend(grant.refresh.clone().map(|refresh| (refresh, resource)));
    let granted = json!({"access_token": grant.token, "token_type": "Bearer",
                         "expires_in": grant.lasts, "refresh_token": grant.refresh});
    Answer::Json(200, granted)
}

/// PKCE's S256: the verifier's SHA-256 in base64url, unpadded.
fn s256(verifier: &str) -> String {
    encode(&Sha256::digest(verifier.as_bytes()))
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

/// The fields of a form or a query, decoded.
fn form(text: &str) -> BTreeMap<String, String> {
    text.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (decoded(name), decoded(value)))
        .collect()
}

/// `text` with its `%XX` escapes and `+` decoded.
fn decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (byte, hex) {
            (b'%', Some(value)) => {
                out.push(value);
                at += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                at += 1;
            }
            (other, _) => {
                out.push(other);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
