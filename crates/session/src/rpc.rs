//! JSON-RPC 2.0 over any stream of lines: one message per line, as on stdio and Unix sockets.
//! These transports are local, so every request comes from the local person.
//! Requests run concurrently, so a client can cancel a turn or answer an approval while another
//! request is still streaming. When the client's input ends, its turns are cancelled.

use std::sync::Arc;

use glave_harness::Caller;
use glave_ports::{BlobStore, EventLog, ModelProvider};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _,
};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::protocol::{ApiError, Notification, Reply, Request};
use crate::server::Server;
use crate::turn::TurnItem;

/// The longest message accepted, in bytes. A longer line ends the connection.
const MAX_LINE: u64 = 16 * 1024 * 1024;
/// Lines written ahead of a slow client.
const WRITE_AHEAD: usize = 256;

/// Every method a request may name.
const METHODS: &[&str] = &[
    "server/discover",
    "thread/start",
    "thread/list",
    "thread/get",
    "thread/events",
    "turn/start",
    "turn/cancel",
    "thread/compact",
    "approval/decide",
    "question/answer",
    "goal/start",
    "goal/stop",
    "goal/get",
];

/// JSON-RPC's own error codes.
pub(crate) mod error {
    pub const PARSE: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const NO_METHOD: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
}

/// A request's id: a number or a string, echoed in its response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Number(i64),
    Text(String),
}

#[derive(Deserialize)]
struct Incoming {
    jsonrpc: Option<String>,
    id: Option<Id>,
    method: Option<String>,
    params: Option<Value>,
}

#[derive(Serialize)]
struct Outgoing<'a> {
    jsonrpc: &'static str,
    /// Null only when the request could not be read far enough to know its id.
    id: Option<&'a Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<&'a Reply>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a ApiError>,
}

/// Serves `server` to one client reading `input` and writing `output` until its input ends.
pub async fn serve<P, L, B, R, W>(
    server: Server<P, L, B>,
    input: R,
    output: W,
) -> std::io::Result<()>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (lines, mut queued) = mpsc::channel::<String>(WRITE_AHEAD);
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(line) = queued.recv().await {
            output.write_all(line.as_bytes()).await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
        Ok::<_, std::io::Error>(())
    });
    let mut requests = JoinSet::new();
    let mut input = input;
    let read = loop {
        let mut line = String::new();
        let got = match (&mut input).take(MAX_LINE).read_line(&mut line).await {
            Ok(got) => got,
            Err(err) => break Err(err),
        };
        if got == 0 {
            break Ok(());
        }
        if !line.ends_with('\n') && u64::try_from(line.len()).unwrap_or(u64::MAX) >= MAX_LINE {
            let too_long = ApiError {
                code: error::PARSE,
                message: format!("a message is longer than {MAX_LINE} bytes"),
                data: None,
            };
            let _ = lines.send(respond(None, Err(&too_long))).await;
            break Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        let (server, lines) = (server.clone(), lines.clone());
        requests.spawn(async move { handle(&server, &line, &lines).await });
        while requests.try_join_next().is_some() {}
    };
    // The client is gone: stop its turns, finish what is in flight, then the writes.
    server.shutdown().await;
    while requests.join_next().await.is_some() {}
    drop(lines);
    let written = writer.await.map_err(std::io::Error::other)?;
    read.and(written)
}

/// Answers one message, streaming a turn's notifications before its response.
async fn handle<P, L, B>(server: &Server<P, L, B>, line: &str, lines: &mpsc::Sender<String>)
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let (id, request) = match parse(line) {
        Ok(parsed) => parsed,
        Err((id, failed)) => {
            let _ = lines.send(respond(id.as_ref(), Err(&failed))).await;
            return;
        }
    };
    // Local transports act as the local person.
    let local = Caller::default();
    let answered = match server.start_streaming(&local, request).await {
        Ok(started) => match started {
            Ok(mut stream) => loop {
                match stream.next().await {
                    Some(TurnItem::Note(note)) => {
                        let _ = lines.send(notify(&note)).await;
                    }
                    Some(TurnItem::Missed(_)) => {}
                    Some(TurnItem::Ended(ended)) => break ended.map(|turn| Reply::Turn { turn }),
                    None => {
                        break Err(ApiError {
                            code: crate::protocol::code::INTERNAL,
                            message: "the turn stopped".into(),
                            data: None,
                        });
                    }
                }
            },
            Err(failed) => Err(failed),
        },
        Err(other) => server.request_as(&local, *other).await,
    };
    // A request without an id is a notification, which is never answered.
    if let Some(id) = id {
        let _ = lines.send(respond(Some(&id), answered.as_ref())).await;
    }
}

