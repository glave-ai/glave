//! Tasks through the session API against a scripted model: a turn or goal started with
//! `background` runs with no one attached, on a worktree of its own, and can be read
//! afterwards; tasks run in every project, each on a copy of its own project's folder, and a
//! folder that is not a repository, or a thread that works in its folder, is refused;
//! `tasks/list` shows their tokens and cost from the events; `tasks/stop` and archiving stop
//! them; a turn stops at its budget; policy limits refuse loudly; `recover_tasks` covers every
//! project; and the model is sent exactly what it is sent when someone follows the turn.

use std::path::Path;
use std::sync::Arc;

use glave_event::{Opened, Outcome};
use glave_harness::ApprovalMode;
use glave_ports::InMemoryLog;
use glave_ports::testing::TempDir;
use glave_session::Answering;
use glave_session::protocol::{Notification, Reply, Request, TaskStatus, code};
use serde_json::json;
use tokio::sync::Notify;

use crate::support::*;

/// `setup` on a repository of its own, its tasks working on worktrees of it.
async fn in_repository(name: &str, setup: Setup) -> (Tested, Seen, std::path::PathBuf, TempDir) {
    setup.build_in_repository(name, InMemoryLog::new()).await
}

/// A thread of `server`'s own project that tasks run in, on a copy of its own.
async fn task_thread(server: &Tested) -> glave_event::ThreadId {
    start_for_tasks(server, None).await.unwrap()
}

/// The root of the worktree `thread` works on, checked to be one of `repository`.
async fn worktree_of(server: &Tested, thread: glave_event::ThreadId, repository: &Path) -> String {
    let (opened, root) = opened(server, thread).await;
    let Opened::Worktree { repository: of, .. } = opened else {
        panic!("a worktree, not {opened:?}");
    };
    let canonical = std::fs::canonicalize(repository).unwrap();
    assert_eq!(Path::new(&of), canonical);
    assert!(!Path::new(&root).starts_with(&canonical), "outside it");
    root
}

#[test]
fn a_background_turn_runs_on_a_worktree_with_no_one_attached_and_can_be_read_afterwards() {
    block_on!(async {
        let setup = Setup::new(vec![
            costing(
                answer(
                    None,
                    Some(("write", json!({"path": "notes.md", "content": "tidy"}))),
                ),
                1_000,
                100,
                2_000,
            ),
            costing(answer(Some("Tidied."), None), 1_200, 50, 3_000),
        ]);
        let (server, _, repo, _dir) = in_repository("task-runs", setup).await;
        let thread = task_thread(&server).await;
        let mut watching = server.watch(thread);
        let task = background(&server, thread, None).await;
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.title, "tidy the notes");
        assert_eq!(task.model, "test-model");
        let finished = loop {
            if let Notification::TaskFinished { task, .. } = watching.recv().await.unwrap().as_ref()
            {
                break task.clone();
            }
        };
        assert_eq!(finished.status, TaskStatus::Finished);
        assert_eq!(finished.outcome, Some(Outcome::Completed));
        assert_eq!(
            (finished.tokens.input, finished.tokens.output),
            (2_200, 150)
        );
        assert_eq!(finished.cost.map(|cost| cost.micros), Some(5_000));
        let listed = tasks(&server).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].thread, thread);
        assert_eq!(listed[0].tokens, finished.tokens);
        assert_eq!(listed[0].cost, finished.cost);
        assert_eq!(
            kinds(&server, thread).await,
            [
                "thread.started",
                "workspace.bound",
                "turn.started",
                "message.added",
                "tool.call.requested",
                "model.responded",
                "tool.call.completed",
                "message.added",
                "model.responded",
                "turn.ended"
            ]
        );
        // On its copy, never in the folder under the person.
        let root = worktree_of(&server, thread, &repo).await;
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("notes.md")).unwrap(),
            "tidy"
        );
        assert!(!repo.join("notes.md").exists());
        // The thread is idle and takes a plain turn again.
        assert!(matches!(
            server.request(Request::ThreadGet { thread }).await.unwrap(),
            Reply::Thread { thread } if thread.status == glave_session::protocol::Status::Idle
        ));
    });
}

