//! What a transport carries for a request that streams: a turn's notifications then its
//! result, or a watch's (of a thread, or of a list) then how it ended, as one stream of
//! notifications and a last answer.

use std::sync::Arc;

use glave_event::ThreadId;

use crate::lists::ListWatch;
use crate::protocol::{ApiError, Notification, Reply};
use crate::turn::{TurnItem, TurnStream};
use crate::watch::{Watch, Watched};

/// A request's answer as a stream.
pub(crate) enum Streamed {
    Turn(TurnStream),
    Watch(Watch),
    List(ListWatch),
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
                    // Only a client of a server elsewhere misses any.
                    TurnItem::Missed(_) => {}
                    TurnItem::Ended(ended) => {
                        let last = ended.map(|turn| Reply::Turn { turn });
                        return Some(Item::Last(Box::new(last)));
                    }
                }
            },
            Self::Watch(watch) => watch.next().await.map(watched),
            Self::List(watch) => watch.next().await.map(watched),
        }
    }

    /// The thread followed, whose events' `seq`s mark the client's place in it; none for a
    /// list, where each summary's `updated_at` does.
    pub(crate) fn watched(&self) -> Option<ThreadId> {
        match self {
            Self::Turn(turn) => Some(turn.thread()),
            Self::Watch(watch) => Some(watch.thread()),
            Self::List(_) => None,
        }
    }
}

/// One thing a watch sends, as a stream sends it.
fn watched(item: Watched) -> Item {
    match item {
        Watched::Note(note) => Item::Note(note),
        Watched::Ended(ended) => Item::Last(Box::new(ended.map(|ended| Reply::Watched { ended }))),
    }
}
