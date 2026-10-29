//! Serving computers over one connection (`glave computer serve`): requests run
//! concurrently, a command's output streams as notifications, a command offered credentials
//! asks the client for each one, and when the connection ends every command it is running is
//! stopped. Commands in the background belong to the computer, not the connection: they go on
//! until they are stopped, their workspace closes, or nothing reads them for an hour.

mod asking;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use glave_ports::computer::{
    Command, ComputerError, Computers, CredentialHost, Output, WorkspaceId,
};
use glave_ports::{BoxFuture, Clock as _, SystemClock};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _,
    BufReader,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;

use crate::cancel::CancelSwitch;
use crate::token::{Token, Tokens};
use crate::wire::{self, Chunk, MAX_LINE, OUTPUT, Request, code};
use asking::{Asking, Asks};

/// Lines written ahead of a slow client.
const WRITE_AHEAD: usize = 256;
/// Requests one connection may have running at once; more wait their turn.
const MAX_IN_FLIGHT: usize = 64;
/// Connections one TCP listener serves at once.
const MAX_CONNECTIONS: usize = 32;
/// How long a client has for each part of its handshake, its TLS and then signing in, before
/// it is dropped: a connection that never signs in would otherwise hold one of the few
/// [`MAX_CONNECTIONS`] for good.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// Request ids one connection remembers, so a request delivered twice runs once.
const SEEN: usize = 4096;

/// One connection's shared state.
struct Connection<C> {
    computers: Arc<C>,
    /// Running commands by run name, so `command/cancel` can stop them.
    runs: Mutex<HashMap<String, CancelSwitch>>,
    seen: Mutex<Seen>,
    lines: mpsc::Sender<String>,
    /// The computer's own credential requests, waiting for the client's answers.
    asks: Asks,
}

/// Serves `computers` to one client until its input ends. With a `token`, the first message
/// must be `connection/authenticate` with that token, before it expires and within 10
/// seconds, or the connection ends.
pub async fn serve<C, R, W>(
    computers: Arc<C>,
    input: R,
    output: W,
    token: Option<Token>,
) -> std::io::Result<()>
where
    C: Computers + 'static,
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (lines, mut queued) = mpsc::channel::<String>(WRITE_AHEAD);
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(mut line) = queued.recv().await {
            // One write per message: a line and its newline written apart wait on each
            // other over TCP (Nagle and delayed acknowledgement, about 40 ms on Linux).
            line.push('\n');
            output.write_all(line.as_bytes()).await?;
            output.flush().await?;
        }
        Ok::<_, std::io::Error>(())
    });
    let connection = Arc::new(Connection {
        computers,
        runs: Mutex::new(HashMap::new()),
        seen: Mutex::new(Seen::default()),
        lines,
        asks: Asks::default(),
    });
    let mut input = input;
    let read = match token {
        Some(token) if !authenticate(&mut input, &connection.lines, &token).await? => Ok(()),
        _ => requests(&connection, &mut input).await,
    };
    // Every request has finished, so this is the last handle on the connection: dropping it
    // closes the queue, and the writer ends once it has written what is queued.
    drop(connection);
    let written = writer.await.map_err(std::io::Error::other)?;
    read.and(written)
}

/// Reads and answers requests until the input ends, then stops what is still running. Input it
/// cannot read (not text, or a message too long) ends it too, answered first as a request it
/// cannot read: a client still there closes the connection on that answer, so it never takes
/// the commands stopped here for commands it cancelled.
async fn requests<C, R>(connection: &Arc<Connection<C>>, input: &mut R) -> std::io::Result<()>
where
    C: Computers + 'static,
    R: AsyncBufRead + Unpin,
{
    let mut running = JoinSet::new();
    let read = loop {
        let line = match next_line(input).await {
            Ok(Some(line)) => line,
            Ok(None) => break Ok(()),
            Err(error) => {
                let unreadable = wire::failure(code::PARSE, &format!("not JSON-RPC: {error}"));
                let _ = connection
                    .lines
                    .send(wire::respond(None, Err(unreadable)))
                    .await;
                break Err(error);
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        // The client's answer to a credential request of the computer's own, taken here so a
        // full set of running requests, each waiting for one, never holds its answer back.
        if let Some((id, answer)) = wire::response(&line) {
            connection.asks.answered(id, answer);
            continue;
        }
        while running.len() >= MAX_IN_FLIGHT {
            running.join_next().await;
        }
        let connection = Arc::clone(connection);
        running.spawn(async move { connection.answer(&line).await });
        while running.try_join_next().is_some() {}
    };
    // The client is gone, or can no longer be read: stop its commands, then let every request
    // finish.
    if let Ok(runs) = connection.runs.lock() {
        #[expect(
            clippy::iter_over_hash_type,
            reason = "every run is cancelled, so the order is never seen"
        )]
        for switch in runs.values() {
            switch.cancel();
        }
    }
    while running.join_next().await.is_some() {}
    read
}

