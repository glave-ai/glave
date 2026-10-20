//! Where a thread works, as the session API offers it: `thread/start` with a `computer` (a
//! person's own laptop dialled in, later) is refused with `-32022` until computers can dial
//! in, and no thread starts. A summary says nothing that assumes a worker.

use glave_ports::computer::ComputerId;
use glave_session::Answering;
use glave_session::protocol::{Reply, Request, Show, code};

use crate::support::*;

#[test]
fn a_thread_on_a_computer_of_one_s_own_is_not_available_yet() {
    let dir = folder("session-placement");
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::NoOne).await;
        let laptop = ComputerId::from_uuid(uuid::Uuid::nil());
        let refused = server
            .request(Request::ThreadStart {
                project: Some(scope().project()),
                worktree: None,
                background: false,
                computer: Some(laptop),
                model: None,
                approval: None,
                effort: None,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        assert!(refused.message.contains("not available yet"), "{refused}");
        let listed = server
            .request(Request::ThreadList {
                project: Some(scope().project()),
                limit: None,
                show: Some(Show::Active),
                cursor: None,
            })
            .await
            .unwrap();
        let Reply::Threads { threads, .. } = listed else {
            panic!("threads, not {listed:?}")
        };
        assert!(threads.is_empty(), "no thread started");
        let thread = start(&server, false).await;
        let Reply::Thread { thread } = server.request(Request::ThreadGet { thread }).await.unwrap()
        else {
            panic!("a thread")
        };
        let shown = serde_json::to_string(&thread).unwrap();
        assert!(!shown.contains("worker"), "{shown}");
    });
}
