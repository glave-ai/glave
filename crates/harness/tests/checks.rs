//! Commands read before they run: a risky one asks whatever the mode, and says why; one that
//! only reads runs without asking even in `ask`; and allowing bash for the thread never silences
//! a risky command. Every risky command here would be harmless if it ran: the folder is not a
//! git repository.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod checks {
    use std::future::{Future, ready};
    use std::sync::Mutex;

    use glave_event::{Part, StopReason};
    use glave_harness::{ApprovalRequest, Approver, Cancel, Decision, NoOneToAsk};
    use glave_ports::Message;
    use serde_json::json;

    use super::support::*;

    /// Answers with each decision in turn, and remembers what it was asked.
    struct Answers {
        decisions: Mutex<Vec<Decision>>,
        asked: Mutex<Vec<ApprovalRequest>>,
    }

    impl Answers {
        fn new(decisions: &[Decision]) -> Self {
            let mut decisions = decisions.to_vec();
            decisions.reverse();
            Self {
                decisions: Mutex::new(decisions),
                asked: Mutex::new(Vec::new()),
            }
        }
    }

    impl Approver for Answers {
        fn decide(&self, request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
            self.asked.lock().unwrap().push(request.clone());
            ready(
                self.decisions
                    .lock()
                    .unwrap()
                    .pop()
                    .unwrap_or(Decision::Deny),
            )
        }
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

    fn bash(command: &str) -> (&'static str, serde_json::Value) {
        ("bash", json!({ "command": command }))
    }

    #[test]
    fn in_auto_a_risky_command_with_no_one_there_is_not_run_and_the_model_learns_why() {
        let dir = folder("checks-auto");
        block_on!(async {
            let answers = vec![
                answer(None, vec![bash("git reset --hard")], StopReason::ToolUse),
                answer(Some("I left it."), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, NoOneToAsk).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "start over", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let said = results(thread.messages());
            assert!(
                said[0].starts_with("Not run, because it discards uncommitted changes"),
                "{said:?}"
            );
        });
    }

    #[test]
    fn in_ask_mode_a_command_that_only_reads_runs_without_asking() {
        let dir = folder("checks-reads");
        std::fs::write(dir.join("notes.txt"), "hello\n").unwrap();
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![bash("cat notes.txt"), bash("touch made.txt")],
                    StopReason::ToolUse,
                ),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ];
            let approver = Answers::new(&[Decision::Deny]);
            let harness = asking_harness(&dir, answers, approver).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "read and touch", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let said = results(thread.messages());
            assert!(said[0].contains("hello"), "{said:?}");
            let asked = harness.approver().asked.lock().unwrap();
            assert_eq!(
                asked.len(),
                1,
                "only the command that changes something asks"
            );
            assert_eq!(asked[0].summary, "bash: touch made.txt");
            assert!(!dir.join("made.txt").exists());
        });
    }

    #[test]
    fn allowing_bash_for_the_thread_never_silences_a_risky_command() {
        let dir = folder("checks-allowed");
        block_on!(async {
            let answers = vec![
                answer(None, vec![bash("touch a.txt")], StopReason::ToolUse),
                answer(None, vec![bash("git clean -fdx")], StopReason::ToolUse),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ];
            let approver = Answers::new(&[Decision::AllowForThread, Decision::Deny]);
            let harness = asking_harness(&dir, answers, approver).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "tidy up", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let asked = harness.approver().asked.lock().unwrap();
            assert_eq!(asked.len(), 2, "the risky command asked again");
            assert_eq!(
                asked[1].summary,
                "bash: git clean -fdx (it deletes untracked files)"
            );
            assert_eq!(
                asked[1].caution.as_deref(),
                Some("it deletes untracked files")
            );
        });
    }
}
