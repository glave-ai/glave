//! A client of a session server over a stream of lines (`specs/session.md`, "Transports"): one
//! JSON-RPC message per line, as on the person's Unix socket. Requests go side by side on the
//! one connection, each answered by its id, so an approval can be answered while a turn
//! streams. A turn's notifications carry no id: each goes to every turn followed on the
//! connection, then the turn's response ends it, as the same [`TurnItem`]s HTTP gives. Leaving
//! stops nothing: a turn goes on in the server when the connection closes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::protocol::{ApiError, Reply, Request, code};
use crate::rpc::{message, outcome};
use crate::turn::TurnItem;

/// Lines written ahead of the server.
const WRITE_AHEAD: usize = 64;
/// Notifications a turn's follower may fall behind before the rest are counted as missed.
const FOLLOW_AHEAD: usize = 1_024;

/// Who waits for the answer to a request.
enum Waiter {
    Reply(oneshot::Sender<Value>),
    Turn(Following),
}

/// What a followed turn is sent: its notifications, how many it missed, then its ending.
struct Following {
    notes: mpsc::Sender<TurnItem>,
    missed: Arc<AtomicU64>,
    end: oneshot::Sender<TurnItem>,
}

/// The requests waiting for an answer, by id; none once the connection has closed.
type Waiting = Arc<Mutex<Option<HashMap<u64, Waiter>>>>;

fn lock(waiting: &Waiting) -> std::sync::MutexGuard<'_, Option<HashMap<u64, Waiter>>> {
    waiting.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One connection to a server, reading and writing on tasks of its own, stopped when it drops.
pub(crate) struct Lines {
    outgoing: mpsc::Sender<String>,
    waiting: Waiting,
    next_id: AtomicU64,
    name: String,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl Drop for Lines {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
    }
}

impl Lines {
    /// A client reading the server's lines from `read` and writing requests to `write`;
    /// `name` names the server in errors.
    pub(crate) fn new<R, W>(read: R, write: W, name: String) -> Self
    where
        R: AsyncBufRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let waiting: Waiting = Arc::new(Mutex::new(Some(HashMap::new())));
        let reader = tokio::spawn(read_all(read, Arc::clone(&waiting)));
        let (outgoing, queued) = mpsc::channel(WRITE_AHEAD);
        let writer = tokio::spawn(write_all(write, queued));
        Self {
            outgoing,
            waiting,
            next_id: AtomicU64::new(1),
            name,
            reader,
            writer,
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Answers `request`, which must not stream.
    pub(crate) async fn request(&self, request: &Request) -> Result<Reply, ApiError> {
        let (answer, answered) = oneshot::channel();
        self.send(Waiter::Reply(answer), request).await?;
        let response = answered.await.map_err(|_| self.closed())?;
        outcome(&response)
    }

    /// Starts `request` (`turn/start` or `goal/start`) and follows it; refused, as over HTTP,
    /// when the server answers before the turn starts.
    pub(crate) async fn start(&self, request: &Request) -> Result<LinesTurn, ApiError> {
        let (notes, items) = mpsc::channel(FOLLOW_AHEAD);
        let (end, ending) = oneshot::channel();
        let missed = Arc::new(AtomicU64::new(0));
        let following = Following {
            notes,
            missed: Arc::clone(&missed),
            end,
        };
        self.send(Waiter::Turn(following), request).await?;
        let mut turn = LinesTurn {
            items,
            missed,
            ending,
            first: None,
            ended: false,
            name: self.name.clone(),
        };
        match turn.next().await {
            Some(TurnItem::Ended(Err(refused))) => Err(refused),
            Some(first) => {
                // Given again by the first `next`.
                turn.first = Some(first);
                turn.ended = false;
                Ok(turn)
            }
            None => Err(self.closed()),
        }
    }

    /// Sends `request` under a new id, `waiter` waiting for its answer.
    async fn send(&self, waiter: Waiter, request: &Request) -> Result<(), ApiError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        match lock(&self.waiting).as_mut() {
            Some(waiting) => waiting.insert(id, waiter),
            None => return Err(self.closed()),
        };
        let line = message(id, request).to_string();
        if self.outgoing.send(line).await.is_err() {
            if let Some(waiting) = lock(&self.waiting).as_mut() {
                waiting.remove(&id);
            }
            return Err(self.closed());
        }
        Ok(())
    }

