//! The threads view (`specs/events.md`, "Storage"): one row per thread, folded from its events
//! as they are appended, in the same transaction, and rebuildable from the log at any time. It
//! holds no content: a thread's title is read from the event that gives it (`title_seq`),
//! sealed like every other payload, so erasing the content erases the title too.

use std::fmt;
use std::str::FromStr;

use glave_event::{
    At, Body, Money, Origin, Outcome, Part, Principal, RawEvent, Role, Seq, ThreadId, TokenCounts,
    decode,
};
use serde::{Deserialize, Serialize};

use crate::log::{EventLog, LogError};
use crate::scope::Scope;

/// The version of [`ThreadView`]'s folding. Stores rebuild every view when the version they
/// built with is older.
pub const THREAD_VIEW_VERSION: u32 = 1;

/// The longest title taken from a thread's first message, in characters.
pub const TITLE_CHARS: usize = 80;

/// Whether a thread is shown: the latest `thread.archived`, `thread.unarchived` or
/// `thread.deleted` decides.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Shown in lists.
    #[default]
    Active,
    /// Hidden from lists and kept in the log; `unarchive` brings it back.
    Archived,
    /// Hidden for good, and waiting for its content to be erased.
    Deleted,
}

impl Standing {
    /// The name stores keep it under.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }
}

/// A thread as lists show it: who it is for, how it started, when, where its title is, and
/// what its own model calls used. Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadView {
    pub thread: ThreadId,
    pub standing: Standing,
    /// Whose work it is; absent in threads from before it was recorded (the local person).
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    pub for_: Option<Principal>,
    pub origin: Origin,
    /// The thread whose call started this one, for a subagent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ThreadId>,
    pub created_at: At,
    /// When its latest event was recorded.
    pub updated_at: At,
    pub last_seq: Seq,
    /// The event its title comes from: its latest `thread.renamed`, else its first message
    /// from the person with text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_seq: Option<Seq>,
    /// Turns started.
    pub turns: u64,
    /// How its latest turn ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// Tokens its own model calls used (a subagent's are in the subagent's thread).
    pub tokens: TokenCounts,
    /// What they cost, while every call reported a cost in one currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    /// True once a call reported no cost, or one in another currency: the cost is unknown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unpriced: bool,
    /// Where its latest event sits in the store's order of writes, which lists follow, most
    /// recent first. Set by the store, not folded, and kept beside the view rather than in it.
    #[serde(skip)]
    pub position: i64,
}

impl ThreadView {
    /// The view after `events`, from `before` (`None` for a thread with no view yet). Events
    /// already folded (at or before `last_seq`) are skipped, so folding is idempotent. `None`
    /// when the events do not start a thread: its first event is not `thread.started` at
    /// `seq` 1, as in the ledgers, or the view before them is missing.
    pub fn fold(before: Option<Self>, events: &[RawEvent]) -> Option<Self> {
        let mut view = before;
        for event in events {
            match view.as_mut() {
                Some(view) if event.seq <= view.last_seq => {}
                Some(view) => view.apply(event),
                None => view = Some(Self::start(event)?),
            }
        }
        view
    }

    fn start(event: &RawEvent) -> Option<Self> {
        if event.seq != Seq::FIRST {
            return None;
        }
        let Ok(decoded) = decode(event.clone()) else {
            return None;
        };
        let Body::ThreadStarted(started) = decoded.body else {
            return None;
        };
        Some(Self {
            thread: event.thread,
            standing: Standing::Active,
            for_: started.for_,
            origin: started.origin,
            parent: started.parent.map(|parent| parent.thread),
            created_at: event.at,
            updated_at: event.at,
            last_seq: event.seq,
            title_seq: None,
            turns: 0,
            outcome: None,
            tokens: TokenCounts::default(),
            cost: None,
            unpriced: false,
            position: 0,
        })
    }

