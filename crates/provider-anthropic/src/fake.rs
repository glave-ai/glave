//! A fake Messages API server for tests: it listens on `127.0.0.1` only, answers each request
//! with the next scripted reply, and records what it was asked. Enabled by the `fake-server`
//! feature.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Read as _, Write as IoWrite};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

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

type Shared<T> = Arc<Mutex<T>>;

/// The requests received so far, and a signal for each new one.
type Log = Arc<(Mutex<Vec<Received>>, Condvar)>;

/// The running server. Dropping it stops the server and waits for its thread.
pub struct FakeServer {
    url: String,
    received: Log,
    stop: Arc<AtomicBool>,
    /// Every connection taken, closed when the server stops, so a client still waiting
    /// on one, as it waits on a stall, is let go.
    open: Shared<Vec<TcpStream>>,
    address: std::net::SocketAddr,
    thread: Option<JoinHandle<()>>,
}

impl FakeServer {
    /// Starts a server that answers with `replies`, in order. After the last one, every request
    /// gets a 500 saying the script ran out.
    pub fn start(replies: Vec<Reply>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let replies: Shared<VecDeque<Reply>> = Arc::new(Mutex::new(replies.into()));
        let received: Log = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let open: Shared<Vec<TcpStream>> = Arc::default();
        let thread = {
            let (replies, received, stop) = (replies, Arc::clone(&received), Arc::clone(&stop));
            let taken = Arc::clone(&open);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let Ok(mut taken) = taken.lock() else { break };
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(held) = stream.try_clone() {
                        taken.push(held);
                    }
                    drop(taken);
                    // A broken exchange only fails the test that caused it.
                    let _ = serve(stream, &replies, &received);
                }
            })
        };
        Ok(Self {
            url: format!("http://{address}"),
            received,
            stop,
            open,
            address,
            thread: Some(thread),
        })
    }

    /// The base URL to configure, such as `http://127.0.0.1:53211`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The requests received so far, in order.
    pub fn received(&self) -> Vec<Received> {
        self.received
            .0
            .lock()
            .map(|received| received.clone())
            .unwrap_or_default()
    }

    /// Waits until `count` requests have arrived, or `limit` has passed; whether they did.
    pub fn wait_for_requests(&self, count: usize, limit: Duration) -> bool {
        let (requests, arrived) = &*self.received;
        let Ok(guard) = requests.lock() else {
            return false;
        };
        arrived
            .wait_timeout_while(guard, limit, |requests| requests.len() < count)
            .is_ok_and(|(requests, _)| requests.len() >= count)
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The flag is set before the lock is taken, so a connection is either closed here or
        // never served.
        if let Ok(open) = self.open.lock() {
            for stream in open.iter() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    stream: TcpStream,
    replies: &Shared<VecDeque<Reply>>,
    received: &Log,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let mut length = 0_usize;
    let (mut authorization, mut api_key, mut version) = (None, None, None);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let value = Some(value.trim().to_owned());
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.and_then(|v| v.parse().ok()).unwrap_or(0),
                "authorization" => authorization = value,
                "x-api-key" => api_key = value,
                "anthropic-version" => version = value,
                _ => {}
            }
        }
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;
    let reply = replies
        .lock()
        .ok()
        .and_then(|mut replies| replies.pop_front())
        .unwrap_or_else(|| Reply::Status(500, "the fake server ran out of replies".into()));
    let (requests, arrived) = &**received;
    if let Ok(mut requests) = requests.lock() {
        requests.push(Received {
            authorization,
            api_key,
            version,
            path,
            body: serde_json::from_slice(&body).unwrap_or(Value::Null),
        });
        arrived.notify_all();
    }
    if matches!(reply, Reply::Stall) {
        // Wait for the client to hang up; a read of nothing means it did.
        let _ = reader.read(&mut [0_u8; 1]);
        return Ok(());
    }
    respond(stream, &reply)
}

fn respond(mut stream: TcpStream, reply: &Reply) -> std::io::Result<()> {
    let (status, content_type, body) = match reply {
        Reply::Stream(events) => {
            let mut body = String::new();
            for event in events {
                let name = event.get("type").and_then(Value::as_str).unwrap_or("data");
                let _ = write!(body, "event: {name}\ndata: {event}\n\n");
            }
            (200, "text/event-stream", body)
        }
        Reply::Status(status, body) => (*status, "application/json", body.clone()),
        Reply::Stall => return Ok(()),
    };
    IoWrite::write_fmt(
        &mut stream,
        format_args!(
            "HTTP/1.1 {status} FAKE\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        ),
    )?;
    IoWrite::flush(&mut stream)
}
