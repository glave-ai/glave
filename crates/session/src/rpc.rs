//! JSON-RPC 2.0 over any stream of lines: one message per line, as on stdio and Unix sockets.
//! These transports are local, so every request comes from the local person.
//! Requests run concurrently, so a client can cancel a turn or answer an approval while another
//! request is still streaming. When the client's input ends, its turns are cancelled over stdio
//! (`serve`), where it was the only client; a long-running server's connections
//! (`serve_connection`, as on a Unix socket) stop nothing when they end.

use std::sync::Arc;

use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _,
};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::dispatch::Answered;
use crate::protocol::{ApiError, Notification, Reply, Request, code};
use crate::server::Server;
use crate::turn::TurnItem;

/// The longest message accepted, in bytes. A longer line ends the connection.
const MAX_LINE: u64 = 16 * 1024 * 1024;
/// Lines written ahead of a slow client.
const WRITE_AHEAD: usize = 256;

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

/// Serves `server` to one client reading `input` and writing `output` until its input ends,
/// then stops the server's turns: the client was the server's only one, as on stdio.
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
    serve_client(server, input, output, true).await
}

/// Serves one of many clients of a long-running server, as on a Unix socket: when its input
/// ends, what it started goes on, tasks above all.
pub async fn serve_connection<P, L, B, R, W>(
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
    serve_client(server, input, output, false).await
}

async fn serve_client<P, L, B, R, W>(
    server: Server<P, L, B>,
    input: R,
    output: W,
    only_client: bool,
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
            let too_long = ApiError::new(
                code::PARSE,
                format!("a message is longer than {MAX_LINE} bytes"),
            );
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
    // The client is gone: stop its turns when it was the only one, finish what is in
    // flight, then the writes.
    if only_client {
        server.shutdown().await;
    }
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
    let answered = match server.call(&Caller::default(), request).await {
        Answered::Reply(reply) => reply,
        Answered::Stream(mut stream) => loop {
            match stream.next().await {
                Some(TurnItem::Note(note)) => {
                    let _ = lines.send(notify(&note)).await;
                }
                Some(TurnItem::Missed(_)) => {}
                Some(TurnItem::Ended(ended)) => break ended.map(|turn| Reply::Turn { turn }),
                None => break Err(ApiError::new(code::INTERNAL, "the turn stopped")),
            }
        },
    };
    // A request without an id is a notification, which is never answered.
    if let Some(id) = id {
        let _ = lines.send(respond(Some(&id), answered.as_ref())).await;
    }
}

/// Reads one message as a request, or says why it is not one.
pub(crate) fn parse(line: &str) -> Result<(Option<Id>, Request), (Option<Id>, ApiError)> {
    let failed = ApiError::new;
    let incoming: Incoming = serde_json::from_str(line)
        .map_err(|err| (None, failed(code::PARSE, format!("not JSON-RPC: {err}"))))?;
    let id = incoming.id;
    if incoming.jsonrpc.as_deref() != Some("2.0") {
        let message = "expected \"jsonrpc\": \"2.0\"".to_owned();
        return Err((id, failed(code::INVALID_REQUEST, message)));
    }
    let Some(method) = incoming.method else {
        return Err((id, failed(code::INVALID_REQUEST, "no method".to_owned())));
    };
    let params = incoming.params.unwrap_or_else(|| json!({}));
    let request =
        serde_json::from_value(json!({"method": method, "params": params})).map_err(|err| {
            // serde names a method the table does not have an unknown variant.
            if err.to_string().starts_with("unknown variant") {
                failed(code::NO_METHOD, format!("no method {method:?}"))
            } else {
                failed(code::INVALID_PARAMS, format!("{method}: {err}"))
            }
        });
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

/// `request` as a client sends it: a JSON-RPC message with `id`.
pub fn message(id: u64, request: &Request) -> Value {
    let mut message = serde_json::to_value(request).unwrap_or(Value::Null);
    if let Value::Object(fields) = &mut message {
        fields.insert("jsonrpc".into(), "2.0".into());
        fields.insert("id".into(), id.into());
    }
    message
}

/// What a JSON-RPC response says, as a client reads it: the reply in its `result`, or its
/// `error`.
pub fn outcome(response: &Value) -> Result<Reply, ApiError> {
    if let Some(error) = response.get("error") {
        return Err(serde_json::from_value(error.clone()).unwrap_or_else(|_| {
            ApiError::new(code::INTERNAL, format!("the server failed: {error}"))
        }));
    }
    let result = response.get("result").cloned().unwrap_or_else(|| json!({}));
    serde_json::from_value(result).map_err(|e| {
        ApiError::new(
            code::INTERNAL,
            format!("the server's answer is not one glave knows: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_messages_get_the_standard_codes() {
        let code = |line: &str| parse(line).err().map(|(_, failed)| failed.code);
        assert_eq!(code("not json"), Some(code::PARSE));
        assert_eq!(
            code(r#"{"id":1,"method":"thread/start"}"#),
            Some(code::INVALID_REQUEST)
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":1,"method":"thread/explode"}"#),
            Some(code::NO_METHOD)
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":1,"method":"thread/get","params":{"thread":7}}"#),
            Some(code::INVALID_PARAMS)
        );
        let (id, request) = parse(r#"{"jsonrpc":"2.0","id":"a","method":"thread/start"}"#).unwrap();
        assert_eq!(id, Some(Id::Text("a".into())));
        assert_eq!(
            request,
            Request::ThreadStart {
                project: None,
                worktree: None,
                background: false,
                computer: None,
                model: None,
                approval: None,
            }
        );
        let (_, request) =
            parse(r#"{"jsonrpc":"2.0","id":2,"method":"thread/start","params":{"worktree":true}}"#)
                .unwrap();
        assert_eq!(
            request,
            Request::ThreadStart {
                project: None,
                worktree: Some(true),
                background: false,
                computer: None,
                model: None,
                approval: None,
            }
        );
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
