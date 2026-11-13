//! A connected service's tool reaches an open world unless it says otherwise, as MCP's default
//! has it (`specs/approvals.md`, "Untrusted content"): in real turns on the test server, a read
//! whose tool says nothing of its world marks the thread untrusted, so a write after it asks in
//! Auto, and a read whose tool says its world is closed leaves the write to the mode.

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use glave_event::{Body, Caution, RawEvent, Risk, ThreadId, decode};
use glave_harness::{ApprovalRequest, Approver, Cancel, Decision, Permissions};
use glave_ports::computer::{Sandbox, SandboxLevel, SandboxNetwork};
use glave_ports::testing::TempDir;
use glave_ports::{InMemoryBlobs, block_on};
use serde_json::json;

use crate::history::{calling, events, harness_in, here, scope, test_server, text};

/// Someone at the keyboard who declines whatever they are asked, and keeps it.
#[derive(Clone, Default)]
struct Declines(Arc<Mutex<Vec<ApprovalRequest>>>);

impl Approver for Declines {
    fn decide(&self, request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        self.0.lock().unwrap().push(request.clone());
        ready(Decision::Deny)
    }

    fn can_answer(&self, _thread: ThreadId) -> bool {
        true
    }
}

#[test]
fn a_read_that_says_nothing_of_its_world_makes_a_later_write_ask_and_a_closed_one_does_not() {
    if !glave_computer::sandbox_available() {
        assert!(
            std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
            "GLAVE_REQUIRE_SANDBOX is set but the sandbox is not available here"
        );
        return;
    }
    block_on!(async {
        let base = TempDir::outside_temp("mcp-open-world").unwrap();
        let project = base.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        // Auto in the sandbox, where a connected service's write asks only after untrusted
        // content.
        let sandbox = Sandbox {
            level: SandboxLevel::Project,
            network: SandboxNetwork::when(false),
            home: Some(base.path().to_owned()),
            glave_home: Some(base.path().join(".glave")),
        };
        let workspace = here(&project).await.with_sandbox(&sandbox).await.unwrap();
        let send = |text: &str| calling(Some(("send", json!({ "text": text }))));
        // One thread reads what it echoed, the next a message someone else wrote; each sends.
        let answers = vec![
            calling(Some(("echo", json!({ "text": "the notes" })))),
            send("after the echo"),
            calling(None),
            calling(Some(("inbox", json!({})))),
            send("after the inbox"),
            calling(None),
        ];
        let person = Declines::default();
        let server = test_server(base.path()).await;
        let mode = (person.clone(), Permissions::Auto);
        let blobs = InMemoryBlobs::new();
        let (harness, _) = harness_in(workspace, blobs, answers, vec![server], mode);
        let mut completed = Vec::new();
        for _ in 0..2 {
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let (cancel, mut heard) = (Cancel::new(), |_| {});
            let turn = harness.run_turn(&mut thread, "mail ana", &cancel, &mut heard);
            turn.await.unwrap();
            completed.push(results(&events(harness.log(), thread.id).await));
        }
        let [echoed, read] = completed.as_slice() else {
            panic!("{completed:?}");
        };
        assert!(!echoed[0].1, "a closed world marks nothing: {echoed:?}");
        assert!(echoed[1].0.contains("Sent: after the echo"), "{echoed:?}");
        assert!(read[0].1, "no word of its world: an open one: {read:?}");
        assert!(read[0].0.contains("accounts@payables.example"), "{read:?}");
        assert!(read[1].0.starts_with("The person declined"), "{read:?}");
        let cautions: Vec<_> = (person.0.lock().unwrap().iter())
            .map(|request| request.caution.clone())
            .collect();
        let tests = Risk::ActsAfterUntrusted {
            service: "tests".into(),
        };
        assert_eq!(
            cautions,
            [Caution::of(Some(tests), None)],
            "only the send after the inbox asked"
        );
    });
}

/// What each call of a thread's `events` gave, as text, and whether it was untrusted.
fn results(events: &[RawEvent]) -> Vec<(String, bool)> {
    (events.iter().cloned())
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::ToolCallCompleted(done) => Some((text(&done.output), done.untrusted)),
            _ => None,
        })
        .collect()
}
