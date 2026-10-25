//! Choosing what the agent may do in a thread through the session API: `thread/start` and
//! `turn/start` take `permissions`, recorded as `permissions.chosen`, so the thread's turns
//! run with that preset from then on, across restarts; choosing the preset a thread already
//! has records nothing; a thread that chooses none runs with the server's; the summary shows
//! the preset; one above the policy's cap is refused with `-32012`, in plain words; and one
//! chosen before a cap came runs capped.

use std::path::Path;

use Permissions::{FullAccess, ReadOnly, Workspace};
use glave_event::{Body, ThreadId, decode};
use glave_ports::Completion;
use glave_session::Answering;
use glave_session::protocol::{ApiError, Meta, Permissions, Reply, Request, code};
use serde_json::json;

use crate::support::*;

/// A server on `log` whose own preset is `own`, under the policy's cap `highest`, with no one
/// to answer what asks. Its commands run without a sandbox, so only Full access lets a write
/// run without asking.
async fn serve(
    dir: &Path,
    answers: Vec<Completion>,
    log: &Shared,
    own: Permissions,
    highest: Permissions,
) -> Tested<Shared> {
    let mut setup = Setup::new(answers);
    setup.answering = Answering::NoOne;
    setup.settings.permissions = own;
    setup.settings.highest = highest;
    setup.build_on(dir, log.clone()).await.0
}

/// The model writes a file, then says it is done.
fn writes(path: &str) -> Vec<Completion> {
    vec![
        answer(None, Some(("write", json!({"path": path, "content": "x"})))),
        answer(Some("Done."), None),
    ]
}

async fn start_in(
    server: &Tested<Shared>,
    permissions: Option<Permissions>,
) -> Result<Reply, ApiError> {
    server
        .request(Request::ThreadStart {
            project: Some(scope().project()),
            worktree: None,
            background: false,
            computer: None,
            model: None,
            permissions,
            effort: None,
        })
        .await
}

async fn started(server: &Tested<Shared>, permissions: Option<Permissions>) -> ThreadId {
    match start_in(server, permissions).await.unwrap() {
        Reply::Thread { thread } => thread.id,
        other => panic!("a thread, not {other:?}"),
    }
}

async fn turn_in(
    server: &Tested<Shared>,
    thread: ThreadId,
    permissions: Option<Permissions>,
) -> Result<Reply, ApiError> {
    server
        .request(Request::TurnStart {
            thread,
            prompt: "write it".into(),
            background: false,
            budget: None,
            meta: Meta::default(),
            model: None,
            permissions,
            unattended: false,
            files: Vec::new(),
            effort: None,
        })
        .await
}

/// The preset the thread's summary shows.
async fn shown(server: &Tested<Shared>, thread: ThreadId) -> Permissions {
    match server.request(Request::ThreadGet { thread }).await.unwrap() {
        Reply::Thread { thread } => thread.permissions,
        other => panic!("a thread, not {other:?}"),
    }
}

