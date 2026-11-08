//! A summary that cannot stand in for the conversation: asked once more, adjusted for why it
//! failed, every call recorded and counted in the turn, and, once both fail, not asked again
//! until the request has grown or the provider refuses it as too long.

use glave_event::{Body, Event, Part, StopReason, TokenCounts, TurnResult, Usage};
use glave_harness::{AllowAll, Cancel, Live};
use glave_ports::{Completion, InMemoryLog, ModelRequest, ProviderError, Purpose};

use crate::compaction::{requests, tiny_window};
use crate::support::*;

/// `completion`, its request reported as `input` tokens long, as a provider reports it.
fn sized(mut completion: Completion, input: u64) -> Completion {
    let counts = TokenCounts {
        input,
        cache_read: 0,
        cache_write: 0,
        output: 10,
        reasoning: 0,
    };
    completion.usage = Usage::new(counts, false, None).unwrap();
    completion
}

fn said(text: &str) -> Completion {
    answer(Some(text), vec![], StopReason::EndTurn)
}

/// A summary that only reasoned until its cap.
fn reasoned_away() -> Completion {
    let mut completion = answer(None, vec![], StopReason::MaxTokens);
    completion.parts = vec![Part::Reasoning {
        text: Some("Let me think about what to keep.".into()),
        opaque: None,
    }];
    completion
}

fn too_long() -> Result<Completion, ProviderError> {
    Err(ProviderError::Http {
        status: 400,
        body: "This model's maximum context length is 20000 tokens.".into(),
    })
}

/// What one turn did.
struct Ran {
    /// What the model was sent.
    sent: Vec<ModelRequest>,
    /// What the person was shown.
    shown: Vec<Live>,
    log: Vec<Event>,
    ended: TurnResult,
}

/// Runs one turn of `answers` in a folder holding `big.txt`, which fills the tiny window, and
/// `small.txt`.
async fn run(name: &str, answers: Vec<Result<Completion, ProviderError>>) -> Ran {
    let dir = folder(name);
    std::fs::write(dir.join("big.txt"), "marigold\n".repeat(3_000)).unwrap();
    std::fs::write(dir.join("small.txt"), "hello\n").unwrap();
    let script = ScriptedModel::answering(answers);
    let workspace = workspace(&dir).await;
    let log = InMemoryLog::new();
    let (harness, seen) = scripted(workspace, script, AllowAll, log, 1, tiny_window());
    let mut thread = harness.start_thread(scope()).await.unwrap();
    let mut shown = Vec::new();
    let ended = harness
        .run_turn(&mut thread, "what is in it?", &Cancel::new(), &mut |live| {
            shown.push(live);
        })
        .await
        .unwrap();
    Ran {
        sent: requests(&seen),
        shown,
        log: events(harness.log(), thread.id).await,
        ended,
    }
}

fn purposes(sent: &[ModelRequest]) -> Vec<Purpose> {
    sent.iter().map(|request| request.purpose).collect()
}

/// How many `model.responded` the log holds.
fn responses(log: &[Event]) -> usize {
    (log.iter())
        .filter(|event| matches!(event.body, Body::ModelResponded(_)))
        .count()
}

#[test]
fn a_summary_whose_reasoning_took_its_room_is_asked_again_with_less_and_both_count() {
    block_on!(async {
        let answers = vec![
            Ok(reads("big.txt")),
            Ok(reasoned_away()),
            Ok(said("big.txt repeats marigold.")),
            Ok(said("It repeats marigold.")),
        ];
        let Ran {
            sent, log, ended, ..
        } = run("summary-reasoned", answers).await;
        let (main, summary) = (Purpose::Main, Purpose::Summary);
        assert_eq!(purposes(&sent), [main, summary, summary, main]);
        assert_eq!(sent[1].effort, None, "the thread's own, as its calls send");
        assert_eq!(sent[2].effort.as_deref(), Some("low"));
        let (first, again) = (&sent[1].messages, &sent[2].messages);
        assert_eq!(first[..first.len() - 1], again[..again.len() - 1]);
        let asked = text_of(again.last().unwrap());
        assert!(asked.contains("under 1000 words"), "and for fewer: {asked}");
        assert!(text_of(&sent[3].messages[0]).contains("big.txt repeats marigold."));
        assert_eq!(ended.model_calls, 4, "the failed summary counts");
        assert_eq!(ended.tokens.input, 400, "and so does what it used");
        assert_eq!(responses(&log), 4, "each recorded");
        let recorded = kinds(&log);
        let compacted = recorded.iter().position(|kind| *kind == "thread.compacted");
        assert_eq!(
            recorded.get(compacted.unwrap() - 2..compacted.unwrap()),
            Some(&["model.responded", "model.responded"][..])
        );
    });
}

