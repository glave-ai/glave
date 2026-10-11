//! A computer across a connection: the client end of `glave computer serve`. Every call is
//! one request; a command's output streams back as notifications, and cancelling sends
//! `command/cancel`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use glave_ports::{
    BoxFuture, Bytes, Cancellation, Command, ComputerError, ComputerId, ComputerInfo, Computers,
    Environment, FileWrite, Finished, GitCredential, Glob, Globbed, Listed, Listing, Output, Save,
    Saved, Search, Searched, Sources, Stat, WorkspaceId, WorkspaceInfo,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::wire::{self, Chunk, MAX_LINE, OUTPUT, Request};

/// Messages queued ahead of a slow connection.
const QUEUE: usize = 256;
/// Output chunks held for a command whose caller is slow to take them.
const CHUNKS: usize = 64;

type Answer = Result<Value, Value>;

struct Shared {
    /// Calls waiting for their answer; `None` once the connection has closed.
    pending: Mutex<Option<HashMap<u64, oneshot::Sender<Answer>>>>,
    streams: Mutex<HashMap<String, mpsc::Sender<String>>>,
}

/// Computers reached over one connection.
pub struct RemoteComputers {
    shared: Arc<Shared>,
    outgoing: mpsc::Sender<String>,
    next: AtomicU64,
    /// The reader and the writer, stopped when this is dropped.
    tasks: [JoinHandle<()>; 2],
}

impl Drop for RemoteComputers {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl RemoteComputers {
    /// Speaks the protocol over `input` and `output`: a child's stdio, or a TCP stream.
    pub fn connect<R, W>(input: R, output: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let shared = Arc::new(Shared {
            pending: Mutex::new(Some(HashMap::new())),
            streams: Mutex::new(HashMap::new()),
        });
        let (outgoing, mut queued) = mpsc::channel::<String>(QUEUE);
        let writer = tokio::spawn(async move {
            let mut output = output;
            while let Some(line) = queued.recv().await {
                let written = async {
                    output.write_all(line.as_bytes()).await?;
                    output.write_all(b"\n").await?;
                    output.flush().await
                };
                if written.await.is_err() {
                    break;
                }
            }
        });
        let reader = tokio::spawn(read(BufReader::new(input), Arc::clone(&shared)));
        Self {
            shared,
            outgoing,
            next: AtomicU64::new(1),
            tasks: [reader, writer],
        }
    }

    /// Presents the token a TCP server asks for; the first call on such a connection.
    pub async fn authenticate(&self, token: &str) -> Result<(), ComputerError> {
        let request = Request::Authenticate {
            token: token.to_owned(),
        };
        self.call(&request).await.map(drop)
    }

    /// Checks that the server speaks this protocol version; a server that does not is
    /// refused before any work is sent to it.
    pub async fn check_protocol(&self) -> Result<(), ComputerError> {
        let info = self.call(&Request::Discover {}).await?;
        let speaks = info
            .get("protocols")
            .and_then(Value::as_array)
            .is_some_and(|protocols| protocols.iter().any(|p| p == wire::PROTOCOL));
        if speaks {
            Ok(())
        } else {
            Err(ComputerError::Unsupported(format!(
                "the computer does not speak protocol {}: {info}",
                wire::PROTOCOL
            )))
        }
    }

    /// Sends a request; its answer arrives on the receiver.
    async fn send(&self, request: &Request) -> Result<oneshot::Receiver<Answer>, ComputerError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        self.shared
            .pending
            .lock()
            .map_err(|_| unreachable("the pending table's lock is poisoned"))?
            .as_mut()
            .ok_or_else(|| unreachable("the connection is closed"))?
            .insert(id, answer);
        self.outgoing
            .send(wire::request(id, request))
            .await
            .map_err(|_| unreachable("the connection is closed"))?;
        Ok(answered)
    }

    async fn call(&self, request: &Request) -> Result<Value, ComputerError> {
        let answered = self.send(request).await?;
        settle(answered.await)
    }

