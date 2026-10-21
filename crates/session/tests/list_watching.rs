//! Watching the threads list (`thread/list/watch`) over every transport: a thread's summary
//! comes the moment it runs, waits for the person, goes on, ends, is renamed or deleted; with
//! `after`, what changed since comes first and nothing older; a watcher far behind still hears
//! the latest; a subagent's wait is its parent's; and nobody hears of threads in a project they
//! do not work in.

use std::sync::Arc;

use glave_event::{At, Body, Principal, ThreadId, decode};
use glave_ports::testing::ManualClock;
use glave_ports::{Caller, ProjectId, Standing};
use glave_session::protocol::{
    Answer, Notification, Reply, Request, Status, ThreadSummary, Update, code,
};
use glave_session::signin::person;
use glave_session::{Answering, TurnItem};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::support::following::*;
use crate::support::*;

/// Reads `watcher` until it is sent a summary of `thread` that `wanted` takes: that one. Every
/// summary sent meanwhile is kept in `heard`.
async fn hear(
    watcher: &mut Watcher,
    thread: ThreadId,
    heard: &mut Vec<ThreadSummary>,
    wanted: impl Fn(&ThreadSummary) -> bool,
) -> ThreadSummary {
    loop {
        match watcher.next().await {
            Said::Note(Notification::Changed { summary, .. }) => {
                heard.push(summary.as_ref().clone());
                if summary.id == thread && wanted(&summary) {
                    return *summary;
                }
            }
            Said::Note(other) => panic!("a list's watch sends summaries, not {other:?}"),
            Said::Answer(answer) => panic!("the watch ended: {answer}"),
        }
    }
}

/// A time before anything the test servers record.
fn long_ago() -> At {
    At::try_from("2026-01-01T00:00:00.000Z".to_owned()).unwrap()
}

fn at_millis(millis: i64) -> At {
    At::from(jiff::Timestamp::from_millisecond(millis).unwrap())
}

#[test]
fn a_list_watch_hears_a_turn_run_wait_for_the_person_go_on_end_and_the_thread_go() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("list-turn-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let gate = Arc::new(Notify::new());
            let answers = vec![
                answer(Some("Hello there."), None),
                answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                answer(Some("Done."), None),
            ];
            // Asking, so the command waits for the person.
            let mut setup = Setup::asking(answers, Answering::Clients);
            setup.gate = Some(Arc::clone(&gate));
            let server = setup.build(&dir).await.0;
            let mut over = Over::new(kind, &server, &name).await;
            let mut watcher = over.watcher().await;
            let mut heard = Vec::new();
            // From long ago, so whether the watch begins before the thread or after it, the
            // thread is heard of: what was recorded first comes from the threads view.
            watcher
                .watch_list(&Caller::default(), None, Some(long_ago()))
                .await;
            let thread = start(&server, false).await;
            hear(&mut watcher, thread, &mut heard, |s| s.turns == 0).await;

            // A turn runs while its answer waits at the gate, and ends once it has it.
            let mut stream = stream_turn(&server, thread, "hi", None).await.unwrap();
            hear(&mut watcher, thread, &mut heard, |s| {
                s.status == Status::Running
            })
            .await;
            gate.notify_one();
            stream.finish().await.unwrap();
            let ended = hear(&mut watcher, thread, &mut heard, |s| {
                s.status == Status::Idle && s.turns == 1
            })
            .await;
            assert_eq!(ended.title.as_deref(), Some("hi"), "{kind:?}");

            ask_and_allow(&server, thread, &gate, &mut watcher, &mut heard).await;
            let rename = Request::ThreadRename {
                thread,
                title: "Garden".into(),
            };
            server.request(rename).await.unwrap();
            hear(&mut watcher, thread, &mut heard, |s| {
                s.title.as_deref() == Some("Garden")
            })
            .await;
            server
                .request(Request::ThreadDelete { thread })
                .await
                .unwrap();
            hear(&mut watcher, thread, &mut heard, |s| {
                s.standing == Standing::Deleted
            })
            .await;
            // Every summary is the caller's own view of a thread, as `thread/list` shows it.
            assert!(heard.iter().all(|s| s.project == scope().project()));
            over.close().await;
        }
    });
}

