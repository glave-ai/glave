//! `thread/start`'s `worktree`: without it a thread works in the folder itself and records
//! that binding; asking for a worktree where none can be made is an error that says why; and
//! `project/get` says whether a new chat may work on one at all.

use glave_event::{Body, Opened, decode};
use glave_ports::InMemoryLog;
use glave_session::Answering;
use glave_session::protocol::{Copies, Reply, Request, code};

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
                project: Some(scope().project()),
                background: false,
                worktree: Some(true),
                computer: None,
                model: None,
                permissions: None,
                effort: None,
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

/// What `project/get` says of a new chat's copies in the server's own project.
async fn copies(server: &Tested) -> Option<Copies> {
    let got = server
        .request(Request::ProjectGet {
            project: scope().project(),
        })
        .await
        .unwrap();
    let Reply::Project { copies, .. } = got else {
        panic!("a project, not {got:?}");
    };
    copies
}

#[test]
fn a_project_says_whether_a_new_chat_may_work_on_a_separate_copy() {
    block_on!(async {
        // A repository with a commit, on this server's computer: one may be offered, and a new
        // chat works in the folder itself unless it asks.
        let (server, _, _repo, _dir) = Setup::new(Vec::new())
            .build_in_repository("copies-offered", InMemoryLog::new())
            .await;
        assert_eq!(copies(&server).await, Some(Copies { by_default: false }));
    });
    let dir = folder("copies-plain");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.worktrees = Some(dir.join("worktrees"));
        let plain = dir.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let (server, _) = setup.build(&plain).await;
        // A folder that is no repository has no copy to give, nor one with no commit yet, nor
        // one that isn't there anymore, which `project/get` still answers.
        assert_eq!(copies(&server).await, None);
        sh(&plain, "git init -q");
        assert_eq!(copies(&server).await, None);
        std::fs::remove_dir_all(&plain).unwrap();
        assert_eq!(copies(&server).await, None);
    });
}