    fn closed(&self) -> ApiError {
        ApiError::new(
            code::INTERNAL,
            format!("{} closed the connection", self.name),
        )
    }
}

/// Writes each queued line, until the connection drops.
async fn write_all<W: AsyncWrite + Unpin>(mut write: W, mut queued: mpsc::Receiver<String>) {
    while let Some(mut line) = queued.recv().await {
        line.push('\n');
        if write.write_all(line.as_bytes()).await.is_err() || write.flush().await.is_err() {
            break;
        }
    }
}

/// Reads the server's lines until it closes the connection, giving each answer to its waiter
/// and each notification to every turn followed; then tells every waiter left it closed.
async fn read_all<R: AsyncBufRead + Unpin>(read: R, waiting: Waiting) {
    let mut lines = read.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if message.get("method").is_some() {
            if let Some(note) = super::item(&line) {
                notify(&waiting, &note);
            }
            continue;
        }
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let waiter = lock(&waiting)
            .as_mut()
            .and_then(|waiting| waiting.remove(&id));
        match waiter {
            Some(Waiter::Reply(answer)) => {
                let _ = answer.send(message);
            }
            Some(Waiter::Turn(following)) => {
                let _ = following.end.send(ended(&message));
            }
            None => {}
        }
    }
    // Dropping every waiter tells each that the connection closed.
    lock(&waiting).take();
}

/// Gives `note` to every turn followed on the connection; one too far behind counts it missed.
fn notify(waiting: &Waiting, note: &TurnItem) {
    let guard = lock(waiting);
    let followed = guard.iter().flat_map(HashMap::values);
    for waiter in followed {
        if let Waiter::Turn(following) = waiter
            && following.notes.try_send(note.clone()).is_err()
        {
            following.missed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// How a turn ended, from its response.
fn ended(response: &Value) -> TurnItem {
    TurnItem::Ended(match outcome(response) {
        Ok(Reply::Turn { turn }) => Ok(turn),
        // Queued to a turn already running, as HTTP answers it with JSON.
        Ok(Reply::Queued { .. }) => Err(ApiError::queued_elsewhere()),
        Ok(other) => Err(ApiError::new(
            code::INTERNAL,
            format!("the turn ended with {other:?}"),
        )),
        Err(error) => Err(error),
    })
}

/// A turn followed over a connection of lines.
pub(crate) struct LinesTurn {
    items: mpsc::Receiver<TurnItem>,
    missed: Arc<AtomicU64>,
    ending: oneshot::Receiver<TurnItem>,
    /// The item `start` read to know the turn started.
    first: Option<TurnItem>,
    ended: bool,
    name: String,
}

impl LinesTurn {
    /// The next item, or `None` after the ending.
    pub(crate) async fn next(&mut self) -> Option<TurnItem> {
        if self.ended {
            return None;
        }
        if let Some(first) = self.first.take() {
            self.ended = matches!(first, TurnItem::Ended(_));
            return Some(first);
        }
        // Notifications come before the response that ends the turn, so they are taken first.
        let item = if let Ok(item) = self.items.try_recv() {
            item
        } else {
            let missed = self.missed.swap(0, Ordering::Relaxed);
            if missed > 0 {
                return Some(TurnItem::Missed(missed));
            }
            tokio::select! {
                biased;
                Some(item) = self.items.recv() => item,
                ending = &mut self.ending => ending.unwrap_or_else(|_| {
                    TurnItem::Ended(Err(ApiError::new(
                        code::INTERNAL,
                        format!("{} closed the connection before the turn ended", self.name),
                    )))
                }),
            }
        };
        self.ended = matches!(item, TurnItem::Ended(_));
        Some(item)
    }
}
