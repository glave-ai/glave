//! `thread/start`'s `worktree`: without it a thread works in the folder itself and records
//! that binding; asking for a worktree where none can be made is an error that says why.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod worktrees {
    use glave_event::{Body, Opened, decode};
    use glave_session::Answering;
    use glave_session::protocol::{Reply, Request, code};

    use super::support::*;

    #[test]
    fn a_thread_works_in_the_folder_unless_it_asks_for_a_worktree() {
        let dir = folder("worktree-param");
        block_on(async {
            let server = server(&dir, Vec::new(), Answering::NoOne).await;
            let Reply::Thread { thread } = server
                .request(Request::ThreadStart {
                    space: None,
                    background: false,
                    worktree: None,
                    computer: None,
                    model: None,
                    approval: None,
                })
                .await
                .unwrap()
            else {
                panic!("a thread")
            };
            let page = Request::ThreadEvents {
                thread: thread.id,
                after: None,
                limit: None,
            };
            let Reply::Events { events, .. } = server.request(page).await.unwrap() else {
                panic!("events")
            };
            let bodies: Vec<Body> = events
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
}
