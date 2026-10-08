//! A fake OpenAI-compatible server for tests: it listens on `127.0.0.1` only, answers each
//! request with the next scripted reply, and records what it was asked. Enabled by the
//! `fake-server` feature.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Read as _, Write as IoWrite};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

/// One scripted answer.
#[derive(Debug, Clone)]
pub enum Reply {
    /// A streamed completion: each item is one `data:` payload; `[DONE]` follows.
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
}

/// A request the server received.
#[derive(Debug, Clone)]
pub struct Received {
    /// The `Authorization` header, if any.
    pub authorization: Option<String>,
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
        let thread = {
            let (replies, received, stop) = (replies, Arc::clone(&received), Arc::clone(&stop));
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        // A broken exchange only fails the test that caused it.
                        let _ = serve(stream, &replies, &received);
                    }
                }
            })
        };
        Ok(Self {
            url: format!("http://{address}/v1"),
            received,
            stop,
            address,
            thread: Some(thread),
        })
    }

    /// The base URL to configure, such as `http://127.0.0.1:53211/v1`.
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
    let mut length = 0_usize;
    let mut authorization = None;
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
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "authorization" => authorization = Some(value.trim().to_owned()),
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
        Reply::Stream(chunks) => {
            let mut body = String::new();
            for chunk in chunks {
                let _ = write!(body, "data: {chunk}\n\n");
            }
            body.push_str("data: [DONE]\n\n");
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
