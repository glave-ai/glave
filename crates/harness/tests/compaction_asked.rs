//! Compaction the person asks for in their own words: the model calls `compact`, listed from the
//! thread's first call, with what they want kept; the call answers at once, and at the next
//! boundary between model calls the whole conversation, the call and its result inside it, is
//! summarized with that kept front and centre. The restart says it is done, so it never reads
//! as another ask; a second call in the turn compacts nothing; a subagent never has it.

use glave_event::{Body, StopReason};
use glave_harness::{AllowAll, Cancel, Live, NoOneToAsk};
use glave_ports::{InMemoryLog, Message, ModelRequest, Purpose};
use serde_json::json;

use crate::support::*;

/// What the person says to ask for it.
const ASKED: &str = "go ahead and compact, and keep the API decisions front and centre";

fn said(text: &str) -> glave_ports::Completion {
    answer(Some(text), vec![], StopReason::EndTurn)
}

fn compacts(focus: serde_json::Value) -> glave_ports::Completion {
    answer(None, vec![("compact", focus)], StopReason::ToolUse)
}

fn sent(seen: &Seen) -> Vec<ModelRequest> {
    seen.lock().unwrap().clone()
}

fn offers_compact(request: &ModelRequest) -> bool {
    request.tools.iter().any(|spec| spec.name == "compact")
}

/// The summary is asked of everything, the call and its answer included, keeping the focus.
fn asked_with_the_call_inside(summary: &ModelRequest) {
    assert_eq!(summary.purpose, Purpose::Summary);
    let asking = &summary.messages;
    assert_eq!(
        asking.len(),
        6,
        "both turns, the call, its result, the instruction"
    );
    assert!(
        matches!(&asking[3], Message::Assistant { tool_calls, .. } if tool_calls[0].tool == "compact")
    );
    let result = results(asking);
    assert!(
        result[0].starts_with("Noted: the conversation is compacted before your next step"),
        "{result:?}"
    );
    let instruction = text_of(asking.last().unwrap());
    assert!(instruction.starts_with("The person asked for this conversation to be compacted"));
    let kept = "whatever else is left out: the API decisions";
    assert!(instruction.ends_with(kept), "{instruction}");
}

/// The conversation restarts from one message that says the compaction is done.
fn restarted_as_done(after: &ModelRequest) {
    assert_eq!(after.messages.len(), 1, "{:?}", after.messages);
    let restart = text_of(&after.messages[0]);
    assert!(
        restart.contains("it is done") && restart.contains("front and centre: the API decisions"),
        "{restart}"
    );
    assert!(restart.contains("Do not compact again unless they ask again"));
    assert!(restart.contains("The API uses REST, not GraphQL."));
    assert!(
        !restart.contains("`compact`"),
        "the call is not kept: {restart}"
    );
    assert!(
        restart.ends_with(&format!("its compaction already done:\n{ASKED}")),
        "{restart}"
    );
}

#[test]
fn asked_in_plain_words_the_conversation_is_compacted_at_the_next_step_keeping_what_was_named() {
    let dir = folder("compact-asked");
    block_on!(async {
        let answers = vec![
            said("Noted."),
            compacts(json!({ "focus": "the API decisions" })),
            said("The API uses REST, not GraphQL."),
            said("Done: compacted, keeping the API decisions."),
        ];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(
            &harness,
            &mut thread,
            "Remember: the API uses REST, not GraphQL.",
        )
        .await;
        let mut shown = Vec::new();
        let ended = harness
            .run_turn(&mut thread, ASKED, &Cancel::new(), &mut |live| {
                shown.push(live);
            })
            .await
            .unwrap();

        let sent = sent(&seen);
        assert_eq!(sent.len(), 4, "a turn, the call, the summary, the reply");
        assert!(
            offers_compact(&sent[0]),
            "listed from the first call, so the prefix never changes for it"
        );
        asked_with_the_call_inside(&sent[2]);
        restarted_as_done(&sent[3]);
        assert_eq!(ended.model_calls, 3, "the summary counts");
        let line = "Compact the conversation, keeping the API decisions";
        assert!(shown.contains(&Live::ToolStarted {
            summary: line.into()
        }));
        assert!(
            shown
                .iter()
                .any(|live| matches!(live, Live::Compacted { .. }))
        );

        let log = events(harness.log(), thread.id).await;
        let recorded = log.iter().find_map(|event| match &event.body {
            Body::ToolCallRequested(requested) => Some(requested.summary.clone()),
            _ => None,
        });
        assert_eq!(recorded.as_deref(), Some(line), "the person's one line");
        let compactions = kinds(&log)
            .iter()
            .filter(|kind| **kind == "thread.compacted")
            .count();
        assert_eq!(compactions, 1);
        let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        assert_eq!(
            resumed.messages(),
            thread.messages(),
            "the log alone rebuilds it"
        );
    });
}

#[test]
fn a_turn_compacts_once_however_often_the_model_calls_it() {
    let dir = folder("compact-once");
    block_on!(async {
        let answers = vec![
            compacts(json!({})),
            said("Said hello."),
            compacts(json!({ "focus": "everything" })),
            said("Done."),
        ];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "hello, and compact please").await;

        let purposes: Vec<Purpose> = sent(&seen).iter().map(|r| r.purpose).collect();
        let (main, summary) = (Purpose::Main, Purpose::Summary);
        assert_eq!(purposes, [main, summary, main, main], "one summary");
        let log = events(harness.log(), thread.id).await;
        let compactions = kinds(&log)
            .iter()
            .filter(|kind| **kind == "thread.compacted")
            .count();
        assert_eq!(compactions, 1);
        let again = log.iter().rev().find_map(|event| match &event.body {
            Body::ToolCallCompleted(done) => Some(done.is_error),
            _ => None,
        });
        assert_eq!(again, Some(true), "the second call compacts nothing");
        let result = results(thread.messages());
        assert!(result[0].contains("a turn compacts once"), "{result:?}");
    });
}

#[test]
fn a_subagent_never_has_compact() {
    let dir = folder("compact-subagent");
    block_on!(async {
        let brief =
            json!({"description": "look", "prompt": "Look around.", "subagent_type": "explore"});
        let answers = vec![
            answer(None, vec![("agent", brief)], StopReason::ToolUse),
            said("Nothing here."),
            said("Done."),
        ];
        let mut settings = settings();
        settings.subagents = true;
        let (harness, seen) =
            harness_seeing(&dir, answers, NoOneToAsk, InMemoryLog::new(), 1, settings).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "look around").await;
        let sent = sent(&seen);
        assert_eq!(sent[1].purpose, Purpose::Subagent);
        assert!(offers_compact(&sent[0]) && !offers_compact(&sent[1]));
    });
}
