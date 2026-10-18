//! AGENTS.md instructions against a scripted model: they open a new thread's first message,
//! are never repeated, survive resuming from the log, and open the conversation again after a
//! compaction.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod instructions {
    use std::path::Path;
    use std::sync::Arc;

    use glave_event::{Part, StopReason};
    use glave_harness::{AllowAll, Cancel, Compaction, Place, Projects, Settings, agents_md};
    use glave_ports::Caller;
    use glave_ports::{BoxFuture, InMemoryLog, Message, ModelRequest};
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
        block_on!(async {
            let answers = vec![
                answer(Some("Hi."), vec![], StopReason::EndTurn),
                answer(Some("Again."), vec![], StopReason::EndTurn),
                answer(Some("Back."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, with_rules()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "and again", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            harness
                .run_turn(&mut resumed, "back", &Cancel::new(), &mut |_| {})
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
        block_on!(async {
            let answers = vec![answer(Some("Hi."), vec![], StopReason::EndTurn)];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(texts(&requests(&seen)[0].messages[0]), ["hello"]);
        });
    }

    #[test]
    fn a_compacted_thread_opens_with_the_instructions_again() {
        let dir = folder("rules-compact");
        std::fs::write(dir.join("big.txt"), "marigold\n".repeat(1_000)).unwrap();
        block_on!(async {
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
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(
                    &mut thread,
                    "what is in big.txt?",
                    &Cancel::new(),
                    &mut |_| {},
                )
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
        block_on!(async {
            let mut settings = settings();
            let loaded = agents_md(&dir, None).await.unwrap().unwrap();
            settings.instructions = Some(loaded.text);
            let answers = vec![answer(Some("Bonjour."), vec![], StopReason::EndTurn)];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let first = texts(&requests(&seen)[0].messages[0]);
            assert!(first[0].contains("Always answer in French."), "{first:?}");
        });
    }

    /// Instructions that name the folder a thread reads them for.
    struct Told;

    impl Projects for Told {
        fn for_folder<'a>(
            &'a self,
            root: &'a Path,
            project: Option<&'a Path>,
            _trusting: bool,
        ) -> BoxFuture<'a, Option<String>> {
            let text = format!("Rules of {project:?} at {}", root.display());
            Box::pin(std::future::ready(Some(text)))
        }
    }

    #[test]
    fn a_thread_in_a_folder_opens_with_that_folder_s_instructions_and_keeps_them() {
        let dir = folder("rules-folder");
        let other = folder("rules-folder-other");
        let other = std::fs::canonicalize(other).unwrap();
        block_on!(async {
            let answers = vec![
                answer(Some("Hi."), vec![], StopReason::EndTurn),
                answer(Some("We said hello."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, with_rules()).await;
            let harness = harness.with_projects(Some(Arc::new(Told)));
            let place = Place::Folder(other.to_string_lossy().into_owned());
            let mut thread = harness
                .start_thread_in(scope(), &Caller::default(), place)
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let told = format!("Rules of Some({other:?}) at {}", other.display());
            let first = texts(&requests(&seen)[0].messages[0]);
            assert_eq!(
                first,
                [told.as_str(), "hello"],
                "the folder's, not the settings'"
            );
            // Resumed from the log, a compaction opens the conversation with them again.
            let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            harness
                .compact(&mut resumed, &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let opening = texts(&resumed.messages()[0]);
            assert_eq!(opening.first(), Some(&told), "{opening:?}");
        });
    }
}
