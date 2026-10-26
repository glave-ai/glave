//! Writers racing each other: calls started together, as two processes on one store make them,
//! so a store whose check and write are not one step lets two of them through.

use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::Poll;

use glave_event::{At, EventId, Header, Outcome, Seq, TurnResult, encode};

use super::{Verdict, ensure, scope, thread, uuid_v7};
use crate::log::{EventLog, LogError};

/// How many writers race for one place.
pub(super) const WRITERS: u64 = 8;

/// Runs `calls` at once, each polled whenever it can go on, and answers their results in order.
/// The calls share this task, so a store that does each call's work in one step still sees them
/// arrive together, and one that waits on a database lets them interleave there.
pub(super) async fn together<F: Future>(calls: Vec<F>) -> Vec<F::Output> {
    let mut running: Vec<Pin<Box<F>>> = calls.into_iter().map(Box::pin).collect();
    let mut answered: Vec<Option<F::Output>> = running.iter().map(|_| None).collect();
    poll_fn(|context| {
        let mut waiting = false;
        for (call, answer) in running.iter_mut().zip(answered.iter_mut()) {
            if answer.is_none() {
                match call.as_mut().poll(context) {
                    Poll::Ready(output) => *answer = Some(output),
                    Poll::Pending => waiting = true,
                }
            }
        }
        if waiting {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    answered.into_iter().flatten().collect()
}

/// Writers racing for one thread's first event, each with an event of its own: exactly one is
/// kept, every other is told of the conflict, and the thread holds that one event.
pub(super) async fn racing_appends<L: EventLog>(log: &L) -> Verdict {
    let here = scope(3);
    let raced = thread(3)?;
    let mut writers = Vec::new();
    for writer in 0..WRITERS {
        let header = Header {
            id: EventId::try_from(uuid_v7(7_000 + writer)).map_err(|e| e.to_string())?,
            thread: raced,
            seq: Seq::FIRST,
            at: At::try_from("2026-10-07T21:14:03.512Z".to_owned()).map_err(|e| e.to_string())?,
        };
        let outcome = Outcome::Completed;
        let event = encode(
            header,
            &TurnResult {
                outcome,
                ..TurnResult::begun()
            },
        )
        .map_err(|e| e.to_string())?;
        writers.push(async move { (writer, log.append(&here, &[event]).await) });
    }
    let results = together(writers).await;
    let won: Vec<u64> = results
        .iter()
        .filter(|(_, appended)| appended.is_ok())
        .map(|(writer, _)| *writer)
        .collect();
    ensure(
        won.len() == 1,
        &format!("exactly one racing writer must take a position, not {won:?}"),
    )?;
    ensure(
        results.iter().all(|(_, appended)| {
            appended.is_ok() || matches!(appended, Err(LogError::Conflict { .. }))
        }),
        "a writer that lost the race must be told of the conflict",
    )?;
    let stored = log
        .read(&here, raced, None, usize::MAX)
        .await
        .map_err(|e| e.to_string())?;
    let winner = won
        .first()
        .and_then(|writer| EventId::try_from(uuid_v7(7_000 + writer)).ok());
    ensure(
        stored.len() == 1 && stored.first().map(|event| event.id) == winner,
        "the thread must hold the winner's event alone",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_run_at_once_and_answer_in_order() {
        crate::block_on!(async {
            // Each waits for all three: run one after another, the first would wait forever.
            let barrier = tokio::sync::Barrier::new(3);
            let barrier = &barrier;
            let calls = (0..3)
                .map(|n| async move {
                    barrier.wait().await;
                    n * 2
                })
                .collect();
            assert_eq!(together(calls).await, [0, 2, 4]);
        });
    }
}
