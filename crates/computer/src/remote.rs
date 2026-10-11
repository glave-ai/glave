//! A computer across a connection: the client end of `glave computer serve`. Every call is
//! one request; a command's output streams back as notifications, and cancelling sends
//! `command/cancel`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use glave_ports::{
    BoxFuture, Bytes, Cancellation, Command, ComputerError, ComputerId, ComputerInfo, Computers,
    Environment, FileWrite, Finished, GitCredential, Glob, Globbed, Listed, Listing, Output, Save,
    Saved, Search, Searched, Sources, Stat, WorkspaceId, WorkspaceInfo,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::wire::{self, Request};
use reader::{Answer, Shared};

mod reader;

/// Messages queued ahead of a slow connection.
const QUEUE: usize = 256;
/// Output chunks held for a command whose caller is slow to take them.
const CHUNKS: usize = 64;
/// How long a quick call (files, listings, status) waits for its answer.
const QUICK: Duration = Duration::from_mins(5);
/// How long a slow call waits: making or removing a computer, opening or saving a workspace
/// (a clone or a push), and a command that names no timeout of its own.
const SLOW: Duration = Duration::from_mins(30);
/// How much longer than its own timeout a command's answer may take.
const GRACE: Duration = Duration::from_secs(60);

/// Computers reached over one connection. Every call either gets its answer or fails within
/// its deadline. A call is never sent twice: when the connection ends first, its error says
/// the computer may or may not have done it.
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

/// A call waiting for its answer; forgotten if it stops waiting.
struct Waiting<'a> {
    shared: &'a Shared,
    id: u64,
    answered: oneshot::Receiver<Answer>,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.shared.pending.lock()
            && let Some(waiting) = pending.as_mut()
        {
            waiting.remove(&self.id);
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
        let shared = Arc::new(Shared::new());
        let (outgoing, queued) = mpsc::channel::<String>(QUEUE);
        let writer = tokio::spawn(write(output, queued, Arc::clone(&shared)));
        let reading = Arc::clone(&shared);
        let reader = tokio::spawn(async move {
            reader::read(BufReader::new(input), &reading).await;
        });
        Self {
            shared,
            outgoing,
            next: AtomicU64::new(1),
            tasks: [reader, writer],
        }
    }

    /// Why the connection ended, once it has: no call on it can succeed after that.
    pub fn closed(&self) -> Option<String> {
        self.shared.ended()
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

    /// Why the connection is gone, for a call that failed because of it.
    fn why_gone(&self) -> String {
        self.shared
            .ended()
            .unwrap_or_else(|| "the connection to the computer closed".into())
    }

    /// Sends a request; its answer arrives on the returned call.
    async fn send(&self, request: &Request) -> Result<Waiting<'_>, ComputerError> {
        let not_sent = || {
            let why = self.why_gone();
            unreachable(&format!("{why}; {} was not sent", request.method()))
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        self.shared
            .pending
            .lock()
            .map_err(|_| unreachable("the pending table's lock is poisoned"))?
            .as_mut()
            .ok_or_else(not_sent)?
            .insert(id, answer);
        let waiting = Waiting {
            shared: &self.shared,
            id,
            answered,
        };
        self.outgoing
            .send(wire::request(id, request))
            .await
            .map_err(|_| not_sent())?;
        Ok(waiting)
    }

    async fn call(&self, request: &Request) -> Result<Value, ComputerError> {
        let mut waiting = self.send(request).await?;
        let limit = deadline(request);
        match tokio::time::timeout(limit, &mut waiting.answered).await {
            Ok(answer) => self.settle(request.method(), answer),
            Err(_) => Err(late(request.method(), limit)),
        }
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
        let limit = deadline(&request);
        let mut waiting = match self.send(&request).await {
            Ok(waiting) => waiting,
            Err(error) => {
                self.forget(&run);
                return Err(error);
            }
        };
        let stop = || wire::notify("command/cancel", &json!({"run": run}));
        let timer = tokio::time::sleep(limit);
        tokio::pin!(timer);
        let mut cancelled = cancel.cancelled();
        let mut asked_to_cancel = false;
        let answer = loop {
            tokio::select! {
                biased;
                answer = &mut waiting.answered => break Some(answer),
                Some(text) = chunks.recv() => output.write(&text).await,
                () = &mut cancelled, if !asked_to_cancel => {
                    asked_to_cancel = true;
                    let _ = self.outgoing.send(stop()).await;
                }
                () = &mut timer => {
                    let _ = self.outgoing.try_send(stop());
                    break None;
                }
            }
        };
        // Output sent before the answer is already here; hand it over before returning.
        while let Ok(text) = chunks.try_recv() {
            output.write(&text).await;
        }
        self.forget(&run);
        let Some(answer) = answer else {
            return Err(late(request.method(), limit));
        };
        let value = self.settle(request.method(), answer)?;
        serde_json::from_value(value)
            .map_err(|error| unreachable(&format!("the computer's answer made no sense: {error}")))
    }

    fn forget(&self, run: &str) {
        if let Ok(mut streams) = self.shared.streams.lock() {
            streams.remove(run);
        }
    }

    /// A response as the computer meant it: its result, or its error as it was raised.
    fn settle(
        &self,
        method: &str,
        answer: Result<Answer, oneshot::error::RecvError>,
    ) -> Result<Value, ComputerError> {
        match answer {
            Err(_) => Err(unreachable(&format!(
                "{} before the computer answered {method}: it may or may not have done it, and \
                 glave does not send it again",
                self.why_gone()
            ))),
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
}

/// Writes queued messages until the connection ends, then closes the output so the
/// computer sees it end too.
async fn write<W: AsyncWrite + Unpin>(
    mut output: W,
    mut queued: mpsc::Receiver<String>,
    shared: Arc<Shared>,
) {
    loop {
        let line = tokio::select! {
            line = queued.recv() => line,
            () = shared.stop.notified() => None,
        };
        let Some(mut line) = line else { break };
        // One write per message, so TCP never holds the newline back.
        line.push('\n');
        let written = async {
            output.write_all(line.as_bytes()).await?;
            output.flush().await
        };
        if let Err(error) = written.await {
            shared.end(format!("the connection to the computer failed: {error}"));
            break;
        }
    }
    let _ = output.shutdown().await;
}

fn unreachable(message: &str) -> ComputerError {
    ComputerError::Unreachable(message.to_owned())
}

/// How long `request` waits for its answer before it fails.
fn deadline(request: &Request) -> Duration {
    match request {
        Request::Run { command, .. } => command
            .timeout_secs
            .map_or(SLOW, |secs| Duration::from_secs(secs).saturating_add(GRACE)),
        Request::Create { .. }
        | Request::Destroy { .. }
        | Request::Open { .. }
        | Request::Save { .. } => SLOW,
        _ => QUICK,
    }
}

/// A call that waited past its deadline.
fn late(method: &str, limit: Duration) -> ComputerError {
    unreachable(&format!(
        "the computer did not answer {method} within {} s: it may still be doing it, and \
         glave does not send it again",
        limit.as_secs()
    ))
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
