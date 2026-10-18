//! Turns and releases on a computer that fails the way remote ones do. A computer that dies
//! in the middle of a command gives the model a failed tool result and the turn ends as
//! usual, and the thread resumes. Releasing a worker whose connection died still saves its
//! work when it can be reached again, and when it cannot, what was lost is recorded and
//! returned, never dropped silently.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod faults {
    use std::path::Path;
    use std::process::Command as Process;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::support::*;
    use glave_computer::{LocalComputers, RemoteComputers, serve};
    use glave_event::{Body, Event, Outcome, RemovedReason, StopReason};
    use glave_harness::{AllowAll, Caller, Cancel, Place, Workers, Workspace, wip_branch};
    use glave_ports::computer::{
        Bytes, Command, ComputerError, ComputerId, ComputerInfo, Computers, Environment, FileWrite,
        Finished, Glob, Globbed, Listed, Listing, Output, Save, Saved, Search, Searched, Sources,
        Stat, WorkspaceId, WorkspaceInfo,
    };
    use glave_ports::{BoxFuture, Cancellation, InMemoryLog};
    use serde_json::json;
    use tokio::io::BufReader;
    use tokio::sync::Notify;

    /// A computer whose calls fail as a dead connection's do, `failing` of them (all, at
    /// `u32::MAX`), and on which a command named `hang` starts and never ends.
    struct Faulty {
        inner: Arc<dyn Computers>,
        failing: AtomicU32,
        hung: Arc<Notify>,
    }

    impl Faulty {
        fn new(inner: Arc<dyn Computers>) -> Self {
            Self {
                inner,
                failing: AtomicU32::new(0),
                hung: Arc::new(Notify::new()),
            }
        }

        fn failed(&self) -> Result<(), ComputerError> {
            let left = self.failing.load(Ordering::SeqCst);
            if left == 0 {
                return Ok(());
            }
            if left != u32::MAX {
                self.failing.store(left - 1, Ordering::SeqCst);
            }
            Err(ComputerError::Unreachable(
                "the connection to worker w0 died and could not be opened again: gone".into(),
            ))
        }
    }

    macro_rules! through {
        ($self:ident, $call:expr) => {
            Box::pin(async move {
                $self.failed()?;
                $call.await
            })
        };
    }

    impl Computers for Faulty {
        fn create<'a>(
            &'a self,
            environment: &'a Environment,
        ) -> BoxFuture<'a, Result<ComputerInfo, ComputerError>> {
            through!(self, self.inner.create(environment))
        }

        fn status(
            &self,
            computer: ComputerId,
        ) -> BoxFuture<'_, Result<ComputerInfo, ComputerError>> {
            through!(self, self.inner.status(computer))
        }

        fn destroy(&self, computer: ComputerId) -> BoxFuture<'_, Result<(), ComputerError>> {
            // Removing a worker goes through its backend, not its connection.
            self.inner.destroy(computer)
        }

        fn open<'a>(
            &'a self,
            computer: ComputerId,
            sources: &'a Sources,
        ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
            through!(self, self.inner.open(computer, sources))
        }

        fn close(&self, workspace: WorkspaceId) -> BoxFuture<'_, Result<(), ComputerError>> {
            through!(self, self.inner.close(workspace))
        }

        fn run<'a>(
            &'a self,
            workspace: WorkspaceId,
            command: &'a Command,
            output: &'a dyn Output,
            cancel: &'a dyn Cancellation,
        ) -> BoxFuture<'a, Result<Finished, ComputerError>> {
            Box::pin(async move {
                self.failed()?;
                if command.script == "hang" {
                    self.hung.notify_one();
                    std::future::pending::<()>().await;
                }
                self.inner.run(workspace, command, output, cancel).await
            })
        }

        fn read<'a>(
            &'a self,
            workspace: WorkspaceId,
            path: &'a str,
        ) -> BoxFuture<'a, Result<Bytes, ComputerError>> {
            through!(self, self.inner.read(workspace, path))
        }

        fn write<'a>(
            &'a self,
            workspace: WorkspaceId,
            files: &'a [FileWrite],
        ) -> BoxFuture<'a, Result<(), ComputerError>> {
            through!(self, self.inner.write(workspace, files))
        }

        fn list<'a>(
            &'a self,
            workspace: WorkspaceId,
            listing: &'a Listing,
        ) -> BoxFuture<'a, Result<Listed, ComputerError>> {
            through!(self, self.inner.list(workspace, listing))
        }

        fn stat<'a>(
            &'a self,
            workspace: WorkspaceId,
            path: &'a str,
        ) -> BoxFuture<'a, Result<Stat, ComputerError>> {
            through!(self, self.inner.stat(workspace, path))
        }

        fn glob<'a>(
            &'a self,
            workspace: WorkspaceId,
            glob: &'a Glob,
        ) -> BoxFuture<'a, Result<Globbed, ComputerError>> {
            through!(self, self.inner.glob(workspace, glob))
        }

        fn search<'a>(
            &'a self,
            workspace: WorkspaceId,
            search: &'a Search,
        ) -> BoxFuture<'a, Result<Searched, ComputerError>> {
            through!(self, self.inner.search(workspace, search))
        }

        fn save<'a>(
            &'a self,
            workspace: WorkspaceId,
            save: &'a Save,
        ) -> BoxFuture<'a, Result<Saved, ComputerError>> {
            through!(self, self.inner.save(workspace, save))
        }
    }

    fn local(dir: &Path) -> Arc<LocalComputers> {
        let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
        Arc::new(LocalComputers::new(dir.join("copies"), environment))
    }

    #[test]
    fn a_computer_that_dies_mid_command_fails_the_tool_and_the_turn_ends_cleanly() {
        let dir = folder("faults-dies");
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        block_on!(async {
            let computers = Arc::new(Faulty::new(local(&dir)));
            let hung = Arc::clone(&computers.hung);
            let (client, server) = tokio::io::duplex(64 * 1024);
            let (server_read, server_write) = tokio::io::split(server);
            let serving = tokio::spawn(serve(
                computers,
                BufReader::new(server_read),
                server_write,
                None,
            ));
            let (client_read, client_write) = tokio::io::split(client);
            let remote = RemoteComputers::connect(client_read, client_write);
            let made = remote.create(&Environment::host()).await.unwrap();
            let path = dir.to_string_lossy().into_owned();
            let info = remote
                .open(made.id, &Sources::Folder { path })
                .await
                .unwrap();
            let workspace = Workspace::new(Arc::new(remote), info);
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({"command": "hang"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("The computer is gone."), vec![], StopReason::EndTurn),
                answer(Some("Still here."), vec![], StopReason::EndTurn),
            ];
            let log = InMemoryLog::new();
            let (harness, seen) = harness_with(workspace, answers, AllowAll, log, 1, settings());
            let mut thread = harness.start_thread(scope()).await.unwrap();
            // The computer's side ends once the command has started, as a killed process's
            // would.
            let kill = async {
                hung.notified().await;
                serving.abort();
            };
            let cancel = Cancel::new();
            let mut on_event = |_| {};
            let (turn, ()) = tokio::join!(
                harness.run_turn(&mut thread, "run it", &cancel, &mut on_event),
                kill
            );
            turn.unwrap();

            let recorded = events(harness.log(), thread.id).await;
            let failed = recorded.iter().find_map(|event: &Event| match &event.body {
                Body::ToolCallCompleted(done) => Some(done.clone()),
                _ => None,
            });
            assert!(failed.unwrap().is_error, "the model sees the tool failed");
            let ended = recorded.iter().rev().find_map(|event| match &event.body {
                Body::TurnEnded(ended) => Some(ended.outcome),
                _ => None,
            });
            assert_eq!(ended, Some(Outcome::Completed));
            let sent = format!("{:?}", seen.lock().unwrap().last().unwrap().messages);
            assert!(sent.contains("may or may not have done it"), "{sent}");

            let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            harness
                .run_turn(&mut resumed, "and now?", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let after = events(harness.log(), thread.id).await;
            let endings = after
                .iter()
                .filter(|event| matches!(event.body, Body::TurnEnded(_)))
                .count();
            assert_eq!(endings, 2);
        });
    }

    /// A bare repository with one commit on `main`, as a worker clones it.
    fn origin(dir: &Path) -> std::path::PathBuf {
        let origin = dir.join("origin.git");
        let script = format!(
            "git init -q --bare {o} && git init -q src && cd src && echo hi > a.txt && \
             git add a.txt && git -c user.name=t -c user.email=t@example.com commit -q -m one \
             && git push -q {o} HEAD:refs/heads/main",
            o = origin.display()
        );
        let made = Process::new("bash")
            .args(["-c", &script])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(made.success());
        origin
    }

    fn branches(origin: &Path) -> String {
        let listed = Process::new("git")
            .args(["for-each-ref", "--format=%(refname:short)"])
            .current_dir(origin)
            .output()
            .unwrap();
        String::from_utf8_lossy(&listed.stdout).into_owned()
    }

    /// A thread on a worker that wrote a file and handed one back, its computers `faulty`.
    async fn worked(
        dir: &Path,
        origin: &Path,
        faulty: &Arc<Faulty>,
    ) -> (
        glave_harness::Harness<Script, InMemoryLog, glave_ports::InMemoryBlobs, AllowAll>,
        glave_harness::Thread,
    ) {
        let answers = vec![
            answer(
                None,
                vec![
                    ("write", json!({"path": "notes.txt", "content": "work\n"})),
                    (
                        "write",
                        json!({"path": ".glave/out/report.md", "content": "done\n"}),
                    ),
                ],
                StopReason::ToolUse,
            ),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let (harness, _) = harness_with(
            workspace(dir).await,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let harness = harness.with_workers(Workers {
            computers: Arc::clone(faulty) as Arc<dyn Computers>,
            sources: Sources::Git {
                url: origin.to_string_lossy().into_owned(),
                at: Some("main".into()),
            },
            attachments: Vec::new(),
        });
        let place = Place::Worker(Environment::host());
        let mut thread = harness
            .start_thread_in(scope(), &Caller::default(), place)
            .await
            .unwrap();
        harness
            .run_turn(&mut thread, "do it", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        (harness, thread)
    }

    #[test]
    fn a_worker_reached_again_after_its_connection_died_is_still_saved() {
        let dir = folder("faults-release-again");
        let origin = origin(&dir);
        block_on!(async {
            let faulty = Arc::new(Faulty::new(local(&dir)));
            let (harness, mut thread) = worked(&dir, &origin, &faulty).await;
            // The call that finds the connection dead fails; the next one is reached again.
            faulty.failing.store(1, Ordering::SeqCst);
            let released = harness.release_worker(&mut thread).await.unwrap().unwrap();
            let wip = wip_branch(thread.id);
            assert_eq!(released.branch.as_deref(), Some(wip.as_str()));
            assert!(branches(&origin).contains(&wip));
            assert_eq!(
                released.files,
                vec![("report.md".to_owned(), Bytes::new(b"done\n"))],
                "reading handed-back files is tried again: it changes nothing"
            );
            assert_eq!(released.lost, None);
        });
    }

    #[test]
    fn a_worker_that_cannot_be_reached_again_records_and_returns_what_was_lost() {
        let dir = folder("faults-release-lost");
        let origin = origin(&dir);
        block_on!(async {
            let faulty = Arc::new(Faulty::new(local(&dir)));
            let (harness, mut thread) = worked(&dir, &origin, &faulty).await;
            faulty.failing.store(u32::MAX, Ordering::SeqCst);
            let released = harness.release_worker(&mut thread).await.unwrap().unwrap();
            assert_eq!(released.branch, None);
            let lost = released.lost.unwrap();
            assert!(lost.contains("could not be pushed to glave/wip/"), "{lost}");
            assert!(lost.contains("could not be opened again"), "{lost}");
            assert!(!branches(&origin).contains("glave/wip"));
            let recorded = events(harness.log(), thread.id).await;
            let removed = recorded.iter().find_map(|event| match &event.body {
                Body::WorkspaceRemoved(removed) => Some(removed.clone()),
                _ => None,
            });
            let removed = removed.unwrap();
            assert_eq!(removed.reason, RemovedReason::Released);
            assert_eq!(
                removed.lost.as_deref(),
                Some(lost.as_str()),
                "recorded, not only said"
            );
        });
    }
}