#[test]
fn from_after_what_changed_since_comes_first_and_nothing_older() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("list-after-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let mut setup = Setup::new(Vec::new());
            let clock = ManualClock::ticking(1_791_001_000_000);
            setup.clock = clock.clone();
            let server = setup.build(&dir).await.0;
            let mut over = Over::new(kind, &server, &name).await;
            let old = start(&server, false).await;
            clock.advance_seconds(600);
            let after = at_millis(clock.millis());
            clock.advance_seconds(1);
            let new = start(&server, false).await;
            let gone = start(&server, false).await;
            server
                .request(Request::ThreadDelete { thread: gone })
                .await
                .unwrap();
            let mut watcher = over.watcher().await;
            let mut heard = Vec::new();
            watcher
                .watch_list(&Caller::default(), None, Some(after))
                .await;
            // What changed since, oldest first: the thread deleted last comes last.
            hear(&mut watcher, gone, &mut heard, |s| {
                s.standing == Standing::Deleted
            })
            .await;
            assert!(heard.iter().any(|s| s.id == new), "{kind:?}");
            assert!(
                heard.iter().all(|s| s.id != old),
                "{kind:?}: caught up from before after: {heard:?}"
            );
            // Then what comes: the older thread too, once it changes.
            let rename = Request::ThreadRename {
                thread: old,
                title: "Older".into(),
            };
            server.request(rename).await.unwrap();
            hear(&mut watcher, old, &mut heard, |s| {
                s.title.as_deref() == Some("Older")
            })
            .await;
            over.close().await;
        }
    });
}

#[test]
fn a_list_watcher_far_behind_still_hears_what_it_missed() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("list-behind-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let missed = start(&server, false).await;
            let busy = start(&server, false).await;
            let mut watcher = over.watcher().await;
            let mut heard = Vec::new();
            // Reading nothing yet: in the server's process the watch holds one summary for the
            // client and waits to hand over the next, so it surely falls behind; elsewhere what
            // the transport buffers may keep it from falling behind, and it must hear it all
            // the same.
            watcher
                .watch_list(&Caller::default(), None, Some(long_ago()))
                .await;
            let rename = |thread, title: String| Request::ThreadRename { thread, title };
            server
                .request(rename(missed, "missed".into()))
                .await
                .unwrap();
            // Two changes each (its event, its work's end): far more than a watcher's backlog,
            // which keeps the latest, so the first thread's change is gone from it.
            for n in 0..600 {
                server
                    .request(rename(busy, format!("rename {n}")))
                    .await
                    .unwrap();
            }
            hear(&mut watcher, missed, &mut heard, |s| {
                s.title.as_deref() == Some("missed")
            })
            .await;
            hear(&mut watcher, busy, &mut heard, |s| {
                s.title.as_deref() == Some("rename 599")
            })
            .await;
            over.close().await;
        }
    });
}

