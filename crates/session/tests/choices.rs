//! What a project chooses for its threads (`specs/one-server.md`, "Each project's own tools,
//! hooks and settings"): its model and its worktrees reach the threads of the person glave
//! works for, never another person's, even in a project of their own with a folder here.

use std::path::Path;
use std::sync::Arc;

use glave_event::{Opened, ThreadId};
use glave_harness::{Brought, ProjectFolders, Settled};
use glave_ports::{BoxFuture, Caller, InMemoryLog, ProjectId};
use glave_session::protocol::{ModelInfo, NewFolder, Reply, Request};
use glave_session::signin::person;

use crate::support::*;

/// A project that chooses its own model, and a worktree for each new thread.
struct Chooses;

impl Chooses {
    fn settled() -> Settled {
        Settled {
            model: Some("project-model".into()),
            worktree: Some(true),
            ..Settled::default()
        }
    }
}

impl ProjectFolders for Chooses {
    fn for_folder<'a>(
        &'a self,
        _root: &'a Path,
        _project: Option<&'a Path>,
        _trusting: bool,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }

    fn settled<'a>(&'a self, _project: &'a Path) -> BoxFuture<'a, Settled> {
        Box::pin(async { Self::settled() })
    }

    fn brought<'a>(&'a self, _project: &'a Path, _id: ProjectId) -> BoxFuture<'a, Brought> {
        Box::pin(async {
            Brought {
                settled: Self::settled(),
                ..Brought::default()
            }
        })
    }
}

/// Starts a thread for `caller` in `project`.
async fn start_as(server: &Tested, caller: &Caller, project: ProjectId) -> ThreadId {
    let started = Request::ThreadStart {
        project: Some(project),
        worktree: None,
        background: false,
        computer: None,
        model: None,
        approval: None,
        effort: None,
    };
    match server.request_as(caller, started).await.unwrap() {
        Reply::Thread { thread } => thread.id,
        other => panic!("a thread, not {other:?}"),
    }
}

/// The model the request at `index` went to.
fn sent_to(seen: &Seen, index: usize) -> serde_json::Value {
    let sent: serde_json::Value = serde_json::from_str(&json(seen)[index]).unwrap();
    sent["model"].clone()
}

#[test]
fn a_project_s_choices_reach_the_person_s_threads_in_it_never_another_s() {
    block_on!(async {
        let answers = (0..2).map(|_| answer(Some("Hi."), None)).collect();
        let mut setup = Setup::new(answers);
        setup.project_folders = Some(Arc::new(Chooses));
        setup.models = vec![ModelInfo::new("project-model")];
        let (server, seen, _, dir) = setup
            .build_in_repository("choices-project", InMemoryLog::new())
            .await;

        let here = Caller::default();
        let ours = start_as(&server, &here, scope().project()).await;
        let (place, _) = opened(&server, ours).await;
        assert!(
            matches!(place, Opened::Worktree { .. }),
            "its own: {place:?}"
        );
        turn(&server, ours, "hi").await.unwrap();
        assert_eq!(sent_to(&seen, 0), "project-model");

        // Another person, as a host embedding the server may call for her, in a project of her
        // own whose folder chooses the same: glave's own transports act as the person here, so
        // this is the guard, kept.
        let elsewhere = Caller {
            person: Some(person("https://id.example.com", "grace").unwrap()),
            ..Caller::default()
        };
        let garden = dir.join("garden");
        std::fs::create_dir_all(&garden).unwrap();
        let made = Request::ProjectCreate {
            name: "Hers".into(),
            folder: Some(NewFolder::Path {
                path: garden.to_string_lossy().into_owned(),
            }),
        };
        let Reply::Project { project, .. } = server.request_as(&elsewhere, made).await.unwrap()
        else {
            panic!("a project");
        };
        let theirs = start_as(&server, &elsewhere, project.id).await;
        let (place, _) = opened_as(&server, &elsewhere, theirs).await;
        assert!(
            matches!(place, Opened::Folder { .. }),
            "none of it: {place:?}"
        );
        let turned = Request::TurnStart {
            thread: theirs,
            prompt: "hi".into(),
            background: false,
            budget: None,
            model: None,
            approval: None,
            effort: None,
            meta: glave_session::protocol::Meta::default(),
            unattended: false,
            files: Vec::new(),
        };
        server.request_as(&elsewhere, turned).await.unwrap();
        assert_eq!(sent_to(&seen, 1), "test-model");
    });
}