/// One line, or `None` at the end. A line that is not UTF-8, or longer than [`MAX_LINE`], is an
/// error; a last line the input ended in the middle of was cut off, so it is not acted on.
async fn next_line<R: AsyncBufRead + Unpin>(input: &mut R) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let got = input.take(MAX_LINE).read_line(&mut line).await?;
    if got == 0 {
        return Ok(None);
    }
    if !line.ends_with('\n') {
        if u64::try_from(line.len()).unwrap_or(u64::MAX) >= MAX_LINE {
            return Err(std::io::Error::other(format!(
                "a message is longer than {MAX_LINE} bytes"
            )));
        }
        return Ok(None);
    }
    Ok(Some(line))
}

/// Whether the first message carries `token`, within [`HANDSHAKE`]. A wrong one, or none in
/// time, is answered, then the connection ends.
async fn authenticate<R: AsyncBufRead + Unpin>(
    input: &mut R,
    lines: &mpsc::Sender<String>,
    token: &Token,
) -> std::io::Result<bool> {
    let Ok(first) = tokio::time::timeout(HANDSHAKE, next_line(input)).await else {
        let late = format!(
            "no connection/authenticate came within {}, so the connection is closed",
            glave_ports::spoken(HANDSHAKE)
        );
        let refused = wire::failure(code::UNAUTHENTICATED, &late);
        let _ = lines.send(wire::respond(None, Err(refused))).await;
        return Ok(false);
    };
    let Some(line) = first? else {
        return Ok(false);
    };
    let (id, accepted) = match wire::parse(&line) {
        Ok((id, Request::Authenticate { token: given })) => (
            id,
            token.admits(&given, SystemTime::from(SystemClock.now())),
        ),
        Ok((id, _)) | Err((id, _)) => (id, false),
    };
    let answer = if accepted {
        Ok(json!({}))
    } else {
        Err(wire::failure(
            code::UNAUTHENTICATED,
            "the first message must be connection/authenticate with a valid token",
        ))
    };
    let _ = lines.send(wire::respond(id.as_ref(), answer)).await;
    Ok(accepted)
}

/// The request ids a connection has seen lately, so a request that arrives twice runs once.
#[derive(Default)]
struct Seen {
    ids: HashSet<String>,
    order: VecDeque<String>,
}

impl Seen {
    /// Whether `id` is new; remembers it, forgetting the oldest past [`SEEN`].
    fn first(&mut self, id: &Value) -> bool {
        let id = id.to_string();
        if !self.ids.insert(id.clone()) {
            return false;
        }
        self.order.push_back(id);
        if self.order.len() > SEEN
            && let Some(oldest) = self.order.pop_front()
        {
            self.ids.remove(&oldest);
        }
        true
    }
}

