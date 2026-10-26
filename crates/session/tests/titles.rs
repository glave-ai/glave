//! Titles: after a thread's first exchange a model writes it a short title, recorded as
//! `thread.renamed` by glave once the turn has answered, with the call's usage counted in the
//! thread. A title the person gave is never replaced; `titles = false` keeps the first line of
//! the first message; a failed call records nothing; a thread past its first exchange keeps
//! the title it has; a first turn the person steered is titled from its first message; and a
//! turn started while the title is recorded runs.

use std::sync::Arc;

use glave_event::{Body, PrincipalKind, ThreadId, decode};
use glave_ports::testing::FaultyLog;
use glave_ports::{Completion, EventLog};
use glave_session::protocol::{Reply, Request, ThreadSummary};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::support::*;

const ASKED: &str = "How do I set up Postgres on my Mac?";

fn setup(answers: Vec<Completion>, titles: bool) -> Setup {
    let mut setup = Setup::new(answers);
    setup.settings.titles = titles;
    setup
}

/// The reply to the first message, and the title the model writes, each with its usage.
fn exchange() -> Vec<Completion> {
    vec![
        costing(answer(Some("Install it with Homebrew."), None), 100, 10, 50),
        costing(answer(Some("\"Postgres setup on macOS.\""), None), 40, 5, 7),
    ]
}

async fn summary<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> ThreadSummary {
    match server.request(Request::ThreadGet { thread }).await.unwrap() {
        Reply::Thread { thread } => thread,
        other => panic!("a thread, not {other:?}"),
    }
}

/// The thread's renames: who gave each title, and the title.
async fn renames<L: EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
) -> Vec<(Option<PrincipalKind>, String)> {
    events(server, thread)
        .await
        .into_iter()
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::ThreadRenamed(renamed) => Some((renamed.by.map(|by| by.kind), renamed.title)),
            _ => None,
        })
        .collect()
}

/// The purpose of each request the model received.
fn purposes(seen: &Seen) -> Vec<String> {
    json(seen)
        .iter()
        .map(|json| {
            let request: Value = serde_json::from_str(json).unwrap();
            request["purpose"].as_str().unwrap_or("main").to_owned()
        })
        .collect()
}

#[test]
fn a_title_is_recorded_after_the_first_reply_and_its_call_counts() {
    let dir = folder("titles-first");
    block_on!(async {
        let (server, seen) = setup(exchange(), true).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, ASKED).await.unwrap();
        server.shutdown().await;
        assert_eq!(purposes(&seen), ["main", "title"]);
        let titling = json(&seen)[1].clone();
        assert!(titling.contains(ASKED), "{titling}");
        assert!(titling.contains("Install it with Homebrew."), "{titling}");
        let kinds = kinds(&server, thread).await;
        let ended = kinds.iter().position(|kind| kind == "turn.ended").unwrap();
        assert_eq!(
            kinds.get(ended..).unwrap(),
            ["turn.ended", "model.responded", "thread.renamed"]
        );
        assert_eq!(
            renames(&server, thread).await,
            [(
                Some(PrincipalKind::Glave),
                "Postgres setup on macOS".to_owned()
            )]
        );
        let shown = summary(&server, thread).await;
        assert_eq!(shown.title.as_deref(), Some("Postgres setup on macOS"));
        assert_eq!((shown.tokens.input, shown.tokens.output), (140, 15));
        assert_eq!(shown.cost.map(|cost| cost.micros), Some(57));
    });
}

#[test]
fn a_title_the_person_gave_is_never_replaced() {
    let dir = folder("titles-renamed");
    block_on!(async {
        let (server, seen) = setup(exchange(), true).build(&dir).await;
        let thread = start(&server, false).await;
        server
            .request(Request::ThreadRename {
                thread,
                title: "Mine".into(),
            })
            .await
            .unwrap();
        turn(&server, thread, ASKED).await.unwrap();
        server.shutdown().await;
        assert_eq!(purposes(&seen), ["main"]);
        assert_eq!(
            summary(&server, thread).await.title.as_deref(),
            Some("Mine")
        );
        assert_eq!(renames(&server, thread).await.len(), 1);
    });
}

#[test]
fn with_titles_off_the_first_message_titles_the_thread() {
    let dir = folder("titles-off");
    block_on!(async {
        let (server, seen) = setup(exchange(), false).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, ASKED).await.unwrap();
        server.shutdown().await;
        assert_eq!(purposes(&seen), ["main"]);
        assert_eq!(renames(&server, thread).await, []);
        assert_eq!(summary(&server, thread).await.title.as_deref(), Some(ASKED));
    });
}

#[test]
fn a_failed_call_records_nothing_and_the_first_message_titles_the_thread() {
    let dir = folder("titles-failed");
    block_on!(async {
        // The script has no answer for the title: the call fails.
        let reply = vec![answer(Some("Install it with Homebrew."), None)];
        let (server, seen) = setup(reply, true).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, ASKED).await.unwrap();
        server.shutdown().await;
        assert_eq!(purposes(&seen), ["main", "title"]);
        assert_eq!(kinds(&server, thread).await.last().unwrap(), "turn.ended");
        assert_eq!(summary(&server, thread).await.title.as_deref(), Some(ASKED));
    });
}

