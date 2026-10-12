//! Hooks through the harness: each point runs and is recorded, a hook before a tool can stop
//! it with a reason the model reads, a hook never skips an approval, a slow hook is stopped,
//! and hooks that pass leave what the model is sent exactly as it was.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod hooks {
    use std::path::Path;

    use glave_event::{Body, Event, HookPoint, HookRan, HookResult, Part, StopReason};
    use glave_harness::{Cancel, HarnessError, Hook, Hooks, NoOneToAsk};
    use glave_ports::{MemoryLog, Message, ModelRequest};
    use serde_json::{Value, json};

    use super::support::*;

    fn hook(point: HookPoint, matcher: Option<&str>, command: &str) -> Hook {
        Hook {
            point,
            command: command.into(),
            matcher: matcher.map(str::to_owned),
            timeout_secs: None,
        }
    }

    fn ran(events: &[Event]) -> Vec<HookRan> {
        events
            .iter()
            .filter_map(|event| match &event.body {
                Body::HookRan(ran) => Some(ran.clone()),
                _ => None,
            })
            .collect()
    }

    /// The text of the tool results the model was sent last.
    fn tool_results(request: &ModelRequest) -> Vec<String> {
        request
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::Tool { output, .. } => Some(
                    output
                        .iter()
                        .filter_map(|part| match part {
                            Part::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                _ => None,
            })
            .collect()
    }

    /// A turn where the model runs `command` with bash, then is done, under `hooks`.
    async fn bash_turn(
        dir: &Path,
        command: &str,
        hooks: Vec<Hook>,
        settings: glave_harness::Settings,
    ) -> (Vec<ModelRequest>, Vec<Event>) {
        let answers = vec![
            answer(
                None,
                vec![("bash", json!({"command": command}))],
                StopReason::ToolUse,
            ),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let log = MemoryLog::new();
        let (harness, seen) = harness_seeing(dir, answers, Refuse, log, 1, settings).await;
        let harness = harness.with_hooks(Hooks::new(hooks).unwrap());
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "go", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let requests = seen.lock().unwrap().clone();
        (requests, events(harness.log(), thread.id).await)
    }

    #[test]
    fn a_hook_before_a_tool_reads_the_call_and_can_stop_it_with_a_reason() {
        let dir = folder("hooks-before");
        let guard = "cat > seen.json; echo 'no files here, ask first' >&2; exit 2";
        let hooks = vec![hook(HookPoint::PreToolUse, Some("bash"), guard)];
        block_on!(async {
            let (requests, events) = bash_turn(&dir, "touch made.txt", hooks, settings()).await;
            assert!(!dir.join("made.txt").exists(), "the call did not run");
            let input: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("seen.json")).unwrap())
                    .unwrap();
            assert_eq!(input["hook_event_name"], "PreToolUse");
            assert_eq!(input["tool_name"], "bash");
            assert_eq!(input["tool_input"]["command"], "touch made.txt");
            assert_eq!(input["permission_mode"], "auto");
            let results = tool_results(requests.last().unwrap());
            assert!(
                results[0].contains("no files here, ask first"),
                "{results:?}"
            );
            assert!(results[0].starts_with("Not run: a hook"), "{results:?}");
            let ran = ran(&events);
            assert_eq!(ran.len(), 1);
            assert_eq!(ran[0].result, HookResult::Blocked);
            assert_eq!(ran[0].exit_code, Some(2));
            assert!(ran[0].call_id.is_some());
            assert!(kinds(&events).contains(&"hook.ran"));
        });
    }

    #[test]
    fn a_hook_that_allows_never_skips_an_approval() {
        let dir = folder("hooks-allow");
        let allow = r#"echo '{"hookSpecificOutput":{"permissionDecision":"allow"}}'"#;
        let hooks = vec![hook(HookPoint::PreToolUse, None, allow)];
        block_on!(async {
            let (_, events) = bash_turn(&dir, "touch made.txt", hooks, asking()).await;
            assert!(!dir.join("made.txt").exists(), "the person said no");
            assert_eq!(ran(&events)[0].result, HookResult::Passed);
            let kinds = kinds(&events);
            assert!(kinds.contains(&"approval.requested"), "{kinds:?}");
        });
    }

    #[test]
    fn a_hook_after_a_tool_can_tell_the_model_something() {
        let dir = folder("hooks-after");
        let lint = "cat > /dev/null; echo 'made.txt is not formatted'; exit 2";
        let hooks = vec![hook(HookPoint::PostToolUse, Some("bash|edit"), lint)];
        block_on!(async {
            let (requests, events) = bash_turn(&dir, "touch made.txt", hooks, settings()).await;
            assert!(dir.join("made.txt").exists(), "the call ran");
            let results = tool_results(requests.last().unwrap());
            assert!(
                results[0]
                    .contains("[A hook that runs after bash says: made.txt is not formatted]"),
                "{results:?}"
            );
            assert_eq!(ran(&events)[0].point, HookPoint::PostToolUse);
        });
    }

    #[test]
    fn a_slow_or_failing_hook_is_recorded_and_the_call_goes_on() {
        let dir = folder("hooks-slow");
        let mut slow = hook(HookPoint::PreToolUse, None, "sleep 30");
        slow.timeout_secs = Some(1);
        let broken = hook(HookPoint::PreToolUse, None, "exit 7");
        block_on!(async {
            let (_, events) =
                bash_turn(&dir, "touch made.txt", vec![slow, broken], settings()).await;
            assert!(
                dir.join("made.txt").exists(),
                "a failing hook does not block"
            );
            let ran = ran(&events);
            assert_eq!(ran.len(), 2);
            assert!(ran.iter().all(|hook| hook.result == HookResult::Failed));
            assert!(
                ran[0].reason.as_deref().unwrap().contains("timed out"),
                "{ran:?}"
            );
            assert_eq!(ran[1].exit_code, Some(7));
        });
    }

    #[test]
    fn a_prompt_hook_can_stop_the_prompt_before_the_model_sees_it() {
        let dir = folder("hooks-prompt");
        let guard = "grep -q secret && { echo 'that looks like a secret'; exit 2; }; exit 0";
        let hooks = Hooks::new(vec![hook(HookPoint::UserPromptSubmit, None, guard)]).unwrap();
        block_on!(async {
            let answers = vec![answer(Some("Hi."), vec![], StopReason::EndTurn)];
            let (harness, seen) =
                harness_seeing(&dir, answers, NoOneToAsk, MemoryLog::new(), 1, settings()).await;
            let harness = harness.with_hooks(hooks);
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let refused = harness
                .run_turn(&mut thread, "my secret is 42", &Cancel::new(), &mut |_| {})
                .await;
            match refused {
                Err(HarnessError::PromptBlocked(reason)) => {
                    assert_eq!(reason, "that looks like a secret");
                }
                other => panic!("expected a blocked prompt, got {other:?}"),
            }
            assert_eq!(seen.lock().unwrap().len(), 0, "the model was not asked");
            assert_eq!(thread.messages().len(), 0);
            let events = events(harness.log(), thread.id).await;
            assert_eq!(kinds(&events), ["thread.started", "hook.ran"]);
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(seen.lock().unwrap().len(), 1);
        });
    }

    #[test]
    fn a_stop_hook_runs_once_the_turn_has_ended_and_cannot_keep_it_going() {
        let dir = folder("hooks-stop");
        let note = "cat > stopped.json; echo '{\"decision\":\"block\",\"reason\":\"more\"}'";
        let hooks = Hooks::new(vec![hook(HookPoint::Stop, None, note)]).unwrap();
        block_on!(async {
            let answers = vec![answer(Some("Hi."), vec![], StopReason::EndTurn)];
            let (harness, seen) =
                harness_seeing(&dir, answers, NoOneToAsk, MemoryLog::new(), 1, settings()).await;
            let harness = harness.with_hooks(hooks);
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(seen.lock().unwrap().len(), 1, "the turn was not kept going");
            let input: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("stopped.json")).unwrap())
                    .unwrap();
            assert_eq!(input["hook_event_name"], "Stop");
            assert_eq!(input["outcome"], "completed");
            let events = events(harness.log(), thread.id).await;
            let kinds = kinds(&events);
            assert_eq!(kinds[kinds.len() - 2..], ["turn.ended", "hook.ran"]);
            assert_eq!(ran(&events)[0].result, HookResult::Failed);
        });
    }

    /// Every request of a turn that reads a file, under `hooks`.
    async fn reading_turn(dir: &Path, hooks: Option<Vec<Hook>>) -> String {
        let answers = vec![
            answer(
                None,
                vec![("read", json!({"path": "notes.txt"}))],
                StopReason::ToolUse,
            ),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let (harness, seen) =
            harness_seeing(dir, answers, NoOneToAsk, MemoryLog::new(), 1, settings()).await;
        let harness = match hooks {
            Some(hooks) => harness.with_hooks(Hooks::new(hooks).unwrap()),
            None => harness,
        };
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "read the notes", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        serde_json::to_string(&*seen.lock().unwrap()).unwrap()
    }

    #[test]
    fn with_no_hooks_or_none_that_apply_the_model_is_sent_exactly_what_it_was_before() {
        let dir = folder("hooks-none");
        std::fs::write(dir.join("notes.txt"), "hello\n").unwrap();
        block_on!(async {
            let before = reading_turn(&dir, None).await;
            assert_eq!(before, reading_turn(&dir, Some(Vec::new())).await);
            let elsewhere = vec![
                hook(HookPoint::PreToolUse, Some("write|edit"), "exit 2"),
                hook(HookPoint::PostToolUse, Some("bash"), "exit 2"),
            ];
            assert_eq!(before, reading_turn(&dir, Some(elsewhere)).await);
        });
    }
}