impl<C: Computers + 'static> Connection<C> {
    /// Whether a request with `id` is the first with it on this connection.
    fn first(&self, id: &Value) -> bool {
        self.seen.lock().is_ok_and(|mut seen| seen.first(id))
    }

    async fn answer(&self, line: &str) -> () {
        let (id, answered) = match wire::parse(line) {
            // A second copy of a request is dropped unanswered: the first is (or was)
            // answered, and running it again could do the work twice.
            Ok((Some(id), _)) if !self.first(&id) => return,
            // A notification (no id) is never answered.
            Ok((None, request)) => {
                let _ = self.handle(request).await;
                return;
            }
            Ok((Some(id), request)) => (Some(id), self.handle(request).await),
            // A message that cannot be read is answered, with no id when its id cannot be
            // read either (JSON-RPC's null id), so the client knows at once.
            Err((id, failure)) => (id, Err(failure)),
        };
        let _ = self.lines.send(wire::respond(id.as_ref(), answered)).await;
    }

    async fn handle(&self, request: Request) -> Result<Value, Value> {
        let computers = &*self.computers;
        match request {
            Request::Authenticate { .. } => Ok(json!({})),
            Request::Discover {} => Ok(json!({
                "name": "glave-computer",
                "version": env!("CARGO_PKG_VERSION"),
                "protocols": [wire::PROTOCOL],
            })),
            Request::Create { environment } => reply(computers.create(&environment).await),
            Request::Status { computer } => reply(computers.status(computer).await),
            Request::Destroy { computer } => reply(computers.destroy(computer).await),
            Request::Open {
                computer,
                sources,
                credential,
            } => reply(match &credential {
                Some(credential) => {
                    computers
                        .open_with_credential(computer, &sources, credential)
                        .await
                }
                None => computers.open(computer, &sources).await,
            }),
            Request::Close { workspace } => reply(computers.close(workspace).await),
            Request::Run {
                workspace,
                command,
                run,
                credentials,
            } => self.run(workspace, &command, run, credentials).await,
            Request::Cancel { run } => {
                if let Some(switch) = self
                    .runs
                    .lock()
                    .ok()
                    .and_then(|runs| runs.get(&run).cloned())
                {
                    switch.cancel();
                }
                Ok(json!({}))
            }
            request @ (Request::Start { .. }
            | Request::Output { .. }
            | Request::Ended { .. }
            | Request::Stop { .. }
            | Request::Commands { .. }) => self.background(request).await,
            Request::Read { workspace, path } => reply(
                computers
                    .read(workspace, &path)
                    .await
                    .map(|content| json!({"content": content})),
            ),
            Request::Write { workspace, files } => reply(computers.write(workspace, &files).await),
            Request::List { workspace, listing } => {
                reply(computers.list(workspace, &listing).await)
            }
            Request::Stat { workspace, path } => reply(computers.stat(workspace, &path).await),
            Request::Glob { workspace, glob } => reply(computers.glob(workspace, &glob).await),
            Request::Search { workspace, search } => {
                reply(computers.search(workspace, &search).await)
            }
            Request::Save { workspace, save } => reply(computers.save(workspace, &save).await),
        }
    }

    /// Answers a request about commands in the background.
    async fn background(&self, request: Request) -> Result<Value, Value> {
        let computers = &*self.computers;
        match request {
            Request::Start { workspace, command } => reply(
                computers
                    .start(workspace, &command)
                    .await
                    .map(|command| json!({"command": command})),
            ),
            Request::Output {
                workspace,
                command,
                wait_ms,
            } => {
                let wait = Duration::from_millis(wait_ms);
                reply(computers.output(workspace, command, wait).await)
            }
            Request::Ended {
                workspace,
                command,
                wait_ms,
            } => {
                let wait = Duration::from_millis(wait_ms);
                let ended = computers.ended(workspace, command, wait).await;
                reply(ended.map(|ended| json!({"ended": ended})))
            }
            Request::Stop { workspace, command } => reply(computers.stop(workspace, command).await),
            Request::Commands { workspace } => reply(
                computers
                    .commands(workspace)
                    .await
                    .map(|commands| json!({"commands": commands})),
            ),
            other => Err(wire::failure(
                code::INVALID_REQUEST,
                &format!("{} is not about commands in the background", other.method()),
            )),
        }
    }

    /// Runs `command` as `run`, streaming its output, asking the client for any credential its
    /// helper needs for the hosts `credentials` names.
    async fn run(
        &self,
        workspace: WorkspaceId,
        command: &Command,
        run: String,
        credentials: Vec<CredentialHost>,
    ) -> Result<Value, Value> {
        let computers = &*self.computers;
        let switch = self.start(&run)?;
        let output = Forward {
            run: run.clone(),
            lines: &self.lines,
        };
        let finished = if credentials.is_empty() {
            computers.run(workspace, command, &output, &switch).await
        } else {
            let asking = Asking {
                asks: &self.asks,
                lines: &self.lines,
                run: run.clone(),
                hosts: credentials,
            };
            computers
                .run_with_credentials(workspace, command, &output, &switch, &asking)
                .await
        };
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(&run);
        }
        reply(finished)
    }

    /// Registers a run under its name, refusing a name already running.
    fn start(&self, run: &str) -> Result<CancelSwitch, Value> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| wire::failure(code::COMPUTER, "the run table's lock is poisoned"))?;
        if runs.contains_key(run) {
            let error =
                ComputerError::Refused(format!("a run named {run:?} is already running").into());
            return Err(wire::computer_failure(&error));
        }
        let switch = CancelSwitch::default();
        runs.insert(run.to_owned(), switch.clone());
        Ok(switch)
    }
}

