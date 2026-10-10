//! The judge in auto mode: a soft caution the person asked for runs, recorded as an approval
//! glave decided; one they didn't ask for still asks; a firm caution never reaches the judge;
//! and in the cautious mode the judge is never consulted.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod judging {
    use glave_event::{Body, Part, StopReason, Verdict};
    use glave_harness::{ApprovalMode, Cancel, NoOneToAsk, Settings};
    use glave_ports::{MemoryLog, Message};
    use serde_json::json;

    use super::support::*;

    fn judged() -> Settings {
        let mut settings = settings();
        settings.judge = Some("judge-model".into());
        settings
    }

    fn bash(command: &str) -> Vec<(&'static str, serde_json::Value)> {
        vec![("bash", json!({ "command": command }))]
    }

    fn results(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .filter_map(|message| match message {
                Message::Tool { output, .. } => output.iter().find_map(|part| match part {
                    Part::Text { text } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .collect()
    }

    fn verdicts(events: &[glave_event::Event]) -> Vec<Verdict> {
        events
            .iter()
            .filter_map(|event| match &event.body {
                Body::ApprovalDecided(decided) => Some(decided.decision),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_action_the_person_asked_for_runs_and_the_judge_is_on_record() {
        let dir = folder("judge-run");
        block_on(async {
            let answers = vec![
                answer(None, bash("git push origin main"), StopReason::ToolUse),
                answer(
                    Some(r#"{"decision":"run","reason":"the person asked to push"}"#),
                    vec![],
                    StopReason::EndTurn,
                ),
                answer(Some("Pushed."), vec![], StopReason::EndTurn),
            ];
            let harness =
                harness_on(&dir, answers, NoOneToAsk, MemoryLog::new(), 1, judged()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(
                    &mut thread,
                    "push my commits to main",
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            let said = results(thread.messages());
            assert!(!said[0].starts_with("Not run"), "{said:?}");
            assert_eq!(
                verdicts(&events(harness.log(), thread.id).await),
                [Verdict::AllowOnce]
            );
        });
    }

    #[test]
    fn an_action_the_person_did_not_ask_for_still_asks() {
        let dir = folder("judge-ask");
        block_on(async {
            let answers = vec![
                answer(None, bash("npm publish"), StopReason::ToolUse),
                answer(
                    Some(r#"{"decision":"ask","reason":"asked to bump, not to publish"}"#),
                    vec![],
                    StopReason::EndTurn,
                ),
                answer(Some("Left it."), vec![], StopReason::EndTurn),
            ];
            let harness =
                harness_on(&dir, answers, NoOneToAsk, MemoryLog::new(), 1, judged()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(
                    &mut thread,
                    "bump the version to 1.4.0",
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            let said = results(thread.messages());
            assert!(
                said[0].starts_with("Not run, because it publishes a package"),
                "{said:?}"
            );
        });
    }

    #[test]
    fn a_firm_caution_never_reaches_the_judge() {
        let dir = folder("judge-firm");
        std::fs::write(dir.join(".env"), "KEY=canary\n").unwrap();
        block_on(async {
            // Had the judge been asked, it would have taken the final answer below.
            let answers = vec![
                answer(None, bash("cat .env"), StopReason::ToolUse),
                answer(Some("I did not read it."), vec![], StopReason::EndTurn),
            ];
            let harness =
                harness_on(&dir, answers, NoOneToAsk, MemoryLog::new(), 1, judged()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(
                    &mut thread,
                    "show me what's in .env",
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            assert_eq!(
                verdicts(&events(harness.log(), thread.id).await),
                [Verdict::NoOneThere]
            );
        });
    }

    #[test]
    fn in_the_cautious_mode_the_judge_is_never_consulted() {
        let dir = folder("judge-ask-mode");
        block_on(async {
            let answers = vec![
                answer(None, bash("git push origin main"), StopReason::ToolUse),
                answer(Some("Waiting for you."), vec![], StopReason::EndTurn),
            ];
            let mut settings = judged();
            settings.approval = ApprovalMode::Ask;
            let harness =
                harness_on(&dir, answers, NoOneToAsk, MemoryLog::new(), 1, settings).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "push my commits", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(
                verdicts(&events(harness.log(), thread.id).await),
                [Verdict::NoOneThere]
            );
        });
    }
}
