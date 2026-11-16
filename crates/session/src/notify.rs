//! Telling a person's devices what needs them (`specs/session.md`, "Push notifications"): an
//! approval, a question or a service's ask that waits in a thread of theirs, a task of theirs
//! that finished or failed, or a chat of theirs whose turn ended while no one watched it,
//! pushed to each browser that takes their notifications. A push says one sentence of the
//! server's own, with the thread's title, and which thread it is about; nothing the thread
//! says. Needs are pushed [`AT_ONCE`] at a time and each person's browsers all
//! at once, each push within glave-push's own limit, so no one's slow push service holds up
//! anyone else's. A browser its push service says is gone, or that keeps failing, is forgotten.

use std::sync::Arc;
use std::time::Duration;

use glave_event::{Body, Outcome, Payload as _, Seq, ServiceAsked, ThreadId, TurnResult, decode};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope};
use glave_push::{Message, Sent, Urgency};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::{JoinError, JoinSet};

use crate::protocol::{ApiError, Notification, Update};
use crate::push::Pushing;
use crate::server::Server;
use crate::words::{Language, Sentence, say};

/// How long a push service keeps a notification for a device that is away: a day, after which
/// what it said is old news.
const KEPT_FOR: Duration = Duration::from_hours(24);

/// The most of a thread's title a notification says, in characters.
const TITLE_CHARS: usize = 60;

/// How many needs are pushed at once.
const AT_ONCE: usize = 8;

/// What needs a person, as the hub hears of it, in the thread they started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Need {
    /// Something waits for them there: an approval, a question, a service's ask.
    Waiting { thread: ThreadId },
    /// A task ended there, as `outcome` says.
    Ended {
        thread: ThreadId,
        outcome: Option<Outcome>,
    },
    /// A turn ended there, as `outcome` says, at its event `ended`: told only of a chat its
    /// person started that no one watches (`Hub::publish`).
    Finished {
        thread: ThreadId,
        ended: Seq,
        outcome: Option<Outcome>,
    },
}

impl Need {
    /// What `notification` says needs a person, in the thread it was published in.
    pub(crate) fn of(notification: &Notification) -> Option<Self> {
        match notification {
            Notification::Live {
                thread,
                update: Update::ApprovalPending { .. } | Update::Asked { .. },
            } => Some(Self::Waiting { thread: *thread }),
            Notification::Event { thread, event } if event.kind.as_str() == ServiceAsked::TYPE => {
                Some(Self::Waiting { thread: *thread })
            }
            Notification::TaskFinished { thread, task } => Some(Self::Ended {
                thread: *thread,
                outcome: task.outcome,
            }),
            Notification::Event { thread, event } if event.kind.as_str() == TurnResult::TYPE => {
                let outcome = match decode(event.clone()).map(|decoded| decoded.body) {
                    Ok(Body::TurnEnded(result)) => Some(result.outcome),
                    _ => None,
                };
                Some(Self::Finished {
                    thread: *thread,
                    ended: event.seq,
                    outcome,
                })
            }
            _ => None,
        }
    }

    /// The same need, in `thread`: the one its person started, of which the thread it arose in
    /// may be a subagent's.
    pub(crate) fn in_thread(self, thread: ThreadId) -> Self {
        match self {
            Self::Waiting { .. } => Self::Waiting { thread },
            Self::Ended { outcome, .. } => Self::Ended { thread, outcome },
            Self::Finished { ended, outcome, .. } => Self::Finished {
                thread,
                ended,
                outcome,
            },
        }
    }

    /// Whether it is a turn's end, told only of a chat its person started that no one watches.
    pub(crate) fn is_turns_end(self) -> bool {
        matches!(self, Self::Finished { .. })
    }