#[test]
fn nobody_hears_of_threads_in_a_project_they_do_not_work_in() {
    block_on!(async {
        for kind in [Kind::InProcess, Kind::Http] {
            let name = format!("list-scope-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            // Over HTTP the watcher is ada, whom the test's server lets work in its own
            // project; in-process, bea, made a member of it here.
            let bea = Caller {
                person: Some(person("https://id.example.com", "bea").unwrap()),
                auth: None,
                trace: None,
            };
            if kind == Kind::InProcess {
                set_members(&server, vec![Principal::person(bea.person.unwrap())]).await;
            }
            let theirs = elsewhere(&server).await;
            let hidden = start_in(&server, theirs).await;
            let mut watcher = over.watcher().await;
            let mut heard = Vec::new();
            watcher.watch_list(&bea, None, Some(long_ago())).await;
            // Named, the other project is not there for them.
            if kind == Kind::InProcess {
                let refused = server
                    .watch_list(&bea, Some(theirs), None)
                    .await
                    .err()
                    .unwrap();
                assert_eq!(refused.code, code::NO_PROJECT);
            }
            let rename = Request::ThreadRename {
                thread: hidden,
                title: "Not yours".into(),
            };
            server.request(rename).await.unwrap();
            let shared = start(&server, false).await;
            hear(&mut watcher, shared, &mut heard, |_| true).await;
            assert!(
                heard.iter().all(|s| s.id != hidden && s.project != theirs),
                "{kind:?} heard of another project: {heard:?}"
            );
            over.close().await;
        }
    });
}

#[test]
fn someone_taken_off_a_project_stops_hearing_of_it_within_a_minute() {
    block_on!(async {
        let dir = folder("list-left");
        let mut setup = Setup::new(Vec::new());
        let clock = ManualClock::ticking(1_791_001_000_000);
        setup.clock = clock.clone();
        let server = setup.build(&dir).await.0;
        let bea = Caller {
            person: Some(person("https://id.example.com", "bea").unwrap()),
            auth: None,
            trace: None,
        };
        let joined = Principal::person(bea.person.unwrap());
        set_members(&server, vec![joined]).await;
        let shared = start(&server, false).await;
        let mut over = Over::new(Kind::InProcess, &server, "list-left").await;
        let mut watcher = over.watcher().await;
        let mut heard = Vec::new();
        watcher.watch_list(&bea, None, Some(long_ago())).await;
        hear(&mut watcher, shared, &mut heard, |_| true).await;
        set_members(&server, Vec::new()).await;
        clock.advance_seconds(61);
        let rename = Request::ThreadRename {
            thread: shared,
            title: "Not yours now".into(),
        };
        server.request(rename).await.unwrap();
        // Something of bea's own, told after the rename was.
        let chat = Request::ThreadStart {
            project: None,
            background: false,
            worktree: None,
            computer: None,
            model: None,
            approval: None,
            effort: None,
        };
        let Reply::Thread { thread: own } = server.request_as(&bea, chat).await.unwrap() else {
            panic!("a thread")
        };
        hear(&mut watcher, own.id, &mut heard, |_| true).await;
        assert!(
            heard
                .iter()
                .all(|s| s.title.as_deref() != Some("Not yours now")),
            "heard of a project they were taken off: {heard:?}"
        );
        over.close().await;
    });
}

/// Sets who works in the server's own project beside its person.
async fn set_members(server: &Tested, members: Vec<Principal>) {
    let mut home = server
        .projects()
        .get(scope().org(), scope().project())
        .await
        .unwrap()
        .unwrap();
    home.members = members;
    server
        .projects()
        .update(scope().org(), &home)
        .await
        .unwrap();
}

#[test]
fn a_childs_wait_is_its_parents_and_both_are_heard() {
    let dir = folder("list-subagent");
    block_on!(async {
        let answers = vec![
            answer(
                None,
                Some((
                    "agent",
                    json!({"description": "touch it", "prompt": "Run: touch ran.txt"}),
                )),
            ),
            answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
            answer(Some("Touched ran.txt."), None),
            answer(Some("Done."), None),
        ];
        let mut setup = Setup::asking(answers, Answering::Clients);
        setup.settings.subagents = true;
        let (server, _) = setup.build(&dir).await;
        let mut over = Over::new(Kind::InProcess, &server, "list-subagent").await;
        let mut watcher = over.watcher().await;
        let mut heard = Vec::new();
        watcher.watch_list(&Caller::default(), None, None).await;
        let parent = start(&server, false).await;
        let mut stream = stream_turn(&server, parent, "touch it", None)
            .await
            .unwrap();
        let (child, approval) = loop {
            let TurnItem::Note(note) = stream.next().await.unwrap() else {
                panic!("the turn ended before its child asked");
            };
            if let Notification::Event { thread, event } = note.as_ref()
                && let Body::ApprovalRequested(asked) = decode(event.clone()).unwrap().body
            {
                break (*thread, asked.call_id);
            }
        };
        hear(&mut watcher, child, &mut heard, |s| {
            s.status == Status::Waiting && s.parent == Some(parent)
        })
        .await;
        hear(&mut watcher, parent, &mut heard, |s| {
            s.status == Status::Waiting
        })
        .await;
        let decide = Request::ApprovalDecide {
            thread: child,
            approval,
            decision: Answer::AllowOnce,
        };
        server.request(decide).await.unwrap();
        stream.finish().await.unwrap();
        hear(&mut watcher, parent, &mut heard, |s| {
            s.status == Status::Idle && s.turns == 1
        })
        .await;
        over.close().await;
    });
}

#[test]
fn a_connection_of_lines_watches_each_list_once_until_it_unwatches_it() {
    block_on!(async {
        for kind in [Kind::Stdio, Kind::Socket] {
            let name = format!("list-once-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let Watcher::Lines(mut lines) = over.watcher().await else {
                unreachable!()
            };
            let all = Request::ThreadListWatch {
                project: None,
                after: None,
            };
            let home = Request::ThreadListWatch {
                project: Some(scope().project()),
                after: None,
            };
            let watching = lines.send(&all).await;
            let again = lines.send(&all).await;
            let other = lines.send(&home).await;
            let unwatch = lines
                .send(&Request::ThreadListUnwatch { project: None })
                .await;
            let unwatch_home = lines
                .send(&Request::ThreadListUnwatch {
                    project: Some(scope().project()),
                })
                .await;
            let mut answers = std::collections::BTreeMap::new();
            while answers.len() < 5 {
                let message = lines.receive().await;
                if let Some(id) = message.get("id").and_then(Value::as_u64) {
                    answers.insert(id, message);
                }
            }
            let refused = |id: u64| answers[&id]["error"]["code"].as_i64();
            assert_eq!(refused(again), Some(code::INVALID_PARAMS), "{kind:?}");
            let ended = json!({"ended": "unwatched"});
            assert_eq!(answers[&watching]["result"], ended, "{kind:?}");
            assert_eq!(answers[&other]["result"], ended, "{kind:?}");
            assert_eq!(answers[&unwatch]["result"], json!({}), "{kind:?}");
            assert_eq!(answers[&unwatch_home]["result"], json!({}), "{kind:?}");
            over.close().await;
        }
    });
}

#[test]
fn over_http_each_summary_carries_its_update_time_as_its_id() {
    block_on!(async {
        let dir = folder("list-ids");
        let server = server(&dir, Vec::new(), Answering::Clients).await;
        let over = Over::new(Kind::Http, &server, "list-ids").await;
        let thread = start(&server, false).await;
        let mut web = over.web();
        let watch = Request::ThreadListWatch {
            project: None,
            after: Some(long_ago()),
        };
        let (message, _) = web.message(&watch);
        let mut response = web.open(&message, Some("thread/list/watch")).await;
        let mut text = String::new();
        while !text.contains("\n\n") {
            let Some(chunk) = response.chunk().await.unwrap() else {
                panic!("the stream ended: {text}");
            };
            text.push_str(std::str::from_utf8(&chunk).unwrap());
        }
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
            panic!("a list")
        };
        let summary = threads.iter().find(|s| s.id == thread).unwrap();
        let id = format!("id: {}\ndata: ", summary.updated_at);
        assert!(text.starts_with(&id), "{text}");
        assert!(text.contains("\"method\":\"thread/changed\""), "{text}");
        drop(response);
        over.close().await;
    });
}

