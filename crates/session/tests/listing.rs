//! Listing threads through the session API: pages from the threads view, most recently written
//! first, with titles from the person's first message or a rename, and what each thread's
//! turns used; and renaming, which changes nothing the model is sent.

use glave_event::{Outcome, ThreadId};
use glave_ports::Standing;
use glave_session::Answering;
use glave_session::protocol::{Reply, Request, Status, ThreadSummary, code};
use serde_json::json;

use crate::support::*;

async fn page(
    server: &Tested,
    limit: usize,
    cursor: Option<String>,
) -> (Vec<ThreadSummary>, Option<String>) {
    let request = Request::ThreadList {
        project: Some(scope().project()),
        limit: Some(limit),
        show: None,
        cursor,
    };
    let Reply::Threads { threads, next } = server.request(request).await.unwrap() else {
        panic!("threads")
    };
    (threads, next)
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
