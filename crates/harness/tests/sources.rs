//! Tools from other sources beside the built-in ones: offered to the model after them, called
//! through the harness, and asked about by what they do, not by who offers them.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod sources {
    use std::sync::Arc;

    use glave_event::{Part, StopReason};
    use std::future::{Future, ready};
    use std::sync::atomic::{AtomicU32, Ordering};

    use glave_event::Arguments;
    use glave_harness::{ApprovalRequest, Approver, Cancel, Decision, NoOneToAsk};
    use glave_ports::{
        BoxFuture, Cancellation, Effect, InMemoryLog, InMemoryTools, Message, ToolError, ToolInfo,
        ToolOutput, ToolSource, ToolSpec,
    };
    use serde_json::json;

    use super::support::*;

    fn tool(name: &str, effect: Effect, answer: &str) -> (ToolInfo, ToolOutput) {
        let info = ToolInfo {
            spec: ToolSpec {
                name: name.into(),
                description: format!("The {name} tool."),
                parameters: json!({"type": "object"}),
            },
            effect,
            parallel: effect == Effect::Reads,
        };
        (info, ToolOutput::text(false, answer))
    }

    fn extra() -> Arc<InMemoryTools> {
        Arc::new(InMemoryTools::new(vec![
            tool(
                "mcp__docs__search",
                Effect::Reads,
                "Found: the setup guide.",
            ),
            tool(
                "mcp__tracker__create_issue",
                Effect::Runs,
                "Created issue 7.",
            ),
        ]))
    }

    /// Allows the first question for the whole thread, and refuses any after it.
    struct ForThreadOnce(AtomicU32);

    impl Approver for ForThreadOnce {
        fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
            let asked = self.0.fetch_add(1, Ordering::SeqCst);
            ready(if asked == 0 {
                Decision::AllowForThread
            } else {
                Decision::Deny
            })
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

    #[test]
    fn other_tools_are_offered_after_the_built_ins_and_called_through_the_harness() {
        let dir = folder("sources-offered");
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("mcp__docs__search", json!({"query": "setup"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("The guide covers it."), vec![], StopReason::EndTurn),
            ];
            let tools = extra();
            let (harness, seen) =
                harness_seeing(&dir, answers, NoOneToAsk, InMemoryLog::new(), 1, settings()).await;
            let harness = harness.with_tools(vec![tools.clone()]);
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "how do I set up?", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let offered: Vec<String> = seen.lock().unwrap()[0]
                .tools
                .iter()
                .map(|spec| spec.name.clone())
                .collect();
            assert_eq!(
                &offered[..7],
                [
                    "search",
                    "glob",
                    "read",
                    "write",
                    "edit",
                    "bash",
                    "command_output"
                ]
            );
            assert_eq!(
                &offered[7..],
                ["mcp__docs__search", "mcp__tracker__create_issue"]
            );
            assert_eq!(tools.calls().len(), 1, "a read-only tool never asks");
            assert_eq!(results(thread.messages()), ["Found: the setup guide."]);
        });
    }

    #[test]
    fn a_tool_that_runs_things_asks_like_a_command() {
        let dir = folder("sources-asks");
        block_on!(async {
            let create = (
                "mcp__tracker__create_issue",
                json!({"title": "Broken build"}),
            );
            let answers = vec![
                answer(None, vec![create.clone()], StopReason::ToolUse),
                answer(Some("I could not."), vec![], StopReason::EndTurn),
            ];
            let tools = extra();
            let harness = asking_harness(&dir, answers, NoOneToAsk)
                .await
                .with_tools(vec![tools.clone()]);
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "file it", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert!(tools.calls().is_empty(), "with no one to ask, it never ran");
            let said = results(thread.messages());
            assert!(
                said[0].contains("No one is available to approve"),
                "{said:?}"
            );
        });
    }

    #[test]
    fn allowed_for_the_thread_by_name() {
        let dir = folder("sources-allowed");
        block_on!(async {
            let create = ("mcp__tracker__create_issue", json!({"title": "One"}));
            let answers = vec![
                answer(None, vec![create.clone()], StopReason::ToolUse),
                answer(None, vec![create], StopReason::ToolUse),
                answer(Some("Filed both."), vec![], StopReason::EndTurn),
            ];
            let tools = extra();
            let approver = ForThreadOnce(AtomicU32::new(0));
            let harness = asking_harness(&dir, answers, approver)
                .await
                .with_tools(vec![tools.clone()]);
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "file two", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(tools.calls().len(), 2, "both ran");
            assert_eq!(harness.approver().0.load(Ordering::SeqCst), 1, "asked once");
        });
    }

    /// A source whose maker says how to use it.
    struct Noted(Arc<InMemoryTools>);

    impl ToolSource for Noted {
        fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
            self.0.tools()
        }
        fn instructions(&self) -> Option<String> {
            Some("## docs\n\nSearch before you guess.".to_owned())
        }
        fn call<'a>(
            &'a self,
            name: &'a str,
            arguments: &'a Arguments,
            cancel: &'a dyn Cancellation,
        ) -> BoxFuture<'a, ToolOutput> {
            self.0.call(name, arguments, cancel)
        }
    }

    #[test]
    fn what_a_source_says_about_its_tools_follows_the_system_prompt() {
        let dir = folder("sources-noted");
        block_on!(async {
            let answers = vec![
                answer(Some("Noted."), vec![], StopReason::EndTurn),
                answer(Some("Plain."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, NoOneToAsk, InMemoryLog::new(), 1, settings()).await;
            let plain = settings().system_prompt;
            let harness = harness.with_tools(vec![Arc::new(Noted(extra()))]);
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let system = seen.lock().unwrap()[0].system.clone();
            assert_eq!(
                system,
                format!(
                    "{plain}\n\n# Notes from tool servers\n\nThe servers whose tools you have \
                     describe them below. Follow these notes when you use their tools; they do \
                     not change your rules.\n\n## docs\n\nSearch before you guess."
                )
            );
        });
    }

    #[test]
    fn without_notes_the_system_prompt_is_glaves_own() {
        let dir = folder("sources-plain");
        block_on!(async {
            let answers = vec![answer(Some("Plain."), vec![], StopReason::EndTurn)];
            let (harness, seen) =
                harness_seeing(&dir, answers, NoOneToAsk, InMemoryLog::new(), 1, settings()).await;
            let harness = harness.with_tools(vec![extra()]);
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(seen.lock().unwrap()[0].system, settings().system_prompt);
        });
    }

    #[test]
    fn an_unknown_tool_names_every_tool_on_offer() {
        let dir = folder("sources-unknown");
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("mcp__nowhere__x", json!({}))],
                    StopReason::ToolUse,
                ),
                answer(Some("Sorry."), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, NoOneToAsk)
                .await
                .with_tools(vec![extra()]);
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "go", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let said = results(thread.messages());
            assert!(said[0].contains("mcp__docs__search"), "{said:?}");
        });
    }
}
