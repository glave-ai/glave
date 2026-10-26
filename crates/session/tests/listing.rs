//! Listing threads through the session API: pages from the threads view, most recently updated
//! first, with titles from the person's first message or a rename, and what each thread's
//! turns used; and renaming, which changes nothing the model is sent.

use glave_event::{ComputerKind, Outcome, ThreadId, TokenCounts, Usage};
use glave_ports::{EventLog, OrgId, ProjectId, Scope, Standing};
use glave_session::Answering;
use glave_session::protocol::{Reply, Request, Status, ThreadSummary, code};
use serde_json::json;

use crate::support::*;

async fn page<L: EventLog + 'static>(
    server: &Tested<L>,
    limit: usize,
    cursor: Option<String>,
) -> (Vec<ThreadSummary>, Option<String>) {
    let Reply::Threads { threads, next } = server.request(list(limit, cursor)).await.unwrap()
    else {
        panic!("threads")
    };
    (threads, next)
}

fn list(limit: usize, cursor: Option<String>) -> Request {
    Request::ThreadList {
        project: Some(scope().project()),
        limit: Some(limit),
        show: None,
        cursor,
    }
}

fn ids(threads: &[ThreadSummary]) -> Vec<ThreadId> {
    threads.iter().map(|thread| thread.id).collect()
}

#[test]
fn lists_come_a_page_at_a_time_with_titles_and_usage() {
    let dir = folder("listing-pages");
    block_on!(async {
        let answers = vec![answer(Some("Hi."), None)];
        let server = server(&dir, answers, Answering::NoOne).await;
        let (a, b, c) = (
            start(&server, false).await,
            start(&server, false).await,
            start(&server, false).await,
        );
        turn(&server, a, "go").await.unwrap();

        let (first, next) = page(&server, 2, None).await;
        assert_eq!(ids(&first), [a, c], "most recently written first");
        let Some(next) = next else {
            panic!("a second page follows")
        };
        let (second, last) = page(&server, 2, Some(next)).await;
        assert_eq!(ids(&second), [b]);
        assert_eq!(last, None, "nothing follows the last page");

        let worked = &first[0];
        assert_eq!(worked.title.as_deref(), Some("go"));
        assert_eq!(worked.turns, 1);
        assert_eq!(worked.outcome, Some(Outcome::Completed));
        assert_eq!(worked.status, Status::Idle);
        assert_eq!(worked.standing, Standing::Active);
        assert!(worked.updated_at > worked.created_at);
        let fresh = &first[1];
        assert_eq!((fresh.title.as_deref(), fresh.turns), (None, 0));

        let bad = Request::ThreadList {
            project: Some(scope().project()),
            limit: None,
            show: None,
            cursor: Some("not a cursor".into()),
        };
        let refused = server.request(bad).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
    });
}

#[test]
fn a_summary_says_where_its_thread_works_how_full_its_context_is_and_what_was_estimated() {
    let dir = folder("listing-facts");
    block_on!(async {
        let mut answered = costing(answer(Some("Hi."), None), 12_800, 0, 5);
        answered.usage = Usage::new(
            TokenCounts {
                input: 12_800,
                ..TokenCounts::default()
            },
            true,
            None,
        )
        .unwrap();
        let server = server(&dir, vec![answered], Answering::NoOne).await;
        let thread = start(&server, false).await;
        let summary = |reply| match reply {
            Reply::Thread { thread } => thread,
            other => panic!("a thread, not {other:?}"),
        };
        let fresh = summary(server.request(Request::ThreadGet { thread }).await.unwrap());
        assert_eq!(fresh.context, None, "no call yet");
        turn(&server, thread, "go").await.unwrap();
        let worked = summary(server.request(Request::ThreadGet { thread }).await.unwrap());
        let Some(workspace) = &worked.workspace else {
            panic!("it works somewhere: {worked:?}");
        };
        assert_eq!(workspace.computer.kind, ComputerKind::Local);
        assert_ne!(workspace.root, "", "the folder its tools act in");
        let context = worked.context.unwrap();
        assert_eq!((context.tokens, context.window), (12_800, 128_000));
        assert_eq!(context.percent, 10, "as compaction measures it");
        assert!(worked.estimated, "its one call's usage was estimated");
        assert_eq!(worked.cost.map(|cost| cost.micros), Some(5));
        let json = serde_json::to_value(&worked).unwrap();
        assert_eq!(
            json["context"],
            json!({"tokens": 12_800, "window": 128_000, "percent": 10})
        );
    });
}

