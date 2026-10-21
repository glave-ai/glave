//! What an HTTP answer sends: one JSON message, or a stream as server-sent events, each
//! notification as it comes and the response last. The followed thread's events carry their
//! `seq` as their id, and a list's summaries their `updated_at`, so a client knows where to
//! reconnect from, and a stream quiet for 15 seconds gets a comment, so proxies keep it open. A client that stops reading stops only its stream: a turn
//! goes on in the server.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use glave_event::ThreadId;
use hyper::body::{Body, Bytes, Frame, SizeHint};
use tokio::time::{Instant, Sleep, sleep};

use crate::protocol::Notification;
use crate::rpc::{Id, notify, respond};
use crate::streamed::{Item, Streamed};

/// How long a stream may say nothing before it sends a comment.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The next item of a stream, with the stream handed back for the one after.
type Next = Pin<Box<dyn Future<Output = (Streamed, Option<Item>)> + Send>>;

/// An HTTP answer's body.
pub(crate) enum Answer {
    /// One whole message, or nothing.
    Once(Option<Bytes>),
    /// A stream as `text/event-stream`, until its response has been sent.
    Stream {
        next: Option<Next>,
        id: Option<Id>,
        /// The thread followed, whose events' ids are their `seq`s; none for a list.
        watched: Option<ThreadId>,
        /// When the stream has been quiet long enough for a comment.
        quiet: Pin<Box<Sleep>>,
    },
}

impl Answer {
    pub(crate) fn once(text: String) -> Self {
        Self::Once(Some(Bytes::from(text)))
    }

    pub(crate) fn bytes(bytes: Bytes) -> Self {
        Self::Once(Some(bytes))
    }

    pub(crate) fn empty() -> Self {
        Self::Once(None)
    }

    /// `streamed` as events, ending with the response to request `id`.
    pub(crate) fn stream(streamed: Streamed, id: Option<Id>) -> Self {
        Self::Stream {
            watched: streamed.watched(),
            next: Some(following(streamed)),
            id,
            quiet: Box::pin(sleep(KEEP_ALIVE)),
        }
    }
}

fn following(mut streamed: Streamed) -> Next {
    Box::pin(async move {
        let item = streamed.next().await;
        (streamed, item)
    })
}

/// One server-sent event carrying `message`, with `id` when it has one.
fn event(id: Option<String>, message: &str) -> Frame<Bytes> {
    let id = id.map(|id| format!("id: {id}\n")).unwrap_or_default();
    Frame::data(Bytes::from(format!("{id}data: {message}\n\n")))
}

/// `note` as an event: an event of the thread `watched` with its `seq` as its id, a list's
/// summary with its `updated_at`.
fn note(note: &Arc<Notification>, watched: Option<ThreadId>) -> Frame<Bytes> {
    let id = match note.as_ref() {
        Notification::Event { thread, event } if Some(*thread) == watched => {
            Some(event.seq.get().to_string())
        }
        Notification::Changed { summary, .. } => Some(summary.updated_at.to_string()),
        _ => None,
    };
    event(id, &notify(note))
}

impl Body for Answer {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        match self.get_mut() {
            Self::Once(bytes) => Poll::Ready(bytes.take().map(|bytes| Ok(Frame::data(bytes)))),
            Self::Stream {
                next,
                id,
                watched,
                quiet,
            } => {
                let Some(pending) = next.as_mut() else {
                    return Poll::Ready(None);
                };
                let Poll::Ready((streamed, item)) = pending.as_mut().poll(cx) else {
                    ready!(quiet.as_mut().poll(cx));
                    quiet.as_mut().reset(Instant::now() + KEEP_ALIVE);
                    return Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(
                        b": keep-alive\n\n",
                    )))));
                };
                quiet.as_mut().reset(Instant::now() + KEEP_ALIVE);
                match item {
                    Some(Item::Note(sent)) => {
                        *next = Some(following(streamed));
                        Poll::Ready(Some(Ok(note(&sent, *watched))))
                    }
                    Some(Item::Last(last)) => {
                        *next = None;
                        let message = respond(id.as_ref(), last.as_ref().as_ref());
                        Poll::Ready(Some(Ok(event(None, &message))))
                    }
                    None => {
                        *next = None;
                        Poll::Ready(None)
                    }
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            Self::Once(bytes) => bytes.is_none(),
            Self::Stream { next, .. } => next.is_none(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Once(Some(bytes)) => {
                SizeHint::with_exact(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            }
            Self::Once(None) => SizeHint::with_exact(0),
            Self::Stream { .. } => SizeHint::default(),
        }
    }
}

#[cfg(test)]
mod tests;
