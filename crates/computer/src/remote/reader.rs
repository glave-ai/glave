//! The client's half that reads: answers matched to their calls by request id only, output
//! handed to its command, and anything the computer sends that is not a well-formed message
//! ends the connection, failing every call still waiting with the reason.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, BufReader};
use tokio::sync::{Notify, mpsc, oneshot};

use crate::wire::{Chunk, MAX_LINE, OUTPUT};

pub(super) type Answer = Result<Value, Value>;

/// What the client's calls and its reader share.
pub(super) struct Shared {
    /// Calls waiting for their answer; `None` once the connection has ended.
    pub(super) pending: Mutex<Option<HashMap<u64, oneshot::Sender<Answer>>>>,
    pub(super) streams: Mutex<HashMap<String, mpsc::Sender<String>>>,
    /// Why the connection ended, once it has.
    ended: Mutex<Option<String>>,
    /// Tells the writer to stop, so the computer sees the connection end too.
    pub(super) stop: Notify,
}

impl Shared {
    pub(super) fn new() -> Self {
        Self {
            pending: Mutex::new(Some(HashMap::new())),
            streams: Mutex::new(HashMap::new()),
            ended: Mutex::new(None),
            stop: Notify::new(),
        }
    }

    /// Why the connection ended, if it has.
    pub(super) fn ended(&self) -> Option<String> {
        self.ended.lock().ok().and_then(|ended| ended.clone())
    }

    /// Ends the connection for `why`: every waiting call fails with it, and the writer stops.
    pub(super) fn end(&self, why: String) {
        if let Ok(mut ended) = self.ended.lock() {
            ended.get_or_insert(why);
        }
        // Dropping the senders tells every waiting call that the connection is gone.
        if let Ok(mut pending) = self.pending.lock() {
            *pending = None;
        }
        if let Ok(mut streams) = self.streams.lock() {
            streams.clear();
        }
        self.stop.notify_one();
    }
}

/// Reads answers and output until the connection ends or breaks the protocol, then fails
/// what is still waiting.
pub(super) async fn read<R: AsyncRead + Unpin>(mut input: BufReader<R>, shared: &Shared) {
    let why = loop {
        let mut line = Vec::new();
        let got = tokio::io::AsyncReadExt::take(&mut input, MAX_LINE)
            .read_until(b'\n', &mut line)
            .await;
        match got {
            Ok(0) => break "the connection to the computer closed".to_owned(),
            Err(error) => break format!("the connection to the computer failed: {error}"),
            Ok(_) if line.last() != Some(&b'\n') => {
                break format!(
                    "the computer sent a message that was cut off (or longer than {MAX_LINE} \
                     bytes), so the connection was closed"
                );
            }
            Ok(_) => {
                if let Err(why) = dispatch(shared, &line).await {
                    break format!("{why}, so the connection was closed");
                }
            }
        }
    };
    shared.end(why);
}

/// Hands one message to whoever waits for it. A message that is not well-formed JSON-RPC
/// is an error: past it, nothing on the connection can be trusted.
async fn dispatch(shared: &Shared, line: &[u8]) -> Result<(), String> {
    let message: Value = serde_json::from_slice(line)
        .map_err(|error| format!("the computer sent a message that is not JSON ({error})"))?;
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err("the computer sent a message that is not JSON-RPC 2.0".into());
    }
    if let Some(method) = message.get("method").and_then(Value::as_str) {
        if method == OUTPUT {
            return output(shared, &message).await;
        }
        // Other notifications are for later versions; they ask nothing of this client.
        return Ok(());
    }
    let id = match message.get("id") {
        Some(Value::Null) => {
            // Only a request the computer could not read is answered without an id: what
            // this client sent arrived damaged.
            let said = message
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("no message");
            return Err(format!("the computer could not read a request ({said})"));
        }
        Some(id) => id.as_u64().ok_or_else(|| {
            format!("the computer answered a request id this client never uses ({id})")
        })?,
        None => return Err("the computer sent a message with neither a method nor an id".into()),
    };
    let answer = match (message.get("result"), message.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) => Err(error.clone()),
        _ => Err(json!({"message": "an answer with both or neither of result and error"})),
    };
    let waiting = shared
        .pending
        .lock()
        .ok()
        .and_then(|mut pending| pending.as_mut().and_then(|waiting| waiting.remove(&id)));
    // An id no call waits for is a second copy of an answer, or one that came after its
    // call gave up: each id is answered once, so it is dropped.
    if let Some(waiting) = waiting {
        let _ = waiting.send(answer);
    }
    Ok(())
}

/// Hands a command's output to the call running it.
async fn output(shared: &Shared, message: &Value) -> Result<(), String> {
    let chunk = message
        .get("params")
        .and_then(|params| Chunk::deserialize(params).ok())
        .ok_or_else(|| format!("the computer sent {OUTPUT} without a run and its text"))?;
    let stream = shared
        .streams
        .lock()
        .ok()
        .and_then(|streams| streams.get(&chunk.run).cloned());
    if let Some(stream) = stream {
        let _ = stream.send(chunk.text).await;
    }
    Ok(())
}