/// A computer's answer as a JSON-RPC result or error; `()` becomes `{}`.
fn reply<T: Serialize>(answered: Result<T, ComputerError>) -> Result<Value, Value> {
    match answered {
        Ok(value) => match serde_json::to_value(value) {
            Ok(Value::Null) => Ok(json!({})),
            Ok(value) => Ok(value),
            Err(error) => Err(wire::failure(code::COMPUTER, &error.to_string())),
        },
        Err(error) => Err(wire::computer_failure(&error)),
    }
}

/// Sends a command's output to the client as notifications.
struct Forward<'a> {
    run: String,
    lines: &'a mpsc::Sender<String>,
}

impl Output for Forward<'_> {
    fn write<'b>(&'b self, text: &'b str) -> BoxFuture<'b, ()> {
        let chunk = Chunk {
            run: self.run.clone(),
            text: text.to_owned(),
        };
        Box::pin(async move {
            let _ = self.lines.send(wire::notify(OUTPUT, &chunk)).await;
        })
    }
}

/// Serves `computers` on a TCP listener on this machine's loopback address, to clients that
/// first present `token`. Connections from other machines need TLS ([`serve_tls`]), so a
/// listener on any other address is refused. Runs until accepting fails.
pub async fn serve_tcp<C: Computers + 'static>(
    computers: Arc<C>,
    listener: TcpListener,
    token: String,
) -> std::io::Result<()> {
    let address = listener.local_addr()?;
    if !address.ip().is_loopback() {
        return Err(std::io::Error::other(format!(
            "{address} is not a loopback address; serving computers across machines needs TLS"
        )));
    }
    let mut connections = JoinSet::new();
    loop {
        while connections.len() >= MAX_CONNECTIONS {
            connections.join_next().await;
        }
        let (stream, _) = listener.accept().await?;
        let _ = stream.set_nodelay(true);
        let (read, write) = stream.into_split();
        let (computers, token) = (Arc::clone(&computers), Token::lasting(token.clone()));
        connections.spawn(serve(computers, BufReader::new(read), write, Some(token)));
        while connections.try_join_next().is_some() {}
    }
}

/// Serves `computers` over TLS on a TCP listener at any address, to clients that first present
/// the token `tokens` holds, before it expires: how a worker is reached from the harness. A
/// client that fails the handshake is dropped without an answer. Runs until accepting fails.
pub async fn serve_tls<C: Computers + 'static>(
    computers: Arc<C>,
    listener: TcpListener,
    tls: TlsAcceptor,
    tokens: impl Into<Tokens>,
) -> std::io::Result<()> {
    let tokens = tokens.into();
    let mut connections = JoinSet::new();
    loop {
        while connections.len() >= MAX_CONNECTIONS {
            connections.join_next().await;
        }
        let (stream, _) = listener.accept().await?;
        let _ = stream.set_nodelay(true);
        let (computers, tokens, tls) = (Arc::clone(&computers), tokens.clone(), tls.clone());
        connections.spawn(async move {
            let handshake = tokio::time::timeout(HANDSHAKE, tls.accept(stream));
            let Ok(Ok(stream)) = handshake.await else {
                return Ok(());
            };
            // The token as it is now: the side that minted it may have renewed it.
            let token = tokens.current(SystemTime::from(SystemClock.now())).await?;
            let (read, write) = tokio::io::split(stream);
            serve(computers, BufReader::new(read), write, Some(token)).await
        });
        while connections.try_join_next().is_some() {}
    }
}
