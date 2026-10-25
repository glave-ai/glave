//! JSON-RPC 2.0 over any stream of lines: one message per line, as on stdio and Unix sockets.
//! These transports are local, so every request comes from the local person.
//! Requests run concurrently, so a client can cancel a turn or answer an approval while another
//! request is still streaming. A connection watches each thread and each list once, at most 64
//! at a time, each until `thread/unwatch` (`thread/list/unwatch`) or its input ends, and writes
//! each thread's notifications once, a turn's in a thread it watches by that watch
//! (`watches.rs`).
//! When the client's input ends, its turns are cancelled over stdio (`serve`), where it was the
//! only client; a long-running server's connections (`serve_connection`, as on a Unix socket)
//! stop nothing when they end.

mod watches;

use std::sync::Arc;

use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _,
};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use self::watches::{Claim, Key, Turn, Watches};
use crate::dispatch::Answered;
use crate::lists::ListWatch;
use crate::protocol::{ApiError, Notification, Reply, Request, WatchEnd, code};
use crate::server::Server;
use crate::turn::{TurnItem, TurnStream};
use crate::watch::{Watch, Watched};
use crate::words::Sentence;

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
    let watches = Arc::new(Watches::default());
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
        let parsed = parse(&line);
        // What the connection watches changes in the order its lines come, before requests run
        // side by side: an unwatch ends the watch sent before it, and frees the thread for one
        // sent after it.
        let (mut claim, mut turn) = (None, None);
        match &parsed {
            Ok((_, Request::ThreadWatch { thread, after })) => {
                claim = Some(watches.claim(Key::Thread(*thread), *after));
            }
            Ok((_, Request::ThreadUnwatch { thread })) => watches.end(Key::Thread(*thread)),
            Ok((_, Request::ThreadListWatch { project, .. })) => {
                claim = Some(watches.claim(Key::List(*project), None));
            }
            Ok((_, Request::ThreadListUnwatch { project })) => watches.end(Key::List(*project)),
            Ok((
                _,
                Request::TurnStart {
                    thread,
                    background: false,
                    ..
                }
                | Request::GoalStart {
                    thread,
                    background: false,
                    ..
                },
            )) => turn = Some(watches.turn(*thread)),
            _ => {}
        }
        let (server, lines) = (server.clone(), lines.clone());
        let held = Held { claim, turn };
        requests.spawn(async move { handle(&server, parsed, held, &lines).await });
        while requests.try_join_next().is_some() {}
    };
    // The client is gone: its watches end, its turns stop when it was the only one, what is in
    // flight finishes, then the writes.
    watches.close();
    if only_client {
        server.shutdown().await;
    }
    while requests.join_next().await.is_some() {}
    drop(lines);
    let written = writer.await.map_err(std::io::Error::other)?;
    read.and(written)
}

/// What the connection holds for one request while it runs: the watch it asks for, or the
/// turn it starts.
struct Held {
    claim: Option<Result<Claim, ApiError>>,
    turn: Option<Turn>,
}

/// Answers one message, streaming a turn's or a watch's notifications before its response.
async fn handle<P, L, B>(
    server: &Server<P, L, B>,
    parsed: Parsed,
    held: Held,
    lines: &mpsc::Sender<String>,
) where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let (id, request) = match parsed {
        Ok(parsed) => parsed,
        Err((id, failed)) => {
            let _ = lines.send(respond(id.as_ref(), Err(&failed))).await;
            return;
        }
    };
    // Local transports act as the local person.
    let caller = Caller::default();
    let Held { claim, mut turn } = held;
    let claim = match claim.transpose() {
        Ok(claim) => claim,
        Err(refused) => {
            if let Some(id) = id {
                let refused = server.in_language(&caller, None, Err(refused)).await;
                let _ = lines.send(respond(Some(&id), refused.as_ref())).await;
            }
            return;
        }
    };
    // A turn the connection's watch of its thread writes starts once that watch follows it.
    if let Some(turn) = &mut turn {
        turn.wait_for_watch().await;
    }
    let answered = match server.call(&caller, request).await {
        Answered::Reply(reply) => reply,
        Answered::Stream(stream) => follow_turn(stream, lines, turn.as_mut()).await,
        Answered::Watch(mut watch) => match &claim {
            Some(claim) => watch_thread(&mut watch, lines, claim).await,
            None => Err(unheld()),
        },
        Answered::List(mut watch) => match &claim {
            Some(claim) => tokio::select! {
                last = follow_list(&mut watch, lines) => last,
                () = claim.ended() => Ok(Reply::Watched {
                    ended: WatchEnd::Unwatched,
                }),
            },
            None => Err(unheld()),
        },
    };
    drop((claim, turn));
    // A request without an id is a notification, which is never answered.
    if let Some(id) = id {
        let answered = server.in_language(&caller, None, answered).await;
        let _ = lines.send(respond(Some(&id), answered.as_ref())).await;
    }
}

/// A watch the connection holds no claim for, which never happens: the claim comes first.
fn unheld() -> ApiError {
    ApiError::new(code::INTERNAL, "a watch the connection does not hold")
}

