//! Where a thread works: in the folder itself by default, sending the model exactly what it was
//! sent before bindings were recorded; or in a worktree of its own, on its own branch outside
//! the repository, found again on resume, on this machine or on a computer across the wire.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod worktrees {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;

    use super::support::*;
    use glave_computer::{LocalComputers, RemoteComputers, serve_tcp};
    use glave_event::{Body, Event, Opened, StopReason, WorkspaceBound};
    use glave_harness::{AllowAll, Caller, Cancel, HarnessError, Place, Workspace, branch_for};
    use glave_ports::{
        Completion, Computers, Environment, EventLog, MemoryLog, Sources, Standing, ThreadQuery,
    };
    use serde_json::json;

    fn environment() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        std::env::vars_os()
            .filter(|(name, _)| name == "PATH" || name == "HOME")
            .collect()
    }

    /// A repository with one commit, and the folder its worktrees go in, beside it.
    fn repository(name: &str) -> (PathBuf, PathBuf) {
        let dir = folder(name);
        let repo = dir.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        let script = "git init -q --initial-branch=main && echo hi > a.txt && git add a.txt && \
            git -c user.name=t -c user.email=t@example.com commit -q -m one";
        let made = Command::new("bash")
            .args(["-c", script])
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(made.success());
        (repo, dir.join("worktrees"))
    }

    /// `repo` on this machine, keeping worktrees under `base`, as the CLI opens it.
    async fn workspace_keeping(repo: &Path, base: &Path) -> Workspace {
        let (computers, info) = LocalComputers::for_here(environment())
            .with_worktrees(base)
            .open_here(repo)
            .await
            .unwrap();
        Workspace::new(computers, info)
    }

    fn writes(path: &str, content: &str) -> Vec<Completion> {
        vec![
            answer(
                None,
                vec![("write", json!({"path": path, "content": content}))],
                StopReason::ToolUse,
            ),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ]
    }

    fn bound(events: &[Event]) -> Vec<&WorkspaceBound> {
        events
            .iter()
            .filter_map(|event| match &event.body {
                Body::WorkspaceBound(bound) => Some(bound),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn by_default_the_model_is_sent_exactly_what_it_was_before() {
        let dir = folder("worktrees-default");
        std::fs::write(dir.join("a.txt"), "hi\n").unwrap();
        let script = || {
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
                script(),
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
                script(),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let mut thread = now
                .start_thread_in(scope(), &Caller::default(), Place::Here)
                .await
                .unwrap();
            now.run_turn(&mut thread, "read it", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();

            let before = format!("{:?}", seen_before.lock().unwrap());
            let now_sent = format!("{:?}", seen_now.lock().unwrap());
            assert_eq!(now_sent, before, "byte for byte the same requests");
            let events = events(now.log(), thread.id).await;
            let recorded = bound(&events);
            let root = std::fs::canonicalize(&dir).unwrap();
            assert!(
                matches!(&recorded[..], [WorkspaceBound { opened: Opened::Folder { .. }, root: at, .. }]
                    if Path::new(at) == root),
                "{recorded:?}"
            );
            assert_eq!(kinds(&events)[..2], ["thread.started", "workspace.bound"]);
        });
    }

    #[test]
    fn a_worktree_thread_works_on_its_own_branch_and_resumes_there() {
        let (repo, base) = repository("worktrees-own");
        let (elsewhere, _) = repository("worktrees-in-place");
        block_on!(async {
            // The same turn in another copy of the project, worked on in place.
            let (here, seen_here) = harness_with(
                workspace_keeping(&elsewhere, &base).await,
                writes("notes.txt", "one\n"),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let mut thread = here
                .start_thread_in(scope(), &Caller::default(), Place::Here)
                .await
                .unwrap();
            here.run_turn(&mut thread, "add notes", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();

            let answers = [writes("notes.txt", "one\n"), writes("more.txt", "two\n")].concat();
            let (harness, seen) = harness_with(
                workspace_keeping(&repo, &base).await,
                answers,
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let mut thread = harness
                .start_thread_in(scope(), &Caller::default(), Place::Worktree)
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "add notes", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let sent_here = format!("{:?}", seen_here.lock().unwrap());
            let sent = format!("{:?}", seen.lock().unwrap());
            assert_eq!(sent, sent_here, "only where the tools act changes");

            let events = events(harness.log(), thread.id).await;
            let recorded = bound(&events);
            let [bound] = &recorded[..] else {
                panic!("one binding: {recorded:?}")
            };
            let branch = branch_for(thread.id);
            assert!(
                matches!(&bound.opened, Opened::Worktree { branch: on, .. } if *on == branch),
                "{bound:?}"
            );
            let root = PathBuf::from(&bound.root);
            assert!(root.starts_with(std::fs::canonicalize(&base).unwrap()));
            assert_eq!(
                std::fs::read_to_string(root.join("notes.txt")).unwrap(),
                "one\n"
            );
            assert!(
                !repo.join("notes.txt").exists(),
                "the folder itself is untouched"
            );
            let on = Command::new("git")
                .args(["symbolic-ref", "--short", "HEAD"])
                .current_dir(&root)
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&on.stdout).trim(), branch);

            let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            harness
                .run_turn(&mut resumed, "add more", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(root.join("more.txt")).unwrap(),
                "two\n"
            );
            assert!(!repo.join("more.txt").exists());
        });
    }

    #[test]
    fn a_scratch_thread_works_in_its_own_folder_and_resumes_there_never_in_the_project() {
        let (repo, base) = repository("worktrees-scratch");
        let scratch = base.with_file_name("scratch");
        block_on!(async {
            let (computers, info) = LocalComputers::for_here(environment())
                .with_scratch(&scratch)
                .open_here(&repo)
                .await
                .unwrap();
            let answers = [writes("notes.txt", "one\n"), writes("more.txt", "two\n")].concat();
            let (harness, _) = harness_with(
                Workspace::new(computers, info),
                answers,
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let mut thread = harness
                .start_thread_in(scope(), &Caller::default(), Place::Scratch)
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "add notes", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let events = events(harness.log(), thread.id).await;
            let recorded = bound(&events);
            let [bound] = &recorded[..] else {
                panic!("one binding: {recorded:?}")
            };
            assert!(matches!(&bound.opened, Opened::Scratch { .. }), "{bound:?}");
            let root = PathBuf::from(&bound.root);
            assert_eq!(
                root,
                std::fs::canonicalize(&scratch)
                    .unwrap()
                    .join(thread.id.to_string())
            );
            assert!(root.join("notes.txt").exists());
            let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            harness
                .run_turn(&mut resumed, "add more", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert!(root.join("more.txt").exists(), "resumed in its own folder");
            assert!(!repo.join("notes.txt").exists() && !repo.join("more.txt").exists());
        });
    }

    #[test]
    fn threads_started_together_get_their_own_worktrees() {
        let (repo, base) = repository("worktrees-together");
        block_on!(async {
            let (harness, _) = harness_with(
                workspace_keeping(&repo, &base).await,
                Vec::new(),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let caller = Caller::default();
            let (a, b) = tokio::join!(
                harness.start_thread_in(scope(), &caller, Place::Worktree),
                harness.start_thread_in(scope(), &caller, Place::Worktree),
            );
            let (a, b) = (a.unwrap(), b.unwrap());
            let root = |events: Vec<Event>| bound(&events).first().map(|bound| bound.root.clone());
            let a_root = root(events(harness.log(), a.id).await).unwrap();
            let b_root = root(events(harness.log(), b.id).await).unwrap();
            assert_ne!(a_root, b_root);
            assert_ne!(branch_for(a.id), branch_for(b.id));
        });
    }

    #[test]
    fn a_worktree_outside_a_repository_fails_the_start_and_records_nothing() {
        let dir = folder("worktrees-plain");
        let base = dir.with_extension("worktrees");
        block_on!(async {
            let (harness, _) = harness_with(
                workspace_keeping(&dir, &base).await,
                Vec::new(),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let failed = harness
                .start_thread_in(scope(), &Caller::default(), Place::Worktree)
                .await;
            let Err(HarnessError::Workspace(error)) = failed else {
                panic!("refused: {failed:?}")
            };
            assert!(
                error.message().contains("not in a git repository"),
                "{error}"
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

    #[test]
    fn a_computer_across_the_wire_makes_the_worktree_through_the_port() {
        let (repo, base) = repository("worktrees-remote");
        block_on!(async {
            let computers = Arc::new(
                LocalComputers::new(base.with_extension("copies"), environment())
                    .with_worktrees(&base),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(serve_tcp(computers, listener, "token".into()));
            let stream = tokio::net::TcpStream::connect(address).await.unwrap();
            let (read, write) = stream.into_split();
            let remote = RemoteComputers::connect(read, write);
            remote.authenticate("token").await.unwrap();
            let computer = remote.create(&Environment::host()).await.unwrap();
            let sources = Sources::Folder {
                path: repo.to_string_lossy().into_owned(),
            };
            let info = remote.open(computer.id, &sources).await.unwrap();
            let (harness, _) = harness_with(
                Workspace::new(Arc::new(remote), info),
                writes("remote.txt", "far\n"),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let mut thread = harness
                .start_thread_in(scope(), &Caller::default(), Place::Worktree)
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "write", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            server.abort();
            let events = events(harness.log(), thread.id).await;
            let root = PathBuf::from(&bound(&events)[0].root);
            assert!(root.starts_with(std::fs::canonicalize(&base).unwrap()));
            assert_eq!(
                std::fs::read_to_string(root.join("remote.txt")).unwrap(),
                "far\n"
            );
            assert!(!repo.join("remote.txt").exists());
        });
    }

    #[test]
    fn a_thread_on_a_computer_that_dialled_in_is_not_available_yet() {
        let dir = folder("dialled-in");
        block_on!(async {
            let (harness, _) = harness_with(
                workspace(&dir).await,
                Vec::new(),
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let laptop = glave_ports::ComputerId::from_uuid(uuid::Uuid::nil());
            let refused = harness
                .start_thread_in(scope(), &Caller::default(), Place::Computer(laptop))
                .await;
            let Err(HarnessError::Invalid(why)) = refused else {
                panic!("refused, not {refused:?}")
            };
            assert!(why.contains("not available yet"), "{why}");
            let query = ThreadQuery {
                standing: Standing::Active,
                limit: 10,
                after: None,
            };
            let listed = harness.log().thread_views(&scope(), &query).await.unwrap();
            assert!(listed.is_empty(), "nothing is recorded");
        });
    }
}
