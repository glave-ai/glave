//! `thread/fork` (`specs/session.md`, "Forking a thread"): a new thread in the same project
//! whose history is another's up to a point between its turns, to try a turn again or send an
//! earlier message again changed. It works where the other works when that is the project's
//! folder itself, and is placed as `thread/start` places one otherwise, a worktree for a
//! worktree, since two threads never share a copy of their own.

use glave_event::{Body, ForkFiles, Opened, Payload as _, Seq, ThreadId, WorkspaceBound};
use glave_harness::Fork;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, Reply};
use crate::server::Server;
use crate::words::{Language, Sentence, say};

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `thread/fork`: `thread` forked for `caller` at `at`, its latest point between
    /// turns when absent, and the new thread's summary.
    pub(crate) async fn fork(
        &self,
        caller: &Caller,
        thread: ThreadId,
        at: Option<Seq>,
    ) -> Result<Reply, ApiError> {
        self.ensure_not_deleted(thread).await?;
        let scope = self.scope_of(thread)?;
        // What the person chose meanwhile applies to the new thread, as to one started.
        self.reread().await;
        let worktree = match self.latest(&scope, thread, WorkspaceBound::TYPE).await? {
            Some(Body::WorkspaceBound(bound)) => match bound.opened {
                Opened::Folder { .. } => Some(false),
                Opened::Worktree { .. } => Some(true),
                Opened::Clone { .. } | Opened::Scratch { .. } => None,
            },
            _ => None,
        };
        let place = self.place_for(caller, scope, worktree, false).await?;
        let fork = Fork {
            from: thread,
            at,
            told,
        };
        let forked = self
            .inner
            .harness
            .fork_thread(scope, caller, fork, place)
            .await
            .map_err(|error| ApiError::from(&error))?;
        let id = forked.id;
        self.place(id, scope);
        self.keep(id, forked).await;
        self.opened(caller, scope, id).await
    }
}

/// What a fork's `thread.forked` tells the person, as `files` stands.
pub(crate) fn forked(files: ForkFiles) -> Sentence {
    match files {
        ForkFiles::Same => Sentence::ForkedInPlace,
        ForkFiles::Other => Sentence::ForkedElsewhere,
    }
}

/// [`forked`], in English, as `thread.forked` records it.
fn told(files: ForkFiles) -> String {
    say(Language::English, &forked(files))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glave_event::{At, EventId, ForkPoint, Header, ThreadForked, encode};

    use super::*;
    use crate::protocol::Notification;
    use crate::words::note_in;

    #[test]
    fn where_a_fork_begins_is_said_in_the_readers_language() {
        let at = jiff::Timestamp::UNIX_EPOCH;
        let thread = ThreadId::new(at, [3; 10]).unwrap();
        for files in [ForkFiles::Same, ForkFiles::Other] {
            let forked = ThreadForked {
                from: ForkPoint {
                    thread: ThreadId::new(at, [1; 10]).unwrap(),
                    seq: Seq::FIRST,
                },
                files,
                told: told(files),
                commands: 0,
            };
            let header = Header {
                id: EventId::new(at, [2; 10]).unwrap(),
                thread,
                seq: Seq::FIRST,
                at: At::from(at),
            };
            let event = encode(header, &forked).unwrap();
            let note = Arc::new(Notification::Event { thread, event });
            let english = note_in(&note, Language::English);
            assert!(
                Arc::ptr_eq(&english, &note),
                "English is as it was recorded"
            );
            let written = note_in(&note, Language::Accented);
            let Notification::Event { event, .. } = written.as_ref() else {
                panic!("an event");
            };
            let said = serde_json::to_value(&event.data).unwrap()["told"].clone();
            let said = said.as_str().unwrap_or_default().to_owned();
            assert!(said.starts_with('[') && said != forked.told, "{said}");
        }
    }
}
