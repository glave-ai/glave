//! The requests most tests make of a test server: starting threads and turns, reading a
//! thread's events, and starting and waiting for tasks.

use glave_event::{RawEvent, ThreadId};
use glave_ports::EventLog;
use glave_session::protocol::{
    ApiError, IdempotencyKey, Meta, Reply, Request, TaskInfo, TaskStatus,
};
use glave_session::{Caller, TurnStream};

use super::setup::Tested;

/// Starts a thread, in a worktree of its own when `worktree`.
pub async fn start<L: EventLog + 'static>(server: &Tested<L>, worktree: bool) -> ThreadId {
    let started = Request::ThreadStart {
        space: None,
        background: false,
        worktree: worktree.then_some(true),
        computer: None,
        model: None,
        approval: None,
    };
    match server.request(started).await.unwrap() {
        Reply::Thread { thread } => thread.id,
        other => panic!("a thread, not {other:?}"),
    }
}

/// Runs a turn saying `prompt`, answered when it has ended.
pub async fn turn<L: EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
    prompt: &str,
) -> Result<Reply, ApiError> {
    server
        .request(Request::TurnStart {
            background: false,
            budget: None,
            thread,
            prompt: prompt.into(),
            meta: Meta::default(),
            model: None,
            approval: None,
        })
        .await
}

/// Starts a turn saying `prompt` and streams it, through the entry every transport uses. With
/// `key`, a turn already started under it is followed instead.
pub async fn stream_turn<L: EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
    prompt: &str,
    key: Option<IdempotencyKey>,
) -> Result<TurnStream, ApiError> {
    let request = Request::TurnStart {
        background: false,
        budget: None,
        thread,
        prompt: prompt.into(),
        meta: Meta {
            idempotency_key: key,
            ..Meta::default()
        },
        model: None,
        approval: None,
    };
    server.start_streaming(&Caller::default(), request).await
}

/// The thread's events, checked to be numbered 1, 2, 3, ... with no gap or repeat.
pub async fn events<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> Vec<RawEvent> {
    let page = Request::ThreadEvents {
        thread,
        after: None,
        limit: None,
    };
    let Reply::Events { events, .. } = server.request(page).await.unwrap() else {
        panic!("events")
    };
    let seqs: Vec<u64> = events.iter().map(|event| event.seq.get()).collect();
    let want: Vec<u64> = (1..=u64::try_from(events.len()).unwrap()).collect();
    assert_eq!(seqs, want, "the thread's numbering has a gap or a repeat");
    events
}

/// The types of the thread's events, in order.
pub async fn kinds<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> Vec<String> {
    events(server, thread)
        .await
        .iter()
        .map(|event| event.kind.as_str().to_owned())
        .collect()
}

/// Starts a turn as a task, with `budget`.
pub async fn background<L: EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
    budget: Option<u64>,
) -> TaskInfo {
    let request = Request::TurnStart {
        thread,
        prompt: "tidy the notes".into(),
        background: true,
        budget,
        meta: Meta::default(),
        model: None,
        approval: None,
    };
    match server.request(request).await.unwrap() {
        Reply::Task { task } => task,
        other => panic!("a task, not {other:?}"),
    }
}

pub async fn tasks<L: EventLog + 'static>(server: &Tested<L>) -> Vec<TaskInfo> {
    match server
        .request(Request::TasksList { limit: None })
        .await
        .unwrap()
    {
        Reply::Tasks { tasks } => tasks,
        other => panic!("tasks, not {other:?}"),
    }
}

/// Lets the server's own work run until no task runs here any more.
pub async fn settle<L: EventLog + 'static>(server: &Tested<L>) {
    for _ in 0..100_000 {
        if tasks(server)
            .await
            .iter()
            .all(|task| task.status != TaskStatus::Running)
        {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("a task never finished");
}
