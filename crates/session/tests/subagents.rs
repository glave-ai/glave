//! Subagents through the session API: a child's notifications reach whoever follows the parent,
//! each under the child's thread, and a child's approval is answered there.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod subagents {
    use glave_event::{Body, Outcome, decode};
    use glave_harness::Settings;
    use glave_ports::MemoryLog;
    use glave_session::protocol::{Answer, Notification, Request, Update};
    use glave_session::{Answering, TurnItem};
    use serde_json::json;

    use super::support::*;

    #[test]
    fn a_childs_approval_reaches_the_parents_follower_and_is_answered_there() {
        let dir = folder("session-subagents");
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    Some((
                        "agent",
                        json!({"description": "touch it", "prompt": "Run: touch ran.txt"}),
                    )),
                ),
                answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                answer(Some("Touched ran.txt."), None),
                answer(Some("Done."), None),
            ];
            let mut settings = Settings::new("test-model", "script");
            settings.subagents = true;
            settings.approval = glave_harness::ApprovalMode::Ask;
            let server = server_with(
                &dir,
                answers,
                None,
                MemoryLog::new(),
                Answering::Clients,
                1,
                settings,
            )
            .await;
            let parent = start(&server, false).await;
            let mut stream = stream_turn(&server, parent, "touch it", None)
                .await
                .unwrap();
            let mut subagent = None;
            let mut answered = false;
            let mut child_events = Vec::new();
            let mut ended = None;
            while let Some(item) = stream.next().await {
                match item {
                    TurnItem::Note(note) => match note.as_ref() {
                        Notification::Live {
                            update:
                                Update::SubagentStarted {
                                    subagent: child, ..
                                },
                            ..
                        } => subagent = Some(*child),
                        Notification::Event { thread, event } if *thread != parent => {
                            child_events.push(event.kind.as_str().to_owned());
                            if let Body::ApprovalRequested(asked) =
                                decode(event.clone()).unwrap().body
                            {
                                assert_eq!(Some(*thread), subagent, "the child's own thread");
                                let decide = Request::ApprovalDecide {
                                    thread: *thread,
                                    approval: asked.call_id,
                                    decision: Answer::AllowOnce,
                                };
                                server.request(decide).await.unwrap();
                                answered = true;
                            }
                        }
                        _ => {}
                    },
                    TurnItem::Ended(result) => ended = Some(result),
                    TurnItem::Missed(_) => {}
                }
            }
            assert!(answered, "the child's approval was seen and answered");
            assert_eq!(
                child_events.first().map(String::as_str),
                Some("thread.started")
            );
            assert_eq!(child_events.last().map(String::as_str), Some("turn.ended"));
            assert!(dir.join("ran.txt").exists());
            assert!(matches!(ended, Some(Ok(turn)) if turn.outcome == Outcome::Completed));
        });
    }
}