/// Runs a turn whose command waits for the person, heard waiting, then going on once allowed
/// while its answer waits at `gate`, then ending.
async fn ask_and_allow(
    server: &Tested,
    thread: ThreadId,
    gate: &Notify,
    watcher: &mut Watcher,
    heard: &mut Vec<ThreadSummary>,
) {
    let mut stream = stream_turn(server, thread, "touch it", None).await.unwrap();
    gate.notify_one();
    let approval = loop {
        match stream.next().await.unwrap() {
            TurnItem::Note(note) => {
                if let Notification::Live {
                    update: Update::ApprovalPending { approval },
                    ..
                } = note.as_ref()
                {
                    break approval.clone();
                }
            }
            other => panic!("the turn went on without asking: {other:?}"),
        }
    };
    hear(watcher, thread, heard, |s| s.status == Status::Waiting).await;
    let decide = Request::ApprovalDecide {
        thread,
        approval,
        decision: Answer::AllowOnce,
    };
    server.request(decide).await.unwrap();
    hear(watcher, thread, heard, |s| s.status == Status::Running).await;
    gate.notify_one();
    stream.finish().await.unwrap();
    hear(watcher, thread, heard, |s| {
        s.status == Status::Idle && s.turns == 2
    })
    .await;
}

/// A project of the local person's own that no one else works in.
async fn elsewhere(server: &Tested) -> ProjectId {
    let made = Request::ProjectCreate {
        name: "private".into(),
        members: Vec::new(),
        folder: None,
    };
    match server.request(made).await.unwrap() {
        Reply::Project { project, .. } => project.id,
        other => panic!("a project, not {other:?}"),
    }
}

/// A thread in `project`.
async fn start_in(server: &Tested, project: ProjectId) -> ThreadId {
    let started = Request::ThreadStart {
        project: Some(project),
        background: false,
        worktree: None,
        computer: None,
        model: None,
        approval: None,
        effort: None,
    };
    match server.request(started).await.unwrap() {
        Reply::Thread { thread } => thread.id,
        other => panic!("a thread, not {other:?}"),
    }
}
