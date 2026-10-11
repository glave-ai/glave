//! A session server over HTTP on 127.0.0.1 for tests, signed in through the fake issuer, and a
//! small client for it: JSON-RPC requests posted one at a time, and server-sent events read
//! as they arrive.

use glave_ports::{BlobStore, EventLog, ModelProvider};
use glave_session::Server;
use glave_session::protocol::Request;
use glave_session::signin::{SignIn, Trust};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::issuer::{AUDIENCE, Fixed, Issuer};

/// A session server listening over HTTP until it is closed.
pub struct Served {
    pub url: String,
    stop: Option<oneshot::Sender<()>>,
    serving: Option<JoinHandle<std::io::Result<()>>>,
}

/// Serves `server` over HTTP, trusting `issuer` for `AUDIENCE`, with `ada` of that issuer
/// working in the server's own space beside its person.
pub async fn serve_http<P, L, B>(server: Server<P, L, B>, issuer: &Issuer) -> Served
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let mut home = server.home_space().await.unwrap();
    let ada = glave_session::signin::person(&issuer.url, "ada").unwrap();
    home.members.push(glave_event::Principal::person(ada));
    let org = glave_ports::OrgId::from_uuid(uuid::Uuid::from_u128(1));
    server.spaces().update(org, &home).await.unwrap();
    let trust = Trust {
        issuers: vec![issuer.url.clone()],
        audience: AUDIENCE.into(),
    };
    serve_with(server, SignIn::new(trust, Box::new(Fixed)).unwrap()).await
}

/// Serves `server` over HTTP on 127.0.0.1, each request signed in through `signin`.
pub async fn serve_with<P, L, B>(server: Server<P, L, B>, signin: SignIn) -> Served
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (stop, stopped) = oneshot::channel::<()>();
    let serving = tokio::spawn(glave_session::http::serve(
        server,
        signin,
        listener,
        async move {
            let _ = stopped.await;
        },
    ));
    Served {
        url,
        stop: Some(stop),
        serving: Some(serving),
    }
}

impl Served {
    /// Stops the server and waits until it has.
    pub async fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(serving) = self.serving.take() {
            serving.await.unwrap().unwrap();
        }
    }
}

/// A client of one served session API, signed in with one token.
pub struct Web {
    http: reqwest::Client,
    url: String,
    pub token: Option<String>,
    next_id: i64,
}

/// What came back for one request.
pub struct Answered {
    pub status: u16,
    pub content_type: String,
    pub www_authenticate: Option<String>,
    /// The JSON answer, or each server-sent event.
    pub messages: Vec<Value>,
}

impl Web {
    pub fn new(served: &Served, token: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            url: served.url.clone(),
            token,
            next_id: 0,
        }
    }

    /// `request` as a JSON-RPC message with the next id, and that id.
    pub fn message(&mut self, request: &Request) -> (Value, i64) {
        self.next_id += 1;
        let mut message = serde_json::to_value(request).unwrap();
        message["jsonrpc"] = json!("2.0");
        message["id"] = json!(self.next_id);
        (message, self.next_id)
    }

    /// Posts `message`, naming `method` in the Glave-Method header when given.
    pub async fn open(&self, message: &Value, method: Option<&str>) -> reqwest::Response {
        let mut post = self.http.post(&self.url).body(message.to_string());
        if let Some(token) = &self.token {
            post = post.bearer_auth(token);
        }
        if let Some(method) = method {
            post = post.header("Glave-Method", method);
        }
        post.send().await.unwrap()
    }

    /// Posts `message` and reads the whole answer.
    pub async fn post(&self, message: &Value) -> Answered {
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
    pub async fn call(&mut self, request: &Request) -> Value {
        let (message, _) = self.message(request);
        let answered = self.post(&message).await;
        answered.messages.into_iter().next().unwrap()
    }
}

/// Server-sent events, read as they arrive.
pub struct Events {
    response: reqwest::Response,
    buffer: String,
}

impl Events {
    pub fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            buffer: String::new(),
        }
    }

    /// The next event's message, or `None` at the end of the stream.
    pub async fn next(&mut self) -> Option<Value> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let event: String = self.buffer.drain(..end + 2).collect();
                let data = event.trim_end().strip_prefix("data: ").unwrap();
                return Some(serde_json::from_str(data).unwrap());
            }
            let chunk = self.response.chunk().await.unwrap()?;
            self.buffer.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    }

    /// Everything not read yet, as text.
    pub async fn rest(&mut self) -> String {
        while let Some(chunk) = self.response.chunk().await.unwrap() {
            self.buffer.push_str(std::str::from_utf8(&chunk).unwrap());
        }
        std::mem::take(&mut self.buffer)
    }
}
