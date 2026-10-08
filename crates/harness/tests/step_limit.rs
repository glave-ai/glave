//! The step limit against a scripted model: the last step a turn allows is asked for a text
//! answer about what was done and what is left, with tools listed but off, and any call the
//! model makes anyway is dropped so the thread stays valid.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod step_limit {
    use glave_event::{Outcome, Part, StopReason};
    use glave_harness::{AllowAll, Cancel};
    use glave_ports::{MemoryLog, Message, ToolChoice};
    use serde_json::json;

    use super::support::*;

    fn text(message: &Message) -> String {
        match message {
            Message::User { parts } | Message::Assistant { parts, .. } => parts
                .iter()
                .filter_map(|part| match part {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect(),
            Message::Tool { .. } => String::new(),
        }
    }

    #[test]
    fn the_last_step_reports_in_text_with_tools_off() {
        let dir = folder("step-limit");
        std::fs::write(dir.join("README.md"), "# notes\n").unwrap();
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![("read", json!({"path": "README.md"}))],
                    StopReason::ToolUse,
                ),
                answer(
                    Some("I reached the step limit. I read README.md; the rest is left."),
                    vec![("read", json!({"path": "README.md"}))],
                    StopReason::ToolUse,
                ),
            ];
            let mut settings = settings();
            settings.max_steps = 2;
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, MemoryLog::new(), 1, settings).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let summary = harness
                .run_turn(&mut thread, "read everything", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(summary.outcome, Outcome::LimitReached);
            assert_eq!(summary.model_calls, 2);
            assert_eq!(summary.tool_calls, 1, "the last step's call never ran");

            let sent = seen.lock().unwrap().clone();
            assert_eq!(sent[0].tool_choice, ToolChoice::Auto);
            assert_eq!(sent[1].tool_choice, ToolChoice::None);
            assert!(
                !sent[1].tools.is_empty(),
                "tools stay listed for strict providers"
            );
            let note = text(sent[1].messages.last().unwrap());
            assert!(note.contains("reached its step limit"), "{note}");

            let last = thread.messages().last().unwrap();
            assert!(
                matches!(last, Message::Assistant { tool_calls, .. } if tool_calls.is_empty()),
                "{last:?}"
            );
            assert!(text(last).contains("the rest is left"));
            let recorded = kinds(&events(harness.log(), thread.id).await);
            assert_eq!(
                recorded
                    .iter()
                    .filter(|kind| **kind == "tool.call.requested")
                    .count(),
                1
            );
            let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            assert_eq!(
                resumed.messages(),
                thread.messages(),
                "the log alone rebuilds it"
            );
        });
    }

    #[test]
    fn a_limit_of_one_answers_in_text_at_once() {
        let dir = folder("step-limit-one");
        block_on(async {
            let answers = vec![answer(
                Some("I reached the step limit before starting."),
                vec![],
                StopReason::EndTurn,
            )];
            let mut settings = settings();
            settings.max_steps = 1;
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, MemoryLog::new(), 1, settings).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let summary = harness
                .run_turn(&mut thread, "go", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(summary.outcome, Outcome::LimitReached);
            assert_eq!(seen.lock().unwrap()[0].tool_choice, ToolChoice::None);
        });
    }
}
