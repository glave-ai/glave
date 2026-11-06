//! Where a person's connections are used (`specs/policy.md`, "Connections"; `specs/invariants.md`,
//! 29): a connection used only in one place is neither listed nor called in any other, the
//! person's Chats and a project alike; moved away mid-thread its calls are refused at once,
//! and turned on mid-thread it comes with a new prefix, once.

use std::sync::Arc;

use glave_event::StopReason;
use glave_harness::{AllowAll, OwnTools};
use glave_ports::{InMemoryLog, ModelRequest, OrgId, ProjectId, Scope, ToolSource};
use serde_json::json;

use crate::project_folders::tool;
use crate::support::*;

/// A place of the person's: their Chats, or a project.
fn place(n: u8) -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(uuid_like(1)),
        ProjectId::from_uuid(uuid_like(n)),
    )
}

/// The person's tools: mail everywhere, and `only`'s connection in `at` alone.
fn own(only: &str, at: Scope) -> OwnTools {
    let mail = Arc::new(tool("mcp__mail__send", "Sent.")) as Arc<dyn ToolSource>;
    let theirs = Arc::new(tool(only, "Done there.")) as Arc<dyn ToolSource>;
    OwnTools::everywhere(vec![Arc::clone(&mail)]).with_place(at.project(), vec![mail, theirs])
}

fn lists(request: &ModelRequest, name: &str) -> bool {
    request.tools.iter().any(|spec| spec.name == name)
}

/// One call of `tool`, then the answer after it.
fn calls(tool: &str) -> [glave_ports::Completion; 2] {
    [
        answer(None, vec![(tool, json!({}))], StopReason::ToolUse),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ]
}

#[test]
fn a_chat_never_reaches_a_project_s_connection_nor_a_project_a_chat_s() {
    let dir = folder("places");
    let (chats, acme) = (place(2), place(3));
    block_on!(async {
        let answers = [
            calls("mcp__stripe__charge"),
            calls("mcp__stripe__charge"),
            calls("mcp__notes__add"),
            calls("mcp__notes__add"),
        ];
        let answers = answers.into_iter().flatten().collect();
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, AllowAll, log, 1, settings()).await;
        // Stripe only in the project.
        harness.set_own_tools(own("mcp__stripe__charge", acme));
        let mut chat = harness.start_thread(chats).await.unwrap();
        turn(&harness, &mut chat, "charge them").await;
        let mut project = harness.start_thread(acme).await.unwrap();
        turn(&harness, &mut project, "charge them").await;
        // And the reverse: notes only in Chats.
        harness.set_own_tools(own("mcp__notes__add", chats));
        let mut project = harness.start_thread(acme).await.unwrap();
        turn(&harness, &mut project, "note it").await;
        let mut chat = harness.start_thread(chats).await.unwrap();
        turn(&harness, &mut chat, "note it").await;

        let requests = seen.lock().unwrap().clone();
        assert!(!lists(&requests[0], "mcp__stripe__charge"), "not in a chat");
        assert!(lists(&requests[0], "mcp__mail__send"), "everywhere is everywhere");
        assert!(lists(&requests[2], "mcp__stripe__charge"), "in its project");
        assert!(!lists(&requests[4], "mcp__notes__add"), "not in a project");
        assert!(lists(&requests[6], "mcp__notes__add"), "in Chats");
        let said = |at: usize| results(requests[at].messages.as_slice()).join("\n");
        assert!(said(1).contains("There is no tool named"), "{}", said(1));
        assert_eq!(said(3), "Done there.");
        assert!(said(5).contains("There is no tool named"), "{}", said(5));
        assert_eq!(said(7), "Done there.");
    });
}

#[test]
fn moved_away_mid_thread_its_calls_are_refused_and_turned_on_it_comes_once() {
    let dir = folder("places-moved");
    let (chats, acme) = (place(2), place(3));
    block_on!(async {
        let said = ["Hi.", "Again.", "Still."];
        let said = said.map(|text| answer(Some(text), vec![], StopReason::EndTurn));
        let answers = [calls("mcp__stripe__charge"), calls("mcp__stripe__charge")];
        let answers = answers.into_iter().flatten().chain(said).collect();
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, AllowAll, log, 1, settings()).await;
        harness.set_own_tools(own("mcp__stripe__charge", acme));
        let mut project = harness.start_thread(acme).await.unwrap();
        turn(&harness, &mut project, "charge them").await;
        // Used only in Chats now: still listed in the project's thread, whose prefix never
        // changes for it, and refused there.
        harness.set_own_tools(own("mcp__stripe__charge", chats));
        turn(&harness, &mut project, "charge them again").await;
        let mut chat = harness.start_thread(chats).await.unwrap();
        harness.set_own_tools(OwnTools::default());
        turn(&harness, &mut chat, "hello").await;
        // Turned on for Chats mid-thread: listed from the next turn, with a new prefix.
        harness.set_own_tools(own("mcp__stripe__charge", chats));
        turn(&harness, &mut chat, "and now?").await;
        turn(&harness, &mut chat, "still?").await;

        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests[2].tools, requests[0].tools, "the prefix stayed");
        let refused = results(requests[3].messages.as_slice()).join("\n");
        assert!(refused.contains("is turned off here"), "{refused}");
        assert!(!lists(&requests[4], "mcp__stripe__charge"));
        assert!(lists(&requests[5], "mcp__stripe__charge"), "came at once");
        assert!(lists(&requests[5], "mcp__mail__send"));
        assert_eq!(requests[6].tools, requests[5].tools, "once");
    });
}
