//! Watching the threads list (`thread/list/watch`) over every transport: a thread's summary
//! comes the moment it runs, waits for the person, goes on, ends, is renamed or deleted; with
//! `after`, what changed since comes first and nothing older; a watcher far behind still hears
//! the latest; a subagent's wait is its parent's; a removed project's threads are heard
//! archived; and nobody hears of threads in another person's projects.

use std::sync::Arc;

use glave_event::{At, Body, ThreadId, decode};
use glave_ports::testing::ManualClock;
use glave_ports::{Caller, ProjectId, Standing};
use glave_session::protocol::{
    Answer, Grouping, Notification, Reply, Request, Setting, Status, ThreadSummary, Update, code,
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
            // A watcher that fell behind is told every setting may have changed.
            Said::Note(Notification::SettingsChanged { .. }) => {}
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
fn nobody_hears_of_threads_in_another_person_s_projects() {
    block_on!(async {
        for kind in [Kind::InProcess, Kind::Http] {
            let name = format!("list-scope-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            // The watcher is the person here; bea, whom a host embedding the server calls for, has
            // a project of her own.
            let bea = Caller {
                person: Some(person("https://id.example.com", "bea").unwrap()),
                auth: None,
                trace: None,
                profile: None,
            };
            let theirs = project_of(&server, &bea).await;
            let hidden = start_in(&server, &bea, theirs).await;
            let mut watcher = over.watcher().await;
            let mut heard = Vec::new();
            watcher
                .watch_list(&Caller::default(), None, Some(long_ago()))
                .await;
            // Named, her project is not there for them.
            if kind == Kind::InProcess {
                let refused = server
                    .watch_list(&Caller::default(), Some(theirs), None)
                    .await
                    .err()
                    .unwrap();
                assert_eq!(refused.code, code::NO_PROJECT);
            }
            let rename = Request::ThreadRename {
                thread: hidden,
                title: "Not yours".into(),
            };
            server.request_as(&bea, rename).await.unwrap();
            let own = start(&server, false).await;
            hear(&mut watcher, own, &mut heard, |_| true).await;
            assert!(
                heard.iter().all(|s| s.id != hidden && s.project != theirs),
                "{kind:?} heard of another person's project: {heard:?}"
            );
            over.close().await;
        }
    });
}

#[test]
fn a_person_hears_their_own_settings_change_and_nobody_else_s() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("list-settings-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let bea = Caller {
                person: Some(person("https://id.example.com", "bea").unwrap()),
                auth: None,
                trace: None,
                profile: None,
            };
            let mut watcher = over.watcher().await;
            // From long ago, so the thread is heard of whether the watch begins before it or
            // after it; heard, the watch follows from there.
            watcher
                .watch_list(&Caller::default(), None, Some(long_ago()))
                .await;
            let first = start(&server, false).await;
            hear(&mut watcher, first, &mut Vec::new(), |_| true).await;
            let grouped = || Request::GroupingSet {
                grouping: Grouping::Status,
            };
            server.request_as(&bea, grouped()).await.unwrap();
            server.request(grouped()).await.unwrap();
            server.request(Request::GroupingGet {}).await.unwrap();
            let chosen = Request::LanguageSet {
                language: Some("en-XA".into()),
            };
            server.request(chosen).await.unwrap();
            let second = start(&server, false).await;
            let mut settings = Vec::new();
            loop {
                match watcher.next().await {
                    Said::Note(Notification::SettingsChanged { setting }) => settings.push(setting),
                    Said::Note(Notification::Changed { summary, .. }) if summary.id == second => {
                        break;
                    }
                    Said::Note(_) => {}
                    Said::Answer(answer) => panic!("the watch ended: {answer}"),
                }
            }
            assert_eq!(
                settings,
                [Setting::Grouping, Setting::Language],
                "{kind:?}: their own changes, once each; not another person's, nor a read"
            );
            over.close().await;
        }
    });
}

#[test]
fn removing_a_project_is_heard_by_whoever_watches_it() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("list-removed-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let garden = dir.join("garden");
            std::fs::create_dir_all(&garden).unwrap();
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let project = make_project(&server, "Garden", &garden).await;
            let thread = start_in(&server, &Caller::default(), project).await;
            let mut over = Over::new(kind, &server, &name).await;
            // From long ago, so whether a watch over a connection begins before the removal or
            // after it, it is heard: what was recorded first comes from the threads view.
            let (mut every, mut named) = (over.watcher().await, over.watcher().await);
            every
                .watch_list(&Caller::default(), None, Some(long_ago()))
                .await;
            named
                .watch_list(&Caller::default(), Some(project), Some(long_ago()))
                .await;
            let removed = server.request(Request::ProjectRemove { project }).await;
            assert!(matches!(removed, Ok(Reply::Done {})), "{removed:?}");
            let mut heard = Vec::new();
            for watcher in [&mut every, &mut named] {
                let archived = hear(watcher, thread, &mut heard, |s| {
                    s.standing == Standing::Archived
                })
                .await;
                assert_eq!(archived.project, project, "{kind:?}");
            }
            over.close().await;
        }
    });
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

/// A project of `caller`'s own, with no folder.
async fn project_of(server: &Tested, caller: &Caller) -> ProjectId {
    let made = Request::ProjectCreate {
        name: "private".into(),
        folder: None,
    };
    match server.request_as(caller, made).await.unwrap() {
        Reply::Project { project, .. } => project.id,
        other => panic!("a project, not {other:?}"),
    }
}

/// A thread of `caller`'s in `project`.
async fn start_in(server: &Tested, caller: &Caller, project: ProjectId) -> ThreadId {
    let started = Request::ThreadStart {
        project: Some(project),
        background: false,
        worktree: None,
        computer: None,
        model: None,
        permissions: None,
        effort: None,
    };
    match server.request_as(caller, started).await.unwrap() {
        Reply::Thread { thread } => thread.id,
        other => panic!("a thread, not {other:?}"),
    }
}
