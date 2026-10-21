//! What a transport carries for a request that streams: a turn's notifications then its
//! result, or a watch's then how it ended, as one stream of notifications and a last answer.

use std::sync::Arc;

use glave_event::ThreadId;

use crate::protocol::{ApiError, Notification, Reply};
use crate::turn::{TurnItem, TurnStream};
use crate::watch::{Watch, Watched};

/// A request's answer as a stream.
pub(crate) enum Streamed {
    Turn(TurnStream),
    Watch(Watch),
}

/// One thing a stream sends.
pub(crate) enum Item {
    Note(Arc<Notification>),
    /// The request's answer. Always the last item.
    Last(Box<Result<Reply, ApiError>>),
}

impl Streamed {
    /// The next item, or `None` after the last.
    pub(crate) async fn next(&mut self) -> Option<Item> {
        match self {
            Self::Turn(turn) => loop {
                match turn.next().await? {
                    TurnItem::Note(note) => return Some(Item::Note(note)),
                    // Missed notifications are in the log; the client reads them by `seq`.
                    TurnItem::Missed(_) => {}
                    TurnItem::Ended(ended) => {
                        let last = ended.map(|turn| Reply::Turn { turn });
                        return Some(Item::Last(Box::new(last)));
                    }
                }
            },
            Self::Watch(watch) => Some(match watch.next().await? {
                Watched::Note(note) => Item::Note(note),
                Watched::Ended(ended) => {
                    Item::Last(Box::new(ended.map(|ended| Reply::Watched { ended })))
                }
            }),
        }
    }

    /// The thread a watch watches, whose events' `seq`s mark the client's place in it.
    pub(crate) fn watched(&self) -> Option<ThreadId> {
        match self {
            Self::Turn(_) => None,
            Self::Watch(watch) => Some(watch.thread()),
        }
    }
}