/// Reads one message as a request, or says why it is not one.
pub(crate) fn parse(line: &str) -> Result<(Option<Id>, Request), (Option<Id>, ApiError)> {
    let failed = |code, message: String| ApiError {
        code,
        message,
        data: None,
    };
    let incoming: Incoming = serde_json::from_str(line)
        .map_err(|err| (None, failed(error::PARSE, format!("not JSON-RPC: {err}"))))?;
    let id = incoming.id;
    if incoming.jsonrpc.as_deref() != Some("2.0") {
        let message = "expected \"jsonrpc\": \"2.0\"".to_owned();
        return Err((id, failed(error::INVALID_REQUEST, message)));
    }
    let Some(method) = incoming.method else {
        return Err((id, failed(error::INVALID_REQUEST, "no method".to_owned())));
    };
    if !METHODS.contains(&method.as_str()) {
        return Err((
            id,
            failed(error::NO_METHOD, format!("no method {method:?}")),
        ));
    }
    let params = incoming.params.unwrap_or_else(|| json!({}));
    let request = serde_json::from_value(json!({"method": method, "params": params}))
        .map_err(|err| failed(error::INVALID_PARAMS, format!("{method}: {err}")));
    match request {
        Ok(request) => Ok((id, request)),
        Err(failed) => Err((id, failed)),
    }
}

pub(crate) fn respond(id: Option<&Id>, answered: Result<&Reply, &ApiError>) -> String {
    let (result, error) = match answered {
        Ok(reply) => (Some(reply), None),
        Err(failed) => (None, Some(failed)),
    };
    let message = Outgoing {
        jsonrpc: "2.0",
        id,
        result,
        error,
    };
    serde_json::to_string(&message).unwrap_or_else(|err| {
        json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32603, "message": err.to_string()}})
            .to_string()
    })
}

pub(crate) fn notify(note: &Arc<Notification>) -> String {
    let mut message = serde_json::to_value(note.as_ref()).unwrap_or(Value::Null);
    if let Value::Object(fields) = &mut message {
        fields.insert("jsonrpc".into(), "2.0".into());
    }
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_messages_get_the_standard_codes() {
        let code = |line: &str| parse(line).err().map(|(_, failed)| failed.code);
        assert_eq!(code("not json"), Some(error::PARSE));
        assert_eq!(
            code(r#"{"id":1,"method":"thread/start"}"#),
            Some(error::INVALID_REQUEST)
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":1,"method":"thread/explode"}"#),
            Some(error::NO_METHOD)
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":1,"method":"thread/get","params":{"thread":7}}"#),
            Some(error::INVALID_PARAMS)
        );
        let (id, request) = parse(r#"{"jsonrpc":"2.0","id":"a","method":"thread/start"}"#).unwrap();
        assert_eq!(id, Some(Id::Text("a".into())));
        assert_eq!(request, Request::ThreadStart {});
    }

    #[test]
    fn responses_echo_the_id_and_carry_one_of_result_or_error() {
        let ok = respond(Some(&Id::Number(3)), Ok(&Reply::Done {}));
        assert_eq!(ok, r#"{"jsonrpc":"2.0","id":3,"result":{}}"#);
        let failed = ApiError {
            code: -32001,
            message: "no thread".into(),
            data: None,
        };
        let err = respond(None, Err(&failed));
        assert_eq!(
            err,
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32001,"message":"no thread"}}"#
        );
    }
}
