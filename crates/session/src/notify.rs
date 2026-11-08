//! Telling a person's devices what needs them (`specs/session.md`, "Push notifications"): an
//! approval, a question or a service's ask that waits in a thread of theirs, or a task of theirs
//! that finished or failed, pushed to each browser that takes their notifications. A push says
//! one sentence of the server's own, with the thread's title, and which thread it is about;
//! nothing the thread says. Needs are pushed [`AT_ONCE`] at a time and each person's browsers all
//! at once, each push within glave-push's own limit, so no one's slow push service holds up
//! anyone else's. A browser its push service says is gone, or that keeps failing, is forgotten.

use std::sync::Arc;
use std::time::Duration;

use glave_event::{Outcome, Payload as _, ServiceAsked, ThreadId};
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
            _ => None,
        }
    }

    /// The same need, in `thread`: the one its person started, of which the thread it arose in
    /// may be a subagent's.
    pub(crate) fn in_thread(self, thread: ThreadId) -> Self {
        match self {
            Self::Waiting { .. } => Self::Waiting { thread },
            Self::Ended { outcome, .. } => Self::Ended { thread, outcome },
        }
    }

    /// What the worker is told it is: what waits, which replaces the thread's last
    /// notification, or a task's end, which always alerts.
    fn kind(self) -> &'static str {
        match self {
            Self::Waiting { .. } => "waiting",
            Self::Ended { .. } => "ended",
        }
    }

    fn thread(self) -> ThreadId {
        match self {
            Self::Waiting { thread } | Self::Ended { thread, .. } => thread,
        }
    }

    /// What a notification of it says and how soon it should arrive: none for a task the
    /// person stopped themselves.
    fn told(self, title: Option<String>) -> Option<(Sentence, Urgency)> {
        let title = title.map(|title| shortened(&title));
        Some(match (self, title) {
            (Self::Waiting { .. }, Some(title)) => (Sentence::PushWaiting { title }, Urgency::High),
            (Self::Waiting { .. }, None) => (Sentence::PushWaitingUntitled, Urgency::High),
            (
                Self::Ended {
                    outcome: Some(Outcome::Cancelled),
                    ..
                },
                _,
            ) => return None,
            (
                Self::Ended {
                    outcome: Some(Outcome::Completed),
                    ..
                },
                title,
            ) => (
                title.map_or(Sentence::PushDoneUntitled, |title| Sentence::PushDone {
                    title,
                }),
                Urgency::Normal,
            ),
            (Self::Ended { .. }, title) => (
                title.map_or(Sentence::PushNotDoneUntitled, |title| {
                    Sentence::PushNotDone { title }
                }),
                Urgency::Normal,
            ),
        })
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
        let text = serde_json::json!({
            "text": say(language, &sentence),
            "thread": thread,
            "org": scope.org(),
            "kind": need.kind(),
        });
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
