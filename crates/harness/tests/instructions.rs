//! AGENTS.md instructions against a scripted model: they open a new thread's first message,
//! are never repeated, survive resuming from the log, and open the conversation again after a
//! compaction.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod instructions {
    use glave_event::{Part, StopReason};
    use glave_harness::{AllowAll, Compaction, Settings, agents_md};
    use glave_ports::{MemoryLog, Message, ModelRequest};
    use serde_json::json;

    use super::support::*;

    const RULES: &str =
        "# AGENTS.md instructions for /p\n\n<INSTRUCTIONS>\nUse tabs.\n</INSTRUCTIONS>";

    fn with_rules() -> Settings {
        let mut settings = settings();
        settings.instructions = Some(RULES.to_owned());
        settings
    }

    fn texts(message: &Message) -> Vec<String> {
        let parts = match message {
            Message::User { parts }
            | Message::Tool { output: parts, .. }
            | Message::Assistant { parts, .. } => parts,
        };
        parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn requests(seen: &Seen) -> Vec<ModelRequest> {
        seen.lock().unwrap().clone()
    }

    #[test]
    fn a_new_thread_opens_with_the_instructions_once() {
        let dir = folder("rules-once");
        block_on(async {
            let answers = vec![
                answer(Some("Hi."), vec![], StopReason::EndTurn),
                answer(Some("Again."), vec![], StopReason::EndTurn),
                answer(Some("Back."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, MemoryLog::new(), 1, with_rules()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &mut |_| {})
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "and again", &mut |_| {})
                .await
                .unwrap();
            let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            harness
                .run_turn(&mut resumed, "back", &mut |_| {})
                .await
                .unwrap();

            let sent = requests(&seen);
            assert_eq!(
                texts(&sent[0].messages[0]),
                [RULES, "hello"],
                "the instructions come first, in the person's first message"
            );
            assert!(
                !sent[0].system.contains("Use tabs."),
                "never in the system prompt"
            );
            let last = &sent[2].messages;
            assert_eq!(
                texts(&last[2]),
                ["and again"],
                "later turns are just the prompt"
            );
            assert_eq!(texts(&last[4]), ["back"], "a resumed thread adds nothing");
            let all: String = last.iter().flat_map(texts).collect();
            assert_eq!(all.matches("Use tabs.").count(), 1);
        });
    }

    #[test]
    fn no_instructions_means_the_prompt_alone() {
        let dir = folder("rules-none");
        block_on(async {
            let answers = vec![answer(Some("Hi."), vec![], StopReason::EndTurn)];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, MemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &mut |_| {})
                .await
                .unwrap();
            assert_eq!(texts(&requests(&seen)[0].messages[0]), ["hello"]);
        });
    }

    #[test]
    fn a_compacted_thread_opens_with_the_instructions_again() {
        let dir = folder("rules-compact");
        std::fs::write(dir.join("big.txt"), "marigold\n".repeat(1_000)).unwrap();
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![("read", json!({"path": "big.txt"}))],
                    StopReason::ToolUse,
                ),
                answer(
                    Some("big.txt repeats marigold."),
                    vec![],
                    StopReason::EndTurn,
                ),
                answer(Some("It repeats marigold."), vec![], StopReason::EndTurn),
            ];
            let mut settings = with_rules();
            settings.compaction = Compaction {
                window_tokens: 1_000,
                at_percent: 80,
            };
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, MemoryLog::new(), 1, settings).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "what is in big.txt?", &mut |_| {})
                .await
                .unwrap();
            let sent = requests(&seen);
            assert_eq!(sent.len(), 3, "the read, the summary, the answer");
            let restart = texts(&sent[2].messages[0]);
            assert_eq!(restart.len(), 2, "{restart:?}");
            assert_eq!(restart[0], RULES);
            assert!(restart[1].contains("big.txt repeats marigold."));
            let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            assert_eq!(
                resumed.messages(),
                thread.messages(),
                "the log alone rebuilds it"
            );
        });
    }

    #[test]
    fn the_workspace_files_reach_the_model() {
        let dir = folder("rules-files");
        std::fs::write(dir.join("AGENTS.md"), "Always answer in French.\n").unwrap();
        block_on(async {
            let mut settings = settings();
            let loaded = agents_md(&dir, None).await.unwrap().unwrap();
            settings.instructions = Some(loaded.text);
            let answers = vec![answer(Some("Bonjour."), vec![], StopReason::EndTurn)];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, MemoryLog::new(), 1, settings).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &mut |_| {})
                .await
                .unwrap();
            let first = texts(&requests(&seen)[0].messages[0]);
            assert!(first[0].contains("Always answer in French."), "{first:?}");
        });
    }
}
