//! A session server over HTTP on 127.0.0.1 for tests, signed in through the fake issuer, and a
//! small client for it: JSON-RPC requests posted one at a time, and server-sent events read
//! as they arrive.

use std::collections::VecDeque;
use std::net::SocketAddr;

use glave_ports::{EventLog, SseDecoder};
use glave_session::http::Site;
use glave_session::protocol::Request;
use glave_session::rpc;
use glave_session::signin::{SignIn, Trust};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::issuer::{AUDIENCE, Issuer, fixed_clock};
use super::setup::Tested;

/// A session server listening over HTTP until it is closed.
pub(crate) struct Served {
    pub address: SocketAddr,
    pub url: String,
    stop: oneshot::Sender<()>,
    serving: JoinHandle<std::io::Result<()>>,
}

/// Serves `server` over HTTP, trusting `issuer` for `AUDIENCE`: whoever signs in there works
/// in their own projects, never the server's own.
pub(crate) async fn serve_http<L: EventLog + 'static>(
    server: Tested<L>,
    issuer: &Issuer,
) -> Served {
    let trust = Trust {
        issuers: vec![issuer.url.clone()],
        audience: AUDIENCE.into(),
    };
    let signin = SignIn::new(trust, Box::new(fixed_clock())).unwrap();
    serve_with(server, signin, None).await
}

/// Serves `server` over HTTP on 127.0.0.1, each request signed in through `signin`, with the
/// web app in `site` beside it.
pub(crate) async fn serve_with<L: EventLog + 'static>(
    server: Tested<L>,
    signin: SignIn,
    site: Option<Site>,
) -> Served {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let serving = tokio::spawn(glave_session::http::serve(
        server,
        signin,
        site,
        listener.into(),
        async move {
            let _ = stopped.await;
        },
    ));
    Served {
        address,
        url: format!("http://{address}/"),
        stop,
        serving,
    }
}

impl Served {
    /// Stops the server and waits until it has.
    pub(crate) async fn close(self) {
        let _ = self.stop.send(());
        self.serving.await.unwrap().unwrap();
    }
}

/// A client of one served session API, signed in with one token.
pub(crate) struct Web {
    http: reqwest::Client,
    url: String,
    pub token: Option<String>,
    /// The origin whose name its requests come to, as a tunnel on this computer forwards a
    /// browser's there; the listener's own address when none.
    reaching: Option<String>,
    /// The languages its requests ask for (`Accept-Language`), if any.
    reading: Option<String>,
    next_id: u64,
}

/// What came back for one request.
pub(crate) struct Answered {
    pub status: u16,
    pub content_type: String,
    pub www_authenticate: Option<String>,
    /// The JSON answer, or each server-sent event.
    pub messages: Vec<Value>,
}

impl Web {
    pub(crate) fn new(served: &Served, token: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            url: served.url.clone(),
            token,
            reaching: None,
            reading: None,
            next_id: 0,
        }
    }

    /// This client's requests come to `origin`'s name, from a page there, as a tunnel on this
    /// computer forwards a browser's requests to the loopback listener.
    pub(crate) fn reaching(mut self, origin: &str) -> Self {
        self.reaching = Some(origin.to_owned());
        self
    }

    /// This client's requests ask for the languages `accepted`, as a browser set to them does.
    pub(crate) fn reading(mut self, accepted: &str) -> Self {
        self.reading = Some(accepted.to_owned());
        self
    }

    /// `request` as a JSON-RPC message with the next id, and that id.
    pub(crate) fn message(&mut self, request: &Request) -> (Value, u64) {
        self.next_id += 1;
        (rpc::message(self.next_id, request), self.next_id)
    }

    /// Posts `message`, naming `method` in the Glave-Method header when given.
    pub(crate) async fn open(&self, message: &Value, method: Option<&str>) -> reqwest::Response {
        let mut post = self.http.post(&self.url).body(message.to_string());
        if let Some(token) = &self.token {
            post = post.bearer_auth(token);
        }
        if let Some(method) = method {
            post = post.header("Glave-Method", method);
        }
        if let Some(accepted) = &self.reading {
            post = post.header("accept-language", accepted);
        }
        if let Some(origin) = &self.reaching {
            let host = origin.strip_prefix("https://").unwrap_or(origin);
            post = post.header("host", host).header("origin", origin);
        }
        post.send().await.unwrap()
    }

    /// Posts `message` and reads the whole answer.
    pub(crate) async fn post(&self, message: &Value) -> Answered {
        let method = message["method"].as_str().map(str::to_owned);
        let response = self.open(message, method.as_deref()).await;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .map(|value| value.to_str().unwrap().to_owned())
        };
        let status = response.status().as_u16();
        let content_type = header("content-type").unwrap_or_default();
        let www_authenticate = header("www-authenticate");
        let mut events = Events::new(response);
        let mut messages = Vec::new();
        if content_type == "text/event-stream" {
            while let Some(message) = events.next().await {
                messages.push(message);
            }
        } else {
            let text = events.rest().await;
            if !text.is_empty() {
                messages.push(serde_json::from_str(&text).unwrap());
            }
        }
        Answered {
            status,
            content_type,
            www_authenticate,
            messages,
        }
    }

    /// Calls `request` and returns its one answer.
    pub(crate) async fn call(&mut self, request: &Request) -> Value {
        let (message, _) = self.message(request);
        let answered = self.post(&message).await;
        answered.messages.into_iter().next().unwrap()
    }
}

/// Server-sent events, read as they arrive.
pub(crate) struct Events {
    response: reqwest::Response,
    decoder: SseDecoder,
    /// Events decoded and not yet read.
    ready: VecDeque<String>,
}

impl Events {
    pub(crate) fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            decoder: SseDecoder::default(),
            ready: VecDeque::new(),
        }
    }

    /// The next event's message, or `None` at the end of the stream.
    pub(crate) async fn next(&mut self) -> Option<Value> {
        loop {
            if let Some(data) = self.ready.pop_front() {
                return Some(serde_json::from_str(&data).unwrap());
            }
            match self.response.chunk().await.unwrap() {
                Some(chunk) => self.ready.extend(self.decoder.push(&chunk).unwrap()),
                None => {
                    return self
                        .decoder
                        .finish()
                        .map(|data| serde_json::from_str(&data).unwrap());
                }
            }
        }
    }

    /// The whole answer, as text, when it is not a stream.
    pub(crate) async fn rest(mut self) -> String {
        let mut text = Vec::new();
        while let Some(chunk) = self.response.chunk().await.unwrap() {
            text.extend_from_slice(&chunk);
        }
        String::from_utf8(text).unwrap()
    }
}
