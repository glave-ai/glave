//! What the server sends unasked while a turn runs or a watch lasts, and why a watch ended.

use glave_event::{RawEvent, ThreadId};
use serde::{Deserialize, Serialize};

use super::{Setting, TaskInfo, ThreadSummary, Update};

/// Why a watch ended (`thread/watch`, `thread/list/watch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatchEnd {
    /// The client ended it.
    Unwatched,
    /// The thread was deleted: nothing more will be recorded in it.
    Deleted,
}

/// Something a turn or a watched thread sends while it runs.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    /// A stored event, as it is appended.
    #[serde(rename = "thread/event")]
    Event { thread: ThreadId, event: RawEvent },
    /// Progress, never stored.
    #[serde(rename = "turn/live")]
    Live { thread: ThreadId, update: Update },
    /// A task ended: sent to the thread's watchers. Never stored as such; the ledger keeps it.
    #[serde(rename = "task/finished")]
    TaskFinished { thread: ThreadId, task: TaskInfo },
    /// A thread a list watch follows changed: its summary now, as `thread/list` shows it.
    #[serde(rename = "thread/changed")]
    Changed {
        thread: ThreadId,
        summary: Box<ThreadSummary>,
    },
    /// The watching caller's own settings or connections changed through this server: read
    /// `setting` again. Sent on their list watches.
    #[serde(rename = "settings/changed")]
    SettingsChanged { setting: Setting },
}
