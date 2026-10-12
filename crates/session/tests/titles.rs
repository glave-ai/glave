//! Titles: after a thread's first exchange a model writes it a short title, recorded as
//! `thread.renamed` by glave once the turn has answered, with the call's usage counted in the
//! thread. A title the person gave is never replaced; `titles = false` keeps the first line of
//! the first message; a failed call records nothing; and a thread past its first exchange
//! keeps the title it has.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/tasks.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod helpers;

#[cfg(test)]
mod titles {
    use glave_event::{Body, PrincipalKind, ThreadId, decode};
    use glave_ports::Completion;
    use glave_session::protocol::{Reply, Request, ThreadSummary};
    use serde_json::Value;

    use super::helpers::*;
    use super::support::*;

    const ASKED: &str = "How do I set up Postgres on my Mac?";

    fn setup(answers: Vec<Completion>, log: &Shared, titles: bool) -> Setup {
        let mut setup = Setup::new(answers);
        setup.log = log.clone();
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

    async fn ask(server: &Tested, thread: ThreadId, prompt: &str) {
        server
            .request(Request::TurnStart {
                thread,
                prompt: prompt.into(),
                background: false,
                budget: None,
                meta: glave_session::protocol::Meta::default(),
                model: None,
                approval: None,
            })
            .await
            .unwrap();
    }

    async fn summary(server: &Tested, thread: ThreadId) -> ThreadSummary {
        match server.request(Request::ThreadGet { thread }).await.unwrap() {
            Reply::Thread { thread } => thread,
            other => panic!("a thread, not {other:?}"),
        }
    }

    /// The thread's renames: who gave each title, and the title.
    async fn renames(server: &Tested, thread: ThreadId) -> Vec<(Option<PrincipalKind>, String)> {
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
    fn purposes(seen: &std::sync::Mutex<Vec<String>>) -> Vec<String> {
        seen.lock()
            .unwrap()
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
        block_on(async {
            let (server, seen) = setup(exchange(), &Shared::default(), true)
                .build(&dir)
                .await;
            let thread = start(&server, false).await;
            ask(&server, thread, ASKED).await;
            server.shutdown().await;
            assert_eq!(purposes(&seen), ["main", "title"]);
            let titling = seen.lock().unwrap()[1].clone();
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
        block_on(async {
            let (server, seen) = setup(exchange(), &Shared::default(), true)
                .build(&dir)
                .await;
            let thread = start(&server, false).await;
            server
                .request(Request::ThreadRename {
                    thread,
                    title: "Mine".into(),
                })
                .await
                .unwrap();
            ask(&server, thread, ASKED).await;
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
        block_on(async {
            let (server, seen) = setup(exchange(), &Shared::default(), false)
                .build(&dir)
                .await;
            let thread = start(&server, false).await;
            ask(&server, thread, ASKED).await;
            server.shutdown().await;
            assert_eq!(purposes(&seen), ["main"]);
            assert_eq!(renames(&server, thread).await, []);
            assert_eq!(summary(&server, thread).await.title.as_deref(), Some(ASKED));
        });
    }

    #[test]
    fn a_failed_call_records_nothing_and_the_first_message_titles_the_thread() {
        let dir = folder("titles-failed");
        block_on(async {
            // The script has no answer for the title: the call fails.
            let reply = vec![answer(Some("Install it with Homebrew."), None)];
            let (server, seen) = setup(reply, &Shared::default(), true).build(&dir).await;
            let thread = start(&server, false).await;
            ask(&server, thread, ASKED).await;
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
        block_on(async {
            let first = vec![answer(Some("Install it with Homebrew."), None)];
            let (before, _) = setup(first, &log, false).build(&dir).await;
            let thread = start(&before, false).await;
            ask(&before, thread, ASKED).await;
            before.shutdown().await;
            let second = vec![answer(Some("Then run initdb."), None)];
            let (after, seen) = setup(second, &log, true).build(&dir).await;
            ask(&after, thread, "And then?").await;
            after.shutdown().await;
            assert_eq!(purposes(&seen), ["main"]);
            assert_eq!(renames(&after, thread).await, []);
            assert_eq!(summary(&after, thread).await.title.as_deref(), Some(ASKED));
        });
    }
}
