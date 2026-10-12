//! Records written to a thread from outside the server that holds it, as `glave threads` and
//! worktree cleanup write them straight to the log: glave's own records are taken in, so the
//! next turn follows them and the model is sent the same; an archive or a delete stops the
//! next turn; anything else stays a conflict.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod outside {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;

    use glave_event::{
        At, Body, EventId, Header, MessageAdded, Opened, Part, RemovedReason, Role, Seq, ThreadId,
        WorkspaceRemoved, decode, encode,
    };
    use glave_harness::{ApprovalMode, Change, Settings, change_standing, record_removed};
    use glave_ports::EventLog;
    use glave_session::Answering;
    use glave_session::protocol::{Reply, Request, code};
    use glave_store_sqlite::{SqliteLog, Store};
    use jiff::Timestamp;
    use serde_json::json;

    use super::support::*;

    /// A store of its own in `name`, outside any workspace, so its files are never listed.
    async fn store(name: &str) -> Store {
        let database = folder(name).join("glave.db");
        let root = glave_keys::KeyFileRoot::open_or_create(database.with_extension("key")).unwrap();
        Store::open(&database, glave_keys::KeySettings::new(Arc::new(root)))
            .await
            .unwrap()
    }

    fn settings() -> Settings {
        let mut settings = Settings::new("test-model", "script");
        settings.approval = ApprovalMode::Ask;
        settings
    }

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + second).unwrap()
    }

    /// Records, from outside, that the thread's worktree at `root` was removed.
    async fn removed(log: &SqliteLog, thread: ThreadId, root: &Path) {
        let removed = WorkspaceRemoved {
            root: root.to_string_lossy().into_owned(),
            branch: None,
            reason: RemovedReason::Pruned,
            trash: None,
            lost: None,
        };
        record_removed(log, &scope(), thread, at(1), [1; 10], &removed)
            .await
            .unwrap();
    }

    fn call_ids(events: Vec<glave_event::RawEvent>) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|raw| match decode(raw).unwrap().body {
                Body::ToolCallRequested(call) => Some(call.call_id.as_str().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn glaves_own_records_are_taken_in_and_the_model_is_sent_the_same() {
        let dir = folder("outside-same");
        block_on!(async {
            let run = |outside: bool| {
                let dir = dir.clone();
                async move {
                    let answers = vec![
                        answer(None, Some(("glob", json!({"pattern": "*"})))),
                        answer(Some("One."), None),
                        answer(None, Some(("glob", json!({"pattern": "*"})))),
                        answer(Some("Two."), None),
                    ];
                    let store = store(if outside { "outside-a" } else { "outside-b" }).await;
                    let (server, seen) = build(
                        &dir,
                        answers,
                        None,
                        store.log(),
                        Answering::NoOne,
                        1,
                        settings(),
                        None,
                    )
                    .await;
                    let thread = start(&server, false).await;
                    turn(&server, thread).await.unwrap();
                    if outside {
                        // While the server holds the thread: a cleanup, then an archive and
                        // its undoing, each appended by another glave.
                        let log = store.log();
                        removed(&log, thread, &dir).await;
                        for (second, change) in [(2, Change::Archive), (3, Change::Unarchive)] {
                            change_standing(&log, &scope(), thread, at(second), [2; 10], change)
                                .await
                                .unwrap();
                        }
                    }
                    let next = turn(&server, thread).await;
                    assert!(matches!(next, Ok(Reply::Turn { .. })), "{next:?}");
                    let ids = call_ids(events(&server, thread).await);
                    let seen = seen.lock().unwrap().clone();
                    (seen, ids)
                }
            };
            let plain = run(false).await;
            let followed = run(true).await;
            assert_eq!(plain.0.len(), 4);
            assert_eq!(plain.1.len(), 2);
            assert_eq!(plain, followed);
        });
    }

    #[test]
    fn a_message_from_outside_is_still_a_conflict() {
        let dir = folder("outside-message");
        block_on!(async {
            let store = store("outside-message-db").await;
            let answers = vec![answer(Some("One."), None), answer(Some("Two."), None)];
            let server = server_with(
                &dir,
                answers,
                None,
                store.log(),
                Answering::NoOne,
                1,
                settings(),
            )
            .await;
            let thread = start(&server, false).await;
            turn(&server, thread).await.unwrap();
            let next = u64::try_from(events(&server, thread).await.len()).unwrap() + 1;
            let header = Header {
                id: EventId::new(at(1), [3; 10]).unwrap(),
                thread,
                seq: Seq::new(next).unwrap(),
                at: At::from(at(1)),
            };
            let message = MessageAdded {
                role: Role::User,
                parts: vec![Part::Text {
                    text: "from another harness".into(),
                }],
                author: None,
            };
            store
                .log()
                .append(&scope(), &[encode(header, &message).unwrap()])
                .await
                .unwrap();

            let refused = turn(&server, thread).await.unwrap_err();
            assert_eq!(refused.code, code::INTERNAL);
            assert!(refused.message.contains("already has event"), "{refused:?}");
        });
    }

    #[test]
    fn an_archive_from_outside_stops_the_next_turn_until_unarchived() {
        let dir = folder("outside-archive");
        block_on!(async {
            let store = store("outside-archive-db").await;
            let answers = vec![answer(Some("One."), None), answer(Some("Two."), None)];
            let server = server_with(
                &dir,
                answers,
                None,
                store.log(),
                Answering::NoOne,
                1,
                settings(),
            )
            .await;
            let thread = start(&server, false).await;
            turn(&server, thread).await.unwrap();
            change_standing(
                &store.log(),
                &scope(),
                thread,
                at(1),
                [4; 10],
                Change::Archive,
            )
            .await
            .unwrap();

            let refused = turn(&server, thread).await.unwrap_err();
            assert_eq!(refused.code, code::ARCHIVED);
            // The refused turn wrote nothing.
            let started =
                |kinds: Vec<String>| kinds.iter().filter(|k| *k == "turn.started").count();
            assert_eq!(started(kinds(&server, thread).await), 1);

            server
                .request(Request::ThreadUnarchive { thread })
                .await
                .unwrap();
            let next = turn(&server, thread).await;
            assert!(matches!(next, Ok(Reply::Turn { .. })), "{next:?}");
            assert_eq!(started(kinds(&server, thread).await), 2);
        });
    }

    #[test]
    fn a_delete_from_outside_makes_the_thread_gone() {
        let dir = folder("outside-delete");
        block_on!(async {
            let store = store("outside-delete-db").await;
            let answers = vec![answer(Some("One."), None)];
            let server = server_with(
                &dir,
                answers,
                None,
                store.log(),
                Answering::NoOne,
                1,
                settings(),
            )
            .await;
            let thread = start(&server, false).await;
            turn(&server, thread).await.unwrap();
            change_standing(
                &store.log(),
                &scope(),
                thread,
                at(1),
                [5; 10],
                Change::Delete,
            )
            .await
            .unwrap();

            assert_eq!(
                turn(&server, thread).await.unwrap_err().code,
                code::NO_THREAD
            );
            let got = server.request(Request::ThreadGet { thread }).await;
            assert_eq!(got.unwrap_err().code, code::NO_THREAD);
        });
    }

    fn sh(at: &Path, script: &str) {
        let out = Command::new("bash")
            .args(["-c", script])
            .current_dir(at)
            .output()
            .unwrap();
        assert!(out.status.success(), "{script}: {out:?}");
    }

    /// Where the thread works, from its `workspace.bound`.
    fn worktree_root(events: Vec<glave_event::RawEvent>) -> PathBuf {
        events
            .into_iter()
            .find_map(|raw| match decode(raw).unwrap().body {
                Body::WorkspaceBound(bound) if matches!(bound.opened, Opened::Worktree { .. }) => {
                    Some(PathBuf::from(bound.root))
                }
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn a_worktree_removed_from_outside_is_opened_again_for_the_next_turn() {
        let dir = folder("outside-worktree");
        block_on!(async {
            sh(
                &dir,
                "git init -q --initial-branch=main app && cd app && echo hi > a.txt && \
                 git add a.txt && git -c user.name=t -c user.email=t@example.com commit -q -m one",
            );
            let repo = dir.join("app");
            let store = store("outside-worktree-db").await;
            let answers = vec![
                answer(Some("One."), None),
                answer(None, Some(("glob", json!({"pattern": "*"})))),
                answer(Some("Two."), None),
            ];
            let (server, _) = build(
                &repo,
                answers,
                None,
                store.log(),
                Answering::NoOne,
                1,
                settings(),
                Some(dir.join("worktrees")),
            )
            .await;
            let thread = start(&server, true).await;
            turn(&server, thread).await.unwrap();
            let root = worktree_root(events(&server, thread).await);
            assert!(root.join("a.txt").exists());

            // A cleanup in another glave removes it and records so.
            let removing = format!("git worktree remove --force '{}'", root.display());
            sh(&repo, &removing);
            assert!(!root.exists());
            removed(&store.log(), thread, &root).await;

            let next = turn(&server, thread).await;
            assert!(matches!(next, Ok(Reply::Turn { .. })), "{next:?}");
            assert!(root.join("a.txt").exists());
            let kinds = kinds(&server, thread).await;
            assert!(kinds.contains(&"workspace.removed".to_owned()));
        });
    }
}
