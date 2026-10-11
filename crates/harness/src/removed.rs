//! A thread's worktree from outside a running harness: whose it is (the owner label kept
//! beside it), when its thread last did anything, and recording its removal as
//! `workspace.removed` on the thread.

use glave_event::{EventId, Header, Payload, RawEvent, Seq, ThreadId, WorkspaceRemoved, encode};
use glave_ports::{EventLog, OrgId, Scope, SpaceId};
use jiff::Timestamp;
use uuid::Uuid;

use crate::harness::HarnessError;

/// Events read per page while looking for a thread's last one.
const PAGE: usize = 1_000;

/// The label a thread's worktree is kept with: `<org>/<space>/<thread>`.
pub fn owner_label(scope: Scope, thread: ThreadId) -> String {
    format!(
        "{}/{}/{}",
        scope.org.as_uuid(),
        scope.space.as_uuid(),
        thread.as_uuid()
    )
}

/// The thread an owner label names, if it is one.
pub fn owner_of(label: &str) -> Option<(Scope, ThreadId)> {
    let mut parts = label.split('/').map(Uuid::parse_str);
    let (Some(Ok(org)), Some(Ok(space)), Some(Ok(thread)), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let scope = Scope {
        org: OrgId::from_uuid(org),
        space: SpaceId::from_uuid(space),
    };
    Some((scope, ThreadId::try_from(thread).ok()?))
}

/// The thread's last event, if it has any.
async fn last_event<L: EventLog>(
    log: &L,
    scope: &Scope,
    thread: ThreadId,
) -> Result<Option<RawEvent>, HarnessError> {
    let mut last = None;
    let mut after: Option<Seq> = None;
    loop {
        let mut page = log.read(scope, thread, after, PAGE).await?;
        let full = page.len() == PAGE;
        if let Some(event) = page.pop() {
            after = Some(event.seq);
            last = Some(event);
        }
        if !full {
            return Ok(last);
        }
    }
}

/// When the thread last did anything: its last event's time.
pub async fn last_active<L: EventLog>(
    log: &L,
    scope: &Scope,
    thread: ThreadId,
) -> Result<Option<Timestamp>, HarnessError> {
    Ok(last_event(log, scope, thread)
        .await?
        .map(|event| event.at.timestamp()))
}

/// Records `removed` as the thread's next event, made `at` with `random` for its id. A thread
/// running in a harness at the same moment makes one of the two appends fail, never both land.
pub async fn record_removed<L: EventLog>(
    log: &L,
    scope: &Scope,
    thread: ThreadId,
    at: Timestamp,
    random: [u8; 10],
    removed: &WorkspaceRemoved,
) -> Result<(), HarnessError> {
    append_next(log, scope, thread, at, random, removed).await
}

/// Appends `payload` as the thread's next event, made `at` with `random` for its id.
pub(crate) async fn append_next<L: EventLog, T: Payload>(
    log: &L,
    scope: &Scope,
    thread: ThreadId,
    at: Timestamp,
    random: [u8; 10],
    payload: &T,
) -> Result<(), HarnessError> {
    let last = last_event(log, scope, thread)
        .await?
        .ok_or(HarnessError::NotFound(thread))?;
    let seq = last
        .seq
        .next()
        .ok_or_else(|| HarnessError::Event(format!("thread {thread} is full")))?;
    let header = Header {
        id: EventId::new(at, random)?,
        thread,
        seq,
        at: at.into(),
    };
    log.append(scope, &[encode(header, payload)?]).await?;
    Ok(())
}
