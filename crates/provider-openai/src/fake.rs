//! A fake OpenAI-compatible server for tests: it listens on `127.0.0.1` only, answers each
//! request with the next scripted reply in chat completions' wire format, and records what it
//! was asked. Enabled by the `fake-server` feature; glave-ports' `ScriptedHttp` serves it.

use std::fmt::Write as _;
use std::time::Duration;

use glave_ports::conformance::Listening;
use glave_ports::testing::{Received as Asked, ScriptedHttp, Written};
use serde_json::{Value, json};

/// One scripted answer.
#[derive(Debug, Clone)]
pub enum Reply {
    /// A streamed completion: each item is one `data:` payload; `[DONE]` follows.
    Stream(Vec<Value>),
    /// A stream that breaks off after these payloads: the connection closes before the
    /// response's end.
    Broken(Vec<Value>),
    /// An error status with this body.
    Status(u16, String),
    /// No answer at all: the request is held open until the client hangs up, like a model
    /// that never starts.
    Stall,
}

impl Reply {
    /// A streamed text answer that ends the turn, with usage.
    pub fn text(text: &str) -> Self {
        Self::Stream(vec![
            json!({"id": "gen-fake", "model": "fake-model", "choices": [{"index": 0, "delta": {"content": text}}]}),
            json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                   "usage": {"prompt_tokens": 100, "completion_tokens": 10}}),
        ])
    }

    /// A streamed call of `tool` with `arguments`, which asks for the tool's result.
    pub fn tool_call(id: &str, tool: &str, arguments: &Value) -> Self {
        Self::Stream(vec![
            json!({"id": "gen-fake", "model": "fake-model", "choices": [{"index": 0, "delta": {"tool_calls": [
                {"index": 0, "id": id, "type": "function", "function": {"name": tool, "arguments": arguments.to_string()}}
            ]}}]}),
            json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}],
                   "usage": {"prompt_tokens": 100, "completion_tokens": 10}}),
        ])
    }

    /// The reply as it goes on the wire.
    fn written(self) -> Written {
        let events = |chunks: Vec<Value>| {
            let mut body = String::new();
            for chunk in chunks {
                let _ = write!(body, "data: {chunk}\n\n");
            }
            body
        };
        match self {
            Self::Stream(chunks) => Written::Whole {
                status: 200,
                content_type: "text/event-stream",
                body: events(chunks) + "data: [DONE]\n\n",
            },
            Self::Broken(chunks) => Written::Broken {
                content_type: "text/event-stream",
                body: events(chunks),
            },
            Self::Status(status, body) => Written::Whole {
                status,
                content_type: "application/json",
                body,
            },
            Self::Stall => Written::Stall,
        }
    }
}

/// A request the server received.
#[derive(Debug, Clone)]
pub struct Received {
    /// The `Authorization` header, if any.
    pub authorization: Option<String>,
    /// Every header, its name in lower case, in the order of their names.
    pub headers: Vec<(String, String)>,
    /// The request's method and path, such as `POST /chat/completions`.
    pub line: String,
    /// The JSON body.
    pub body: Value,
}

impl From<Asked> for Received {
    fn from(asked: Asked) -> Self {
        Self {
            authorization: asked.headers.get("authorization").cloned(),
            line: format!("{} {}", asked.method, asked.path),
            body: serde_json::from_str(&asked.body).unwrap_or(Value::Null),
            headers: asked.headers.into_iter().collect(),
        }
    }
}

/// The running server. Dropping it stops the server and waits for its thread.
pub struct FakeServer {
    url: String,
    server: ScriptedHttp,
}

impl FakeServer {
    /// Starts a server that answers with `replies`, in order. After the last one, every request
    /// gets a 500 saying the script ran out.
    pub fn start(replies: Vec<Reply>) -> std::io::Result<Self> {
        let server = ScriptedHttp::start(replies.into_iter().map(Reply::written).collect())?;
        Ok(Self {
            url: format!("{}/v1", server.url()),
            server,
        })
    }

    /// The base URL to configure, such as `http://127.0.0.1:53211/v1`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The requests received so far, in order.
    pub fn received(&self) -> Vec<Received> {
        self.server
            .received()
            .into_iter()
            .map(Received::from)
            .collect()
    }

    /// Waits until `count` requests have arrived, or `limit` has passed; whether they did.
    pub fn wait_for_requests(&self, count: usize, limit: Duration) -> bool {
        self.server.wait_for_requests(count, limit)
    }
}

impl Listening for FakeServer {
    fn heard(&self, limit: Duration) -> bool {
        self.wait_for_requests(1, limit)
    }
}
