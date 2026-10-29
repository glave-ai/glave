//! Steering a running turn (`specs/session.md`, "Steering a turn"): what the person sends while
//! a turn runs waits in the thread's inbox, and the turn gives it to the model at its next
//! call, after the tool results already due, as the person's messages. What arrives after the
//! model's last call starts the next turn at once; what the model will not read (the turn was
//! stopped or failed first, or a hook stopped it) is given back.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::{EventId, MessageAdded, Part, Role, TurnResult};
use glave_ports::{BlobStore, Caller, EventLog, Message, ModelProvider};
use thiserror::Error;

use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::turn::{Live, Prompt};

/// The most messages that wait for the model at once; one more is refused.
pub const MAX_QUEUED: usize = 20;

/// A message the person sent while a turn ran, waiting for the model's next call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    /// The id of the `message.added` it is recorded as once the model reads it, made when it
    /// was sent, so whoever sent it knows the event that says it was read.
    pub id: EventId,
    /// What the person wrote; a `/command` is sent as the command's text, as a prompt is.
    pub text: String,
    /// The files they added to it, as `file` parts.
    pub files: Vec<Part>,
    /// Who sent it: its `author` when that is not the person the thread is for.
    pub caller: Caller,
}

/// Why a message was not queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NotQueued {
    #[error(
        "{MAX_QUEUED} messages are already waiting for the model to read them: wait until it \
         has, or stop it"
    )]
    Full,
    /// The work it was opened for has made its last model call and is ending: send the
    /// message again once it has ended, as a new turn.
    #[error("the turn is ending")]
    Ending,
}

/// What the person sent a thread's running turn, waiting for the model's next call. Clones
/// share it: whoever may send holds one while the turn reads another. Its lock is never held
/// across an await.
#[derive(Debug, Clone, Default)]
pub struct Inbox {
    waiting: Arc<Mutex<Waiting>>,
}

#[derive(Debug, Default)]
struct Waiting {
    queued: VecDeque<Queued>,
    /// Set once the work it was opened for takes no more.
    ended: bool,
}

impl Inbox {
    fn lock(&self) -> MutexGuard<'_, Waiting> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes messages again, for new work on the thread.
    pub fn open(&self) {
        self.lock().ended = false;
    }

    /// Queues `message` behind those already waiting.
    pub fn push(&self, message: Queued) -> Result<(), NotQueued> {
        let mut waiting = self.lock();
        if waiting.ended {
            return Err(NotQueued::Ending);
        }
        if waiting.queued.len() >= MAX_QUEUED {
            return Err(NotQueued::Full);
        }
        waiting.queued.push_back(message);
        Ok(())
    }

    /// Ends it: it takes nothing more until opened again, and what waits is given back, oldest
    /// first.
    pub fn end(&self) -> Vec<Queued> {
        let mut waiting = self.lock();
        waiting.ended = true;
        waiting.queued.drain(..).collect()
    }

    /// Takes what waits, oldest first, for the model's next call: from then on the turn that
    /// took it records it or gives it back, and nothing else can give it back meanwhile.
    fn take(&self) -> VecDeque<Queued> {
        std::mem::take(&mut self.lock().queued)
    }

    /// The oldest message, for a turn of its own; with none, ends the inbox in the same step,
    /// so a message sent from then on starts new work once this has ended, and none is
    /// stranded.
    fn next_or_end(&self) -> Option<Queued> {
        let mut waiting = self.lock();
        let next = waiting.queued.pop_front();
        waiting.ended = next.is_none();
        next
    }
}

