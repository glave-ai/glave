//! Threads on workers, through the computer port: any adapter can be the workers, so this
//! machine stands in for Docker. A worker thread clones the repository, gets its attachments,
//! is told its sources in one paragraph, and on release hands back its files, pushes unpushed
//! work to `glave/wip/<thread>` and is destroyed. With workers configured but unused, the
//! model is sent exactly what it was before.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod workers {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;

    use super::support::*;
    use glave_computer::LocalComputers;
    use glave_event::{Body, ComputerKind, Event, Opened, RemovedReason, StopReason};
    use glave_harness::{AllowAll, Caller, Cancel, HarnessError, Place, Workers, wip_branch};
    use glave_ports::{
        Bytes, Completion, Environment, EventLog as _, FileWrite, MemoryLog, Sources, Standing,
        ThreadQuery,
    };
    use serde_json::json;

    fn environment() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        std::env::vars_os()
            .filter(|(name, _)| name == "PATH" || name == "HOME")
            .collect()
    }

    /// A bare repository with one commit on `main`, as a worker clones it.
    fn origin(dir: &Path) -> PathBuf {
        let origin = dir.join("origin.git");
        let script = format!(
            "git init -q --bare {o} && git init -q src && cd src && echo hi > a.txt && \
             git add a.txt && git -c user.name=t -c user.email=t@example.com commit -q -m one \
             && git push -q {o} HEAD:refs/heads/main",
            o = origin.display()
        );
        let made = Command::new("bash")
            .args(["-c", &script])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(made.success());
        origin
    }

    fn workers(dir: &Path, origin: &Path) -> Workers {
        Workers {
            computers: Arc::new(LocalComputers::new(dir.join("workers"), environment())),
            sources: Sources::Git {
                url: origin.to_string_lossy().into_owned(),
                at: Some("main".into()),
            },
            attachments: vec![FileWrite::new("brief.md", Bytes::new(b"do the thing\n"))],
        }
    }

    fn script() -> Vec<Completion> {
        vec![
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
        ]
    }

    fn branches(origin: &Path) -> String {
        let listed = Command::new("git")
            .args(["for-each-ref", "--format=%(refname:short)"])
            .current_dir(origin)
            .output()
            .unwrap();
        String::from_utf8_lossy(&listed.stdout).into_owned()
    }

    /// A file at a revision of `origin`, if it is there.
    fn shown(origin: &Path, revision: &str) -> Option<String> {
        let shown = Command::new("git")
            .args(["show", revision])
            .current_dir(origin)
            .output()
            .unwrap();
        shown
            .status
            .success()
            .then(|| String::from_utf8_lossy(&shown.stdout).into_owned())
    }

    #[test]
    fn with_workers_configured_but_unused_the_model_is_sent_exactly_what_it_was() {
        let dir = folder("workers-unused");
        let origin = origin(&dir);
        std::fs::write(dir.join("a.txt"), "hi\n").unwrap();
        let read = || {
            vec![
                answer(
                    None,
                    vec![("read", json!({"path": "a.txt"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ]
        };
        block_on!(async {
            let (before, seen_before) = harness_with(
                workspace(&dir).await,
                read(),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let mut thread = before.start_thread(scope()).await.unwrap();
            before
                .run_turn(&mut thread, "read it", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let (now, seen_now) = harness_with(
                workspace(&dir).await,
                read(),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let now = now.with_workers(workers(&dir, &origin));
            let mut thread = now
                .start_thread_in(scope(), &Caller::default(), Place::Here)
                .await
                .unwrap();
            now.run_turn(&mut thread, "read it", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(
                format!("{:?}", seen_now.lock().unwrap()),
                format!("{:?}", seen_before.lock().unwrap()),
                "byte for byte the same requests"
            );
            assert_eq!(now.release_worker(&mut thread).await.unwrap(), None);
        });
    }

    #[test]
    fn a_worker_thread_is_told_its_sources_and_released_with_its_work_kept() {
        let dir = folder("workers-thread");
        let origin = origin(&dir);
        let here = dir.join("here");
        std::fs::create_dir_all(&here).unwrap();
        block_on!(async {
            let (harness, seen) = harness_with(
                workspace(&here).await,
                script(),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let harness = harness.with_workers(workers(&dir, &origin));
            let mut thread = harness
                .start_thread_in(
                    scope(),
                    &Caller::default(),
                    Place::Worker(Environment::host()),
                )
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "do it", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let sent = format!("{:?}", seen.lock().unwrap().first().unwrap().messages);
            assert!(sent.contains("You are working on a cloud worker"), "{sent}");
            assert!(sent.contains(".glave/out/"), "{sent}");
            assert!(
                !here.join("notes.txt").exists(),
                "the folder here is untouched"
            );

            let recorded = events(harness.log(), thread.id).await;
            let bound = recorded.iter().find_map(|event| match &event.body {
                Body::WorkspaceBound(bound) => Some(bound.clone()),
                _ => None,
            });
            let bound = bound.unwrap();
            assert_eq!(bound.computer.kind, ComputerKind::Worker);
            assert!(matches!(&bound.opened, Opened::Clone { at: Some(at), .. } if at == "main"));
            let root = PathBuf::from(&bound.root);
            assert_eq!(
                std::fs::read_to_string(root.join(".glave/in/brief.md")).unwrap(),
                "do the thing\n"
            );

            let released = harness.release_worker(&mut thread).await.unwrap().unwrap();
            let wip = wip_branch(thread.id);
            assert_eq!(released.branch.as_deref(), Some(wip.as_str()));
            assert_eq!(
                released.files,
                vec![("report.md".to_owned(), Bytes::new(b"done\n"))]
            );
            assert!(branches(&origin).contains(&wip), "{}", branches(&origin));
            assert_eq!(
                shown(&origin, &format!("{wip}:notes.txt")).as_deref(),
                Some("work\n")
            );
            let handed = shown(&origin, &format!("{wip}:.glave/out/report.md"));
            assert_eq!(handed, None, "handed-back files are never committed");
            assert!(!root.exists(), "the worker is destroyed");
            let after = events(harness.log(), thread.id).await;
            let removed = after.iter().find_map(|event: &Event| match &event.body {
                Body::WorkspaceRemoved(removed) => Some(removed.clone()),
                _ => None,
            });
            assert_eq!(removed.unwrap().reason, RemovedReason::Released);

            let resumed = harness.resume_thread(scope(), thread.id).await;
            assert!(
                matches!(&resumed, Err(HarnessError::Workspace(error)) if error.message().contains("released")),
                "a released worker is not silently replaced by this folder"
            );
        });
    }

    #[test]
    fn a_worker_with_nothing_unpushed_pushes_nothing() {
        let dir = folder("workers-clean");
        let origin = origin(&dir);
        block_on!(async {
            let (harness, _) = harness_with(
                workspace(&dir).await,
                Vec::new(),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let harness = harness.with_workers(Workers {
                attachments: Vec::new(),
                ..workers(&dir, &origin)
            });
            let mut thread = harness
                .start_thread_in(
                    scope(),
                    &Caller::default(),
                    Place::Worker(Environment::host()),
                )
                .await
                .unwrap();
            let released = harness.release_worker(&mut thread).await.unwrap().unwrap();
            assert_eq!(released.branch, None);
            assert!(!branches(&origin).contains("glave/wip"));
        });
    }

    #[test]
    fn a_released_worker_does_not_shift_the_call_ids_the_model_sees() {
        let dir = folder("workers-numbering");
        let origin = origin(&dir);
        let here = dir.join("here");
        std::fs::create_dir_all(&here).unwrap();
        block_on!(async {
            let mut answers = script();
            answers.push(answer(
                None,
                vec![("read", json!({"path": "a.txt"}))],
                StopReason::ToolUse,
            ));
            answers.push(answer(Some("Read it."), vec![], StopReason::EndTurn));
            let (harness, _) = harness_with(
                workspace(&here).await,
                answers,
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let harness = harness.with_workers(workers(&dir, &origin));
            let mut thread = harness
                .start_thread_in(
                    scope(),
                    &Caller::default(),
                    Place::Worker(Environment::host()),
                )
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "do it", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            harness.release_worker(&mut thread).await.unwrap().unwrap();
            // The next tool call after the release is numbered by the in-memory thread. A
            // fresh resume numbers from the log, where each binding and removal is a record
            // of glave's own that call ids skip; a released worker cannot be resumed, so the
            // expected id is worked out from the log the same way.
            let _ = harness
                .run_turn(&mut thread, "read it", &Cancel::new(), &mut |_| {})
                .await;
            let recorded = events(harness.log(), thread.id).await;
            let mut own = 0_u64;
            let mut expected = Vec::new();
            for event in &recorded {
                match &event.body {
                    Body::WorkspaceBound(_) | Body::WorkspaceRemoved(_) => own += 1,
                    Body::ToolCallRequested(call) => {
                        let seq = event.header.seq.get();
                        expected.push((call.call_id.as_str().to_owned(), seq - own));
                    }
                    _ => {}
                }
            }
            let (last_id, last_expected) = expected.last().unwrap().clone();
            assert_eq!(last_id, format!("call_{last_expected}"), "{expected:?}");
        });
    }

    #[test]
    fn without_workers_a_worker_thread_is_refused_and_nothing_is_recorded() {
        let dir = folder("workers-none");
        block_on!(async {
            let (harness, _) = harness_with(
                workspace(&dir).await,
                Vec::new(),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let refused = harness
                .start_thread_in(
                    scope(),
                    &Caller::default(),
                    Place::Worker(Environment::host()),
                )
                .await;
            assert!(
                matches!(&refused, Err(HarnessError::Workspace(error)) if error.message().contains("[workers]")),
                "{:?}",
                refused.err()
            );
            let query = ThreadQuery {
                standing: Standing::Active,
                limit: 10,
                after: None,
            };
            let listed = harness.log().thread_views(&scope(), &query).await.unwrap();
            assert_eq!(listed, []);
        });
    }
}
