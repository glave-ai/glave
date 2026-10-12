//! `thread/start`'s `worktree`: without it a thread works in the folder itself and records
//! that binding; asking for a worktree where none can be made is an error that says why.

use glave_event::{Body, Opened, decode};
use glave_session::Answering;
use glave_session::protocol::{Request, code};

use crate::support::*;

#[test]
fn a_thread_works_in_the_folder_unless_it_asks_for_a_worktree() {
    let dir = folder("worktree-param");
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::NoOne).await;
        let thread = start(&server, false).await;
        let bodies: Vec<Body> = events(&server, thread)
            .await
            .into_iter()
            .map(|raw| decode(raw).unwrap().body)
            .collect();
        assert!(
            matches!(&bodies[..], [Body::ThreadStarted(_), Body::WorkspaceBound(bound)]
                if matches!(bound.opened, Opened::Folder { .. })),
            "{bodies:?}"
        );

        // This server's computer keeps no worktrees, so asking for one is refused.
        let refused = server
            .request(Request::ThreadStart {
                space: None,
                background: false,
                worktree: Some(true),
                computer: None,
                model: None,
                approval: None,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::WORKSPACE);
        assert!(
            refused.message.contains("keeps no worktrees"),
            "{refused:?}"
        );
    });
}
