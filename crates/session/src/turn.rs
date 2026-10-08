//! Following a turn as it runs: its notifications in order, then how it ended.

use std::future::poll_fn;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::task::Poll;

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, oneshot};

use crate::protocol::{ApiError, Notification, TurnResult, code};
use crate::server::{Ended, api};

/// One thing a running turn sends.
#[derive(Debug, Clone)]
pub enum TurnItem {
    Note(Arc<Notification>),
    /// The follower fell behind and missed this many notifications. The stored events are
    /// still in the log: read them with `thread/events` after the last `seq` seen.
    Missed(u64),
    /// How the turn ended. Always the last item.
    Ended(Result<TurnResult, ApiError>),
}

/// A turn's notifications, then its ending.
pub struct TurnStream {
    notes: broadcast::Receiver<Arc<Notification>>,
    done: oneshot::Receiver<Ended>,
    ended: bool,
}

impl TurnStream {
    pub(crate) fn new(
        notes: broadcast::Receiver<Arc<Notification>>,
        done: oneshot::Receiver<Ended>,
    ) -> Self {
        Self {
            notes,
            done,
            ended: false,
        }
    }

    /// The next item, or `None` after the ending.
    pub async fn next(&mut self) -> Option<TurnItem> {
        if self.ended {
            return None;
        }
        let Self { notes, done, .. } = self;
        let mut note = pin!(notes.recv());
        let item = poll_fn(|cx| {
            // Everything sent before the turn ended is in the channel by then, so notes are
            // always taken first and the ending comes last.
            match note.as_mut().poll(cx) {
                Poll::Ready(Ok(note)) => return Poll::Ready(TurnItem::Note(note)),
                Poll::Ready(Err(RecvError::Lagged(missed))) => {
                    return Poll::Ready(TurnItem::Missed(missed));
                }
                Poll::Ready(Err(RecvError::Closed)) | Poll::Pending => {}
            }
            Pin::new(&mut *done).poll(cx).map(|ended| {
                TurnItem::Ended(match ended {
                    Ok(ran) => ran.map(TurnResult::from).map_err(|error| api(&error)),
                    Err(_) => Err(ApiError::new(code::INTERNAL, "the turn stopped")),
                })
            })
        })
        .await;
        self.ended = matches!(item, TurnItem::Ended(_));
        Some(item)
    }

    /// Follows the turn to its end, ignoring its notifications.
    pub async fn finish(&mut self) -> Result<TurnResult, ApiError> {
        while let Some(item) = self.next().await {
            if let TurnItem::Ended(ended) = item {
                return ended;
            }
        }
        Err(ApiError::new(code::INTERNAL, "the turn already ended"))
    }
}