#[test]
fn a_thread_past_its_first_exchange_keeps_its_title() {
    let dir = folder("titles-old");
    let log = Shared::default();
    block_on!(async {
        let first = vec![answer(Some("Install it with Homebrew."), None)];
        let (before, _) = setup(first, false).build_on(&dir, log.clone()).await;
        let thread = start(&before, false).await;
        turn(&before, thread, ASKED).await.unwrap();
        before.shutdown().await;
        let second = vec![answer(Some("Then run initdb."), None)];
        let (after, seen) = setup(second, true).build_on(&dir, log.clone()).await;
        turn(&after, thread, "And then?").await.unwrap();
        after.shutdown().await;
        assert_eq!(purposes(&seen), ["main"]);
        assert_eq!(renames(&after, thread).await, []);
        assert_eq!(summary(&after, thread).await.title.as_deref(), Some(ASKED));
    });
}

#[test]
fn a_first_turn_steered_while_it_ran_is_titled_from_its_first_message() {
    let dir = folder("titles-steered");
    block_on!(async {
        let answers = vec![
            answer(None, Some(("bash", json!({"command": "echo one"})))),
            answer(Some("Install it with Homebrew."), None),
            answer(Some("\"Postgres setup on macOS.\""), None),
        ];
        let gate = Arc::new(Notify::new());
        let mut setup = setup(answers, true);
        setup.gate = Some(Arc::clone(&gate));
        let (server, seen) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        let mut first = stream_turn(&server, thread, ASKED, None).await.unwrap();
        asked(&seen, 1).await;
        let steered = server
            .request(turn_start(thread, "with Postgres 17", None))
            .await;
        assert!(matches!(steered, Ok(Reply::Queued { .. })), "{steered:?}");
        // The command's call, the reply that read the message, then the title.
        for calls in [2, 3] {
            gate.notify_one();
            asked(&seen, calls).await;
        }
        gate.notify_one();
        first.finish().await.unwrap();
        server.shutdown().await;
        assert_eq!(purposes(&seen), ["main", "main", "title"]);
        let titling = json(&seen)[2].clone();
        assert!(titling.contains(ASKED), "{titling}");
        assert!(!titling.contains("with Postgres 17"), "{titling}");
        assert_eq!(
            renames(&server, thread).await,
            [(
                Some(PrincipalKind::Glave),
                "Postgres setup on macOS".to_owned()
            )]
        );
    });
}

#[test]
fn a_turn_started_while_the_title_is_recorded_runs() {
    let dir = folder("titles-meanwhile");
    block_on!(async {
        let log = FaultyLog::default();
        log.hold("thread.renamed");
        let mut answers = exchange();
        answers.push(costing(answer(Some("Then run initdb."), None), 20, 3, 9));
        let (server, seen) = setup(answers, true).build_on(&dir, log.clone()).await;
        let thread = start(&server, false).await;
        turn(&server, thread, ASKED).await.unwrap();
        // The title is being recorded: its append has begun and not yet landed.
        log.held().await;
        let second = turn(&server, thread, "And then?").await;
        log.release();
        server.shutdown().await;
        assert!(second.is_ok(), "{second:?}");
        assert_eq!(purposes(&seen), ["main", "title", "main"]);
        // Numbered 1, 2, 3 with no gap: the title was written after the second turn.
        let kinds = kinds(&server, thread).await;
        assert_eq!(kinds.iter().filter(|kind| *kind == "turn.ended").count(), 2);
        assert_eq!(
            kinds.get(kinds.len() - 3..).unwrap(),
            ["turn.ended", "model.responded", "thread.renamed"]
        );
        assert_eq!(
            renames(&server, thread).await,
            [(
                Some(PrincipalKind::Glave),
                "Postgres setup on macOS".to_owned()
            )]
        );
        let shown = summary(&server, thread).await;
        assert_eq!((shown.tokens.input, shown.tokens.output), (160, 18));
        assert_eq!(shown.cost.map(|cost| cost.micros), Some(66));
    });
}

#[test]
fn a_rename_while_the_title_is_recorded_wins() {
    let dir = folder("titles-renamed-meanwhile");
    block_on!(async {
        let log = FaultyLog::default();
        log.hold("thread.renamed");
        let (server, _) = setup(exchange(), true).build_on(&dir, log.clone()).await;
        let thread = start(&server, false).await;
        turn(&server, thread, ASKED).await.unwrap();
        log.held().await;
        let renamed = server
            .request(Request::ThreadRename {
                thread,
                title: "Mine".into(),
            })
            .await;
        log.release();
        server.shutdown().await;
        assert!(renamed.is_ok(), "{renamed:?}");
        // The title's append met the rename, read the thread again and kept only its usage.
        let kinds = kinds(&server, thread).await;
        assert_eq!(
            kinds.get(kinds.len() - 2..).unwrap(),
            ["thread.renamed", "model.responded"]
        );
        assert_eq!(renames(&server, thread).await, [(None, "Mine".to_owned())]);
        let shown = summary(&server, thread).await;
        assert_eq!(shown.title.as_deref(), Some("Mine"));
        assert_eq!((shown.tokens.input, shown.tokens.output), (140, 15));
    });
}
