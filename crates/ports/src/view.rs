//! The threads view (`specs/events.md`, "Storage"): one row per thread, folded from its events
//! as they are appended, in the same transaction, and rebuildable from the log at any time. It
//! holds no content: a thread's title is read from the event that gives it (`title_seq`),
//! sealed like every other payload, so erasing the content erases the title too.

use glave_event::{
    At, Body, CallId, Money, Origin, Outcome, ParentCall, Part, Principal, PrincipalId, RawEvent,
    Role, Seq, ThreadId, TokenCounts, decode,
};
use serde::{Deserialize, Serialize};

use crate::log::{EventLog, LogError};
use crate::scope::Scope;

mod turn;

pub use turn::{AGENT_TOOL, CallKind, OpenTurn, QUESTION_TOOL, Unfinished, Wait, WaitKind};

/// The version of [`ThreadView`]'s folding. Stores rebuild every view when the version they
/// built with is older.
pub const THREAD_VIEW_VERSION: u32 = 5;

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
    /// That call, in the parent's thread: the parent's log never names the child, so a parent
    /// finds its subagent's thread by it (`ThreadQuery::parent`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_call: Option<CallId>,
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
    /// True once a call's usage was estimated, because its stream ended before the provider
    /// reported it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub estimated: bool,
    /// Its latest `workspace.bound`: where it works, read from that event as the title is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_seq: Option<Seq>,
    /// What its conversation last sent and got back: its latest call that was the
    /// conversation's own (the judge's, a title's and a summary's are not); none before its
    /// first and since it was compacted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<LastCall>,
    /// Its turn still open: running, waiting, or left open by a glave that stopped without
    /// ending it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<OpenTurn>,
}

/// A conversation's latest model call, as the threads view keeps it: its tokens in and out,
/// which is the context the next call starts from, and the model that answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LastCall {
    pub tokens: u64,
    pub model: String,
}

/// The gateway routes whose calls are a thread's conversation: its own model's, and a
/// subagent's; none when no gateway was in between.
const CONVERSATION_ROUTES: [&str; 2] = ["main", "subagent"];

impl ThreadView {
    /// The person whose work it is, when its `for` was recorded: the person it was started for.
    pub fn whose(&self) -> Option<PrincipalId> {
        self.for_.as_ref().and_then(|whose| whose.id)
    }

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
            parent: started.parent.as_ref().map(|parent| parent.thread),
            parent_call: started.parent.map(|parent| parent.call_id),
            created_at: event.at,
            updated_at: event.at,
            last_seq: event.seq,
            title_seq: None,
            turns: 0,
            outcome: None,
            tokens: TokenCounts::default(),
            cost: None,
            unpriced: false,
            estimated: false,
            workspace_seq: None,
            context: None,
            turn: None,
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
            "workspace.bound" => self.workspace_seq = Some(event.seq),
            "thread.compacted" => self.context = None,
            "turn.started" => {
                self.turns = self.turns.saturating_add(1);
                self.turn = Some(OpenTurn::at(event.seq));
            }
            "message.added" if self.title_seq.is_none() => {
                if title_of(event).is_some() {
                    self.title_seq = Some(event.seq);
                }
            }
            "turn.ended" | "model.responded" => self.count(event),
            _ => {}
        }
        if let Some(turn) = self.turn.as_mut() {
            turn.apply(event);
        }
    }

    /// Takes in a turn's outcome, or a response's usage and cost.
    fn count(&mut self, event: &RawEvent) {
        let Ok(decoded) = decode(event.clone()) else {
            return;
        };
        match decoded.body {
            Body::TurnEnded(ended) => {
                self.outcome = Some(ended.outcome);
                self.turn = None;
            }
            Body::ModelResponded(responded) => {
                let counts = responded.usage.counts();
                self.tokens += counts;
                self.estimated |= responded.usage.estimated();
                let route = responded.route.as_deref();
                if route.is_none_or(|route| CONVERSATION_ROUTES.contains(&route)) {
                    self.context = Some(LastCall {
                        tokens: counts.input.saturating_add(counts.output),
                        model: responded.model.name,
                    });
                }
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

    /// Where this view sits in a listing: its own sort key, nothing about any other thread.
    pub fn cursor(&self) -> ThreadCursor {
        ThreadCursor {
            updated_at: self.updated_at,
            thread: self.thread,
        }
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

/// A place in a listing, most recently updated first: the views after it were updated before
/// the view it names, or at the same instant with a lower id. It is that view's own sort key,
/// so it says nothing about any other thread, in its scope or out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadCursor {
    pub updated_at: At,
    pub thread: ThreadId,
}

/// Which views to list: one standing, most recently written first, after a cursor; only those
/// with a turn open when `open` says so, and only the subagent's thread a call started when
/// `parent` names the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadQuery {
    pub standing: Standing,
    pub limit: usize,
    pub after: Option<ThreadCursor>,
    pub open: bool,
    pub parent: Option<ParentCall>,
}

impl ThreadQuery {
    /// Whether `view` belongs in this listing, before the limit.
    pub fn admits(&self, view: &ThreadView) -> bool {
        view.standing == self.standing
            && self.after.is_none_or(|after| view.cursor() < after)
            && (!self.open || view.turn.is_some())
            && self.parent.as_ref().is_none_or(|parent| {
                view.parent == Some(parent.thread)
                    && view.parent_call.as_ref() == Some(&parent.call_id)
            })
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