/// The presets chosen, and how each write was decided (`ran` when nothing asked), in order.
async fn recorded(server: &Tested<Shared>, thread: ThreadId) -> Vec<String> {
    let Reply::Events { events, .. } = server
        .request(Request::ThreadEvents {
            thread,
            after: None,
            limit: None,
        })
        .await
        .unwrap()
    else {
        panic!("events");
    };
    let mut asked = false;
    events
        .into_iter()
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::PermissionsChosen(chosen) => Some(format!("chose {}", chosen.permissions.name())),
            Body::ApprovalRequested(_) => {
                asked = true;
                None
            }
            Body::ApprovalDecided(decided) => Some(format!("asked: {:?}", decided.decision)),
            Body::ToolCallCompleted(_) if !std::mem::take(&mut asked) => Some("ran".into()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_chosen_preset_holds_for_the_turns_after_it_and_a_restart() {
    let dir = folder("permissions-choose");
    let log = Shared::default();
    block_on!(async {
        let all = Permissions::ALL;
        let server = serve(&dir, writes("one.txt"), &log, FullAccess, FullAccess).await;
        let Reply::Server(info) = server.request(Request::Discover {}).await.unwrap() else {
            panic!("server info");
        };
        assert_eq!(info.permissions, FullAccess);
        assert_eq!(info.presets, all);
        let thread = started(&server, Some(ReadOnly)).await;
        assert_eq!(shown(&server, thread).await, ReadOnly);
        // In Read-only the write asks, and with no one there it does not run.
        turn_in(&server, thread, None).await.unwrap();
        server.shutdown().await;
        let mut answers = writes("two.txt");
        answers.extend(writes("three.txt"));
        let restarted = serve(&dir, answers, &log, FullAccess, FullAccess).await;
        assert_eq!(shown(&restarted, thread).await, ReadOnly);
        // Choosing the preset the thread has records nothing; then full access lets it run.
        turn_in(&restarted, thread, Some(ReadOnly)).await.unwrap();
        turn_in(&restarted, thread, Some(FullAccess)).await.unwrap();
        assert_eq!(shown(&restarted, thread).await, FullAccess);
        assert_eq!(
            recorded(&restarted, thread).await,
            [
                "chose read-only",
                "asked: NoOneThere",
                "asked: NoOneThere",
                "chose full-access",
                "ran"
            ]
        );
    });
}

#[test]
fn a_thread_that_chooses_nothing_runs_with_the_servers_preset() {
    let dir = folder("permissions-default");
    block_on!(async {
        let server = serve(
            &dir,
            writes("a.txt"),
            &Shared::default(),
            Workspace,
            Workspace,
        )
        .await;
        let thread = started(&server, None).await;
        assert_eq!(shown(&server, thread).await, Workspace);
        // No sandbox here: Workspace asks as Read-only does.
        turn_in(&server, thread, None).await.unwrap();
        assert_eq!(recorded(&server, thread).await, ["asked: NoOneThere"]);
    });
}

#[test]
fn a_preset_the_policy_does_not_leave_is_refused_in_plain_words() {
    let dir = folder("permissions-refused");
    let log = Shared::default();
    block_on!(async {
        let capped = [ReadOnly, Workspace];
        let server = serve(&dir, Vec::new(), &log, Workspace, Workspace).await;
        let Reply::Server(info) = server.request(Request::Discover {}).await.unwrap() else {
            panic!("server info");
        };
        assert_eq!(info.presets, capped, "only what a thread may choose");
        let refused = start_in(&server, Some(FullAccess)).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED, "{refused:?}");
        assert_eq!(
            refused.message.to_string(),
            "Full access isn't allowed here: choose Read-only or Workspace"
        );
        let Reply::Threads { threads, .. } = server
            .request(Request::ThreadList {
                project: Some(scope().project()),
                limit: None,
                show: None,
                cursor: None,
            })
            .await
            .unwrap()
        else {
            panic!("threads");
        };
        assert!(threads.is_empty(), "no thread starts: {threads:?}");
        let thread = started(&server, None).await;
        let refused = turn_in(&server, thread, Some(FullAccess))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        assert_eq!(recorded(&server, thread).await, Vec::<String>::new());
    });
}

#[test]
fn a_preset_chosen_before_the_policy_narrowed_runs_capped() {
    let dir = folder("permissions-stale");
    let log = Shared::default();
    block_on!(async {
        let server = serve(&dir, Vec::new(), &log, ReadOnly, FullAccess).await;
        let thread = started(&server, Some(FullAccess)).await;
        server.shutdown().await;
        let narrowed = serve(&dir, writes("a.txt"), &log, ReadOnly, ReadOnly).await;
        assert_eq!(
            shown(&narrowed, thread).await,
            ReadOnly,
            "what it runs with now"
        );
        // Nothing to choose again: the turn runs under the cap, and its write asks.
        turn_in(&narrowed, thread, None).await.unwrap();
        assert_eq!(
            recorded(&narrowed, thread).await,
            ["chose full-access", "asked: NoOneThere"]
        );
        let refused = turn_in(&narrowed, thread, Some(FullAccess))
            .await
            .unwrap_err();
        assert_eq!(
            refused.code,
            code::NOT_ALLOWED,
            "asked for again, it is refused"
        );
    });
}
