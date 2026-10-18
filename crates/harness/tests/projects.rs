//! What a project chooses and turns on for the threads that work in it, read again at each
//! turn: its tools offered after the harness's, its hooks run after the harness's, the approval
//! mode its settings loosen to, its commands out of the sandbox where it turned the sandbox
//! off, and its model, judge, titles and `web_fetch` in place of the harness's. Nothing of it
//! before the person trusts it, all of it from the next turn after, none of it in a thread
//! elsewhere, and nothing of the project's, nor the harness's own hooks, in another person's.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod projects {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use glave_event::{Body, HookPoint, StopReason};
    use glave_harness::{
        AllowAll, ApprovalMode, Brought, Caller, Cancel, Hook, Hooks, Judge, NoOneToAsk, Place,
        Projects, Protections, Settled,
    };
    use glave_ports::{
        BoxFuture, Effect, MemoryLog, MemoryTools, Purpose, Sandbox, SandboxLevel, SpaceId,
        ToolInfo, ToolOutput, ToolSpec,
    };
    use serde_json::json;

    use super::support::*;

    /// A project the test trusts between turns, with a tool, a hook and settings.
    struct Project {
        trusted: AtomicBool,
        protections: Protections,
    }

    impl Project {
        fn new(protections: Protections) -> Arc<Self> {
            Arc::new(Self {
                trusted: AtomicBool::new(false),
                protections,
            })
        }

        fn trust(&self) {
            self.trusted.store(true, Ordering::SeqCst);
        }

        fn on(&self) -> bool {
            self.trusted.load(Ordering::SeqCst)
        }
    }

    impl Projects for Project {
        fn for_folder<'a>(
            &'a self,
            _root: &'a Path,
            _project: Option<&'a Path>,
            _trusting: bool,
        ) -> BoxFuture<'a, Option<String>> {
            Box::pin(async { None })
        }

        fn settled<'a>(&'a self, _project: &'a Path) -> BoxFuture<'a, Settled> {
            let protections = if self.on() {
                self.protections
            } else {
                Protections::default()
            };
            Box::pin(async move {
                Settled {
                    protections,
                    ..Settled::default()
                }
            })
        }

        fn brought<'a>(&'a self, project: &'a Path, _space: SpaceId) -> BoxFuture<'a, Brought> {
            if !self.on() {
                return Box::pin(async { Brought::default() });
            }
            Box::pin(async move {
                Brought {
                    settled: self.settled(project).await,
                    tools: vec![Arc::new(tool("mcp__docs__search", "Found it."))],
                    hooks: Hooks::new(vec![prompt_hook()]).unwrap(),
                    web: None,
                }
            })
        }
    }

    /// A tool `name` that answers every call with `says`.
    fn tool(name: &str, says: &str) -> MemoryTools {
        let info = ToolInfo {
            spec: ToolSpec {
                name: name.into(),
                description: "A tool.".into(),
                parameters: json!({"type": "object"}),
            },
            effect: Effect::Reads,
            parallel: true,
        };
        MemoryTools::new(vec![(info, ToolOutput::text(false, says))])
    }

    /// A hook that runs, and lets through, every prompt.
    fn prompt_hook() -> Hook {
        Hook {
            point: HookPoint::UserPromptSubmit,
            command: "true".into(),
            matcher: None,
            timeout_secs: None,
        }
    }

    fn tool_names(seen: &Seen, request: usize) -> Vec<String> {
        let requests = seen.lock().unwrap();
        requests[request]
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect()
    }

    #[test]
    fn a_project_s_tools_hooks_and_mode_are_on_from_the_turn_after_it_is_trusted() {
        let dir = folder("projects-on");
        let project_dir = std::fs::canonicalize(folder("projects-on-project")).unwrap();
        let project = Project::new(Protections {
            approval: Some(ApprovalMode::Auto),
            unconfined: false,
            ..Protections::default()
        });
        block_on!(async {
            let write = |path: &str| ("write", json!({"path": path, "content": "x"}));
            let answers = vec![
                answer(None, vec![write("before.txt")], StopReason::ToolUse),
                answer(Some("Not allowed."), vec![], StopReason::EndTurn),
                answer(
                    None,
                    vec![("mcp__docs__search", json!({})), write("after.txt")],
                    StopReason::ToolUse,
                ),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, Refuse, MemoryLog::new(), 1, asking()).await;
            let harness = harness.with_projects(Some(project.clone() as Arc<dyn Projects>));
            let place = Place::Folder(project_dir.to_string_lossy().into_owned());
            let mut thread = harness
                .start_thread_in(scope(), &Caller::default(), place)
                .await
                .unwrap();
            assert_eq!(harness.settled(&project_dir).await, Settled::default());
            harness
                .run_turn(&mut thread, "write before", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert!(!tool_names(&seen, 0).contains(&"mcp__docs__search".to_owned()));
            assert!(
                !project_dir.join("before.txt").exists(),
                "ask mode refused it"
            );
            let before = events(harness.log(), thread.id).await.len();

            project.trust();
            assert_eq!(
                harness.settled(&project_dir).await.protections.approval,
                Some(ApprovalMode::Auto)
            );
            harness
                .run_turn(
                    &mut thread,
                    "search, then write",
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            assert!(tool_names(&seen, 2).contains(&"mcp__docs__search".to_owned()));
            assert!(
                project_dir.join("after.txt").exists(),
                "auto, from the project"
            );
            let after = events(harness.log(), thread.id).await;
            let second = &after[before..];
            assert!(
                !second
                    .iter()
                    .any(|event| matches!(event.body, Body::ApprovalRequested(_))),
                "nothing asked in auto"
            );
            assert!(
                second
                    .iter()
                    .any(|event| matches!(&event.body, Body::HookRan(ran) if ran.point == HookPoint::UserPromptSubmit)),
                "the project's hook ran"
            );
            let results: Vec<String> = second
                .iter()
                .filter_map(|event| match &event.body {
                    Body::ToolCallCompleted(done) => Some(format!("{:?}", done.output)),
                    _ => None,
                })
                .collect();
            assert!(results[0].contains("Found it."), "{results:?}");
        });
    }

    #[test]
    fn another_person_s_thread_gets_nothing_the_project_brings() {
        let dir = folder("projects-another");
        let project_dir = std::fs::canonicalize(folder("projects-another-project")).unwrap();
        let project = Project::new(Protections {
            approval: Some(ApprovalMode::Auto),
            unconfined: false,
            ..Protections::default()
        });
        project.trust();
        block_on!(async {
            let write = ("write", json!({"path": "theirs.txt", "content": "x"}));
            let answers = vec![
                answer(None, vec![write], StopReason::ToolUse),
                answer(Some("Not allowed."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, Refuse, MemoryLog::new(), 1, asking()).await;
            // The harness's own hooks are the person's, run with their sign-ins: not for grace.
            let harness = harness
                .with_projects(Some(project.clone() as Arc<dyn Projects>))
                .with_hooks(Hooks::new(vec![prompt_hook()]).unwrap());
            let someone = glave_event::PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [9; 10]);
            let grace = Caller {
                person: Some(someone.unwrap()),
                ..Caller::default()
            };
            let place = Place::Folder(project_dir.to_string_lossy().into_owned());
            let mut thread = harness
                .start_thread_in(scope(), &grace, place)
                .await
                .unwrap();
            harness
                .run_turn_as(
                    &mut thread,
                    "write theirs",
                    &grace,
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            assert!(
                !tool_names(&seen, 0).contains(&"mcp__docs__search".to_owned()),
                "no tools"
            );
            assert_eq!(
                harness.approval_for(&thread),
                ApprovalMode::Ask,
                "no settings"
            );
            assert!(
                !project_dir.join("theirs.txt").exists(),
                "asked, and refused"
            );
            let recorded = events(harness.log(), thread.id).await;
            assert!(
                !recorded
                    .iter()
                    .any(|event| matches!(event.body, Body::HookRan(_))),
                "no hooks, the project's nor the person's own"
            );
        });
    }

    /// A project that chooses its own model, judge and titles and sets up its own
    /// `web_fetch`, all at once, needing no trust in this test.
    struct Choosing;

    impl Projects for Choosing {
        fn for_folder<'a>(
            &'a self,
            _root: &'a Path,
            _project: Option<&'a Path>,
            _trusting: bool,
        ) -> BoxFuture<'a, Option<String>> {
            Box::pin(async { None })
        }

        fn brought<'a>(&'a self, _project: &'a Path, _space: SpaceId) -> BoxFuture<'a, Brought> {
            Box::pin(async {
                Brought {
                    settled: Settled {
                        model: Some("project-model".into()),
                        judge: Some(Judge::Model("project-judge".into())),
                        titles: Some(false),
                        ..Settled::default()
                    },
                    web: Some(Arc::new(tool("web_fetch", "the project's web"))),
                    ..Brought::default()
                }
            })
        }
    }

    /// What the tool calls of `thread` returned, in order.
    fn returned(recorded: &[glave_event::Event]) -> Vec<String> {
        recorded
            .iter()
            .filter_map(|event| match &event.body {
                Body::ToolCallCompleted(done) => Some(format!("{:?}", done.output)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_project_s_model_judge_titles_and_web_reach_its_threads_alone() {
        let dir = folder("projects-choosing");
        let project_dir = std::fs::canonicalize(folder("projects-choosing-project")).unwrap();
        block_on!(async {
            let fetch = || vec![("web_fetch", json!({"url": "https://example.com"}))];
            let push = vec![("bash", json!({"command": "git push origin main"}))];
            let answers = vec![
                answer(None, fetch(), StopReason::ToolUse),
                answer(None, push, StopReason::ToolUse),
                answer(
                    Some(r#"{"decision":"run","reason":"the person asked to push"}"#),
                    vec![],
                    StopReason::EndTurn,
                ),
                answer(Some("Pushed."), vec![], StopReason::EndTurn),
                answer(None, fetch(), StopReason::ToolUse),
                answer(Some("Read."), vec![], StopReason::EndTurn),
            ];
            let mut settings = settings();
            settings.judge = Judge::Main;
            settings.titles = true;
            let (harness, seen) =
                harness_seeing(&dir, answers, NoOneToAsk, MemoryLog::new(), 1, settings).await;
            let harness = harness
                .with_projects(Some(Arc::new(Choosing) as Arc<dyn Projects>))
                .with_web(Some(Arc::new(tool("web_fetch", "the harness's web"))));
            let place = Place::Folder(project_dir.to_string_lossy().into_owned());
            let mut thread = harness
                .start_thread_in(scope(), &Caller::default(), place)
                .await
                .unwrap();
            harness
                .run_turn(
                    &mut thread,
                    "read it, then push",
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            // A thread in no project here: the harness's own of each.
            let mut chat = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut chat, "read it", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let asked: Vec<(Purpose, String)> = seen
                .lock()
                .unwrap()
                .iter()
                .map(|request| (request.purpose, request.model.clone()))
                .collect();
            let main = |model: &str| (Purpose::Main, model.to_owned());
            assert_eq!(
                asked,
                [
                    main("project-model"),
                    main("project-model"),
                    (Purpose::Judge, "project-judge".to_owned()),
                    main("project-model"),
                    main("test-model"),
                    main("test-model"),
                ]
            );
            let ours = returned(&events(harness.log(), thread.id).await);
            assert!(ours[0].contains("the project's web"), "{ours:?}");
            let theirs = returned(&events(harness.log(), chat.id).await);
            assert!(theirs[0].contains("the harness's web"), "{theirs:?}");
            assert!(
                harness.title_case(&thread).is_none(),
                "the project's: no titles"
            );
            assert!(harness.title_case(&chat).is_some(), "the harness's own");
        });
    }

    #[test]
    fn a_project_that_turned_the_sandbox_off_runs_its_commands_without_it_once_trusted() {
        if !glave_computer::sandbox_available() {
            assert!(
                std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
                "GLAVE_REQUIRE_SANDBOX is set but the sandbox is not available here"
            );
            return;
        }
        // Outside the system temp folder, which the sandbox lets commands write.
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/sandbox-tests")
            .join(format!("projects-unconfined-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("project")).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let project_dir = base.join("project");
        let project = Project::new(Protections {
            approval: Some(ApprovalMode::Auto),
            unconfined: true,
            ..Protections::default()
        });
        block_on!(async {
            let sandbox = Sandbox {
                level: SandboxLevel::Project,
                network: false,
                home: Some(base.clone()),
                glave_home: Some(base.join(".glave")),
            };
            let workspace = workspace(&project_dir)
                .await
                .with_sandbox(&sandbox)
                .await
                .unwrap();
            let outside = |name: &str| {
                let command = format!("echo x > {}", base.join(name).display());
                ("bash", json!({ "command": command }))
            };
            let answers = vec![
                answer(None, vec![outside("before.txt")], StopReason::ToolUse),
                answer(Some("Refused."), vec![], StopReason::EndTurn),
                answer(None, vec![outside("after.txt")], StopReason::ToolUse),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ];
            let (harness, _) = harness_with(
                workspace,
                answers,
                AllowAll,
                MemoryLog::new(),
                1,
                settings(),
            );
            let harness = harness.with_projects(Some(project.clone() as Arc<dyn Projects>));
            let place = Place::Folder(project_dir.to_string_lossy().into_owned());
            let mut thread = harness
                .start_thread_in(scope(), &Caller::default(), place)
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "write outside", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert!(!base.join("before.txt").exists(), "the sandbox held");
            project.trust();
            harness
                .run_turn(&mut thread, "write outside", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert!(base.join("after.txt").exists(), "the sandbox is off");
        });
        let _ = std::fs::remove_dir_all(&base);
    }
}
