//! Conformance suites: the behavior every implementation of a storage port must show. Adapter
//! crates enable the `conformance` feature and run these against their own implementation.
//! Each suite returns the first violation it finds as an error message.

use glave_event::{At, EventId, Header, Outcome, RawEvent, Seq, ThreadId, TurnEnded, encode};
use uuid::{Builder, Uuid};

use crate::blob::{BlobStore, digest};
use crate::log::{EventLog, LogError};
use crate::scope::{OrgId, Scope, SpaceId};

type Verdict = Result<(), String>;

fn ensure(condition: bool, violation: &str) -> Verdict {
    if condition {
        Ok(())
    } else {
        Err(violation.to_owned())
    }
}

fn uuid_v7(n: u64) -> Uuid {
    let mut random = [0u8; 10];
    for (slot, byte) in random.iter_mut().zip(n.to_be_bytes()) {
        *slot = byte;
    }
    Builder::from_unix_timestamp_millis(1_791_000_000_000, &random).into_uuid()
}

fn scope(n: u64) -> Scope {
    Scope {
        org: OrgId::from_uuid(uuid_v7(1_000 + n)),
        space: SpaceId::from_uuid(uuid_v7(2_000 + n)),
    }
}

fn thread(n: u64) -> Result<ThreadId, String> {
    ThreadId::try_from(uuid_v7(3_000 + n)).map_err(|e| e.to_string())
}

fn event(thread: ThreadId, seq: u64) -> Result<RawEvent, String> {
    let header = Header {
        id: EventId::try_from(uuid_v7(seq)).map_err(|e| e.to_string())?,
        thread,
        seq: Seq::new(seq).map_err(|e| e.to_string())?,
        at: At::try_from("2026-10-07T21:14:03.512Z".to_owned()).map_err(|e| e.to_string())?,
    };
    encode(
        header,
        &TurnEnded {
            outcome: Outcome::Completed,
        },
    )
    .map_err(|e| e.to_string())
}

fn events(thread: ThreadId, seqs: &[u64]) -> Result<Vec<RawEvent>, String> {
    seqs.iter().map(|seq| event(thread, *seq)).collect()
}

fn seqs(events: &[RawEvent]) -> Vec<u64> {
    events.iter().map(|event| event.seq.get()).collect()
}

/// Every [`EventLog`] must number threads gap-free, reject conflicting or gapped appends
/// all-or-nothing, read in pages, and keep scopes apart.
pub async fn event_log<L: EventLog>(log: &L) -> Verdict {
    let here = scope(1);
    let elsewhere = scope(2);
    let t = thread(1)?;
    let other = thread(2)?;
    let read_all = |s: Scope, t: ThreadId| async move { log.read(&s, t, None, usize::MAX).await };

    let empty = read_all(here, t).await.map_err(|e| e.to_string())?;
    ensure(
        empty.is_empty(),
        "a thread that was never written must read as empty",
    )?;

    log.append(&here, &events(t, &[1, 2, 3])?)
        .await
        .map_err(|e| e.to_string())?;
    let stored = read_all(here, t).await.map_err(|e| e.to_string())?;
    ensure(seqs(&stored) == [1, 2, 3], "events must read back in order")?;

    let conflict = log.append(&here, &events(t, &[3])?).await;
    ensure(
        matches!(conflict, Err(LogError::Conflict { found: 3, .. })),
        "appending an existing position must be a conflict",
    )?;
    let gap = log.append(&here, &events(t, &[5])?).await;
    ensure(
        matches!(
            gap,
            Err(LogError::Gap {
                expected: 4,
                found: 5,
                ..
            })
        ),
        "skipping a position must be a gap",
    )?;
    let partial = log.append(&here, &events(t, &[4, 6])?).await;
    ensure(
        partial.is_err(),
        "an append with a gap in the middle must fail",
    )?;
    let after_partial = read_all(here, t).await.map_err(|e| e.to_string())?;
    ensure(
        seqs(&after_partial) == [1, 2, 3],
        "a failed append must store nothing",
    )?;
    let mut mixed = events(t, &[4])?;
    mixed.extend(events(other, &[1])?);
    ensure(
        matches!(
            log.append(&here, &mixed).await,
            Err(LogError::MixedThreads { .. })
        ),
        "an append across two threads must be rejected",
    )?;

    let page = log
        .read(&here, t, Some(Seq::FIRST), 1)
        .await
        .map_err(|e| e.to_string())?;
    ensure(seqs(&page) == [2], "reads must page from after the cursor")?;

    let isolated = read_all(elsewhere, t).await.map_err(|e| e.to_string())?;
    ensure(
        isolated.is_empty(),
        "another scope must not see this thread",
    )?;
    log.append(&elsewhere, &events(t, &[1])?)
        .await
        .map_err(|e| format!("the same thread id in another scope must be independent: {e}"))?;
    let unchanged = read_all(here, t).await.map_err(|e| e.to_string())?;
    ensure(
        seqs(&unchanged) == [1, 2, 3],
        "writing another scope must not touch this one",
    )?;

    thread_listing(log, here, elsewhere, t, other).await
}

/// Thread listings order by most recent write, honor the limit, and stay inside the scope.
async fn thread_listing<L: EventLog>(
    log: &L,
    here: Scope,
    elsewhere: Scope,
    t: ThreadId,
    other: ThreadId,
) -> Verdict {
    log.append(&here, &events(other, &[1])?)
        .await
        .map_err(|e| e.to_string())?;
    let recent = log.threads(&here, 10).await.map_err(|e| e.to_string())?;
    ensure(
        recent == [other, t],
        "threads must list most recently written first",
    )?;
    log.append(&here, &events(t, &[4])?)
        .await
        .map_err(|e| e.to_string())?;
    let recent = log.threads(&here, 1).await.map_err(|e| e.to_string())?;
    ensure(
        recent == [t],
        "writing a thread must move it to the front, and limit must hold",
    )?;
    let there = log
        .threads(&elsewhere, 10)
        .await
        .map_err(|e| e.to_string())?;
    ensure(there == [t], "thread listings must not cross scopes")
}

/// Every [`BlobStore`] must name content by its SHA-256, keep one copy of identical content,
/// and keep scopes apart.
pub async fn blob_store<B: BlobStore>(blobs: &B) -> Verdict {
    let here = scope(1);
    let elsewhere = scope(2);
    let bytes = b"turn config".to_vec();

    let reference = blobs
        .put(&here, bytes.clone(), "application/json")
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        reference.sha256 == digest(&bytes),
        "a blob must be named by its SHA-256",
    )?;
    ensure(
        reference.bytes == 11,
        "a blob reference must record its size",
    )?;
    ensure(
        reference.media_type == "application/json",
        "a blob reference must keep its media type",
    )?;

    let again = blobs
        .put(&here, bytes.clone(), "application/json")
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        again == reference,
        "identical content must get the identical reference",
    )?;

    let read = blobs
        .get(&here, &reference.sha256)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        read.as_deref() == Some(bytes.as_slice()),
        "a blob must read back exactly",
    )?;

    let isolated = blobs
        .get(&elsewhere, &reference.sha256)
        .await
        .map_err(|e| e.to_string())?;
    ensure(isolated.is_none(), "another scope must not see this blob")?;

    let missing = blobs
        .get(&here, &digest(b"never stored"))
        .await
        .map_err(|e| e.to_string())?;
    ensure(missing.is_none(), "unknown content must read as missing")
}
