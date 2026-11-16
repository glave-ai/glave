//! Files a call made for the person (`specs/session.md`, "Files it made"): a document, sheet,
//! deck or PDF a `write` or a command left new or changed in the thread's folder is kept for the
//! thread and named in its result's `structured`; scratch in a hidden folder, files of other
//! kinds and files a command only read are not.

use glave_event::{Body, Event, FileMade, StopReason};
use glave_harness::AllowAll;
use glave_ports::{BlobStore as _, InMemoryLog};
use serde_json::json;

use crate::support::*;

/// What each call's result says it made, by the call's place in the turn.
fn made(events: &[Event]) -> Vec<Vec<FileMade>> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ToolCallCompleted(done) => Some(
                done.structured
                    .as_ref()
                    .map(FileMade::read)
                    .unwrap_or_default(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn what_a_call_wrote_for_the_person_is_kept_with_the_thread() {
    let dir = folder("made");
    block_on!(async {
        std::fs::write(dir.join("given.pdf"), "%PDF-1.7 given").unwrap();
        let calls = [
            (
                "write",
                json!({"path": "table.csv", "content": "a,b\n1,2\n"}),
            ),
            (
                "write",
                json!({"path": "notes.txt", "content": "not a document"}),
            ),
            (
                "write",
                json!({"path": ".cache/scratch.csv", "content": "a\n"}),
            ),
            (
                "bash",
                json!({"command": "printf '%%PDF-1.7 made' > report.pdf"}),
            ),
            ("bash", json!({"command": "cat given.pdf report.pdf"})),
        ];
        let mut answers: Vec<_> = calls
            .into_iter()
            .map(|(tool, args)| answer(None, vec![(tool, args)], StopReason::ToolUse))
            .collect();
        answers.push(answer(Some("Done."), vec![], StopReason::EndTurn));
        let workspace = workspace(&dir).await;
        let (harness, _) = harness_with(
            workspace,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "make a table and a report").await;
        let made = made(&events(harness.log(), thread.id).await);
        let paths: Vec<Vec<&str>> = made
            .iter()
            .map(|call| call.iter().map(|file| file.path.as_str()).collect())
            .collect();
        assert_eq!(
            paths,
            [
                vec!["table.csv"],
                vec![],
                vec![],
                vec!["report.pdf"],
                vec![]
            ]
        );
        let report = &made[3][0].blob;
        assert_eq!(report.media_type, "application/pdf");
        let found = harness.blobs().get(&scope(), &report.sha256).await;
        let mut kept = found.unwrap().unwrap();
        let bytes = glave_ports::read_all(kept.bytes.as_mut(), u64::MAX).await;
        assert_eq!(
            bytes.unwrap(),
            b"%PDF-1.7 made",
            "kept whole, apart from the computer"
        );
    });
}
