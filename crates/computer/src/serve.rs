//! Serving computers over one connection (`glave computer serve`): requests run
//! concurrently, a command's output streams as notifications, and when the connection ends
//! every command it started is stopped.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use glave_ports::{BoxFuture, ComputerError, Computers, Output};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _,
    BufReader,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::cancel::Switch;
use crate::wire::{self, Chunk, MAX_LINE, OUTPUT, Request, code};

/// Lines written ahead of a slow client.
const WRITE_AHEAD: usize = 256;
/// Requests one connection may have running at once; more wait their turn.
const MAX_IN_FLIGHT: usize = 64;
/// Connections one TCP listener serves at once.
const MAX_CONNECTIONS: usize = 32;

/// One connection's shared state.
struct Connection<C> {
    computers: Arc<C>,
    /// Running commands by run name, so `command/cancel` can stop them.
    runs: Mutex<HashMap<String, Switch>>,
    lines: mpsc::Sender<String>,
}

/// Serves `computers` to one client until its input ends. With a `token`, the first message
/// must be `connection/authenticate` with that token, or the connection ends.
pub async fn serve<C, R, W>(
    computers: Arc<C>,
    input: R,
    output: W,
    token: Option<String>,
) -> std::io::Result<()>
where
    C: Computers + 'static,
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
    let connection = Arc::new(Connection {
        computers,
        runs: Mutex::new(HashMap::new()),
        lines,
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

/// Reads and answers requests until the input ends, then stops what is still running.
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
            Err(error) => break Err(error),
        };
        if line.trim().is_empty() {
            continue;
        }
        while running.len() >= MAX_IN_FLIGHT {
            running.join_next().await;
        }
        let connection = Arc::clone(connection);
        running.spawn(async move { connection.answer(&line).await });
        while running.try_join_next().is_some() {}
    };
    // The client is gone: stop its commands, then let every request finish.
    if let Ok(runs) = connection.runs.lock() {
        for switch in runs.values() {
            switch.cancel();
        }
    }
    while running.join_next().await.is_some() {}
    read
}

/// One line, or `None` at the end. A line longer than [`MAX_LINE`] is an error.
async fn next_line<R: AsyncBufRead + Unpin>(input: &mut R) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let got = input.take(MAX_LINE).read_line(&mut line).await?;
    if got == 0 {
        return Ok(None);
    }
    if !line.ends_with('\n') && u64::try_from(line.len()).unwrap_or(u64::MAX) >= MAX_LINE {
        return Err(std::io::Error::other(format!(
            "a message is longer than {MAX_LINE} bytes"
        )));
    }
    Ok(Some(line))
}

/// Whether the first message carries `token`. A wrong one is answered, then the connection
/// ends.
async fn authenticate<R: AsyncBufRead + Unpin>(
    input: &mut R,
    lines: &mpsc::Sender<String>,
    token: &str,
) -> std::io::Result<bool> {
    let Some(line) = next_line(input).await? else {
        return Ok(false);
    };
    let (id, accepted) = match wire::parse(&line) {
        Ok((id, Request::Authenticate { token: given })) => (id, same(&given, token)),
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

/// Compares secrets in time that does not depend on where they differ.
fn same(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |differ, (a, b)| differ | (a ^ b))
            == 0
}

impl<C: Computers + 'static> Connection<C> {
    async fn answer(&self, line: &str) -> () {
        let (id, answered) = match wire::parse(line) {
            Ok((id, request)) => (id, self.handle(request).await),
            Err((id, failure)) => (id, Err(failure)),
        };
        // A request without an id is a notification, which is never answered.
        if let Some(id) = id {
            let _ = self.lines.send(wire::respond(Some(&id), answered)).await;
        }
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
            Request::Open { computer, sources } => reply(computers.open(computer, &sources).await),
            Request::Close { workspace } => reply(computers.close(workspace).await),
            Request::Run {
                workspace,
                command,
                run,
            } => {
                let switch = self.start(&run)?;
                let output = Forward {
                    run: run.clone(),
                    lines: &self.lines,
                };
                let finished = computers.run(workspace, &command, &output, &switch).await;
                if let Ok(mut runs) = self.runs.lock() {
                    runs.remove(&run);
                }
                reply(finished)
            }
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
            Request::Save { workspace, save } => reply(computers.save(workspace, &save).await),
        }
    }

    /// Registers a run under its name, refusing a name already running.
    fn start(&self, run: &str) -> Result<Switch, Value> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| wire::failure(code::COMPUTER, "the run table's lock is poisoned"))?;
        if runs.contains_key(run) {
            let error = ComputerError::Refused(format!("a run named {run:?} is already running"));
            return Err(wire::computer_failure(&error));
        }
        let switch = Switch::default();
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
/// first present `token`. Connections from other machines need TLS, which comes later, so a
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
        let (read, write) = stream.into_split();
        let (computers, token) = (Arc::clone(&computers), token.clone());
        connections.spawn(serve(computers, BufReader::new(read), write, Some(token)));
        while connections.try_join_next().is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_whole() {
        assert!(same("abc", "abc"));
        assert!(!same("abd", "abc"));
        assert!(!same("ab", "abc"));
    }
}
