//! Choosing a thread's approval mode through the session API: `thread/start` and `turn/start`
//! take `approval`, recorded as `approval.mode.chosen`, so the thread's turns ask that much
//! from then on, across restarts; choosing the mode a thread already has records nothing; a
//! thread that chooses none runs in the server's mode; the summary shows the mode; and a mode
//! the policy does not leave is refused with `-32012`.

use std::path::Path;

use glave_event::{Body, ThreadId, decode};
use glave_harness::ApprovalMode as Mode;
use glave_ports::Completion;
use glave_session::Answering;
use glave_session::protocol::{ApiError, ApprovalMode, Meta, Reply, Request, code};
use serde_json::json;

use crate::support::*;

/// A server on `log` whose own mode is `own`, where threads may choose `modes`, with no one
/// to answer what asks.
async fn serve(
    dir: &Path,
    answers: Vec<Completion>,
    log: &Shared,
    own: Mode,
    modes: &[Mode],
) -> Tested<Shared> {
    let mut setup = Setup::new(answers);
    setup.answering = Answering::NoOne;
    setup.settings.approval = own;
    setup.modes = modes.to_vec();
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
    approval: Option<ApprovalMode>,
) -> Result<Reply, ApiError> {
    server
        .request(Request::ThreadStart {
            space: None,
            worktree: None,
            background: false,
            computer: None,
            model: None,
            approval,
        })
        .await
}

async fn started(server: &Tested<Shared>, approval: Option<ApprovalMode>) -> ThreadId {
    match start_in(server, approval).await.unwrap() {
        Reply::Thread { thread } => thread.id,
        other => panic!("a thread, not {other:?}"),
    }
}

async fn turn_in(
    server: &Tested<Shared>,
    thread: ThreadId,
    approval: Option<ApprovalMode>,
) -> Result<Reply, ApiError> {
    server
        .request(Request::TurnStart {
            thread,
            prompt: "write it".into(),
            background: false,
            budget: None,
            meta: Meta::default(),
            model: None,
            approval,
        })
        .await
}

/// The mode the thread's summary shows.
async fn shown(server: &Tested<Shared>, thread: ThreadId) -> ApprovalMode {
    match server.request(Request::ThreadGet { thread }).await.unwrap() {
        Reply::Thread { thread } => thread.approval,
        other => panic!("a thread, not {other:?}"),
    }
}

/// The modes chosen, and how each write was decided (`ran` when nothing asked), in order.
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
            Body::ApprovalModeChosen(chosen) => Some(format!("chose {:?}", chosen.mode)),
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
fn a_chosen_mode_holds_for_the_turns_after_it_and_a_restart() {
    let dir = folder("modes-choose");
    let log = Shared::default();
    block_on!(async {
        let both = [Mode::Ask, Mode::Auto];
        let server = serve(&dir, writes("one.txt"), &log, Mode::Auto, &both).await;
        let Reply::Server(info) = server.request(Request::Discover {}).await.unwrap() else {
            panic!("server info");
        };
        assert_eq!(info.approval, ApprovalMode::Auto);
        let thread = started(&server, Some(ApprovalMode::Ask)).await;
        assert_eq!(shown(&server, thread).await, ApprovalMode::Ask);
        // In ask mode the write asks, and with no one there it does not run.
        turn_in(&server, thread, None).await.unwrap();
        server.shutdown().await;
        let mut answers = writes("two.txt");
        answers.extend(writes("three.txt"));
        let restarted = serve(&dir, answers, &log, Mode::Auto, &both).await;
        assert_eq!(shown(&restarted, thread).await, ApprovalMode::Ask);
        // Choosing the mode the thread has records nothing; then auto lets the write run.
        turn_in(&restarted, thread, Some(ApprovalMode::Ask))
            .await
            .unwrap();
        turn_in(&restarted, thread, Some(ApprovalMode::Auto))
            .await
            .unwrap();
        assert_eq!(shown(&restarted, thread).await, ApprovalMode::Auto);
        assert_eq!(
            recorded(&restarted, thread).await,
            [
                "chose Ask",
                "asked: NoOneThere",
                "asked: NoOneThere",
                "chose Auto",
                "ran"
            ]
        );
    });
}

#[test]
fn a_thread_that_chooses_nothing_runs_in_the_servers_mode() {
    let dir = folder("modes-default");
    block_on!(async {
        let server = serve(
            &dir,
            writes("a.txt"),
            &Shared::default(),
            Mode::Ask,
            &[Mode::Ask],
        )
        .await;
        let thread = started(&server, None).await;
        assert_eq!(shown(&server, thread).await, ApprovalMode::Ask);
        turn_in(&server, thread, None).await.unwrap();
        assert_eq!(recorded(&server, thread).await, ["asked: NoOneThere"]);
    });
}

#[test]
fn a_mode_the_policy_does_not_leave_is_refused() {
    let dir = folder("modes-refused");
    let log = Shared::default();
    block_on!(async {
        let ask = [Mode::Ask];
        let server = serve(&dir, Vec::new(), &log, Mode::Ask, &ask).await;
        let refused = start_in(&server, Some(ApprovalMode::Auto))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED, "{refused:?}");
        assert!(refused.message.contains("auto"), "{}", refused.message);
        let Reply::Threads { threads, .. } = server
            .request(Request::ThreadList {
                space: None,
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
        let refused = turn_in(&server, thread, Some(ApprovalMode::Auto))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        assert_eq!(recorded(&server, thread).await, Vec::<String>::new());
    });
}

#[test]
fn a_mode_chosen_before_the_policy_narrowed_is_refused_until_chosen_again() {
    let dir = folder("modes-stale");
    let log = Shared::default();
    block_on!(async {
        let both = [Mode::Ask, Mode::Auto];
        let server = serve(&dir, Vec::new(), &log, Mode::Ask, &both).await;
        let thread = started(&server, Some(ApprovalMode::Auto)).await;
        server.shutdown().await;
        let narrowed = serve(&dir, writes("a.txt"), &log, Mode::Ask, &[Mode::Ask]).await;
        let refused = turn_in(&narrowed, thread, None).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        turn_in(&narrowed, thread, Some(ApprovalMode::Ask))
            .await
            .unwrap();
        assert_eq!(
            recorded(&narrowed, thread).await,
            ["chose Auto", "chose Ask", "asked: NoOneThere"]
        );
    });
}
