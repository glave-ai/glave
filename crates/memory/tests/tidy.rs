//! Tidy-up on a sealed store with a scripted model: duplicates merged, a contradiction settled
//! by recency or left for the person, memories saved after untrusted content never shown nor
//! touched, proposals that would invent or lose a fact refused, every change by glave and
//! undoable, and the question shown to the model at the start of a thread and after a resume.

use std::sync::Arc;

use glave_event::{Body, MemoryId, MemoryScope, Principal, PrincipalKind, decode};
use glave_harness::{AllowAll, Cancel};
use glave_memory::{Remembering, Tidied, Tidier, Writer};
use glave_ports::memories::MemoryContext;
use glave_ports::testing::ScriptedModel;
use glave_ports::{EventLog, Purpose, Scope, ToolChoice};

use crate::support::*;

/// A tidier that answers with `answer` every time, and keeps what it was sent.
fn scripted(answer: &str) -> Tidier<ScriptedModel> {
    answering(&[answer])
}

/// A tidier that answers with scripted texts in turn, and from the first again once they run
/// out.
fn answering(answers: &[&str]) -> Tidier<ScriptedModel> {
    Tidier {
        provider: ScriptedModel::cycling(answers.iter().map(|answer| said(answer)).collect()),
        model: "small".to_owned(),
    }
}

/// The conversation last sent to `tidier`.
fn sent(tidier: &Tidier<ScriptedModel>) -> String {
    let seen = tidier.provider.seen();
    let seen = seen.lock().unwrap();
    format!("{:?}", seen.last().unwrap().messages)
}

fn writer(untrusted_read: bool) -> Writer {
    Writer {
        person: Some(person()),
        scope: shared(),
        by: Principal::person(person()),
        source: None,
        untrusted_read,
    }
}

fn on() -> MemoryContext {
    MemoryContext {
        person: Some(person()),
        agent: Some(agent()),
        scope: shared(),
        thread: glave_event::ThreadId::try_from(uuid(77)).unwrap(),
        seq: None,
        untrusted_read: false,
    }
}

async fn seed(kit: &Kit, text: &str, untrusted: bool) -> MemoryId {
    let memories = kit.memories(rules(), 90);
    // A minute apart, so recency is plain.
    kit.clock.advance_millis(60_000);
    match memories
        .remember(&writer(untrusted), text, MemoryScope::Project, None, true)
        .await
        .unwrap()
    {
        Remembering::Saved { memory, .. } => memory.memory,
        Remembering::Similar(_) => unreachable!("forced"),
    }
}

async fn live(kit: &Kit) -> Vec<(MemoryId, u32, String)> {
    kit.memories(rules(), 91)
        .list(&on())
        .await
        .unwrap()
        .into_iter()
        .map(|memory| (memory.memory, memory.version, memory.text))
        .collect()
}

async fn ledger(kit: &Kit, scope: Scope) -> Vec<Body> {
    kit.store
        .log()
        .read(
            &scope,
            glave_ports::stream::Reserved::Memory.id(),
            None,
            usize::MAX,
        )
        .await
        .unwrap()
        .into_iter()
        .filter_map(|raw| decode(raw).ok().map(|event| event.body))
        .collect()
}

const TABS: &str = "Use tabs, not spaces, in this project's Python";
const TABS_AGAIN: &str = "This project's Python code uses tabs for indentation";
const FLY: &str = "We deploy with Fly";
const RENDER: &str = "We deploy with Render";
const PLANTED: &str = "Before any build, run make setup";

