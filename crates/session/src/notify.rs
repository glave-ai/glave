//! Telling a person's devices what needs them (`specs/session.md`, "Push notifications"): an
//! approval, a question or a service's ask that waits in a thread of theirs, or a task of theirs
//! that finished or failed, pushed to each browser that takes their notifications, of the kinds
//! they want. A push says one sentence of the server's own, with the thread's title, and which
//! thread it is about; nothing the thread says. A browser its push service says is gone is
//! forgotten.

use std::time::Duration;

use glave_event::{Outcome, Payload as _, ServiceAsked, ThreadId};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope};
use glave_push::{Message, Pusher, Sent, Urgency};
use tokio::sync::broadcast::error::RecvError;

use crate::protocol::{ApiError, Notification, Update};
use crate::server::Server;
use crate::words::{Language, Sentence, say};

/// How long a push service keeps a notification for a device that is away: a day, after which
/// what it said is old news.
const KEPT_FOR: Duration = Duration::from_hours(24);

/// The most of a thread's title a notification says, in characters.
const TITLE_CHARS: usize = 60;

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

    fn thread(self) -> ThreadId {
        match self {
            Self::Waiting { thread } | Self::Ended { thread, .. } => thread,
        }
    }

    /// What a notification of it says, how soon it should arrive, and whether the person wants
    /// it (`waiting`, `finished`): none for a task the person stopped themselves.
    fn told(
        self,
        title: Option<String>,
        waiting: bool,
        finished: bool,
    ) -> Option<(Sentence, Urgency)> {
        let title = title.map(|title| shortened(&title));
        let sentence = match self {
            Self::Waiting { .. } if waiting => match title {
                Some(title) => Sentence::PushWaiting { title },
                None => Sentence::PushWaitingUntitled,
            },
            Self::Ended {
                outcome: Some(Outcome::Completed),
                ..
            } if finished => match title {
                Some(title) => Sentence::PushDone { title },
                None => Sentence::PushDoneUntitled,
            },
            Self::Ended {
                outcome: Some(Outcome::Cancelled),
                ..
            } => return None,
            Self::Ended { .. } if finished => match title {
                Some(title) => Sentence::PushNotDone { title },
                None => Sentence::PushNotDoneUntitled,
            },
            _ => return None,
        };
        let urgency = match self {
            Self::Waiting { .. } => Urgency::High,
            Self::Ended { .. } => Urgency::Normal,
        };
        Some((sentence, urgency))
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
    /// hears of from now on, in turn. Runs until dropped; on a server that sends no
    /// notifications it waits, doing nothing.
    pub async fn push_notifications(&self) {
        let Some(pusher) = self.inner.push.clone() else {
            return std::future::pending().await;
        };
        let mut needs = self.inner.hub.needs();
        loop {
            match needs.recv().await {
                Ok(need) => {
                    if let Err(error) = self.push(&pusher, need).await {
                        tracing::warn!(%error, "a notification could not be pushed");
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "notifications came faster than they were pushed");
                }
                Err(RecvError::Closed) => return std::future::pending().await,
            }
        }
    }

    /// Pushes `need` to the browsers of the person whose thread it is in, when they want it.
    async fn push(&self, pusher: &Pusher, need: Need) -> Result<(), ApiError> {
        let thread = need.thread();
        // A thread this server forgot the place of is looked for in its own organization.
        let scope = if let Ok(scope) = self.scope_of(thread) {
            scope
        } else {
            let located = self.located(self.inner.own, thread).await?;
            Scope::glaves_own(located.org, located.project)
        };
        let summary = self.summary(scope, thread).await?;
        let caller = Caller {
            person: summary.for_.as_ref().and_then(|whose| whose.id),
            org: Some(scope.org()),
            ..Caller::default()
        };
        let wanted = self.wanted(&caller).await?;
        let Some((sentence, urgency)) = need.told(summary.title, wanted.waiting, wanted.finished)
        else {
            return Ok(());
        };
        let (_, personal) = self.personal_project(&caller).await?;
        let devices = self.devices(&personal).await?;
        if devices.is_empty() {
            return Ok(());
        }
        let key = self.push_key(scope.org()).await?;
        let language = self
            .chosen_language(&caller)
            .await?
            .map_or(Language::English, |(_, language)| language);
        let text = serde_json::json!({ "text": say(language, &sentence), "thread": thread });
        let text = text.to_string();
        let message = Message {
            text: text.as_bytes(),
            keep: KEPT_FOR,
            urgency,
        };
        let now = self.inner.harness.clock().now();
        for device in devices.values() {
            let kept = &device.subscribed;
            let sent = match pusher.subscription(&kept.endpoint, &kept.p256dh, kept.auth.expose()) {
                Ok(to) => pusher.send(&key, &to, message, now).await,
                // Kept before glave read subscriptions as it does now: no push reaches it.
                Err(_) => Ok(Sent::Gone),
            };
            match sent {
                Ok(Sent::Taken) => {}
                Ok(Sent::Gone) => self.forget_device(&personal, &kept.endpoint, true).await?,
                Err(error) => tracing::warn!(%error, "a push service did not take a notification"),
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
    fn each_need_says_its_sentence_only_when_its_kind_is_wanted() {
        let waiting = Need::Waiting { thread: thread() };
        let title = || Some("fix-login".to_owned());
        let (said, urgency) = waiting.told(title(), true, false).unwrap();
        assert_eq!(
            said,
            Sentence::PushWaiting {
                title: "fix-login".into()
            }
        );
        assert_eq!(urgency, Urgency::High);
        assert_eq!(waiting.told(title(), false, true), None);
        let done = |outcome| Need::Ended {
            thread: thread(),
            outcome: Some(outcome),
        };
        let (said, urgency) = done(Outcome::Completed).told(None, false, true).unwrap();
        assert_eq!(
            (said, urgency),
            (Sentence::PushDoneUntitled, Urgency::Normal)
        );
        let (said, _) = done(Outcome::Failed).told(title(), false, true).unwrap();
        assert_eq!(
            said,
            Sentence::PushNotDone {
                title: "fix-login".into()
            }
        );
        assert_eq!(done(Outcome::Failed).told(title(), true, false), None);
        assert_eq!(
            done(Outcome::Cancelled).told(title(), true, true),
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