    /// What the worker is told it is: what waits, which replaces the thread's last
    /// notification, or a task's or a chat's end, which always alerts.
    fn kind(self) -> &'static str {
        match self {
            Self::Waiting { .. } => "waiting",
            Self::Ended { .. } => "ended",
            Self::Finished { .. } => "finished",
        }
    }

    fn thread(self) -> ThreadId {
        match self {
            Self::Waiting { thread }
            | Self::Ended { thread, .. }
            | Self::Finished { thread, .. } => thread,
        }
    }

    /// The `seq` of the event a chat's turn ended at, where a tap opens the chat.
    fn ended(self) -> Option<Seq> {
        match self {
            Self::Finished { ended, .. } => Some(ended),
            Self::Waiting { .. } | Self::Ended { .. } => None,
        }
    }

    /// What a notification of it says and how soon it should arrive: none for a task or a
    /// turn the person stopped themselves.
    fn told(self, title: Option<String>) -> Option<(Sentence, Urgency)> {
        let title = title.map(|title| shortened(&title));
        let outcome = match self {
            Self::Waiting { .. } => {
                return Some(match title {
                    Some(title) => (Sentence::PushWaiting { title }, Urgency::High),
                    None => (Sentence::PushWaitingUntitled, Urgency::High),
                });
            }
            Self::Ended { outcome, .. } | Self::Finished { outcome, .. } => outcome,
        };
        let chat = matches!(self, Self::Finished { .. });
        let sentence = match (outcome, title) {
            (Some(Outcome::Cancelled), _) => return None,
            (Some(Outcome::Completed), Some(title)) => Sentence::PushDone { title },
            (Some(Outcome::Completed), None) if chat => Sentence::PushChatDoneUntitled,
            (Some(Outcome::Completed), None) => Sentence::PushDoneUntitled,
            (_, Some(title)) => Sentence::PushNotDone { title },
            (_, None) if chat => Sentence::PushChatNotDoneUntitled,
            (_, None) => Sentence::PushNotDoneUntitled,
        };
        Some((sentence, Urgency::Normal))
    }
}

/// Says why a push of a need, `done` now, did not get through.
fn told(done: Option<Result<Result<(), ApiError>, JoinError>>) {
    match done {
        Some(Ok(Err(error))) => tracing::warn!(%error, "a notification could not be pushed"),
        Some(Err(error)) => tracing::warn!(%error, "a notification's push stopped"),
        Some(Ok(Ok(()))) | None => {}
    }
}