/// Follows a thread's watch until it ends or the connection ends it: its answer.
async fn watch_thread(
    watch: &mut Watch,
    lines: &mpsc::Sender<String>,
    claim: &Claim,
) -> Result<Reply, ApiError> {
    claim.following();
    let ended = tokio::select! {
        last = follow_watch(watch, lines, claim) => Some(last),
        () = claim.ended() => None,
    };
    if let Some(last) = ended {
        return last;
    }
    // Ended by the connection: what it already has, it writes first.
    while let Some(Watched::Note(note)) = watch.rest().await {
        write(&note, watch, lines, claim).await;
    }
    Ok(Reply::Watched {
        ended: WatchEnd::Unwatched,
    })
}

/// Writes each of a list's summaries to the connection as they come: its answer once it ends,
/// which is only ever why it could not go on.
async fn follow_list(
    watch: &mut ListWatch,
    lines: &mpsc::Sender<String>,
) -> Result<Reply, ApiError> {
    loop {
        match watch.next().await {
            Some(Watched::Note(note)) => {
                let _ = lines.send(notify(&note)).await;
            }
            Some(Watched::Ended(ended)) => return ended.map(|ended| Reply::Watched { ended }),
            None => return Err(ApiError::new(code::INTERNAL, Sentence::WatchStopped)),
        }
    }
}

/// Writes each of a watch's notifications to the connection, noting how far it has written
/// its thread: its answer once it ends.
async fn follow_watch(
    watch: &mut Watch,
    lines: &mpsc::Sender<String>,
    claim: &Claim,
) -> Result<Reply, ApiError> {
    loop {
        match watch.next().await {
            Some(Watched::Note(note)) => write(&note, watch, lines, claim).await,
            Some(Watched::Ended(ended)) => return ended.map(|ended| Reply::Watched { ended }),
            None => return Err(ApiError::new(code::INTERNAL, Sentence::WatchStopped)),
        }
    }
}

/// Writes one of a watch's notifications to the connection, noting how far it has written.
async fn write(
    note: &Arc<Notification>,
    watch: &Watch,
    lines: &mpsc::Sender<String>,
    claim: &Claim,
) {
    let _ = lines.send(notify(note)).await;
    if let Notification::Event { thread, event } = note.as_ref()
        && *thread == watch.thread()
    {
        claim.sent(event.seq);
    }
}

/// Writes each of a turn's notifications to the connection, unless the connection watches its
/// thread, whose watch writes them: then its answer waits until the watch has written the last
/// event the turn saw recorded.
async fn follow_turn(
    mut stream: TurnStream,
    lines: &mpsc::Sender<String>,
    mut turn: Option<&mut Turn>,
) -> Result<Reply, ApiError> {
    let thread = stream.thread();
    let quiet = turn.as_ref().is_some_and(|turn| turn.quiet());
    let mut last = None;
    loop {
        match stream.next().await {
            Some(TurnItem::Note(note)) => {
                if let Notification::Event { thread: of, event } = note.as_ref()
                    && *of == thread
                {
                    last = Some(event.seq);
                }
                if !quiet {
                    let _ = lines.send(notify(&note)).await;
                }
            }
            // Only a client of a server elsewhere misses any.
            Some(TurnItem::Missed(_)) => {}
            Some(TurnItem::Ended(ended)) => {
                if let (Some(turn), Some(last)) = (turn.as_mut(), last) {
                    turn.written(last).await;
                }
                return ended.map(|turn| Reply::Turn { turn });
            }
            None => return Err(ApiError::new(code::INTERNAL, Sentence::TurnStopped)),
        }
    }
}

/// One message read as a request with its id, or why it is not one.
type Parsed = Result<(Option<Id>, Request), (Option<Id>, ApiError)>;

/// Reads one message as a request, or says why it is not one.
pub(crate) fn parse(line: &str) -> Parsed {
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

/// A reply of `thread/events`, as a client reads it.
#[derive(Deserialize)]
struct EventsPage {
    events: Vec<glave_event::RawEvent>,
    #[serde(default)]
    next: Option<glave_event::Seq>,
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
    if result.get("events").is_some() {
        // Events hold their payloads as raw JSON, which a reply read by its shape cannot
        // give them, so a page of events is read as itself, from text.
        return serde_json::from_str::<EventsPage>(&result.to_string())
            .map(|page| Reply::Events {
                events: page.events,
                next: page.next,
            })
            .map_err(|e| {
                ApiError::new(
                    code::INTERNAL,
                    format!("the server's page of events is not one glave knows: {e}"),
                )
            });
    }
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
                permissions: None,
                effort: None,
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
                permissions: None,
                effort: None,
            }
        );
    }

    #[test]
    fn a_client_reads_a_page_of_events_with_their_payloads_as_written() {
        let event = r#"{"glave":1,"id":"0192f5c0-0000-7000-8000-000000000001","thread":"0192f5c0-0000-7000-8000-000000000002","seq":1,"at":"2026-10-01T00:00:00.000Z","type":"thread.renamed","v":1,"data":{"title":"Garden"}}"#;
        let page = format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"events":[{event}]}}}}"#);
        let response: Value = serde_json::from_str(&page).unwrap();
        let Ok(Reply::Events { events, next }) = outcome(&response) else {
            panic!("a page of events")
        };
        assert_eq!(next, None);
        assert_eq!(events[0].data.get(), r#"{"title":"Garden"}"#);
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
