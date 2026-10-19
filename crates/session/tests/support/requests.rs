//! The requests most tests make of a test server: starting threads and turns, reading a
//! thread's events and where it works, and starting and waiting for tasks.

use std::path::Path;

use glave_event::{Body, Opened, RawEvent, ThreadId, decode};
use glave_ports::{Caller, EventLog, ProjectId};
use glave_session::TurnStream;
use glave_session::protocol::{
    ApiError, IdempotencyKey, Meta, NewFolder, Reply, Request, TaskInfo, TaskStatus,
};

use super::setup::{Tested, scope};

/// A project called `name` whose folder is `path`, on this computer.
pub async fn make_project<L: EventLog + 'static>(
    server: &Tested<L>,
    name: &str,
    path: &Path,
) -> ProjectId {
    let made = Request::ProjectCreate {
        name: name.into(),
        members: Vec::new(),
        folder: Some(NewFolder::Path {
            path: path.to_string_lossy().into_owned(),
        }),
    };
    match server.request(made).await.unwrap() {
        Reply::Project { project, .. } => project.id,
        other => panic!("a project, not {other:?}"),
    }
}

/// Starts a thread to run tasks in, in `project` (by default the server's own): on a copy of
/// its own.
pub async fn start_for_tasks<L: EventLog + 'static>(
    server: &Tested<L>,
    project: Option<ProjectId>,
) -> Result<ThreadId, ApiError> {
    let started = Request::ThreadStart {
        project: project.or(Some(scope().project)),
        background: true,
        worktree: None,
        computer: None,
        model: None,
        approval: None,
    };
    match server.request(started).await? {
        Reply::Thread { thread } => Ok(thread.id),
        other => panic!("a thread, not {other:?}"),
    }
}

/// Where `thread` was bound to work, and its root there.
pub async fn opened<L: EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
) -> (Opened, String) {
    events(server, thread)
        .await
        .into_iter()
        .find_map(|raw| match decode(raw).unwrap().body {
            Body::WorkspaceBound(bound) => Some((bound.opened, bound.root)),
            _ => None,
        })
        .unwrap()
}

/// Starts a thread, in a worktree of its own when `worktree`.
pub async fn start<L: EventLog + 'static>(server: &Tested<L>, worktree: bool) -> ThreadId {
    let started = Request::ThreadStart {
        project: Some(scope().project),
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

/// `turn/start` saying `prompt` in `thread`, with `key` when given.
pub fn turn_start(thread: ThreadId, prompt: &str, key: Option<IdempotencyKey>) -> Request {
    Request::TurnStart {
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
    }
}

/// Starts a turn saying `prompt` and streams it, through the entry every transport uses. With
/// `key`, a turn already started under it is followed instead.
pub async fn stream_turn<L: EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
    prompt: &str,
    key: Option<IdempotencyKey>,
) -> Result<TurnStream, ApiError> {
    let request = turn_start(thread, prompt, key);
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
