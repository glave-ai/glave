//! Compaction the person asks for in their own words: the model calls `compact`, listed from the
//! thread's first call, with what they want kept, which the summary is asked to keep front and
//! centre; the call's line says so, and its result follows it in the restarted conversation.

use glave_event::{Body, Event, StopReason};
use glave_harness::{AllowAll, Cancel, Live};
use glave_ports::{Completion, InMemoryLog, Message};

use crate::compaction_background::apart;
use crate::support::*;

/// What the person says to ask for it.
const ASKED: &str = "go ahead and compact, and keep the API decisions front and centre";

/// A turn that notes a decision, then one whose model compacts as the person asks, keeping
/// "the API decisions".
fn script() -> (Vec<Completion>, Vec<Completion>) {
    let compact = serde_json::json!({ "focus": "the API decisions" });
    let done = "Done: compacted, keeping the API decisions.";
    let answers = vec![
        answer(Some("Noted."), vec![], StopReason::EndTurn),
        answer(None, vec![("compact", compact)], StopReason::ToolUse),
        answer(Some(done), vec![], StopReason::EndTurn),
    ];
    let summary = "The API uses REST, not GraphQL.";
    let summaries = vec![answer(Some(summary), vec![], StopReason::EndTurn)];
    (answers, summaries)
}

/// The conversation after the call: the summary, keeping what was named, with the request word
/// for word; the call; its result.
fn restarted_with_the_call_after(after: &[Message]) {
    assert_eq!(after.len(), 3, "the summary, the call, its result");
    let restart = text_of(&after[0]);
    assert!(
        restart.contains("it is done") && restart.contains("front and centre: the API decisions"),
        "{restart}"
    );
    assert!(restart.contains("The API uses REST, not GraphQL."));
    assert!(
        restart.ends_with(&format!("word for word:\n{ASKED}")),
        "{restart}"
    );
    assert!(
        matches!(&after[1], Message::Assistant { tool_calls, .. } if tool_calls[0].tool == "compact")
    );
    let result = results(after);
    assert!(result[0].starts_with("Compacted:"), "{result:?}");
}

/// The call's line, as recorded, and how many messages the compaction kept.
fn recorded(log: &[Event]) -> (Option<String>, Option<u32>) {
    let line = log.iter().find_map(|event| match &event.body {
        Body::ToolCallRequested(requested) => Some(requested.summary.clone()),
        _ => None,
    });
    let kept = log.iter().find_map(|event| match &event.body {
        Body::ThreadCompacted(compacted) => Some(compacted.kept),
        _ => None,
    });
    (line, kept)
}

#[test]
fn asked_in_plain_words_the_model_compacts_keeping_what_the_person_named() {
    let dir = folder("compact-asked");
    block_on!(async {
        let (answers, summaries) = script();
        let script = ScriptedModel::new(answers).summarizing(summaries, 0);
        let workspace = workspace(&dir).await;
        let (harness, seen) = scripted(
            workspace,
            script,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let noted = "Remember: the API uses REST, not GraphQL.";
        turn(&harness, &mut thread, noted).await;
        let mut shown = Vec::new();
        let mut show = |live| shown.push(live);
        let cancel = Cancel::new();
        harness
            .run_turn(&mut thread, ASKED, &cancel, &mut show)
            .await
            .unwrap();

        let (calls, summaries) = apart(&seen);
        assert!(
            calls[0].tools.iter().any(|spec| spec.name == "compact"),
            "listed from the first call, so the prefix never changes for it"
        );
        assert_eq!(summaries.len(), 1);
        let asking = &summaries[0].messages;
        assert_eq!(asking.len(), 4, "everything before the call's response");
        let instruction = text_of(asking.last().unwrap());
        assert!(instruction.starts_with("The person asked for this conversation to be compacted"));
        let kept = "whatever else is left out: the API decisions";
        assert!(instruction.ends_with(kept), "{instruction}");
        restarted_with_the_call_after(&calls[2].messages);
        assert!(
            shown
                .iter()
                .any(|live| matches!(live, Live::Compacted { waited: true, .. }))
        );

        let line = "Compact the conversation, keeping the API decisions";
        let log = events(harness.log(), thread.id).await;
        assert_eq!(recorded(&log), (Some(line.to_owned()), Some(1)));
        let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        assert_eq!(resumed.messages(), thread.messages());
    });
}
