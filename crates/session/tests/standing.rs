//! Archiving and deleting threads, and the worktree cleanup that goes with them: lists hide
//! what is archived or deleted, a change is the thread's next event, a safe worktree is removed
//! and an unsafe one held, cleanup in a server never fights the threads it holds, and none of
//! it changes what the model is sent.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod standing {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;

    use glave_event::{Body, Opened, RemovedReason, ThreadId, decode};
    use glave_harness::{ApprovalMode, Settings};
    use glave_ports::{EventLog, MemoryLog, ModelProvider};
    use glave_session::protocol::{Meta, Reply, Request, Show, code};
    use glave_session::{Answering, Server};
    use serde_json::json;

    use super::support::*;

    async fn start<P, L, B>(server: &Server<P, L, B>, worktree: bool) -> ThreadId
    where
        P: ModelProvider + 'static,
        L: EventLog + 'static,
        B: glave_ports::BlobStore + 'static,
    {
        let Reply::Thread { thread } = server
            .request(Request::ThreadStart {
                worktree: worktree.then_some(true),
            })
            .await
            .unwrap()
        else {
            panic!("a thread")
        };
        thread.id
    }

    async fn listed<P, L, B>(server: &Server<P, L, B>, show: Show) -> Vec<ThreadId>
    where
        P: ModelProvider + 'static,
        L: EventLog + 'static,
        B: glave_ports::BlobStore + 'static,
    {
        let request = Request::ThreadList {
            limit: None,
            show: Some(show),
        };
        let Reply::Threads { threads } = server.request(request).await.unwrap() else {
            panic!("threads")
        };
        threads.into_iter().map(|thread| thread.id).collect()
    }

    async fn turn<P, L, B>(
        server: &Server<P, L, B>,
        thread: ThreadId,
    ) -> Result<Reply, glave_session::protocol::ApiError>
    where
        P: ModelProvider + 'static,
        L: EventLog + 'static,
        B: glave_ports::BlobStore + 'static,
    {
        server
            .request(Request::TurnStart {
                thread,
                prompt: "go".into(),
                meta: Meta::default(),
            })
            .await
    }

    async fn kinds<P, L, B>(server: &Server<P, L, B>, thread: ThreadId) -> Vec<String>
    where
        P: ModelProvider + 'static,
        L: EventLog + 'static,
        B: glave_ports::BlobStore + 'static,
    {
        let page = Request::ThreadEvents {
            thread,
            after: None,
            limit: None,
        };
        let Reply::Events { events, .. } = server.request(page).await.unwrap() else {
            panic!("events")
        };
        let seqs: Vec<u64> = events.iter().map(|event| event.seq.get()).collect();
        let want: Vec<u64> = (1..=u64::try_from(events.len()).unwrap()).collect();
        assert_eq!(seqs, want, "the thread's numbering has a gap or a repeat");
        events
            .iter()
            .map(|event| event.kind.as_str().to_owned())
            .collect()
    }

    #[test]
    fn archiving_hides_a_thread_and_unarchiving_brings_it_back() {
        let dir = folder("archive-list");
        block_on(async {
            let answers = vec![answer(Some("Hi."), None)];
            let server = server(&dir, answers, Answering::NoOne).await;
            let (a, b) = (start(&server, false).await, start(&server, false).await);
            assert_eq!(listed(&server, Show::Active).await, [b, a]);

            let archived = server
                .request(Request::ThreadArchive { thread: a })
                .await
                .unwrap();
            assert!(matches!(archived, Reply::Cleaned { .. }));
            assert_eq!(listed(&server, Show::Active).await, [b]);
            assert_eq!(listed(&server, Show::Archived).await, [a]);
            // Kept in the log: still readable, but no turn runs in it.
            assert!(
                kinds(&server, a)
                    .await
                    .contains(&"thread.archived".to_owned())
            );
            let refused = turn(&server, a).await.unwrap_err();
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
            assert!(matches!(turn(&server, a).await, Ok(Reply::Turn { .. })));
        });
    }

    #[test]
    fn deleting_hides_a_thread_for_good_and_lists_it_for_erasure() {
        let dir = folder("delete");
        block_on(async {
            let database = dir.join("glave.db");
            let root =
                glave_keys::KeyFileRoot::open_or_create(database.with_extension("key")).unwrap();
            let keys = glave_keys::KeySettings::new(Arc::new(root));
            let store = glave_store_sqlite::Store::open(&database, keys.clone())
                .await
                .unwrap();
            let first = server_on(&dir, Vec::new(), None, store.log(), Answering::NoOne, 1).await;
            let (keep, gone) = (start(&first, false).await, start(&first, false).await);
            first
                .request(Request::ThreadDelete { thread: gone })
                .await
                .unwrap();
            first.shutdown().await;

            // A fresh server, from the log alone, sees the same.
            let again = glave_store_sqlite::Store::open(&database, keys)
                .await
                .unwrap();
            let second = server_on(&dir, Vec::new(), None, again.log(), Answering::NoOne, 2).await;
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
            assert_eq!(turn(&second, gone).await.unwrap_err().code, code::NO_THREAD);
        });
    }

    fn sh(at: &Path, script: &str) -> String {
        let out = Command::new("bash")
            .args(["-c", script])
            .current_dir(at)
            .output()
            .unwrap();
        assert!(out.status.success(), "{script}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// A repository with one pushed commit, and a server working in it that keeps worktrees.
    async fn repo_server(
        name: &str,
        answers: Vec<glave_ports::Completion>,
    ) -> (Server<Script, MemoryLog, glave_ports::MemoryBlobs>, PathBuf) {
        let dir = folder(name);
        sh(
            &dir,
            "git init -q --bare --initial-branch=main origin.git && \
             git init -q --initial-branch=main app && cd app && echo hi > a.txt && \
             git add a.txt && git -c user.name=t -c user.email=t@example.com commit -q -m one && \
             git remote add origin \"$(cd .. && pwd)/origin.git\" && git push -q origin main",
        );
        let repo = dir.join("app");
        let mut settings = Settings::new("test-model", "script");
        settings.approval = ApprovalMode::Ask;
        let (server, _) = build(
            &repo,
            answers,
            None,
            MemoryLog::new(),
            Answering::NoOne,
            1,
            settings,
            Some(dir.join("worktrees")),
        )
        .await;
        (server, repo)
    }

    /// Where the thread works, from its `workspace.bound`.
    async fn worktree_root<P, L, B>(server: &Server<P, L, B>, thread: ThreadId) -> PathBuf
    where
        P: ModelProvider + 'static,
        L: EventLog + 'static,
        B: glave_ports::BlobStore + 'static,
    {
        let page = Request::ThreadEvents {
            thread,
            after: None,
            limit: None,
        };
        let Reply::Events { events, .. } = server.request(page).await.unwrap() else {
            panic!("events")
        };
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

    async fn removals<P, L, B>(
        server: &Server<P, L, B>,
        thread: ThreadId,
    ) -> Vec<glave_event::WorkspaceRemoved>
    where
        P: ModelProvider + 'static,
        L: EventLog + 'static,
        B: glave_ports::BlobStore + 'static,
    {
        let page = Request::ThreadEvents {
            thread,
            after: None,
            limit: None,
        };
        let Reply::Events { events, .. } = server.request(page).await.unwrap() else {
            panic!("events")
        };
        events
            .into_iter()
            .filter_map(|raw| match decode(raw).unwrap().body {
                Body::WorkspaceRemoved(removed) => Some(removed),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn archiving_removes_a_safe_worktree_and_records_it() {
        block_on(async {
            let (server, repo) = repo_server("archive-safe", Vec::new()).await;
            let thread = start(&server, true).await;
            let root = worktree_root(&server, thread).await;
            assert!(root.exists());

            let Reply::Cleaned { notes } = server
                .request(Request::ThreadArchive { thread })
                .await
                .unwrap()
            else {
                panic!("notes")
            };
            assert!(notes[0].starts_with("removed"), "{notes:?}");
            assert!(!root.exists());
            let removed = removals(&server, thread).await;
            assert_eq!(removed.len(), 1);
            assert_eq!(removed[0].reason, RemovedReason::Archived);
            // Archived, then removed, with no gap in the thread's numbering.
            let all = kinds(&server, thread).await;
            assert_eq!(
                all[all.len() - 2..],
                ["thread.archived", "workspace.removed"]
            );
            // Its branch stays.
            assert!(sh(&repo, "git branch --list 'glave/*'").contains("glave/"));
        });
    }

    #[test]
    fn archiving_holds_a_worktree_with_unpushed_work() {
        block_on(async {
            let (server, _repo) = repo_server("archive-unsafe", Vec::new()).await;
            let thread = start(&server, true).await;
            let root = worktree_root(&server, thread).await;
            sh(
                &root,
                "echo b > b.txt && git add b.txt && \
                 git -c user.name=t -c user.email=t@example.com commit -q -m two",
            );

            let Reply::Cleaned { notes } = server
                .request(Request::ThreadArchive { thread })
                .await
                .unwrap()
            else {
                panic!("notes")
            };
            assert!(notes[0].contains("1 commit not pushed"), "{notes:?}");
            assert!(root.exists());
            assert_eq!(removals(&server, thread).await, vec![]);
            // Held, and listed as held.
            let Reply::Worktrees { worktrees } =
                server.request(Request::WorktreesList {}).await.unwrap()
            else {
                panic!("worktrees")
            };
            assert_eq!(worktrees[0].held.as_deref(), Some("archived"));
        });
    }

    #[test]
    fn cleanup_in_a_server_records_without_fighting_the_thread_it_holds() {
        block_on(async {
            let answers = vec![answer(Some("One."), None), answer(Some("Two."), None)];
            let (server, _repo) = repo_server("prune-open", answers).await;
            let thread = start(&server, true).await;
            turn(&server, thread).await.unwrap();
            let root = worktree_root(&server, thread).await;

            // The thread is loaded in this server; the cleanup appends through it.
            let Reply::Cleaned { notes } = server
                .request(Request::WorktreesPrune { force: Vec::new() })
                .await
                .unwrap()
            else {
                panic!("notes")
            };
            assert!(notes[0].starts_with("removed"), "{notes:?}");
            assert!(!root.exists());
            assert_eq!(
                removals(&server, thread).await[0].reason,
                RemovedReason::Pruned
            );

            // Its next turn appends after the removal instead of colliding with it.
            assert!(matches!(
                turn(&server, thread).await,
                Ok(Reply::Turn { .. })
            ));
            kinds(&server, thread).await;
        });
    }

    #[test]
    fn a_peer_on_a_stream_archives_and_leaves_the_server_running() {
        let dir = folder("peer");
        block_on(async {
            let server = server(&dir, vec![answer(Some("Hi."), None)], Answering::NoOne).await;
            let thread = start(&server, false).await;
            let (client, pipe) = tokio::io::duplex(64 * 1024);
            let (pipe_in, pipe_out) = tokio::io::split(pipe);
            let peer = tokio::spawn(glave_session::rpc::serve_peer(
                server.clone(),
                tokio::io::BufReader::new(pipe_in),
                pipe_out,
            ));
            let (client_in, mut client_out) = tokio::io::split(client);
            let mut client_in = tokio::io::BufReader::new(client_in);
            let reply = glave_session::rpc::call(
                &mut client_in,
                &mut client_out,
                &Request::ThreadArchive { thread },
            )
            .await
            .unwrap();
            assert!(matches!(reply, Reply::Cleaned { .. }));
            drop((client_in, client_out));
            peer.await.unwrap().unwrap();

            // The peer going away did not shut the server down.
            assert_eq!(listed(&server, Show::Active).await, Vec::<ThreadId>::new());
            server
                .request(Request::ThreadUnarchive { thread })
                .await
                .unwrap();
            assert!(matches!(
                turn(&server, thread).await,
                Ok(Reply::Turn { .. })
            ));
        });
    }

    #[test]
    fn archiving_changes_nothing_the_model_is_sent() {
        let dir = folder("same-requests");
        block_on(async {
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
                    let mut settings = Settings::new("test-model", "script");
                    settings.approval = ApprovalMode::Ask;
                    let (server, seen) = build(
                        &dir,
                        answers,
                        None,
                        MemoryLog::new(),
                        Answering::NoOne,
                        1,
                        settings,
                        None,
                    )
                    .await;
                    let thread = start(&server, false).await;
                    turn(&server, thread).await.unwrap();
                    if archive {
                        for request in [
                            Request::ThreadArchive { thread },
                            Request::ThreadUnarchive { thread },
                            Request::WorktreesPrune { force: Vec::new() },
                        ] {
                            server.request(request).await.unwrap();
                        }
                    }
                    turn(&server, thread).await.unwrap();
                    seen.lock().unwrap().clone()
                }
            };
            let plain = run(false).await;
            let changed = run(true).await;
            assert_eq!(plain.len(), 4);
            assert_eq!(plain, changed);
        });
    }
}