    async fn typed<T: DeserializeOwned>(&self, request: &Request) -> Result<T, ComputerError> {
        let value = self.call(request).await?;
        serde_json::from_value(value)
            .map_err(|error| unreachable(&format!("the computer's answer made no sense: {error}")))
    }

    async fn run_now(
        &self,
        workspace: WorkspaceId,
        command: &Command,
        output: &dyn Output,
        cancel: &dyn Cancellation,
    ) -> Result<Finished, ComputerError> {
        let run = format!("run-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (chunks_in, mut chunks) = mpsc::channel(CHUNKS);
        self.shared
            .streams
            .lock()
            .map_err(|_| unreachable("the stream table's lock is poisoned"))?
            .insert(run.clone(), chunks_in);
        let request = Request::Run {
            workspace,
            command: command.clone(),
            run: run.clone(),
        };
        let answered = self.send(&request).await;
        let answer = match answered {
            Ok(mut answered) => {
                let mut cancelled = cancel.cancelled();
                let mut asked_to_cancel = false;
                loop {
                    tokio::select! {
                        biased;
                        answer = &mut answered => break answer,
                        Some(text) = chunks.recv() => output.write(&text).await,
                        () = &mut cancelled, if !asked_to_cancel => {
                            asked_to_cancel = true;
                            let stop = wire::notify("command/cancel", &json!({"run": run}));
                            let _ = self.outgoing.send(stop).await;
                        }
                    }
                }
            }
            Err(error) => {
                self.forget(&run);
                return Err(error);
            }
        };
        // Output sent before the answer is already here; hand it over before returning.
        while let Ok(text) = chunks.try_recv() {
            output.write(&text).await;
        }
        self.forget(&run);
        let value = settle(answer)?;
        serde_json::from_value(value)
            .map_err(|error| unreachable(&format!("the computer's answer made no sense: {error}")))
    }

    fn forget(&self, run: &str) {
        if let Ok(mut streams) = self.shared.streams.lock() {
            streams.remove(run);
        }
    }
}

fn unreachable(message: &str) -> ComputerError {
    ComputerError::Unreachable(message.to_owned())
}

/// A response as the computer meant it: its result, or its error as it was raised.
fn settle(answer: Result<Answer, oneshot::error::RecvError>) -> Result<Value, ComputerError> {
    match answer {
        Err(_) => Err(unreachable(
            "the connection closed before the computer answered",
        )),
        Ok(Ok(result)) => Ok(result),
        Ok(Err(error)) => {
            if let Some(raised) = error
                .get("data")
                .and_then(|data| ComputerError::deserialize(data).ok())
            {
                return Err(raised);
            }
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message");
            Err(unreachable(&format!(
                "the computer refused the request: {message}"
            )))
        }
    }
}

/// Reads answers and output until the connection ends, then fails what is still waiting.
async fn read<R: AsyncRead + Unpin>(mut input: BufReader<R>, shared: Arc<Shared>) {
    loop {
        let mut line = String::new();
        let got = tokio::io::AsyncReadExt::take(&mut input, MAX_LINE)
            .read_line(&mut line)
            .await;
        match got {
            Ok(0) | Err(_) => break,
            Ok(_) => dispatch(&shared, &line).await,
        }
    }
    // Dropping the senders tells every waiting call that the connection is gone.
    if let Ok(mut pending) = shared.pending.lock() {
        *pending = None;
    }
    if let Ok(mut streams) = shared.streams.lock() {
        streams.clear();
    }
}

async fn dispatch(shared: &Shared, line: &str) {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if message.get("method").and_then(Value::as_str) == Some(OUTPUT) {
        let Some(chunk) = message
            .get("params")
            .and_then(|params| Chunk::deserialize(params).ok())
        else {
            return;
        };
        let stream = shared
            .streams
            .lock()
            .ok()
            .and_then(|streams| streams.get(&chunk.run).cloned());
        if let Some(stream) = stream {
            let _ = stream.send(chunk.text).await;
        }
        return;
    }
    let Some(id) = message.get("id").and_then(Value::as_u64) else {
        return;
    };
    let waiting = shared
        .pending
        .lock()
        .ok()
        .and_then(|mut pending| pending.as_mut().and_then(|waiting| waiting.remove(&id)));
    if let Some(waiting) = waiting {
        let answer = match (message.get("result"), message.get("error")) {
            (Some(result), None) => Ok(result.clone()),
            (_, Some(error)) => Err(error.clone()),
            (None, None) => Err(json!({"message": "an answer with neither result nor error"})),
        };
        let _ = waiting.send(answer);
    }
}

impl Computers for RemoteComputers {
    fn create<'a>(
        &'a self,
        environment: &'a Environment,
    ) -> BoxFuture<'a, Result<ComputerInfo, ComputerError>> {
        let request = Request::Create {
            environment: environment.clone(),
        };
        Box::pin(async move { self.typed(&request).await })
    }

    fn status(&self, computer: ComputerId) -> BoxFuture<'_, Result<ComputerInfo, ComputerError>> {
        Box::pin(async move { self.typed(&Request::Status { computer }).await })
    }

    fn destroy(&self, computer: ComputerId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(async move { self.call(&Request::Destroy { computer }).await.map(drop) })
    }

    fn open<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        let request = Request::Open {
            computer,
            sources: sources.clone(),
            credential: None,
        };
        Box::pin(async move { self.typed(&request).await })
    }

    fn open_with_credential<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
        credential: &'a GitCredential,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        let request = Request::Open {
            computer,
            sources: sources.clone(),
            credential: Some(credential.clone()),
        };
        Box::pin(async move { self.typed(&request).await })
    }

    fn close(&self, workspace: WorkspaceId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(async move { self.call(&Request::Close { workspace }).await.map(drop) })
    }

    fn run<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>> {
        Box::pin(self.run_now(workspace, command, output, cancel))
    }

    fn read<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ComputerError>> {
        #[derive(Deserialize)]
        struct Read {
            content: Bytes,
        }
        let request = Request::Read {
            workspace,
            path: path.to_owned(),
        };
        Box::pin(async move { self.typed::<Read>(&request).await.map(|read| read.content) })
    }

    fn write<'a>(
        &'a self,
        workspace: WorkspaceId,
        files: &'a [FileWrite],
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        let request = Request::Write {
            workspace,
            files: files.to_vec(),
        };
        Box::pin(async move { self.call(&request).await.map(drop) })
    }

    fn list<'a>(
        &'a self,
        workspace: WorkspaceId,
        listing: &'a Listing,
    ) -> BoxFuture<'a, Result<Listed, ComputerError>> {
        let request = Request::List {
            workspace,
            listing: listing.clone(),
        };
        Box::pin(async move { self.typed(&request).await })
    }

    fn stat<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Stat, ComputerError>> {
        let request = Request::Stat {
            workspace,
            path: path.to_owned(),
        };
        Box::pin(async move { self.typed(&request).await })
    }

    fn glob<'a>(
        &'a self,
        workspace: WorkspaceId,
        glob: &'a Glob,
    ) -> BoxFuture<'a, Result<Globbed, ComputerError>> {
        let request = Request::Glob {
            workspace,
            glob: glob.clone(),
        };
        Box::pin(async move { self.typed(&request).await })
    }

    fn search<'a>(
        &'a self,
        workspace: WorkspaceId,
        search: &'a Search,
    ) -> BoxFuture<'a, Result<Searched, ComputerError>> {
        let request = Request::Search {
            workspace,
            search: search.clone(),
        };
        Box::pin(async move { self.typed(&request).await })
    }

    fn save<'a>(
        &'a self,
        workspace: WorkspaceId,
        save: &'a Save,
    ) -> BoxFuture<'a, Result<Saved, ComputerError>> {
        let request = Request::Save {
            workspace,
            save: save.clone(),
        };
        Box::pin(async move { self.typed(&request).await })
    }
}
