//! A thread's prefix as its log keeps it: the system prompt and tools its last turn's
//! configuration recorded (`turn.rs`, `store_config`), read back when the thread is read again,
//! so a restart, or a thread let go of while idle, sends the same bytes.

use glave_event::{Body, Event, Permissions};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, ToolSpec, read_all};
use serde::Deserialize;

use super::told::Told;
use super::{Prefix, Prefixed};
use crate::approval::Approver;
use crate::harness::Harness;
use crate::toolset::ToolSet;
use crate::turn::CONFIG_LIMIT;

/// A prefix read back from the log.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Kept {
    /// The model it was sent to.
    pub(super) model: String,
    system: String,
    tools: Vec<ToolSpec>,
    /// The thread's mode then, which says whether the model knew of plan mode.
    permissions: Permissions,
    /// What of memory the model knew then: present only where the thread read memory.
    #[serde(default)]
    memory: Option<serde_json::Value>,
}

impl Kept {
    /// These tools as the prefix lists them, called through `now`, what is on offer now.
    pub(super) fn tools(&self, now: ToolSet) -> ToolSet {
        now.listed_as(self.system.clone(), self.tools.clone())
    }

    /// The prefix again, with what the model knew as far as the prefix and the turn's
    /// configuration say: a note since may be told again, never missed.
    pub(super) fn prefix(self) -> Prefix {
        let told = Told {
            signed_in: Told::signed_in_by(&self.system),
            planning: self.permissions == Permissions::Plan,
            memory: self.memory.is_some(),
            tools: Told::services(&self.tools),
        };
        // The servers whose tools it lists: a connection among them was offered when it was made.
        let connections = (self.tools.iter())
            .filter_map(|spec| spec.name.strip_prefix("mcp__")?.split_once("__"))
            .map(|(server, _)| server.to_owned())
            .collect();
        Prefix {
            model: self.model,
            system: self.system,
            specs: self.tools,
            told,
            connections,
        }
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The prefix `events`, a thread's log, keep: its last turn's, unless a compaction came
    /// after it; none when there is no turn, or its configuration cannot be read.
    pub(crate) async fn prefix_kept(&self, scope: &Scope, events: &[Event]) -> Prefixed {
        let last = events.iter().rev().find_map(|event| match &event.body {
            Body::TurnStarted(started) => Some(Ok(started)),
            Body::ThreadCompacted(_) => Some(Err(())),
            _ => None,
        });
        let Some(Ok(started)) = last else {
            return Prefixed::None;
        };
        let read = async {
            let mut blob = self.blobs.get(scope, &started.config.sha256).await.ok()??;
            let bytes = read_all(blob.bytes.as_mut(), CONFIG_LIMIT).await.ok()?;
            serde_json::from_slice::<Kept>(&bytes).ok()
        };
        read.await
            .map_or(Prefixed::None, |kept| Prefixed::Kept(Box::new(kept)))
    }
}