impl Thread {
    /// Where messages sent to this thread's running turn wait for the model's next call.
    pub fn inbox(&self) -> &Inbox {
        &self.inbox
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Gives the model what the person sent while the turn ran, oldest first, recorded together
    /// as their messages. Each passes the `UserPromptSubmit` hooks first, as a prompt does; one
    /// a hook stops is never sent, and is given back with the hook's reason. What waits is
    /// taken at once, so each message is recorded or given back, never both: stopped before
    /// they are recorded, the turn takes none of them, and every one a hook did not stop is
    /// given back, as it is when this fails.
    pub(crate) async fn read_queued(
        &self,
        thread: &mut Thread,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<(), HarnessError> {
        let mut waiting = thread.inbox.take();
        if waiting.is_empty() {
            return Ok(());
        }
        let mut passed = Vec::with_capacity(waiting.len());
        let mut hooks = Ok(());
        // Once stopped, no more hooks run.
        while hooks.is_ok()
            && !cancel.is_cancelled()
            && let Some(queued) = waiting.pop_front()
        {
            // A server's prompt that could not be had is given back with why, as a hook's stop is.
            let text = match self.expand(thread, &queued.text, cancel).await {
                Ok(text) => text,
                Err(why) => {
                    live(Live::Unread {
                        id: queued.id,
                        reason: Some(why),
                    });
                    continue;
                }
            };
            let blocked = if thread.delegated() {
                Ok(None)
            } else {
                self.on_prompt(thread, &text, cancel).await
            };
            match blocked {
                Ok(Some(reason)) => live(Live::Unread {
                    id: queued.id,
                    reason: Some(reason),
                }),
                Ok(None) => passed.push((queued, text)),
                Err(error) => {
                    waiting.push_front(queued);
                    hooks = Err(error);
                }
            }
        }
        // Stopped, the turn takes none of them.
        let stopped = hooks.is_ok() && cancel.is_cancelled();
        let recorded = match hooks {
            Ok(()) if !stopped => self.record_read(thread, &passed).await,
            hooks => hooks,
        };
        if stopped || recorded.is_err() {
            let left = passed.into_iter().map(|(queued, _)| queued).chain(waiting);
            unread(left, live);
            return recorded;
        }
        for (queued, text) in passed {
            thread.messages.push(Message::User {
                parts: said(&text, &queued.files),
            });
            if !thread.delegated() {
                thread.asked.push(text.clone());
            }
            thread.request = Some(text);
        }
        Ok(())
    }

    /// Records the messages that `passed` their hooks as the person's, in one append, each
    /// under the id it was queued with.
    async fn record_read(
        &self,
        thread: &mut Thread,
        passed: &[(Queued, String)],
    ) -> Result<(), HarnessError> {
        let mut events = Vec::with_capacity(passed.len());
        for (queued, text) in passed {
            let message = MessageAdded {
                role: Role::User,
                parts: said(text, &queued.files),
                author: Self::other_than_for(thread, queued.caller.person),
                thought: None,
            };
            self.stage_as(thread, &mut events, &message, queued.id)?;
        }
        self.commit(thread, events).await
    }

    /// Runs what the person sent too late for the turn that just ended (`ran`): a turn for the
    /// oldest message, the rest read at its first call, and so on until nothing waits, every
    /// turn added up as a goal's are. When `ran` failed or was stopped, nothing more runs: what
    /// waits is given back, and the inbox ends.
    pub async fn run_queued(
        &self,
        thread: &mut Thread,
        ran: Result<TurnResult, HarnessError>,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        let mut total = match ran {
            Ok(total) => total,
            // A turn that waits keeps what was sent to it for when it goes on.
            Err(waits @ HarnessError::Waits(_)) => return Err(waits),
            Err(error) => {
                give_back(thread, live);
                return Err(error);
            }
        };
        loop {
            if cancel.is_cancelled() {
                give_back(thread, live);
                break;
            }
            let Some(next) = thread.inbox.next_or_end() else {
                break;
            };
            let prompt = Prompt {
                text: &next.text,
                files: &next.files,
                caller: &next.caller,
                by_glave: false,
                id: Some(next.id),
            };
            let ran = self.run_prompt(thread, prompt, cancel, live).await;
            match ran {
                Ok(turn) => {
                    total.absorb(&turn);
                    total.outcome = turn.outcome;
                }
                Err(HarnessError::PromptBlocked(reason) | HarnessError::PromptFailed(reason)) => {
                    live(Live::Unread {
                        id: next.id,
                        reason: Some(reason),
                    });
                }
                Err(error) => {
                    give_back(thread, live);
                    return Err(error);
                }
            }
        }
        Ok(total)
    }
}

/// Ends the thread's inbox, telling whoever sent what still waits that the model will not read
/// it.
fn give_back(thread: &Thread, live: &mut (dyn FnMut(Live) + Send)) {
    unread(thread.inbox.end(), live);
}

/// A message read mid-turn as the person sent it: its `text`, then its `files`, as a message
/// that starts a turn holds them.
fn said(text: &str, files: &[Part]) -> Vec<Part> {
    let mut parts = vec![Part::Text {
        text: text.to_owned(),
    }];
    parts.extend_from_slice(files);
    parts
}

/// Tells whoever sent each of `given` that the model will not read it.
fn unread(given: impl IntoIterator<Item = Queued>, live: &mut (dyn FnMut(Live) + Send)) {
    for queued in given {
        live(Live::Unread {
            id: queued.id,
            reason: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(n: u8) -> Queued {
        Queued {
            id: EventId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap(),
            text: format!("message {n}"),
            files: Vec::new(),
            caller: Caller::default(),
        }
    }

    #[test]
    fn messages_wait_in_order_up_to_the_cap() {
        let inbox = Inbox::default();
        let sender = inbox.clone();
        for n in 0..20 {
            sender.push(queued(n)).unwrap();
        }
        assert_eq!(sender.push(queued(20)), Err(NotQueued::Full));
        let taken = inbox.take();
        assert_eq!(taken.len(), MAX_QUEUED, "clones share what waits");
        assert_eq!(taken.front(), Some(&queued(0)));
        assert_eq!(taken.back(), Some(&queued(19)));
        // Taken, they no longer count: as many more may wait while they are read.
        sender.push(queued(20)).unwrap();
        assert_eq!(inbox.take(), [queued(20)]);
        assert_eq!(inbox.end(), [], "nothing taken is given back again");
    }

    #[test]
    fn an_inbox_ends_only_when_nothing_waits_and_opens_again() {
        let inbox = Inbox::default();
        inbox.push(queued(1)).unwrap();
        assert_eq!(inbox.next_or_end(), Some(queued(1)));
        inbox.push(queued(2)).unwrap();
        assert_eq!(inbox.next_or_end(), Some(queued(2)));
        assert_eq!(inbox.next_or_end(), None);
        assert_eq!(inbox.push(queued(3)), Err(NotQueued::Ending));
        inbox.open();
        inbox.push(queued(3)).unwrap();
        inbox.push(queued(4)).unwrap();
        assert_eq!(inbox.end(), [queued(3), queued(4)]);
        assert_eq!(inbox.push(queued(5)), Err(NotQueued::Ending));
        assert_eq!(inbox.take(), []);
    }
}
