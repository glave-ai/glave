//! Conformance suites: the behavior every implementation of a port must show. Adapter crates
//! enable the `conformance` feature and run these against their own implementation.
//! Each suite returns the first violation it finds as an error message.

mod at_rest;
mod blobs;
mod computers;
mod leases;
mod memory;
mod model;
mod objects;
mod projects;
mod racing;
mod search;
mod sweep;
mod threads;

pub use at_rest::{AtRest, KeptBlob, KeptPayload, at_rest};
pub use blobs::blob_store;
pub use computers::{
    Confinement, computers, confinement_case, confinement_escapes, confinement_fixture,
    confinement_paths, confinement_targets, confinement_untouched,
};
pub use leases::leases;
pub use memory::{MemoryPlaces, memory_scopes};
pub use model::{Script, model_provider};
pub use objects::objects;
pub use projects::projects;
pub use search::search;
pub use threads::thread_views;

use glave_event::{
    At, EventId, Header, Outcome, RawEvent, Seq, ThreadId, ThreadRenamed, TurnEnded, encode,
};
use uuid::{Builder, Uuid};

use crate::log::{EventLog, LogError};
use crate::scope::{Located, OrgId, ProjectId, Scope};
use crate::tools::NeverCancelled;

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
    Scope::glaves_own(
        OrgId::from_uuid(uuid_v7(1_000 + n)),
        ProjectId::from_uuid(uuid_v7(2_000 + n)),
    )
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
/// all-or-nothing, let exactly one of writers racing for a place take it, read in pages, and
/// keep scopes apart.
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

    latest_of_types(log, here, t).await?;
    locating(log, here, t).await?;

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

    racing::racing_appends(log).await
}

/// `latest_of` finds a thread's last event of the given types, past later events of other
/// types, and nothing when there is none or the thread is another scope's.
async fn latest_of_types<L: EventLog>(log: &L, here: Scope, t: ThreadId) -> Verdict {
    let last = log
        .latest_of(&here, t, &["turn.started", "turn.ended"])
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        last.is_some_and(|event| event.seq.get() == 3),
        "latest_of must find the thread's last event of the given types",
    )?;
    let none = log
        .latest_of(&here, t, &["thread.archived"])
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        none.is_none(),
        "latest_of must find nothing when no event has a given type",
    )?;

    let mixed = thread(4)?;
    let renamed = |seq: u64, title: &str| -> Result<RawEvent, String> {
        let header = Header {
            id: EventId::try_from(uuid_v7(8_000 + seq)).map_err(|e| e.to_string())?,
            thread: mixed,
            seq: Seq::new(seq).map_err(|e| e.to_string())?,
            at: At::try_from("2026-10-07T21:14:03.512Z".to_owned()).map_err(|e| e.to_string())?,
        };
        let title = title.to_owned();
        encode(header, &ThreadRenamed { title, by: None }).map_err(|e| e.to_string())
    };
    let written = [
        renamed(1, "first")?,
        event(mixed, 2)?,
        renamed(3, "second")?,
        event(mixed, 4)?,
    ];
    log.append(&here, &written)
        .await
        .map_err(|e| e.to_string())?;
    let title = log
        .latest_of(&here, mixed, &["thread.renamed"])
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        title.is_some_and(|event| {
            event.seq.get() == 3
                && event.kind.as_str() == "thread.renamed"
                && event.data.get().contains("second")
        }),
        "latest_of must find the last event of a type, past later events of other types",
    )?;
    let unseen = log
        .latest_of(&scope(4), mixed, &["thread.renamed"])
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        unseen.is_none(),
        "latest_of must not find another scope's events",
    )
}

/// `locate` names the scope a thread is stored in, and nothing for a thread never written.
async fn locating<L: EventLog>(log: &L, here: Scope, t: ThreadId) -> Verdict {
    let located = log.locate(t).await.map_err(|e| e.to_string())?;
    ensure(
        located
            == Some(Located {
                org: here.org(),
                project: here.project(),
            }),
        "locate must name the scope a thread is stored in",
    )?;
    let nowhere = log.locate(thread(9)?).await.map_err(|e| e.to_string())?;
    ensure(
        nowhere.is_none(),
        "locate must find nothing for a thread never written",
    )
}

/// What every tool source must do: list tools with names a model can call (1 to 128 of
/// `A-Za-z0-9_.-`, as MCP advises) and no name twice, give each an object schema, and answer a
/// call to a tool it does not have with an error the model can read, never a failure.
pub async fn tool_source(source: &dyn crate::tools::ToolSource) -> Verdict {
    let tools = source.tools().await.map_err(|err| err.to_string())?;
    let mut seen = std::collections::HashSet::new();
    for tool in &tools {
        let name = &tool.spec.name;
        ensure(
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
            &format!("tool name {name:?} is not one a model can call"),
        )?;
        ensure(
            seen.insert(name.clone()),
            &format!("{name:?} is listed twice"),
        )?;
        ensure(
            tool.spec.parameters.get("type").and_then(|t| t.as_str()) == Some("object"),
            &format!("{name:?} does not take an object"),
        )?;
    }
    let missing = "conformance__no_such_tool";
    ensure(
        !seen.contains(missing),
        "the suite's made-up tool name is taken",
    )?;
    let arguments = glave_event::Arguments::Json(serde_json::Map::new());
    let output = source.call(missing, &arguments, &NeverCancelled).await;
    ensure(
        output.is_error,
        "a call to a tool it does not have was not an error",
    )?;
    ensure(
        !output.parts.is_empty(),
        "the error for a missing tool says nothing",
    )
}
