//! What an HTTP answer sends: one JSON message, or a turn as server-sent events, each
//! notification as it comes and the response last. A client that stops reading stops only
//! its stream: the turn goes on in the server.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use hyper::body::{Body, Bytes, Frame, SizeHint};

use crate::protocol::{Notification, Reply};
use crate::rpc::{Id, notify, respond};
use crate::turn::{TurnItem, TurnStream};

/// The next item of a turn, with the stream handed back for the one after.
type Next = Pin<Box<dyn Future<Output = (TurnStream, Option<TurnItem>)> + Send>>;

/// An HTTP answer's body.
pub(crate) enum Answer {
    /// One whole message, or nothing.
    Once(Option<Bytes>),
    /// A turn as `text/event-stream`, until its response has been sent.
    Turn { next: Option<Next>, id: Option<Id> },
}

impl Answer {
    pub(crate) fn once(text: String) -> Self {
        Self::Once(Some(Bytes::from(text)))
    }

    pub(crate) fn empty() -> Self {
        Self::Once(None)
    }

    pub(crate) fn turn(stream: TurnStream, id: Option<Id>) -> Self {
        Self::Turn {
            next: Some(following(stream)),
            id,
        }
    }
}

fn following(mut stream: TurnStream) -> Next {
    Box::pin(async move {
        let item = stream.next().await;
        (stream, item)
    })
}

/// One server-sent event carrying `message`.
fn event(message: &str) -> Frame<Bytes> {
    Frame::data(Bytes::from(format!("data: {message}\n\n")))
}

fn note(note: &Arc<Notification>) -> Frame<Bytes> {
    event(&notify(note))
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
            Self::Turn { next, id } => loop {
                let Some(pending) = next.as_mut() else {
                    return Poll::Ready(None);
                };
                let (stream, item) = ready!(pending.as_mut().poll(cx));
                match item {
                    Some(TurnItem::Note(sent)) => {
                        *next = Some(following(stream));
                        return Poll::Ready(Some(Ok(note(&sent))));
                    }
                    // Missed notifications are in the log; the client reads them by `seq`.
                    Some(TurnItem::Missed(_)) => *next = Some(following(stream)),
                    Some(TurnItem::Ended(ended)) => {
                        *next = None;
                        let reply = ended.map(|turn| Reply::Turn { turn });
                        let message = respond(id.as_ref(), reply.as_ref());
                        return Poll::Ready(Some(Ok(event(&message))));
                    }
                    None => {
                        *next = None;
                        return Poll::Ready(None);
                    }
                }
            },
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            Self::Once(bytes) => bytes.is_none(),
            Self::Turn { next, .. } => next.is_none(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Once(Some(bytes)) => {
                SizeHint::with_exact(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            }
            Self::Once(None) => SizeHint::with_exact(0),
            Self::Turn { .. } => SizeHint::default(),
        }
    }
}