    fn apply(&mut self, event: &RawEvent) {
        self.updated_at = event.at;
        self.last_seq = event.seq;
        match event.kind.as_str() {
            "thread.archived" => self.standing = Standing::Archived,
            "thread.unarchived" => self.standing = Standing::Active,
            "thread.deleted" => self.standing = Standing::Deleted,
            "thread.renamed" => self.title_seq = Some(event.seq),
            "turn.started" => self.turns = self.turns.saturating_add(1),
            "message.added" if self.title_seq.is_none() => {
                if title_of(event).is_some() {
                    self.title_seq = Some(event.seq);
                }
            }
            "turn.ended" | "model.responded" => self.count(event),
            _ => {}
        }
    }

    /// Takes in a turn's outcome, or a response's usage and cost.
    fn count(&mut self, event: &RawEvent) {
        let Ok(decoded) = decode(event.clone()) else {
            return;
        };
        match decoded.body {
            Body::TurnEnded(ended) => self.outcome = Some(ended.outcome),
            Body::ModelResponded(responded) => {
                self.tokens += responded.usage.counts();
                let cost = responded.cost.map(|cost| cost.amount);
                // Without a cost and not unpriced, no response has been counted yet.
                self.cost = match (self.unpriced, self.cost, cost) {
                    (false, None, Some(cost)) => Some(cost),
                    (false, Some(sum), Some(cost)) => sum.checked_add(cost),
                    _ => None,
                };
                self.unpriced |= self.cost.is_none();
            }
            _ => {}
        }
    }

    /// Where this view sits in a listing.
    pub fn cursor(&self) -> ThreadCursor {
        ThreadCursor(self.position)
    }
}

/// The title `event` gives a thread: a `thread.renamed`'s, or the one [`title_from`] makes of
/// what the person wrote in a message. What they wrote is the message's last text part: a
/// thread's first message opens with the project's instructions. `None` for any other event.
pub fn title_of(event: &RawEvent) -> Option<String> {
    match decode(event.clone()).ok()?.body {
        Body::ThreadRenamed(renamed) => Some(renamed.title),
        Body::MessageAdded(message) if message.role == Role::User => title_from(
            message
                .parts
                .iter()
                .filter_map(|part| match part {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .next_back()?,
        ),
        _ => None,
    }
}

/// The title `text` makes: its first line that is not blank, trimmed, and cut to
/// [`TITLE_CHARS`] with an ellipsis when it is longer. `None` when every line is blank.
pub fn title_from(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    if line.chars().count() <= TITLE_CHARS {
        return Some(line.to_owned());
    }
    let mut short: String = line.chars().take(TITLE_CHARS.saturating_sub(1)).collect();
    short.truncate(short.trim_end().len());
    short.push('…');
    Some(short)
}

/// A place in a listing, most recently written first: the views after it were last written
/// before the view it names. Written as the view's position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadCursor(pub i64);

impl fmt::Display for ThreadCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ThreadCursor {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse()
            .map(Self)
            .map_err(|_| format!("{text:?} is not a thread list cursor"))
    }
}

/// Which views to list: one standing, most recently written first, after a cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadQuery {
    pub standing: Standing,
    pub limit: usize,
    pub after: Option<ThreadCursor>,
}

impl ThreadQuery {
    /// Whether `view` belongs in this listing, before the limit.
    pub fn admits(&self, view: &ThreadView) -> bool {
        view.standing == self.standing && self.after.is_none_or(|after| view.cursor() < after)
    }
}

/// Events read per page while a view is folded from the log.
const FOLD_PAGE: usize = 1_000;

/// The view of `thread` folded from every event the log holds of it: what a rebuild writes.
/// `None` when it is not a thread (no `thread.started` at `seq` 1).
pub async fn fold_from_log<L: EventLog + ?Sized>(
    log: &L,
    scope: &Scope,
    thread: ThreadId,
) -> Result<Option<ThreadView>, LogError> {
    let mut view = None;
    let mut after = None;
    loop {
        let page = log.read(scope, thread, after, FOLD_PAGE).await?;
        let Some(last) = page.last() else {
            return Ok(view);
        };
        after = Some(last.seq);
        let first = view.is_none();
        view = ThreadView::fold(view, &page);
        if first && view.is_none() {
            return Ok(None);
        }
        if page.len() < FOLD_PAGE {
            return Ok(view);
        }
    }
}

#[cfg(test)]
mod tests;
