//! Compaction in the background against a scripted model whose summaries come back at a step
//! the test knows: the older half is summarized while the turn goes on, no model call waits for
//! it, and it is swapped in at the next boundary between calls, the newer half kept as it was;
//! the prefix changes once, there; one that frees nothing falls back to compacting the whole at
//! the threshold.

use std::sync::Arc;

use glave_event::{Body, StopReason, TokenCounts, Usage};
use glave_harness::{AllowAll, Cancel, Compaction, Defaults, Harness, Live};
use glave_ports::{
    Completion, InMemoryBlobs, InMemoryLog, Message, ModelRequest, Purpose, ToolSource,
};

use crate::project_folders::tool;
use crate::support::*;

/// A window whose threshold is at half of it, 50,000 tokens: a summary of the older half
/// starts in the background from 40,000.
fn half_window() -> glave_harness::Settings {
    let mut settings = settings();
    settings.compaction = Compaction {
        window_tokens: 100_000,
        at_percent: 50,
    };
    settings
}

/// `completion`, as if the provider counted `input` tokens sent: what the next request's size
/// is estimated from.
fn sent(mut completion: Completion, input: u64) -> Completion {
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

/// A summary the model writes.
fn summary(text: &str) -> Completion {
    answer(Some(text), vec![], StopReason::EndTurn)
}

/// Writes `names` into `dir`, each 8,000 characters: about 2,000 tokens a read.
fn files(dir: &std::path::Path, names: &[&str]) {
    for name in names {
        std::fs::write(
            dir.join(name),
            format!("{name}\n").repeat(8_000 / (name.len() + 1)),
        )
        .unwrap();
    }
}

/// A harness on `dir` whose conversation follows `answers` and whose summaries `summaries`,
/// each back once `after` more of the conversation's calls came; and what it is sent.
fn summarizing(
    workspace: glave_harness::Workspace,
    answers: Vec<Completion>,
    summaries: Vec<Completion>,
    after: usize,
) -> (
    Harness<ScriptedModel, InMemoryLog, InMemoryBlobs, AllowAll>,
    Seen,
) {
    let script = ScriptedModel::new(answers).summarizing(summaries, after);
    scripted(
        workspace,
        script,
        AllowAll,
        InMemoryLog::new(),
        1,
        half_window(),
    )
}

/// The requests of the conversation, and the summaries', apart.
pub(crate) fn apart(seen: &Seen) -> (Vec<ModelRequest>, Vec<ModelRequest>) {
    seen.lock()
        .unwrap()
        .iter()
        .cloned()
        .partition(|request| request.purpose != Purpose::Summary)
}

/// What a request opens with, as the provider's prompt cache reads it.
fn prefix(request: &ModelRequest) -> (String, Vec<u8>) {
    (
        request.system.clone(),
        serde_json::to_vec(&request.tools).unwrap(),
    )
}

#[test]
fn the_older_half_is_summarized_while_the_work_goes_on_and_swapped_in_between_calls() {
    let dir = folder("compact-background");
    files(&dir, &["a.txt", "b.txt", "c.txt"]);
    block_on!(async {
        let answers = vec![
            sent(reads("a.txt"), 1_000),
            sent(reads("b.txt"), 41_000),
            sent(summary("Read both."), 44_000),
            sent(reads("c.txt"), 46_000),
            sent(summary("Done."), 20_000),
        ];
        let summaries = vec![summary("The person asked for a.txt; it holds a.txt lines.")];
        let (harness, seen) = summarizing(workspace(&dir).await, answers, summaries, 2);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "read a.txt and b.txt").await;
        // Between turns, a tool comes that the prefix does not list: it waits for a compaction.
        let web = Arc::new(tool("web_fetch", "A page.")) as Arc<dyn ToolSource>;
        harness.set_defaults(Defaults {
            web: Some(web),
            ..Defaults::clone(&harness.defaults())
        });
        let mut shown = Vec::new();
        harness
            .run_turn(&mut thread, "now c.txt", &Cancel::new(), &mut |live| {
                shown.push(live);
            })
            .await
            .unwrap();

        // Every call went on while the summary was written: it came back only once the
        // conversation had made two more, and the turn would have hung had one waited for it.
        let (calls, summaries) = apart(&seen);
        assert_eq!((calls.len(), summaries.len()), (5, 1));
        let asked = &summaries[0];
        assert_eq!(
            asked.messages.len(),
            4,
            "the older half and the instruction"
        );
        assert!(text_of(&asked.messages[3]).contains("older part of a longer one"));
        assert_eq!(
            prefix(asked),
            prefix(&calls[0]),
            "it shares the cache's prefix"
        );

        // Swapped in at the boundary after the fourth call: the summary, then the newer half.
        let after = &calls[4].messages;
        let restart = text_of(&after[0]);
        assert!(restart.starts_with("[The older part of this conversation was summarized"));
        assert!(restart.contains("it holds a.txt lines."), "{restart}");
        assert!(
            !restart.contains("word for word"),
            "the latest request is in the newer half, as it was: {restart}"
        );
        assert_eq!(
            after[1..5],
            calls[3].messages[3..],
            "the newer half, as it was"
        );
        assert_eq!(
            after.len(),
            7,
            "with the fourth call and its result after it"
        );
        assert!(shown.iter().any(|live| matches!(
            live,
            Live::Compacted { waited: false, after_tokens, before_tokens } if after_tokens < before_tokens
        )));

        // The prefix stayed the same through every call and the summary, and changed once, at
        // the swap, taking in the tool that came meanwhile.
        let first = prefix(&calls[0]);
        assert!(calls[..4].iter().all(|call| prefix(call) == first));
        assert_ne!(prefix(&calls[4]), first);
        assert!(calls[4].tools.iter().any(|spec| spec.name == "web_fetch"));

        // Recorded so the log alone rebuilds it, from its end.
        let log = events(harness.log(), thread.id).await;
        let compacted = log.iter().find_map(|event| match &event.body {
            Body::ThreadCompacted(compacted) => Some(compacted),
            _ => None,
        });
        assert_eq!(compacted.map(|compacted| compacted.kept), Some(6));
        let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        assert_eq!(resumed.messages(), thread.messages());
    });
}