#[test]
fn a_task_in_each_of_two_projects_works_on_a_worktree_of_its_own_project() {
    block_on!(async {
        let write = |name: &str| {
            answer(
                None,
                Some(("write", json!({"path": "done.md", "content": name}))),
            )
        };
        let setup = Setup::new(vec![
            write("one"),
            answer(Some("One."), None),
            write("two"),
            answer(Some("Two."), None),
        ]);
        let (server, _, repo, _dir) = in_repository("task-projects", setup).await;
        let others = repo.with_extension("others");
        let mut ran = Vec::new();
        for name in ["one", "two"] {
            let folder = repository(&others, name);
            let project = make_project(&server, name, &folder).await;
            let thread = start_for_tasks(&server, Some(project)).await.unwrap();
            background(&server, thread, None).await;
            settle(&server).await;
            ran.push((thread, project, folder, name));
        }
        let listed = tasks(&server).await;
        assert_eq!(listed.len(), 2, "{listed:?}");
        for (thread, project, folder, name) in &ran {
            let root = worktree_of(&server, *thread, folder).await;
            let written = std::fs::read_to_string(Path::new(&root).join("done.md")).unwrap();
            assert_eq!(written, *name);
            assert!(!folder.join("done.md").exists());
            let task = listed.iter().find(|task| task.thread == *thread).unwrap();
            assert_eq!(task.project, *project);
            assert_eq!(task.outcome, Some(Outcome::Completed));
        }
    });
}

#[test]
fn a_task_never_works_in_a_folder_itself() {
    let dir = folder("task-in-place");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.worktrees = Some(dir.with_extension("worktrees"));
        let plain = dir.with_extension("plain");
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        let (server, _) = setup.build(&plain).await;
        // A folder that is not a repository has no copy to work on.
        let refused = start_for_tasks(&server, None).await.unwrap_err();
        assert_eq!(refused.code, code::WORKSPACE);
        assert!(
            refused
                .message
                .contains("not a git repository with a commit"),
            "{refused}"
        );
        // Nor has a repository with no commit yet.
        sh(&plain, "git init -q");
        let refused = start_for_tasks(&server, None).await.unwrap_err();
        assert_eq!(refused.code, code::WORKSPACE);
        assert!(refused.message.contains("make it one"), "{refused}");
        let listed = server
            .request(Request::ThreadList {
                project: Some(scope().project()),
                limit: None,
                show: None,
                cursor: None,
            })
            .await
            .unwrap();
        assert!(
            matches!(&listed, Reply::Threads { threads, .. } if threads.is_empty()),
            "nothing recorded: {listed:?}"
        );
        // Asked for in the folder itself, or on a thread that works there, it is refused.
        let in_place = server
            .request(Request::ThreadStart {
                project: Some(scope().project()),
                background: true,
                worktree: Some(false),
                computer: None,
                model: None,
                approval: None,
            })
            .await
            .unwrap_err();
        assert_eq!(in_place.code, code::INVALID_PARAMS);
        let thread = start(&server, false).await;
        let refused = server
            .request(Request::TurnStart {
                thread,
                prompt: "go".into(),
                background: true,
                budget: None,
                meta: glave_session::protocol::Meta::default(),
                model: None,
                approval: None,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        assert!(refused.message.contains("new thread"), "{refused}");
        assert_eq!(tasks(&server).await, Vec::new());
    });
}

