//! Steering a running turn against a scripted model: what the person sends while a tool runs
//! reaches the model at its next call, after the tool's result, recorded under the id it was
//! queued with; several arrive together, in order; what arrives after the model's last call
//! starts the next turn; a stopped turn reads nothing more and gives back what waited; a hook
//! can stop a message as it can a prompt; and the log alone rebuilds what the model saw.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod steering {
    use glave_event::StopReason;
    use glave_event::{Body, Event, EventId, HookPoint, Outcome, Part, PrincipalKind, Role};
    use glave_harness::{
        AllowAll, Caller, Cancel, Hook, Hooks, Inbox, Live, NotQueued, Queued, conversation,
    };
    use glave_ports::{InMemoryLog, Message, ModelRequest};
    use serde_json::json;

    use super::support::*;

    fn queued(n: u8, text: &str) -> Queued {
        Queued {
            id: EventId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap(),
            text: text.into(),
            caller: Caller::default(),
        }
    }

    /// The text of a message the model was sent.
    fn text(message: &Message) -> String {
        let parts = match message {
            Message::User { parts }
            | Message::Tool { output: parts, .. }
            | Message::Assistant { parts, .. } => parts,
        };
        parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// What the model was sent in `request` from its last tool result on.
    fn after_tools(request: &ModelRequest) -> Vec<(&'static str, String)> {
        let from = request
            .messages
            .iter()
            .rposition(|message| matches!(message, Message::Tool { .. }))
            .unwrap();
        request.messages[from..]
            .iter()
            .map(|message| match message {
                // A command's output, without the line of how it exited.
                Message::Tool { .. } => ("tool", text(message).lines().next().unwrap().to_owned()),
                Message::User { .. } => ("person", text(message)),
                Message::Assistant { .. } => ("model", text(message)),
            })
            .collect()
    }

    /// The person's messages in `events`, with the ids they were recorded under.
    fn said(events: &[Event]) -> Vec<(EventId, String)> {
        events
            .iter()
            .filter_map(|event| match &event.body {
                Body::MessageAdded(added) if added.role == Role::User => Some((
                    event.header.id,
                    added
                        .parts
                        .iter()
                        .map(|p| match p {
                            Part::Text { text } => text.clone(),
                            _ => String::new(),
                        })
                        .collect(),
                )),
                _ => None,
            })
            .collect()
    }

    /// Sends what `queue` holds to `inbox` once a tool has finished, as a person typing while
    /// the agent works.
    fn send_after_tool(inbox: Inbox, queue: Vec<Queued>) -> impl FnMut(Live) + Send {
        let mut queue = Some(queue);
        move |live| {
            if matches!(live, Live::ToolFinished { .. })
                && let Some(queue) = queue.take()
            {
                for message in queue {
                    inbox.push(message).unwrap();
                }
            }
        }
    }

    #[test]
    fn what_is_sent_while_a_tool_runs_reaches_the_model_at_its_next_call() {
        let dir = folder("steer-next-call");
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({"command": "echo 3000"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("Set to 4000."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let sent = vec![
                queued(1, "actually, 4000"),
                queued(2, "and keep the comment"),
            ];
            let mut live = send_after_tool(thread.inbox().clone(), sent.clone());
            let summary = harness
                .run_turn(
                    &mut thread,
                    "set the port to 3000",
                    &Cancel::new(),
                    &mut live,
                )
                .await
                .unwrap();
            assert_eq!(summary.outcome, Outcome::Completed);
            assert_eq!(summary.model_calls, 2, "read in the same turn");
            let second = seen.lock().unwrap()[1].clone();
            assert_eq!(
                after_tools(&second),
                [
                    ("tool", "3000".to_owned()),
                    ("person", "actually, 4000".to_owned()),
                    ("person", "and keep the comment".to_owned()),
                ],
                "after the tool's result, together and in order"
            );
            let events = events(harness.log(), thread.id).await;
            let recorded = said(&events);
            assert_eq!(
                recorded[1..],
                [
                    (sent[0].id, "actually, 4000".to_owned()),
                    (sent[1].id, "and keep the comment".to_owned()),
                ],
                "recorded under the ids they were queued with"
            );
            // Recorded where the model saw them: after the result, before its next answer.
            let kinds = kinds(&events);
            let read = kinds
                .iter()
                .position(|kind| *kind == "tool.call.completed")
                .unwrap();
            assert_eq!(
                kinds[read..],
                [
                    "tool.call.completed",
                    "message.added",
                    "message.added",
                    "message.added",
                    "model.responded",
                    "turn.ended"
                ]
            );
            // The log alone gives what the model saw.
            assert_eq!(conversation(&events), thread.messages());
            let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            assert_eq!(resumed.messages(), thread.messages());
        });
    }

    #[test]
    fn what_is_sent_after_the_last_call_starts_the_next_turn() {
        let dir = folder("steer-after-last");
        block_on!(async {
            let answers = vec![
                answer(Some("Here it is."), vec![], StopReason::EndTurn),
                answer(Some("And the other thing."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let late = queued(1, "one more thing");
            let inbox = thread.inbox().clone();
            let mut once = Some(late.clone());
            // Sent while the model writes its last answer.
            let mut live = |live| {
                if matches!(live, Live::Text(_))
                    && let Some(late) = once.take()
                {
                    inbox.push(late).unwrap();
                }
            };
            let cancel = Cancel::new();
            let ran = harness
                .run_turn(&mut thread, "first", &cancel, &mut live)
                .await;
            let first = ran.as_ref().unwrap().clone();
            assert_eq!(first.model_calls, 1, "the turn ended without it");
            let total = harness
                .run_queued(&mut thread, ran, &cancel, &mut live)
                .await
                .unwrap();
            assert_eq!(total.model_calls, 2, "its own turn, added up");
            assert_eq!(total.outcome, Outcome::Completed);
            assert_eq!(seen.lock().unwrap().len(), 2);
            let events = events(harness.log(), thread.id).await;
            let turns = kinds(&events)
                .iter()
                .filter(|k| **k == "turn.started")
                .count();
            assert_eq!(turns, 2);
            assert_eq!(
                said(&events).last(),
                Some(&(late.id, "one more thing".to_owned()))
            );
            assert_eq!(
                thread.inbox().push(queued(2, "later")),
                Err(NotQueued::Ending),
                "nothing waits, so the inbox ended for this work"
            );
        });
    }

    #[test]
    fn a_stopped_turn_reads_nothing_more_and_gives_back_what_waited() {
        let dir = folder("steer-stopped");
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({"command": "echo one"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("never"), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let waiting = queued(1, "and then this");
            let inbox = thread.inbox().clone();
            let cancel = Cancel::new();
            let stopper = cancel.clone();
            let mut unread = Vec::new();
            let mut sent = Some(waiting.clone());
            let mut live = |live| match live {
                Live::ToolFinished { .. } => {
                    if let Some(message) = sent.take() {
                        inbox.push(message).unwrap();
                        stopper.cancel();
                    }
                }
                Live::Unread { id, reason } => unread.push((id, reason)),
                _ => {}
            };
            let ran = harness
                .run_turn(&mut thread, "go", &cancel, &mut live)
                .await;
            assert_eq!(ran.as_ref().unwrap().outcome, Outcome::Cancelled);
            let total = harness
                .run_queued(&mut thread, ran, &cancel, &mut live)
                .await
                .unwrap();
            assert_eq!(total.outcome, Outcome::Cancelled);
            assert_eq!(unread, [(waiting.id, None)]);
            assert_eq!(
                seen.lock().unwrap().len(),
                1,
                "the model was not asked again"
            );
            let events = events(harness.log(), thread.id).await;
            assert!(said(&events).iter().all(|(id, _)| *id != waiting.id));
            // The note about the stop is glave's own words, never the person's.
            let note = events.iter().rev().find_map(|event| match &event.body {
                Body::MessageAdded(added) if added.role == Role::User => Some(added.clone()),
                _ => None,
            });
            assert_eq!(
                note.and_then(|n| n.author).map(|a| a.kind),
                Some(PrincipalKind::Glave)
            );
        });
    }

    #[test]
    fn a_hook_stops_a_message_sent_mid_turn_as_it_stops_a_prompt() {
        let dir = folder("steer-hook");
        let guard = "grep -q secret && { echo 'that looks like a secret'; exit 2; }; exit 0";
        let hook = Hook {
            point: HookPoint::UserPromptSubmit,
            command: guard.into(),
            matcher: None,
            timeout_secs: None,
        };
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({"command": "echo one"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let harness = harness.with_hooks(Hooks::new(vec![hook]).unwrap());
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let (blocked, passed) = (queued(1, "my secret is 42"), queued(2, "go on"));
            let inbox = thread.inbox().clone();
            let mut sent = Some(vec![blocked.clone(), passed.clone()]);
            let mut unread = Vec::new();
            let mut live = |live| match live {
                Live::ToolFinished { .. } => {
                    for message in sent.take().unwrap_or_default() {
                        inbox.push(message).unwrap();
                    }
                }
                Live::Unread { id, reason } => unread.push((id, reason)),
                _ => {}
            };
            harness
                .run_turn(&mut thread, "start", &Cancel::new(), &mut live)
                .await
                .unwrap();
            assert_eq!(
                unread,
                [(blocked.id, Some("that looks like a secret".to_owned()))]
            );
            let seen = seen.lock().unwrap();
            assert_eq!(
                after_tools(&seen[1]),
                [("tool", "one".to_owned()), ("person", "go on".to_owned())]
            );
        });
    }
}
