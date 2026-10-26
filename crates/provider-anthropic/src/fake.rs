//! A fake Messages API server for tests: it listens on `127.0.0.1` only, answers each request
//! with the next scripted reply in the Messages API's wire format, and records what it was
//! asked. Enabled by the `fake-server` feature; glave-ports' `ScriptedHttp` serves it.

use std::fmt::Write as _;
use std::time::Duration;

use glave_ports::testing::{Received as Asked, ScriptedHttp, Written};
use serde_json::{Value, json};

/// One scripted answer.
#[derive(Debug, Clone)]
pub enum Reply {
    /// A streamed response: each item is one event's data, sent under its own `type` as the
    /// event name.
    Stream(Vec<Value>),
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
            message_start(),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "ping"}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
            json!({"type": "content_block_stop", "index": 0}),
            message_delta("end_turn"),
            json!({"type": "message_stop"}),
        ])
    }

    /// A streamed call of `tool` with `arguments`, which asks for the tool's result.
    pub fn tool_call(id: &str, tool: &str, arguments: &Value) -> Self {
        let json = arguments.to_string();
        // Sent in two pieces, as the API streams a call's input.
        let (head, tail) = json
            .split_at_checked(json.len() / 2)
            .unwrap_or((json.as_str(), ""));
        Self::Stream(vec![
            message_start(),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": id, "name": tool, "input": {}}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": head}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": tail}}),
            json!({"type": "content_block_stop", "index": 0}),
            message_delta("tool_use"),
            json!({"type": "message_stop"}),
        ])
    }

    /// The reply as it goes on the wire.
    fn written(self) -> Written {
        match self {
            Self::Stream(events) => {
                let mut body = String::new();
                for event in events {
                    let name = event.get("type").and_then(Value::as_str).unwrap_or("data");
                    let _ = write!(body, "event: {name}\ndata: {event}\n\n");
                }
                Written::Whole {
                    status: 200,
                    content_type: "text/event-stream",
                    body,
                }
            }
            Self::Status(status, body) => Written::Whole {
                status,
                content_type: "application/json",
                body,
            },
            Self::Stall => Written::Stall,
        }
    }
}

fn message_start() -> Value {
    json!({"type": "message_start", "message": {
        "id": "msg_fake", "type": "message", "role": "assistant", "model": "fake-model",
        "content": [], "stop_reason": null,
        "usage": {"input_tokens": 100, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0, "output_tokens": 1}
    }})
}

fn message_delta(stop: &str) -> Value {
    json!({"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": null},
           "usage": {"output_tokens": 10}})
}

/// A request the server received.
#[derive(Debug, Clone)]
pub struct Received {
    /// The `Authorization` header, if any.
    pub authorization: Option<String>,
    /// The `x-api-key` header, if any.
    pub api_key: Option<String>,
    /// The `anthropic-version` header, if any.
    pub version: Option<String>,
    /// The request path, such as `/v1/messages`.
    pub path: String,
    /// The JSON body.
    pub body: Value,
}

impl From<Asked> for Received {
    fn from(asked: Asked) -> Self {
        let header = |name: &str| asked.headers.get(name).cloned();
        Self {
            authorization: header("authorization"),
            api_key: header("x-api-key"),
            version: header("anthropic-version"),
            body: serde_json::from_str(&asked.body).unwrap_or(Value::Null),
            path: asked.path,
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
            url: server.url(),
            server,
        })
    }

    /// The base URL to configure, such as `http://127.0.0.1:53211`.
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