#[test]
fn a_summary_that_calls_a_tool_is_asked_again_without_tools_and_a_cut_one_for_fewer_words() {
    block_on!(async {
        let called = answer(
            Some("Let me read it first."),
            vec![("read", serde_json::json!({ "path": "small.txt" }))],
            StopReason::ToolUse,
        );
        let answers = vec![
            Ok(reads("big.txt")),
            Ok(called),
            Ok(said("big.txt repeats marigold.")),
            Ok(said("It repeats marigold.")),
        ];
        let sent = run("summary-called", answers).await.sent;
        assert_ne!(sent[1].tools.len(), 0, "listed, as the calls list them");
        assert_eq!(sent[2].tools.len(), 0, "none to call the second time");

        let cut = answer(Some("1. Request"), vec![], StopReason::MaxTokens);
        let answers = vec![
            Ok(reads("big.txt")),
            Ok(cut),
            Ok(said("big.txt repeats marigold.")),
            Ok(said("It repeats marigold.")),
        ];
        let sent = run("summary-cut", answers).await.sent;
        let asked = |at: usize| text_of(sent[at].messages.last().unwrap());
        assert!(asked(1).contains("under 2000 words"), "{}", asked(1));
        assert!(asked(2).contains("under 1000 words"), "{}", asked(2));
        assert_eq!(sent[2].effort, None, "it did not reason");
    });
}

#[test]
fn a_summary_cut_off_twice_stands_in_with_what_it_wrote() {
    block_on!(async {
        let cut = |text: &str| answer(Some(text), vec![], StopReason::MaxTokens);
        let answers = vec![
            Ok(reads("big.txt")),
            Ok(cut("1. Request: what is in it")),
            Ok(cut("1. Request: what is in big.txt. 2. Done: read it")),
            Ok(said("It repeats marigold.")),
        ];
        let Ran { sent, shown, .. } = run("summary-cut-twice", answers).await;
        assert_eq!(sent.len(), 4);
        let restart = text_of(&sent[3].messages[0]);
        assert!(restart.contains("2. Done: read it"), "{restart}");
        assert!(
            !(shown.iter()).any(|live| matches!(live, Live::CompactionFailed(_))),
            "{shown:?}"
        );
    });
}

#[test]
fn a_failed_compaction_waits_for_the_request_to_grow_before_it_is_tried_again() {
    block_on!(async {
        // The window is 20,000 tokens and compacts at a fifth, 4,000; it waits for 1,000 more.
        let answers = vec![
            Ok(sized(reads("small.txt"), 4_100)),
            Ok(said("")),
            Ok(said(" ")),
            Ok(sized(reads("small.txt"), 4_200)),
            Ok(sized(reads("small.txt"), 5_200)),
            Ok(said("Read small.txt three times.")),
            Ok(said("It says hello.")),
        ];
        let Ran {
            sent,
            shown,
            log,
            ended,
        } = run("summary-regrow", answers).await;
        let (main, summary) = (Purpose::Main, Purpose::Summary);
        assert_eq!(
            purposes(&sent),
            [main, summary, summary, main, main, summary, main],
            "not tried again at 4,210 tokens, tried at 5,210"
        );
        let failed: Vec<&String> = (shown.iter())
            .filter_map(|live| match live {
                Live::CompactionFailed(why) => Some(why),
                _ => None,
            })
            .collect();
        assert_eq!(failed.len(), 1, "{shown:?}");
        assert!(
            failed[0].contains("the model wrote nothing"),
            "{}",
            failed[0]
        );
        assert_eq!(ended.model_calls, 7);
        assert_eq!(responses(&log), 7);
        assert!(kinds(&log).contains(&"thread.compacted"));
    });
}

#[test]
fn a_failed_compaction_is_tried_again_at_once_when_the_provider_refuses_the_request() {
    block_on!(async {
        let answers = vec![
            Ok(sized(reads("small.txt"), 4_100)),
            Ok(said("")),
            Ok(said("")),
            too_long(),
            Ok(said("Read small.txt.")),
            Ok(said("It says hello.")),
        ];
        let Ran {
            sent, log, ended, ..
        } = run("summary-refused", answers).await;
        let (main, summary) = (Purpose::Main, Purpose::Summary);
        assert_eq!(
            purposes(&sent),
            [main, summary, summary, main, summary, main]
        );
        assert_eq!(ended.model_calls, 6, "the refused call counts too");
        assert_eq!(responses(&log), 5, "a refusal answers nothing");
        assert!(kinds(&log).contains(&"thread.compacted"));
        assert!(text_of(&sent[5].messages[0]).contains("Read small.txt."));
    });
}