/// Our threads, paged two at a time, on a store another organization shares: each page's
/// cursor, and every thread listed. With `busy`, the other organization writes between each
/// of our threads.
async fn our_pages(name: &str, busy: bool) -> (Vec<String>, Vec<ThreadId>, Vec<ThreadId>) {
    let dir = folder(name);
    let database = dir.join("glave.db");
    let store = sqlite(&database).await;
    let (ours, _) = Setup::new(Vec::new()).build_on(&dir, store.log()).await;
    let mut theirs = Setup::new(Vec::new());
    theirs.seed = 2;
    theirs.home = Scope::glaves_own(
        OrgId::from_uuid(uuid::Uuid::from_u128(3)),
        ProjectId::from_uuid(uuid::Uuid::from_u128(4)),
    );
    let elsewhere = theirs.home.project();
    let (theirs, _) = theirs.build_on(&dir, store.log()).await;
    let mut started = Vec::new();
    for _ in 0..5 {
        started.push(start(&ours, false).await);
        for _ in 0..u8::from(busy) * 7 {
            let theirs_too = Request::ThreadStart {
                project: Some(elsewhere),
                background: false,
                worktree: None,
                computer: None,
                model: None,
                permissions: None,
                effort: None,
            };
            theirs.request(theirs_too).await.unwrap();
        }
    }
    let (mut cursors, mut listed, mut cursor) = (Vec::new(), Vec::new(), None);
    loop {
        let (threads, next) = page(&ours, 2, cursor).await;
        listed.extend(ids(&threads));
        let Some(next) = next else { break };
        cursors.push(next.clone());
        cursor = Some(next);
    }
    // Their cursor is refused on our list, as one that does not read is.
    let request = Request::ThreadList {
        project: Some(elsewhere),
        limit: Some(1),
        show: None,
        cursor: None,
    };
    if let Reply::Threads {
        next: Some(foreign),
        ..
    } = theirs.request(request).await.unwrap()
    {
        let refused = ours.request(list(2, Some(foreign))).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
    }
    ours.shutdown().await;
    theirs.shutdown().await;
    (cursors, listed, started)
}

#[test]
fn a_cursor_says_nothing_of_anyone_elses_activity_and_pages_list_each_thread_once() {
    block_on!(async {
        let (alone, listed, started) = our_pages("cursors-alone", false).await;
        let (beside, listed_beside, _) = our_pages("cursors-beside", true).await;
        assert_eq!(
            alone, beside,
            "the other organization's writes show in no cursor"
        );
        assert_eq!(listed, listed_beside);
        let mut newest_first = started;
        newest_first.reverse();
        assert_eq!(listed, newest_first, "every thread once, most recent first");
        assert_eq!(alone.len(), 2);
    });
}

#[test]
fn a_thread_opened_with_the_projects_instructions_is_titled_by_the_prompt() {
    let dir = folder("listing-instructions");
    std::fs::write(dir.join("AGENTS.md"), "# Rules\n\nKeep answers short.\n").unwrap();
    block_on!(async {
        let answers = vec![answer(Some("Hi."), None)];
        let mut setup = Setup::new(answers);
        setup.answering = Answering::NoOne;
        setup.settings.instructions = glave_harness::agents_md(&dir, None)
            .await
            .unwrap()
            .map(|loaded| loaded.text);
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "go").await.unwrap();
        let opening = events(&server, thread)
            .await
            .into_iter()
            .find(|event| event.kind.as_str() == "message.added")
            .unwrap();
        assert!(
            opening.data.get().contains("Keep answers short."),
            "the first message opens with the instructions"
        );
        let (threads, _) = page(&server, 10, None).await;
        assert_eq!(threads[0].title.as_deref(), Some("go"));
    });
}

#[test]
fn a_rename_titles_the_thread() {
    let dir = folder("listing-rename");
    block_on!(async {
        let server = server(&dir, vec![answer(Some("Hi."), None)], Answering::NoOne).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "go").await.unwrap();
        let rename = |title: &str| Request::ThreadRename {
            thread,
            title: title.into(),
        };
        let Reply::Thread { thread: renamed } =
            server.request(rename("  Release notes ")).await.unwrap()
        else {
            panic!("the renamed thread")
        };
        assert_eq!(renamed.title.as_deref(), Some("Release notes"));
        let (listed, _) = page(&server, 10, None).await;
        assert_eq!(listed[0].title.as_deref(), Some("Release notes"));
        for title in ["   ", &"x".repeat(201)] {
            let refused = server.request(rename(title)).await.unwrap_err();
            assert_eq!(refused.code, code::INVALID_PARAMS, "{title:?}");
        }
        server
            .request(Request::ThreadDelete { thread })
            .await
            .unwrap();
        let gone = server.request(rename("Too late")).await.unwrap_err();
        assert_eq!(gone.code, code::NO_THREAD);
    });
}

#[test]
fn renaming_changes_nothing_the_model_is_sent() {
    let dir = folder("listing-same-requests");
    block_on!(async {
        let script = || {
            vec![
                answer(None, Some(("glob", json!({"pattern": "*"})))),
                answer(Some("One."), None),
                answer(None, Some(("glob", json!({"pattern": "*"})))),
                answer(Some("Two."), None),
            ]
        };
        let run = |rename: bool| {
            let dir = dir.to_path_buf();
            let answers = script();
            async move {
                let (server, seen) = Setup::asking(answers, Answering::NoOne).build(&dir).await;
                let thread = start(&server, false).await;
                turn(&server, thread, "go").await.unwrap();
                if rename {
                    let title = "Renamed".to_owned();
                    let request = Request::ThreadRename { thread, title };
                    server.request(request).await.unwrap();
                }
                turn(&server, thread, "go").await.unwrap();
                json(&seen)
            }
        };
        let plain = run(false).await;
        let renamed = run(true).await;
        assert_eq!(plain.len(), 4);
        assert_eq!(plain, renamed);
    });
}