#[test]
fn approvals_in_a_task_are_answered_no_one_there_and_the_model_is_sent_the_same() {
    block_on!(async {
        let go = || {
            vec![
                answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                answer(Some("Could not run it."), None),
            ]
        };
        // Asks for every command, with a client that could answer.
        let mut setup = Setup::new(go());
        setup.settings.approval = ApprovalMode::Ask;
        let (server, seen, repo, _dir) = in_repository("task-unattended", setup).await;
        let thread = task_thread(&server).await;
        background(&server, thread, None).await;
        settle(&server).await;
        let events = events(&server, thread).await;
        let decided = events
            .iter()
            .find(|event| event.kind.as_str() == "approval.decided")
            .unwrap();
        assert!(
            decided.data.get().contains("no_one_there"),
            "{}",
            decided.data.get()
        );
        // The same turn followed by no one, in the folder itself, from a server that never
        // has anyone to ask.
        let mut followed = Setup::new(go());
        followed.settings.approval = ApprovalMode::Ask;
        followed.answering = Answering::NoOne;
        let (plain, plain_seen) = followed.build(&repo).await;
        let other = start(&plain, false).await;
        plain
            .request(Request::TurnStart {
                thread: other,
                prompt: "tidy the notes".into(),
                background: false,
                budget: None,
                meta: glave_session::protocol::Meta::default(),
                model: None,
                approval: None,
            })
            .await
            .unwrap();
        let strip = |requests: &Seen| -> Vec<serde_json::Value> {
            json(requests)
                .iter()
                .map(|json| {
                    let mut value: serde_json::Value = serde_json::from_str(json).unwrap();
                    // Which thread a call is for; providers never see it.
                    value.as_object_mut().unwrap().remove("threads");
                    value
                })
                .collect()
        };
        assert_eq!(strip(&seen), strip(&plain_seen));
        assert_eq!(strip(&seen).len(), 2);
    });
}

#[test]
fn stopping_a_task_cancels_it_cleanly() {
    block_on!(async {
        let mut setup = Setup::new(vec![
            answer(Some("never"), None),
            answer(Some("again"), None),
        ]);
        setup.gate = Some(Arc::new(Notify::new()));
        let (server, _, _, _dir) = in_repository("task-stop", setup).await;
        let thread = task_thread(&server).await;
        background(&server, thread, None).await;
        assert_eq!(tasks(&server).await[0].status, TaskStatus::Running);
        server.request(Request::TasksStop { thread }).await.unwrap();
        settle(&server).await;
        let listed = tasks(&server).await;
        assert_eq!(listed[0].outcome, Some(Outcome::Cancelled));
        let kinds = kinds(&server, thread).await;
        assert_eq!(kinds.last().map(String::as_str), Some("turn.ended"));
        let stopped_again = server.request(Request::TasksStop { thread }).await;
        assert_eq!(stopped_again.unwrap_err().code, code::NO_TURN);
    });
}

#[test]
fn a_task_a_stop_cuts_short_is_recorded_interrupted() {
    block_on!(async {
        let mut setup = Setup::new(vec![answer(Some("never"), None)]);
        setup.gate = Some(Arc::new(Notify::new()));
        let (server, _, _, _dir) = in_repository("task-interrupted", setup).await;
        let thread = task_thread(&server).await;
        background(&server, thread, None).await;
        server.stop(std::time::Duration::ZERO).await;
        assert_eq!(tasks(&server).await[0].outcome, Some(Outcome::Interrupted));
    });
}

#[test]
fn archiving_a_task_s_thread_stops_the_task() {
    block_on!(async {
        let mut setup = Setup::new(vec![answer(Some("never"), None)]);
        setup.gate = Some(Arc::new(Notify::new()));
        let (server, _, _, _dir) = in_repository("task-archive", setup).await;
        let thread = task_thread(&server).await;
        background(&server, thread, None).await;
        server
            .request(Request::ThreadArchive { thread })
            .await
            .unwrap();
        settle(&server).await;
        assert_eq!(tasks(&server).await[0].outcome, Some(Outcome::Cancelled));
        let kinds = kinds(&server, thread).await;
        assert_eq!(kinds.last().map(String::as_str), Some("thread.archived"));
    });
}

