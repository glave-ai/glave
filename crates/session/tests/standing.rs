//! Archiving and deleting threads through the session API: lists hide what is archived or
//! deleted, a change is the thread's next event, a deleted thread is gone from every method
//! but the list of deleted ones, and none of it changes what the model is sent.

use glave_event::ThreadId;
use glave_ports::EventLog;
use glave_session::Answering;
use glave_session::protocol::{Reply, Request, Show, code};
use serde_json::json;

use crate::support::*;

async fn listed<L: EventLog + 'static>(server: &Tested<L>, show: Show) -> Vec<ThreadId> {
    let request = Request::ThreadList {
        space: None,
        limit: None,
        show: Some(show),
        cursor: None,
    };
    let Reply::Threads { threads, .. } = server.request(request).await.unwrap() else {
        panic!("threads")
    };
    threads.into_iter().map(|thread| thread.id).collect()
}

#[test]
fn archiving_hides_a_thread_and_unarchiving_brings_it_back() {
    let dir = folder("archive-list");
    block_on!(async {
        let answers = vec![answer(Some("Hi."), None)];
        let server = server(&dir, answers, Answering::NoOne).await;
        let (a, b) = (start(&server, false).await, start(&server, false).await);
        assert_eq!(listed(&server, Show::Active).await, [b, a]);

        let archived = server
            .request(Request::ThreadArchive { thread: a })
            .await
            .unwrap();
        assert!(matches!(archived, Reply::Done {}));
        assert_eq!(listed(&server, Show::Active).await, [b]);
        assert_eq!(listed(&server, Show::Archived).await, [a]);
        // Kept in the log: still readable, but no turn runs in it.
        assert!(
            kinds(&server, a)
                .await
                .contains(&"thread.archived".to_owned())
        );
        let refused = turn(&server, a, "go").await.unwrap_err();
        assert_eq!(refused.code, code::ARCHIVED);
        // Archiving again records nothing more.
        let before = kinds(&server, a).await.len();
        server
            .request(Request::ThreadArchive { thread: a })
            .await
            .unwrap();
        assert_eq!(kinds(&server, a).await.len(), before);

        server
            .request(Request::ThreadUnarchive { thread: a })
            .await
            .unwrap();
        assert_eq!(listed(&server, Show::Active).await, [a, b]);
        assert_eq!(
            listed(&server, Show::Archived).await,
            Vec::<ThreadId>::new()
        );
        assert!(matches!(
            turn(&server, a, "go").await,
            Ok(Reply::Turn { .. })
        ));
    });
}

#[test]
fn deleting_hides_a_thread_for_good_and_lists_it_for_erasure() {
    let dir = folder("delete");
    block_on!(async {
        let database = dir.join("glave.db");
        let (first, _) = Setup::asking(Vec::new(), Answering::NoOne)
            .build_on(&dir, sqlite(&database).await.log())
            .await;
        let (keep, gone) = (start(&first, false).await, start(&first, false).await);
        first
            .request(Request::ThreadDelete { thread: gone })
            .await
            .unwrap();
        first.shutdown().await;

        // A fresh server, from the log alone, sees the same.
        let mut again = Setup::asking(Vec::new(), Answering::NoOne);
        again.seed = 2;
        let (second, _) = again.build_on(&dir, sqlite(&database).await.log()).await;
        assert_eq!(listed(&second, Show::Active).await, [keep]);
        assert_eq!(
            listed(&second, Show::Archived).await,
            Vec::<ThreadId>::new()
        );
        assert_eq!(listed(&second, Show::Deleted).await, [gone]);
        for request in [
            Request::ThreadGet { thread: gone },
            Request::ThreadEvents {
                thread: gone,
                after: None,
                limit: None,
            },
            Request::ThreadUnarchive { thread: gone },
            Request::ThreadArchive { thread: gone },
        ] {
            let error = second.request(request).await.unwrap_err();
            assert_eq!(error.code, code::NO_THREAD);
        }
        assert_eq!(
            turn(&second, gone, "go").await.unwrap_err().code,
            code::NO_THREAD
        );
    });
}

#[test]
fn archiving_changes_nothing_the_model_is_sent() {
    let dir = folder("same-requests");
    block_on!(async {
        let script = || {
            vec![
                answer(None, Some(("glob", json!({"pattern": "*"})))),
                answer(Some("One."), None),
                answer(None, Some(("glob", json!({"pattern": "*"})))),
                answer(Some("Two."), None),
            ]
        };
        let run = |archive: bool| {
            let dir = dir.clone();
            let answers = script();
            async move {
                let (server, seen) = Setup::asking(answers, Answering::NoOne).build(&dir).await;
                let thread = start(&server, false).await;
                turn(&server, thread, "go").await.unwrap();
                if archive {
                    for request in [
                        Request::ThreadArchive { thread },
                        Request::ThreadUnarchive { thread },
                    ] {
                        server.request(request).await.unwrap();
                    }
                }
                turn(&server, thread, "go").await.unwrap();
                seen.lock().unwrap().clone()
            }
        };
        let plain = run(false).await;
        let changed = run(true).await;
        assert_eq!(plain.len(), 4);
        assert_eq!(plain, changed);
    });
}