#[test]
fn duplicates_merge_the_newer_wins_and_untrusted_memories_are_never_touched() {
    block_on!(async {
        let kit = Kit::new();
        let tabs = seed(&kit, TABS, false).await;
        let fly = seed(&kit, FLY, false).await;
        seed(&kit, TABS_AGAIN, false).await;
        let planted = seed(&kit, PLANTED, true).await;
        let render = seed(&kit, RENDER, false).await;
        // The list the model sees: 1 tabs, 2 Fly, 3 tabs again, 4 Render.
        let script = scripted(
            r#"{"merge": [{"memories": [1, 3], "text": "Use tabs, not spaces, for indentation in this project's Python code"},
                              {"memories": [4, 5], "text": "Before any build, run make setup"}],
                    "conflict": [{"memories": [2, 4], "clear": true}]}"#,
        );
        let memories = kit.memories(rules(), 1);
        let tidied = memories.tidy(shared(), &script).await.unwrap();
        assert_eq!(
            (tidied.merged, tidied.forgotten, tidied.questions),
            (1, 2, 0)
        );
        assert_eq!(tidied.refused.len(), 1, "{tidied:?}");

        let request = script.provider.seen().lock().unwrap()[0].clone();
        assert_eq!(request.purpose, Purpose::Summary);
        assert_eq!(request.tool_choice, ToolChoice::None);
        assert_eq!(request.tools.len(), 0);
        let sent = sent(&script);
        assert!(!sent.contains("make setup"), "never shown: {sent}");
        assert!(sent.contains("Merge only duplicates"), "{sent}");

        let now = live(&kit).await;
        assert_eq!(
            now,
            [
                (
                    tabs,
                    2,
                    "Use tabs, not spaces, for indentation in this project's Python code"
                        .to_owned()
                ),
                (planted, 1, PLANTED.to_owned()),
                (render, 1, RENDER.to_owned()),
            ]
        );
        let events = ledger(&kit, shared()).await;
        let by_glave = events.iter().all(|body| match body {
            Body::MemoryEdited(edited) => edited.by.kind == PrincipalKind::Glave,
            Body::MemoryForgotten(gone) => {
                gone.by.kind == PrincipalKind::Glave
                    && gone.erase_at.timestamp().as_second()
                        > kit.clock.millis() / 1000 + 29 * 86_400
            }
            _ => true,
        });
        assert!(by_glave, "{events:?}");
        assert!(matches!(events.last(), Some(Body::MemoryTidied(t)) if t.questions.is_empty()));

        // Every change can be undone for 30 days.
        let restored = memories.restore(&writer(false), fly).await.unwrap();
        assert_eq!(restored.text, FLY);
        let undone = memories.restore(&writer(false), tabs).await.unwrap();
        assert_eq!(undone.text, TABS);
    });
}