#[test]
fn a_turn_stops_at_its_budget() {
    block_on!(async {
        let gate = Arc::new(Notify::new());
        let mut setup = Setup::new(vec![
            costing(
                answer(
                    None,
                    Some(("write", json!({"path": "a.md", "content": "a"}))),
                ),
                900,
                200,
                10,
            ),
            answer(Some("never sent"), None),
        ]);
        setup.gate = Some(Arc::clone(&gate));
        let (server, _, _, _dir) = in_repository("task-budget", setup).await;
        let thread = task_thread(&server).await;
        let task = background(&server, thread, Some(1_000)).await;
        assert_eq!(task.budget, Some(1_000));
        gate.notify_one();
        settle(&server).await;
        let listed = tasks(&server).await;
        assert_eq!(listed[0].outcome, Some(Outcome::LimitReached));
        assert_eq!(listed[0].tokens.input, 900);
        // A budget is only for a turn that runs as a task.
        let followed = server
            .request(Request::TurnStart {
                thread,
                prompt: "go".into(),
                background: false,
                budget: Some(5),
                meta: glave_session::protocol::Meta::default(),
                model: None,
                approval: None,
            })
            .await;
        assert_eq!(followed.unwrap_err().code, code::INVALID_PARAMS);
    });
}

#[test]
fn policy_limits_how_many_tasks_run_at_once() {
    block_on!(async {
        let mut setup = Setup::new(vec![answer(Some("a"), None), answer(Some("b"), None)]);
        setup.gate = Some(Arc::new(Notify::new()));
        setup.limits.max_running = Some(1);
        let (server, _, _, _dir) = in_repository("task-limit", setup).await;
        let first = task_thread(&server).await;
        let second = task_thread(&server).await;
        background(&server, first, None).await;
        let refused = server
            .request(Request::TurnStart {
                thread: second,
                prompt: "go".into(),
                background: true,
                budget: None,
                meta: glave_session::protocol::Meta::default(),
                model: None,
                approval: None,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::LIMIT);
        assert!(
            refused.message.contains("max_running"),
            "{}",
            refused.message
        );
        server.shutdown().await;
    });
}

#[test]
fn a_background_goal_runs_to_its_end() {
    block_on!(async {
        let setup = Setup::new(vec![
            answer(None, Some(("update_goal", json!({"status": "complete"})))),
            answer(Some("Done."), None),
        ]);
        let (server, _, _, _dir) = in_repository("task-goal", setup).await;
        let thread = task_thread(&server).await;
        let started = server
            .request(Request::GoalStart {
                thread,
                objective: Some("say done".into()),
                budget: Some(50_000),
                background: true,
            })
            .await
            .unwrap();
        let Reply::Task { task } = started else {
            panic!("a task, not {started:?}");
        };
        assert_eq!(task.budget, Some(50_000));
        settle(&server).await;
        assert_eq!(tasks(&server).await[0].outcome, Some(Outcome::Completed));
        let Reply::Goal { goal: Some(goal) } =
            server.request(Request::GoalGet { thread }).await.unwrap()
        else {
            panic!("a goal");
        };
        assert!(!goal.active());
    });
}

#[test]
fn recovering_tasks_covers_every_project() {
    block_on!(async {
        let log = Shared::default();
        // A server that runs a task in its own project and one in another, then is gone
        // without ending them.
        let mut first = Setup::new(vec![answer(Some("a"), None), answer(Some("b"), None)]);
        first.gate = Some(Arc::new(Notify::new()));
        let (gone, _, repo, _dir) = first.build_in_repository("task-recover", log.clone()).await;
        let other = make_project(
            &gone,
            "other",
            &repository(&repo.with_extension("others"), "b"),
        )
        .await;
        for project in [None, Some(other)] {
            let thread = start_for_tasks(&gone, project).await.unwrap();
            background(&gone, thread, None).await;
        }
        // The next server on the same store finds both open, and records them interrupted.
        let mut next = Setup::new(Vec::new());
        next.seed = 2;
        next.projects = Some(Arc::clone(gone.projects()));
        next.settings.person = gone.person();
        let (server, _) = next.build_on(&repo, log).await;
        assert_eq!(server.recover_tasks().await.unwrap(), 2);
        let listed = tasks(&server).await;
        assert_eq!(listed.len(), 2, "{listed:?}");
        assert!(
            listed
                .iter()
                .all(|task| task.outcome == Some(Outcome::Interrupted)),
            "{listed:?}"
        );
        assert_eq!(server.recover_tasks().await.unwrap(), 0, "once");
    });
}