/// `title`, cut to [`TITLE_CHARS`] with an ellipsis where it is longer.
fn shortened(title: &str) -> String {
    let mut chars = title.chars();
    let mut short: String = chars.by_ref().take(TITLE_CHARS).collect();
    if chars.next().is_some() {
        short.push('…');
    }
    short
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Tells people's browsers what needs them, for as long as it runs: each need this server
    /// hears of from now on, a few at a time. Runs until dropped; on a server that pushes
    /// nothing it waits, doing nothing.
    pub async fn push_notifications(&self) {
        let Some(pushing) = self.inner.push.clone() else {
            return std::future::pending().await;
        };
        let mut needs = self.inner.hub.needs();
        let mut pushes = JoinSet::new();
        loop {
            tokio::select! {
                heard = needs.recv() => match heard {
                    Ok(need) => {
                        if pushes.len() >= AT_ONCE {
                            told(pushes.join_next().await);
                        }
                        let (server, pushing) = (self.clone(), Arc::clone(&pushing));
                        pushes.spawn(async move { server.push(&pushing, need).await });
                    }
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!(missed, "notifications came faster than they were pushed");
                    }
                    Err(RecvError::Closed) => return std::future::pending().await,
                },
                done = pushes.join_next(), if !pushes.is_empty() => told(done),
            }
        }
    }

    /// Pushes `need` to every browser of the person whose thread it is in, at once.
    async fn push(&self, pushing: &Arc<Pushing>, need: Need) -> Result<(), ApiError> {
        let thread = need.thread();
        // A thread this server forgot the place of is looked for in its own organization.
        let scope = if let Ok(scope) = self.scope_of(thread) {
            scope
        } else {
            let located = self.located(self.inner.own, thread).await?;
            Scope::glaves_own(located.org, located.project)
        };
        let summary = self.summary(scope, thread).await?;
        let Some((sentence, urgency)) = need.told(summary.title) else {
            return Ok(());
        };
        let caller = Caller {
            person: summary.for_.as_ref().and_then(|whose| whose.id),
            org: Some(scope.org()),
            ..Caller::default()
        };
        let (_, personal) = self.personal_project(&caller).await?;
        let devices = self.devices(&personal, self.person_of(&caller)?).await?;
        if devices.is_empty() {
            return Ok(());
        }
        let key = Arc::new(self.push_key().await?);
        let language = self
            .chosen_language(&caller)
            .await?
            .map_or(Language::English, |(_, language)| language);
        let mut text = serde_json::json!({
            "text": say(language, &sentence),
            "thread": thread,
            "org": scope.org(),
            "kind": need.kind(),
        });
        if let (Some(ended), Some(said)) = (need.ended(), text.as_object_mut()) {
            said.insert("ended".to_owned(), serde_json::json!(ended));
        }
        let text: Arc<str> = Arc::from(text.to_string());
        let now = self.inner.harness.clock().now();
        let mut sends = JoinSet::new();
        for device in devices {
            let (pushing, key, text) = (Arc::clone(pushing), Arc::clone(&key), Arc::clone(&text));
            sends.spawn(async move {
                let message = Message {
                    text: text.as_bytes(),
                    keep: KEPT_FOR,
                    urgency,
                };
                let sent = pushing.send(&key, &device.subscribed, message, now).await;
                (device.subscribed.device, sent)
            });
        }
        while let Some(sent) = sends.join_next().await {
            match sent {
                Ok((_, Ok(Sent::Taken))) => {}
                Ok((device, Ok(Sent::Gone))) => {
                    // One that can't be forgotten now is forgotten at its next push: its push
                    // service says it is gone again, or its count of refusals still stands.
                    if let Err(error) = self.forget_device(&personal, device, true).await {
                        tracing::warn!(%error, "a gone browser could not be forgotten");
                    }
                }
                Ok((_, Err(error))) => {
                    tracing::warn!(%error, "a push service did not take a notification");
                }
                Err(error) => tracing::warn!(%error, "a push stopped before its answer"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread() -> ThreadId {
        ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [7; 10]).unwrap()
    }

    #[test]
    fn each_need_says_its_sentence_and_how_soon() {
        let waiting = Need::Waiting { thread: thread() };
        let title = || Some("fix-login".to_owned());
        let (said, urgency) = waiting.told(title()).unwrap();
        assert_eq!(
            said,
            Sentence::PushWaiting {
                title: "fix-login".into()
            }
        );
        assert_eq!(urgency, Urgency::High);
        let done = |outcome| Need::Ended {
            thread: thread(),
            outcome: Some(outcome),
        };
        let (said, urgency) = done(Outcome::Completed).told(None).unwrap();
        assert_eq!(
            (said, urgency),
            (Sentence::PushDoneUntitled, Urgency::Normal)
        );
        let (said, _) = done(Outcome::Failed).told(title()).unwrap();
        assert_eq!(
            said,
            Sentence::PushNotDone {
                title: "fix-login".into()
            }
        );
        assert_eq!(
            done(Outcome::Cancelled).told(title()),
            None,
            "a task the person stopped tells no one"
        );
        let finished = |outcome| Need::Finished {
            thread: thread(),
            ended: Seq::FIRST,
            outcome: Some(outcome),
        };
        let (said, urgency) = finished(Outcome::Completed).told(title()).unwrap();
        assert_eq!(
            (said, urgency),
            (
                Sentence::PushDone {
                    title: "fix-login".into()
                },
                Urgency::Normal
            )
        );
        let (said, _) = finished(Outcome::Completed).told(None).unwrap();
        assert_eq!(said, Sentence::PushChatDoneUntitled);
        let (said, _) = finished(Outcome::Failed).told(None).unwrap();
        assert_eq!(said, Sentence::PushChatNotDoneUntitled);
        assert_eq!(finished(Outcome::Cancelled).told(title()), None);
    }

    #[test]
    fn a_long_title_is_cut_with_an_ellipsis() {
        assert_eq!(shortened("short"), "short");
        let long = "é".repeat(TITLE_CHARS + 5);
        let cut = shortened(&long);
        assert_eq!(cut.chars().count(), TITLE_CHARS + 1);
        assert!(cut.ends_with('…'));
    }
}
