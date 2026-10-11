//! Tasks through the session API against a scripted model: a turn or goal started with
//! `background` runs with no one attached and can be read afterwards; `tasks/list` shows its
//! tokens and cost from the events; `tasks/stop` and archiving stop it; a turn stops at its
//! budget; policy limits refuse loudly; and the model is sent exactly what it is sent when
//! someone follows the turn.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/tasks.rs"]
mod helpers;

#[cfg(test)]
mod tasks {
    use std::sync::Arc;

    use glave_event::Outcome;
    use glave_harness::ApprovalMode;
    use glave_session::Answering;
    use glave_session::protocol::{Notification, Reply, Request, TaskStatus, code};
    use serde_json::json;
    use tokio::sync::Notify;

    use super::helpers::*;
    use super::support::*;

    #[test]
    fn a_background_turn_runs_with_no_one_attached_and_can_be_read_afterwards() {
        let dir = folder("task-runs");
        block_on(async {
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
            let (server, _) = setup.build(&dir).await;
            let thread = start(&server, false).await;
            let mut watching = server.watch(thread);
            let task = background(&server, thread, None).await;
            assert_eq!(task.status, TaskStatus::Running);
            assert_eq!(task.title, "tidy the notes");
            assert_eq!(task.model, "test-model");
            let finished = loop {
                if let Notification::TaskFinished { task, .. } =
                    watching.recv().await.unwrap().as_ref()
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
            assert_eq!(
                std::fs::read_to_string(dir.join("notes.md")).unwrap(),
                "tidy"
            );
            // The thread is idle and takes a plain turn again.
            assert!(matches!(
                server.request(Request::ThreadGet { thread }).await.unwrap(),
                Reply::Thread { thread } if thread.status == glave_session::protocol::Status::Idle
            ));
        });
    }

    #[test]
    fn approvals_in_a_task_are_answered_no_one_there_and_the_model_is_sent_the_same() {
        let dir = folder("task-unattended");
        block_on(async {
            let go = || {
                vec![
                    answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                    answer(Some("Could not run it."), None),
                ]
            };
            // Asks for every command, with a client that could answer.
            let mut setup = Setup::new(go());
            setup.settings.approval = ApprovalMode::Ask;
            let (server, seen) = setup.build(&dir).await;
            let thread = start(&server, false).await;
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
            // The same turn followed by no one, from a server that never has anyone to ask.
            let mut followed = Setup::new(go());
            followed.settings.approval = ApprovalMode::Ask;
            followed.answering = Answering::NoOne;
            let (plain, plain_seen) = followed.build(&dir).await;
            let other = start(&plain, false).await;
            plain
                .request(Request::TurnStart {
                    thread: other,
                    prompt: "tidy the notes".into(),
                    background: false,
                    budget: None,
                    meta: glave_session::protocol::Meta::default(),
                    model: None,
                })
                .await
                .unwrap();
            let strip = |requests: &Arc<std::sync::Mutex<Vec<String>>>| -> Vec<serde_json::Value> {
                requests
                    .lock()
                    .unwrap()
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
        let dir = folder("task-stop");
        block_on(async {
            let mut setup = Setup::new(vec![
                answer(Some("never"), None),
                answer(Some("again"), None),
            ]);
            setup.gate = Some(Arc::new(Notify::new()));
            let (server, _) = setup.build(&dir).await;
            let thread = start(&server, false).await;
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
    fn archiving_a_task_s_thread_stops_the_task() {
        let dir = folder("task-archive");
        block_on(async {
            let mut setup = Setup::new(vec![answer(Some("never"), None)]);
            setup.gate = Some(Arc::new(Notify::new()));
            let (server, _) = setup.build(&dir).await;
            let thread = start(&server, false).await;
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
        let dir = folder("task-budget");
        block_on(async {
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
            let (server, _) = setup.build(&dir).await;
            let thread = start(&server, false).await;
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
                })
                .await;
            assert_eq!(followed.unwrap_err().code, code::INVALID_PARAMS);
        });
    }

    #[test]
    fn policy_limits_how_many_tasks_run_at_once() {
        let dir = folder("task-limit");
        block_on(async {
            let mut setup = Setup::new(vec![answer(Some("a"), None), answer(Some("b"), None)]);
            setup.gate = Some(Arc::new(Notify::new()));
            setup.limits.max_running = Some(1);
            let (server, _) = setup.build(&dir).await;
            let first = start(&server, false).await;
            let second = start(&server, false).await;
            background(&server, first, None).await;
            let refused = server
                .request(Request::TurnStart {
                    thread: second,
                    prompt: "go".into(),
                    background: true,
                    budget: None,
                    meta: glave_session::protocol::Meta::default(),
                    model: None,
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
        let dir = folder("task-goal");
        block_on(async {
            let setup = Setup::new(vec![
                answer(None, Some(("update_goal", json!({"status": "complete"})))),
                answer(Some("Done."), None),
            ]);
            let (server, _) = setup.build(&dir).await;
            let thread = start(&server, false).await;
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
}