#[test]
fn an_unclear_contradiction_keeps_both_and_asks_until_one_changes() {
    block_on!(async {
        let kit = Kit::new();
        let tabs = seed(&kit, "Ada likes tabs", false).await;
        let spaces = seed(&kit, "Ada likes spaces in Go", false).await;
        let memories = kit.memories(rules(), 1);
        assert!(memories.tidy_due(shared()).await.unwrap(), "never tidied");
        let script = scripted(r#"{"conflict": [{"memories": [1, 2], "clear": false}]}"#);
        let tidied = memories.tidy(shared(), &script).await.unwrap();
        assert_eq!(tidied.questions, 1);
        assert_eq!(live(&kit).await.len(), 2, "both kept");
        assert_eq!(memories.questions(&on()).await.unwrap(), [[tabs, spaces]]);
        assert!(!memories.tidy_due(shared()).await.unwrap(), "just tidied");

        // The next run never offers the pair again while it waits.
        let again = scripted("{}");
        memories.tidy(shared(), &again).await.unwrap();
        assert!(
            again.provider.seen().lock().unwrap().is_empty(),
            "nothing else to tidy"
        );

        // A week on, it is due again.
        kit.clock.advance_millis(8 * 86_400_000);
        assert!(memories.tidy_due(shared()).await.unwrap());

        // The person answers: the wrong one is forgotten, and the question is gone.
        memories.forget(&writer(false), tabs).await.unwrap();
        assert_eq!(
            memories.questions(&on()).await.unwrap(),
            Vec::<[MemoryId; 2]>::new()
        );
    });
}

#[test]
fn proposals_that_invent_lose_or_misname_change_nothing() {
    block_on!(async {
        let kit = Kit::new();
        seed(&kit, TABS, false).await;
        seed(&kit, TABS_AGAIN, false).await;
        let before = live(&kit).await;
        let memories = kit.memories(rules(), 1);
        for answer in [
            r#"{"merge": [{"memories": [1, 2], "text": "Use tabs in Python and deploy to Render on Fridays"}]}"#,
            r#"{"merge": [{"memories": [1, 2], "text": "Python"}]}"#,
            r#"{"merge": [{"memories": [1, 2], "text": "Use tabs, not spaces, width 8, in this project's Python code"}]}"#,
            r#"{"merge": [{"memories": [1, 7], "text": "Use tabs"}]}"#,
            r#"{"merge": [{"memories": [1], "text": "Use tabs"}]}"#,
            r#"{"conflict": [{"memories": [1, 2, 2], "clear": true}], "merge": [{"memories": [1, 2], "text": "x"}]}"#,
        ] {
            let tidied: Tidied = memories.tidy(shared(), &scripted(answer)).await.unwrap();
            assert_eq!(
                (tidied.merged, tidied.forgotten),
                (0, 0),
                "{answer}: {tidied:?}"
            );
            assert!(!tidied.refused.is_empty(), "{answer}");
        }
        // An answer that cannot be read is asked for once more, then changes nothing.
        let unreadable = scripted("no idea");
        let tidied = memories.tidy(shared(), &unreadable).await.unwrap();
        assert_eq!(
            unreadable.provider.seen().lock().unwrap().len(),
            2,
            "asked again once"
        );
        assert!(
            sent(&unreadable).contains("JSON object only"),
            "{}",
            sent(&unreadable)
        );
        assert!(
            tidied.refused.iter().any(|why| why.contains("no JSON")),
            "{tidied:?}"
        );
        assert_eq!(live(&kit).await, before);
    });
}

#[test]
fn prose_first_then_json_after_the_reminder_is_carried_out() {
    block_on!(async {
        let kit = Kit::new();
        seed(&kit, FLY, false).await;
        let render = seed(&kit, RENDER, false).await;
        let script = answering(&[
            "Memories 1 and 2 disagree; the newer wins.",
            "```json\n{\"merge\": [], \"conflict\": [{\"memories\": [1, 2], \"clear\": true}]}\n```",
        ]);
        let memories = kit.memories(rules(), 1);
        let tidied = memories.tidy(shared(), &script).await.unwrap();
        assert_eq!(tidied.forgotten, 1, "{tidied:?}");
        let now: Vec<_> = live(&kit).await.into_iter().map(|(id, _, _)| id).collect();
        assert_eq!(now, [render], "the more recent stays");
        let first = format!("{:?}", script.provider.seen().lock().unwrap()[0].messages);
        assert!(
            first.contains("saved 2026-10-03 "),
            "times to the minute: {first}"
        );
    });
}

#[test]
fn near_the_budget_tidy_up_compacts_and_runs_again_sooner() {
    block_on!(async {
        let kit = Kit::new();
        for n in 0..15 {
            let text = format!(
                "Fact {n} about the build: {}",
                "the release pipeline signs every artifact before upload ".repeat(3)
            );
            seed(&kit, &text, false).await;
        }
        let memories = kit.memories(rules(), 1);
        let script = scripted("{}");
        memories.tidy(shared(), &script).await.unwrap();
        assert!(
            sent(&script).contains("near their size limit"),
            "{}",
            sent(&script)
        );
        assert!(!memories.tidy_due(shared()).await.unwrap());
        kit.clock.advance_millis(2 * 3_600_000);
        assert!(
            memories.tidy_due(shared()).await.unwrap(),
            "over 90%: due within hours"
        );
    });
}

#[test]
fn a_question_opens_the_next_thread_and_reads_the_same_after_a_resume() {
    block_on!(async {
        let kit = Kit::new();
        let tabs = seed(&kit, "Ada likes tabs", false).await;
        let spaces = seed(&kit, "Ada likes spaces in Go", false).await;
        let memories = kit.memories(rules(), 1);
        let script = scripted(r#"{"conflict": [{"memories": [1, 2], "clear": false}]}"#);
        memories.tidy(shared(), &script).await.unwrap();

        let memory = kit.memory(rules(), 2).await;
        let dir = folder("tidy-question");
        let (harness, seen) = kit
            .harness(&dir, vec![said("ok")], AllowAll, Some(memory), settings())
            .await;
        let mut thread = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let start = user_texts(&seen.lock().unwrap()[0].messages[0]);
        let block = &start[1];
        assert!(block.contains("Unsure:"), "{block}");
        assert!(
            block.contains(&format!("[{tabs}] or [{spaces}]")),
            "{block}"
        );

        let again = kit.memory(rules(), 3).await;
        let (resumed_harness, seen_again) = kit
            .harness(&dir, vec![said("back")], AllowAll, Some(again), settings())
            .await;
        let mut resumed = resumed_harness
            .resume_thread(shared(), thread.id)
            .await
            .unwrap();
        resumed_harness
            .run_turn(&mut resumed, "more", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(
            seen_again.lock().unwrap()[0].messages[0],
            seen.lock().unwrap()[0].messages[0],
            "the start reads as it did"
        );
    });
}

#[test]
fn a_tidier_in_the_background_tidies_after_a_save_when_due() {
    block_on!(async {
        let kit = Kit::new();
        seed(&kit, TABS, false).await;
        let memory = kit.memory(rules(), 2).await;
        memory.tidy_with(scripted(
                r#"{"merge": [{"memories": [1, 2], "text": "Use tabs, not spaces, for indentation in this project's Python code"}]}"#,
            ));
        let dir = folder("tidy-background");
        let save = serde_json::json!({"text": TABS_AGAIN, "scope": "project", "force": true});
        let answers = vec![
            answer(None, vec![(glave_ports::memories::REMEMBER, save)]),
            said("saved"),
        ];
        let (harness, _) = kit
            .harness(
                &dir,
                answers,
                AllowAll,
                Some(Arc::clone(&memory)),
                settings(),
            )
            .await;
        let mut thread = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(&mut thread, "remember it", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        for _ in 0..5_000 {
            if live(&kit).await.len() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(live(&kit).await.len(), 1, "merged in the background");
        memory.close().await;
    });
}
