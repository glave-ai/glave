//! What a project turns on for the threads that work in it, read again at each turn: its
//! tools offered after the harness's, its hooks run after the harness's, the approval mode its
//! settings loosen to, and its commands out of the sandbox where it turned the sandbox off.
//! Nothing of it before the person trusts it, and all of it from the next turn after.

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
        AllowAll, ApprovalMode, Brought, Caller, Cancel, Hook, Hooks, Loosened, Place, Projects,
    };
    use glave_ports::{
        BoxFuture, Effect, MemoryLog, MemoryTools, Sandbox, SandboxLevel, ToolInfo, ToolOutput,
        ToolSpec,
    };
    use serde_json::json;

    use super::support::*;

    /// A project the test trusts between turns, with a tool, a hook and settings.
    struct Project {
        trusted: AtomicBool,
        loosened: Loosened,
    }

    impl Project {
        fn new(loosened: Loosened) -> Arc<Self> {
            Arc::new(Self {
                trusted: AtomicBool::new(false),
                loosened,
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
        ) -> BoxFuture<'a, Option<String>> {
            Box::pin(async { None })
        }

        fn loosened<'a>(&'a self, _project: &'a Path) -> BoxFuture<'a, Loosened> {
            let loosened = if self.on() {
                self.loosened
            } else {
                Loosened::default()
            };
            Box::pin(async move { loosened })
        }

        fn brought<'a>(&'a self, _project: &'a Path) -> BoxFuture<'a, Brought> {
            if !self.on() {
                return Box::pin(async { Brought::default() });
            }
            let info = ToolInfo {
                spec: ToolSpec {
                    name: "mcp__docs__search".into(),
                    description: "Searches the docs.".into(),
                    parameters: json!({"type": "object"}),
                },
                effect: Effect::Reads,
                parallel: true,
            };
            let docs = MemoryTools::new(vec![(info, ToolOutput::text(false, "Found it."))]);
            let hook = Hook {
                point: HookPoint::UserPromptSubmit,
                command: "true".into(),
                matcher: None,
                timeout_secs: None,
            };
            let brought = Brought {
                tools: vec![Arc::new(docs)],
                hooks: Hooks::new(vec![hook]).unwrap(),
                loosened: self.loosened,
            };
            Box::pin(async move { brought })
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
        let project = Project::new(Loosened {
            approval: Some(ApprovalMode::Auto),
            unconfined: false,
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
            assert_eq!(harness.loosened(&project_dir).await, Loosened::default());
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
                harness.loosened(&project_dir).await.approval,
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
    fn another_person_s_thread_gets_the_project_s_tools_but_not_its_settings() {
        let dir = folder("projects-another");
        let project_dir = std::fs::canonicalize(folder("projects-another-project")).unwrap();
        let project = Project::new(Loosened {
            approval: Some(ApprovalMode::Auto),
            unconfined: false,
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
            let harness = harness.with_projects(Some(project.clone() as Arc<dyn Projects>));
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
            assert!(tool_names(&seen, 0).contains(&"mcp__docs__search".to_owned()));
            assert_eq!(harness.approval_for(&thread), ApprovalMode::Ask);
            assert!(
                !project_dir.join("theirs.txt").exists(),
                "asked, and refused"
            );
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
        let project = Project::new(Loosened {
            approval: Some(ApprovalMode::Auto),
            unconfined: true,
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
