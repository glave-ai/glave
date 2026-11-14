//! glave's `web_search` in a turn (`specs/session.md`, "Web search"): offered where the provider
//! searches and the person has it on, a call searched through the provider as the thread's work,
//! its line saying what was searched, what it found untrusted.

use glave_event::{Body, StopReason, ToolKind};
use glave_harness::AllowAll;
use glave_ports::{Hit, InMemoryLog, Message, Purpose, Searched};
use serde_json::json;

use crate::support::*;

fn found() -> Searched {
    Searched {
        hits: vec![Hit {
            title: "Tokio".into(),
            url: "https://tokio.rs/".into(),
            snippet: "An asynchronous runtime for Rust.".into(),
        }],
        cost: None,
    }
}

fn searches(query: &str) -> Vec<glave_ports::Completion> {
    vec![
        answer(
            None,
            vec![("web_search", json!({"query": query, "count": 3}))],
            StopReason::ToolUse,
        ),
        answer(Some("Tokio."), Vec::new(), StopReason::EndTurn),
    ]
}

#[test]
fn a_search_is_the_thread_s_work_said_in_a_line_and_its_results_untrusted() {
    let dir = folder("web-search");
    block_on!(async {
        let script = ScriptedModel::new(searches("rust async runtimes")).searching(Ok(found()));
        let asked = script.seen_searched();
        let (harness, seen) = scripted(
            workspace(&dir).await,
            script,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "which async runtimes are there?").await;

        let (request, ask) = asked.lock().unwrap().first().cloned().unwrap();
        assert_eq!(request.purpose, Purpose::WebSearch);
        assert!(request.payer.is_some(), "the thread's work, charged as it");
        assert_eq!((ask.query.as_str(), ask.count), ("rust async runtimes", 3));

        let requests = seen.lock().unwrap().clone();
        let offered: Vec<&str> = requests[0].tools.iter().map(|t| t.name.as_str()).collect();
        assert!(offered.contains(&"web_search"), "{offered:?}");
        let Some(Message::Tool { is_error, .. }) = requests[1].messages.last() else {
            panic!("the result is sent back");
        };
        assert!(!is_error);
        let sent = results(&requests[1].messages).join("\n");
        assert!(sent.contains("https://tokio.rs/"), "{sent}");

        let log = events(harness.log(), thread.id).await;
        let requested = log.iter().find_map(|event| match &event.body {
            Body::ToolCallRequested(call) => Some(call.clone()),
            _ => None,
        });
        let requested = requested.unwrap();
        assert_eq!(requested.summary, "Searched the web: rust async runtimes");
        assert_eq!(requested.kind, ToolKind::Web);
        let completed = log.iter().find_map(|event| match &event.body {
            Body::ToolCallCompleted(done) => Some(done.clone()),
            _ => None,
        });
        assert!(
            completed.unwrap().untrusted,
            "what the web says is untrusted"
        );
    });
}

#[test]
fn search_is_offered_only_where_the_provider_searches_and_the_person_has_it_on() {
    let dir = folder("web-search-off");
    block_on!(async {
        let ends = || vec![answer(Some("Hi."), Vec::new(), StopReason::EndTurn)];
        let offered = |requests: &[glave_ports::ModelRequest]| {
            requests[0]
                .tools
                .iter()
                .any(|tool| tool.name == "web_search")
        };
        let (none, seen) = scripted(
            workspace(&dir).await,
            ScriptedModel::new(ends()),
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let mut thread = none.start_thread(scope()).await.unwrap();
        turn(&none, &mut thread, "hi").await;
        assert!(
            !offered(&seen.lock().unwrap()),
            "the provider searches nothing"
        );

        let script = ScriptedModel::new(ends()).searching(Ok(found()));
        let (off, seen) = scripted(
            workspace(&dir).await,
            script,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        off.set_searching(false);
        let mut thread = off.start_thread(scope()).await.unwrap();
        turn(&off, &mut thread, "hi").await;
        assert!(!offered(&seen.lock().unwrap()), "the person turned it off");
    });
}
