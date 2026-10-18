//! Goals against a scripted model: a goal keeps the thread working across turns until the
//! model marks it complete or paused, the person stops it, its budget runs out, or it stalls;
//! a paused goal resumes; it survives a restart; and its tool is offered only during its
//! turns.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod goals {
    use glave_event::{Body, Event, GoalReason, Part, PrincipalKind, StopReason};
    use glave_harness::{AllowAll, Cancel, Goal, UPDATE_GOAL};
    use glave_ports::Caller;
    use glave_ports::{Completion, EventLog, InMemoryLog, Message, ModelRequest};
    use serde_json::json;

    use super::support::*;

    fn done(text: &str) -> Completion {
        answer(Some(text), vec![], StopReason::EndTurn)
    }

    fn call(tool: &str, args: serde_json::Value) -> Completion {
        answer(None, vec![(tool, args)], StopReason::ToolUse)
    }

    fn complete() -> Completion {
        call(UPDATE_GOAL, json!({"status": "complete"}))
    }

    fn ended(events: &[Event]) -> Vec<GoalReason> {
        events
            .iter()
            .filter_map(|event| match &event.body {
                Body::GoalEnded(ended) => Some(ended.reason),
                _ => None,
            })
            .collect()
    }

    /// The text of every user message glave wrote itself: the continuations.
    fn continuations(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match &event.body {
                Body::MessageAdded(added)
                    if added
                        .author
                        .as_ref()
                        .is_some_and(|author| author.kind == PrincipalKind::Glave) =>
                {
                    added.parts.iter().find_map(|part| match part {
                        Part::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn offers(request: &ModelRequest, tool: &str) -> bool {
        request.tools.iter().any(|spec| spec.name == tool)
    }

    fn goal(objective: &str, budget: u64) -> Goal {
        Goal {
            objective: objective.into(),
            token_budget: budget,
        }
    }

    #[test]
    fn the_model_verifies_then_says_the_goal_is_met() {
        let dir = folder("goal-met");
        block_on!(async {
            let answers = vec![
                done("I looked around."),
                call("write", json!({"path": "done.txt", "content": "yes\n"})),
                call("read", json!({"path": "done.txt"})),
                complete(),
                done("Wrote it and read it back."),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let objective = "make done.txt; read it back to check";
            let summary = harness
                .start_goal(
                    &mut thread,
                    goal(objective, 1_000_000),
                    &Caller::default(),
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            assert_eq!(summary.model_calls, 5);
            let events = events(harness.log(), thread.id).await;
            assert_eq!(ended(&events), [GoalReason::Met]);
            let kinds = kinds(&events);
            assert_eq!(kinds.get(1), Some(&"goal.set"));
            assert_eq!(kinds.last(), Some(&"goal.ended"));
            let continued = continuations(&events);
            assert_eq!(continued.len(), 1, "one continuation: {continued:?}");
            let first = continued.first().unwrap();
            assert!(first.contains(&format!("<objective>\n{objective}\n</objective>")));
            assert!(first.contains("verify every requirement"));
            let seen = seen.lock().unwrap();
            assert!(seen.iter().all(|request| offers(request, UPDATE_GOAL)));
            // The tool and the continuation carry the goal; the system prompt stays as it is.
            assert!(
                seen.iter()
                    .all(|request| !request.system.contains("# Goal"))
            );
            // The person's objective opens the goal; glave writes only the continuation.
            let opening = seen.first().unwrap().messages.last().unwrap();
            assert!(
                matches!(opening, Message::User { parts } if parts.iter().any(
                |part| matches!(part, Part::Text { text } if text == objective)))
            );
            let progress = thread.goal().unwrap();
            assert_eq!(progress.ended, Some(GoalReason::Met));
            assert_eq!(progress.turns, 2);
            assert_eq!(progress.tokens, 550);
        });
    }

    #[test]
    fn the_model_pauses_with_a_reason_and_the_person_resumes_the_goal() {
        let dir = folder("goal-pause");
        block_on!(async {
            let answers = vec![
                call(
                    UPDATE_GOAL,
                    json!({"status": "paused", "reason": "Which port?"}),
                ),
                done("I need the port to go on."),
                done("Noted: 8080."),
                done("Hi."),
                complete(),
                done("Serving on 8080."),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let caller = Caller::default();
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let serve = goal("serve it", 1_000_000);
            harness
                .start_goal(&mut thread, serve, &caller, &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let paused = harness.goal(scope(), thread.id).await.unwrap().unwrap();
            assert_eq!(paused.ended, Some(GoalReason::Paused));
            assert_eq!(paused.note.as_deref(), Some("Which port?"));
            assert!(paused.resumable() && !paused.active());
            assert!(!harness.stop_goal(&mut thread, &caller).await.unwrap());

            // The person answers in a plain turn, which looks like any turn without a goal.
            harness
                .run_turn(&mut thread, "Use 8080.", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let mut fresh = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut fresh, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            {
                let seen = seen.lock().unwrap();
                let (plain, other) = (seen.get(2).unwrap(), seen.get(3).unwrap());
                assert!(!offers(plain, UPDATE_GOAL));
                assert_eq!((&plain.tools, &plain.system), (&other.tools, &other.system));
            }

            harness
                .continue_goal(&mut thread, &caller, &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let events = events(harness.log(), thread.id).await;
            assert_eq!(ended(&events), [GoalReason::Paused, GoalReason::Met]);
            assert!(kinds(&events).contains(&"goal.resumed"));
            let note = events.iter().find_map(|event| match &event.body {
                Body::GoalEnded(ended) => ended.note.clone(),
                _ => None,
            });
            assert_eq!(note.as_deref(), Some("Which port?"));
            let read = harness.goal(scope(), thread.id).await.unwrap().unwrap();
            assert_eq!(Some(&read), thread.goal(), "the log and the thread agree");
            assert_eq!((read.turns, read.tokens, read.note), (2, 440, None));
        });
    }

    #[test]
    fn a_goal_stops_when_its_budget_runs_out() {
        let dir = folder("goal-budget");
        std::fs::write(dir.join("a.md"), "a\n").unwrap();
        block_on!(async {
            // Every model call uses 110 tokens; the budget is 250.
            let answers = vec![
                call("read", json!({"path": "a.md"})),
                done("Not yet."),
                call("glob", json!({"pattern": "*"})),
                done("Out of budget: here is where I got to."),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .start_goal(
                    &mut thread,
                    goal("the impossible", 250),
                    &Caller::default(),
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            let events = events(harness.log(), thread.id).await;
            assert_eq!(ended(&events), [GoalReason::Budget]);
            let seen = seen.lock().unwrap();
            assert_eq!(
                seen.len(),
                4,
                "the turn that spends the budget gets a last text step"
            );
            let last = seen.last().unwrap();
            assert_eq!(last.tool_choice, glave_ports::ToolChoice::None);
            assert_eq!(thread.goal().unwrap().tokens, 440);
        });
    }

    #[test]
    fn cancelling_stops_the_goal_and_stopping_an_idle_goal_ends_it() {
        let dir = folder("goal-stop");
        block_on!(async {
            let harness = harness(&dir, vec![], AllowAll).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let cancel = Cancel::new();
            cancel.cancel();
            let caller = Caller::default();
            harness
                .start_goal(&mut thread, goal("x", 1_000), &caller, &cancel, &mut |_| {})
                .await
                .unwrap();
            assert_eq!(
                ended(&events(harness.log(), thread.id).await),
                [GoalReason::Stopped]
            );
            assert!(!harness.stop_goal(&mut thread, &caller).await.unwrap());
        });
        let dir = folder("goal-stop-idle");
        block_on!(async {
            // The model fails mid-goal: the goal stays active until the person stops it.
            let harness = harness(&dir, vec![done("Started.")], AllowAll).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let caller = Caller::default();
            let failed = harness
                .start_goal(
                    &mut thread,
                    goal("x", 1_000),
                    &caller,
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await;
            assert!(failed.is_err());
            assert!(thread.goal().unwrap().active());
            assert!(harness.stop_goal(&mut thread, &caller).await.unwrap());
            let read = harness.goal(scope(), thread.id).await.unwrap().unwrap();
            assert_eq!(read.ended, Some(GoalReason::Stopped));
        });
    }

    #[test]
    fn a_goal_survives_a_restart_and_continues_from_the_log() {
        let dir = folder("goal-resume");
        block_on!(async {
            let first = harness(&dir, vec![done("Started.")], AllowAll).await;
            let mut thread = first.start_thread(scope()).await.unwrap();
            let caller = Caller::default();
            let crashed = first
                .start_goal(
                    &mut thread,
                    goal("ship it", 1_000_000),
                    &caller,
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await;
            assert!(crashed.is_err(), "the script runs out mid-goal");
            // A new process on the same log.
            let log = InMemoryLog::new();
            let stored = first
                .log()
                .read(&scope(), thread.id, None, usize::MAX)
                .await
                .unwrap();
            log.append(&scope(), &stored).await.unwrap();

            let (second, seen) = harness_seeing(
                &dir,
                vec![complete(), done("Shipped.")],
                AllowAll,
                log,
                2,
                settings(),
            )
            .await;
            let mut resumed = second.resume_thread(scope(), thread.id).await.unwrap();
            let progress = resumed.goal().unwrap();
            assert!(progress.active());
            assert_eq!((progress.turns, progress.tokens), (2, 110));
            second
                .continue_goal(&mut resumed, &caller, &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let events = events(second.log(), thread.id).await;
            assert_eq!(ended(&events), [GoalReason::Met]);
            assert!(!kinds(&events).contains(&"goal.resumed"), "it never paused");
            let opening = seen
                .lock()
                .unwrap()
                .first()
                .unwrap()
                .messages
                .last()
                .cloned()
                .unwrap();
            assert!(
                matches!(opening, Message::User { parts } if parts.iter().any(
                |part| matches!(part, Part::Text { text } if text.contains("<objective>\nship it"))))
            );
        });
    }

    #[test]
    fn turns_without_a_tool_call_stall_a_goal() {
        let dir = folder("goal-stall");
        block_on!(async {
            let answers = vec![done("Hmm."), done("Hmm."), done("Hmm.")];
            let harness = harness(&dir, answers, AllowAll).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .start_goal(
                    &mut thread,
                    goal("x", 1_000_000),
                    &Caller::default(),
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            assert_eq!(
                ended(&events(harness.log(), thread.id).await),
                [GoalReason::Stalled]
            );
        });
    }

    #[test]
    fn the_goal_tool_is_offered_only_in_a_goals_turns() {
        let dir = folder("goal-absent");
        block_on!(async {
            let (harness, seen) = harness_seeing(
                &dir,
                vec![done("Hi.")],
                AllowAll,
                InMemoryLog::new(),
                1,
                settings(),
            )
            .await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let seen = seen.lock().unwrap();
            let request = seen.first().unwrap();
            assert!(!offers(request, UPDATE_GOAL));
            assert!(!request.system.contains("# Goal"));
            assert!(thread.goal().is_none());
        });
    }
}