#[test]
fn a_summary_that_frees_nothing_falls_back_to_compacting_the_whole_at_the_threshold() {
    let dir = folder("compact-fallback");
    files(&dir, &["a.txt", "b.txt", "c.txt", "d.txt"]);
    block_on!(async {
        let answers = vec![
            sent(reads("a.txt"), 1_000),
            sent(reads("b.txt"), 41_000),
            sent(reads("c.txt"), 44_000),
            sent(reads("d.txt"), 49_000),
            sent(summary("Done."), 5_000),
        ];
        // The first, in the background, comes back empty; the second is the whole's.
        let summaries = vec![
            answer(None, vec![], StopReason::EndTurn),
            summary("Read a to d."),
        ];
        let (harness, seen) = summarizing(workspace(&dir).await, answers, summaries, 0);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let mut shown = Vec::new();
        harness
            .run_turn(&mut thread, "read them all", &Cancel::new(), &mut |live| {
                shown.push(live);
            })
            .await
            .unwrap();

        let (calls, summaries) = apart(&seen);
        assert_eq!(summaries.len(), 2, "none started again after the empty one");
        assert!(text_of(summaries[0].messages.last().unwrap()).contains("older part"));
        assert!(text_of(summaries[1].messages.last().unwrap()).contains("about to outgrow"));
        let last = &calls.last().unwrap().messages;
        assert_eq!(last.len(), 1, "the whole restarts from one message");
        assert!(text_of(&last[0]).contains("Read a to d."));
        let compactions: Vec<bool> = shown
            .iter()
            .filter_map(|live| match live {
                Live::Compacted { waited, .. } => Some(*waited),
                _ => None,
            })
            .collect();
        assert_eq!(compactions, [true], "the turn waited for the whole, once");
        let recorded = kinds(&events(harness.log(), thread.id).await);
        let responded = recorded.iter().filter(|kind| **kind == "model.responded");
        assert_eq!(
            responded.count(),
            calls.len() + 2,
            "the empty summary's usage is recorded too: {recorded:?}"
        );
        assert!(
            !matches!(last[0], Message::Assistant { .. }),
            "the conversation opens with the person's side"
        );
    });
}
