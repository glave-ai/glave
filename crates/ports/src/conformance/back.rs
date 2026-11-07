//! Reading a thread back from its end: newest first, before a place, of some types or all.

use glave_event::Seq;

use super::{Verdict, ensure, event, renamed, scope, seqs, thread};
use crate::log::EventLog;
use crate::scope::Scope;

/// `read_back` pages a thread from its end, newest first, before `before`, of the types asked
/// for or of all, and finds nothing of a thread another scope holds.
pub(super) async fn reading_back<L: EventLog>(log: &L, here: Scope) -> Verdict {
    let t = thread(31)?;
    let written = [
        event(t, 1)?,
        renamed(t, 2, "first")?,
        event(t, 3)?,
        event(t, 4)?,
        renamed(t, 5, "second")?,
        event(t, 6)?,
    ];
    log.append(&here, &written)
        .await
        .map_err(|e| e.to_string())?;
    let back = |kinds: Option<&'static [&'static str]>, before: Option<u64>, limit: usize| {
        let before = before.and_then(|seq| Seq::new(seq).ok());
        async move {
            log.read_back(&here, t, kinds, before, limit)
                .await
                .map(|events| seqs(&events))
                .map_err(|e| e.to_string())
        }
    };
    ensure(
        back(None, None, 4).await? == [6, 5, 4, 3],
        "read_back must read from the thread's end, newest first, at most the limit",
    )?;
    ensure(
        back(None, Some(4), 10).await? == [3, 2, 1],
        "read_back must read only what comes before `before`",
    )?;
    ensure(
        back(Some(&["thread.renamed"]), None, 10).await? == [5, 2],
        "read_back must read only the types asked for",
    )?;
    ensure(
        back(Some(&["thread.renamed", "turn.ended"]), Some(5), 2).await? == [4, 3],
        "read_back must page the types asked for from before `before`",
    )?;
    ensure(
        back(Some(&["thread.archived"]), None, 10).await?.is_empty(),
        "read_back must read nothing when no event has a type asked for",
    )?;
    let elsewhere = log
        .read_back(&scope(32), t, None, None, 10)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        elsewhere.is_empty(),
        "read_back must not read another scope's events",
    )
}
