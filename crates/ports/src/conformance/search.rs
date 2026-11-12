//! What every event log's keyword search must do (`EventLog::search`): find messages and tool
//! output by their words as soon as they are appended, best match first, never a failed call's
//! output, a memory tool's result or any other event, never another scope's, never more than
//! asked, and nothing of a thread once it is deleted.

use glave_event::{
    At, CallId, EventId, Header, MessageAdded, Outcome, Part, RawEvent, Role, Seq, ThreadDeleted,
    ThreadId, ToolCallCompleted, TurnResult, encode,
};

use super::{Verdict, ensure, scope, thread, uuid_v7};
use crate::log::EventLog;
use crate::memories::FROM_MEMORY;
use crate::scope::Scope;
use crate::search::Found;

fn header(thread: ThreadId, seq: u64) -> Result<Header, String> {
    Ok(Header {
        id: EventId::try_from(uuid_v7(9_000 + seq)).map_err(|e| e.to_string())?,
        thread,
        seq: Seq::new(seq).map_err(|e| e.to_string())?,
        at: At::try_from("2026-10-07T21:14:03.512Z".to_owned()).map_err(|e| e.to_string())?,
    })
}

pub(super) fn said(thread: ThreadId, seq: u64, role: Role, text: &str) -> Result<RawEvent, String> {
    let message = MessageAdded {
        role,
        parts: vec![Part::Text { text: text.into() }],
        author: None,
        thought: None,
        key: None,
    };
    encode(header(thread, seq)?, &message).map_err(|e| e.to_string())
}

fn output(thread: ThreadId, seq: u64, text: &str, is_error: bool) -> Result<RawEvent, String> {
    let done = ToolCallCompleted {
        call_id: CallId::new(format!("call_{seq}")).map_err(|e| e.to_string())?,
        is_error,
        output: vec![Part::Text { text: text.into() }],
        structured: None,
        untrusted: false,
    };
    encode(header(thread, seq)?, &done).map_err(|e| e.to_string())
}

/// A memory tool's result, marked so as the harness records it.
fn from_memory(thread: ThreadId, seq: u64, text: &str) -> Result<RawEvent, String> {
    let done = ToolCallCompleted {
        call_id: CallId::new(format!("call_{seq}")).map_err(|e| e.to_string())?,
        is_error: false,
        output: vec![Part::Text { text: text.into() }],
        structured: Some(serde_json::json!({ FROM_MEMORY: true })),
        untrusted: false,
    };
    encode(header(thread, seq)?, &done).map_err(|e| e.to_string())
}

fn ended(thread: ThreadId, seq: u64) -> Result<RawEvent, String> {
    let ended = TurnResult {
        outcome: Outcome::Completed,
        ..TurnResult::begun()
    };
    encode(header(thread, seq)?, &ended).map_err(|e| e.to_string())
}

fn at(thread: ThreadId, seq: u64) -> Result<Found, String> {
    Ok(Found {
        thread,
        seq: Seq::new(seq).map_err(|e| e.to_string())?,
    })
}

/// Every [`EventLog`] must search its conversations by keyword as [`EventLog::search`] says.
pub async fn search<L: EventLog>(log: &L) -> Verdict {
    let here = scope(21);
    let elsewhere = scope(22);
    let (first, second, third) = (thread(21)?, thread(22)?, thread(23)?);
    let appended = [
        (
            here,
            vec![
                said(
                    first,
                    1,
                    Role::User,
                    "We deploy this project with Fly; the tests need make db first.",
                )?,
                said(first, 2, Role::Assistant, "Noted.")?,
                ended(first, 3)?,
            ],
        ),
        (
            here,
            vec![
                output(second, 1, "We chose Postgres over MySQL for JSONB.", false)?,
                output(second, 2, "deploy failed: no such host", true)?,
                from_memory(second, 3, "Remembered: the favourite flower is marigold.")?,
            ],
        ),
        (
            elsewhere,
            vec![said(third, 1, Role::User, "Deploy the other site.")?],
        ),
    ];
    for (scope, events) in &appended {
        log.append(scope, events).await.map_err(|e| e.to_string())?;
    }
    let find = |scope, query: &'static str, limit| async move {
        log.search(&scope, query, None, limit)
            .await
            .map_err(|e| format!("search for {query:?} failed: {e}"))
    };
    ensure(
        find(here, "Which DEPLOYS?", 10).await? == [at(first, 1)?],
        "a search must find a message by a word in any case or plural, and never a failed \
         call's output or another scope's",
    )?;
    ensure(
        find(here, "postgres jsonb", 10).await? == [at(second, 1)?],
        "a search must find a tool's output by its words",
    )?;
    ensure(
        find(here, "favourite flower marigold", 10)
            .await?
            .is_empty(),
        "a search must never find a memory tool's result, unlike any other tool's",
    )?;
    ensure(
        find(here, "make db first postgres", 10).await? == [at(first, 1)?, at(second, 1)?],
        "the event matching more of the words must come first",
    )?;
    let asking = log
        .search(&here, "make db first postgres", Some(first), 10)
        .await
        .map_err(|e| format!("a search leaving out a thread failed: {e}"))?;
    ensure(
        asking == [at(second, 1)?],
        "a search must leave out the thread asking",
    )?;
    ensure(
        find(here, "deploy postgres", 1).await?.len() == 1,
        "a search must return at most `limit` events",
    )?;
    ensure(
        find(here, "the of and", 10).await?.is_empty() && find(here, "", 10).await?.is_empty(),
        "a query of no searchable words must find nothing",
    )?;
    ensure(
        find(here, "noted completed", 10).await? == [at(first, 2)?],
        "only messages and tool output are searched",
    )?;
    ensure(
        find(elsewhere, "deploy postgres", 10).await? == [at(third, 1)?],
        "a search must stay inside its scope",
    )?;
    deleting(log, (here, elsewhere), first, (second, third)).await
}

/// Deleting `gone` (`thread.deleted`) takes its words out of the index, and only its: `kept`
/// in `here` and `away` in `elsewhere` are still found.
async fn deleting<L: EventLog>(
    log: &L,
    (here, elsewhere): (Scope, Scope),
    gone: ThreadId,
    (kept, away): (ThreadId, ThreadId),
) -> Verdict {
    let deleted =
        encode(header(gone, 4)?, &ThreadDeleted { by: None }).map_err(|e| e.to_string())?;
    log.append(&here, &[deleted])
        .await
        .map_err(|e| e.to_string())?;
    let find = |scope, query: &'static str| async move {
        log.search(&scope, query, None, 10)
            .await
            .map_err(|e| format!("search for {query:?} failed: {e}"))
    };
    ensure(
        find(here, "deploy make db noted").await?.is_empty(),
        "a deleted thread's words must leave the index with it",
    )?;
    ensure(
        find(here, "postgres jsonb").await? == [at(kept, 1)?]
            && find(elsewhere, "deploy").await? == [at(away, 1)?],
        "deleting a thread must leave every other thread's words in the index",
    )
}
