//! Commands in the background as a thread records them: started under the call that started
//! them and stopped under the one that stopped them, neither counted in call ids; known again
//! when the thread resumes, and recorded as gone when its computer no longer has them; stopped
//! by the person; and stopped when the thread is archived.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod background {
    use glave_event::{Body, CommandEnding, Event, Principal, PrincipalId, StopReason};
    use glave_harness::{Caller, Cancel, Change, NoOneToAsk};
    use glave_ports::{EventLog as _, MemoryLog};
    use serde_json::json;

    use super::support::*;

    fn started(events: &[Event]) -> Vec<&glave_event::CommandStarted> {
        events
            .iter()
            .filter_map(|event| match &event.body {
                Body::CommandStarted(started) => Some(started),
                _ => None,
            })
            .collect()
    }

    fn endings(events: &[Event]) -> Vec<CommandEnding> {
        events
            .iter()
            .filter_map(|event| match &event.body {
                Body::CommandEnded(ended) => Some(ended.ending.clone()),
                _ => None,
            })
            .collect()
    }

    fn call_ids(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match &event.body {
                Body::ToolCallRequested(requested) => Some(requested.call_id.as_str().to_owned()),
                _ => None,
            })
            .collect()
    }

    fn start(script: &str) -> glave_ports::Completion {
        answer(
            None,
            vec![("bash", json!({"command": script, "background": true}))],
            StopReason::ToolUse,
        )
    }

    fn done() -> glave_ports::Completion {
        answer(Some("Done."), vec![], StopReason::EndTurn)
    }

    #[test]
    fn a_command_is_recorded_from_the_call_that_started_it_to_the_one_that_stopped_it() {
        let dir = folder("bg-recorded");
        block_on!(async {
            let stop = answer(
                None,
                vec![("command_output", json!({"command": 1, "stop": true}))],
                StopReason::ToolUse,
            );
            let answers = vec![start("sleep 30"), stop, done()];
            let harness = harness(&dir, answers, NoOneToAsk).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(
                    &mut thread,
                    "start it, then stop it",
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await
                .unwrap();
            let events = events(harness.log(), thread.id).await;
            let kinds = kinds(&events);
            let first_result = kinds.iter().position(|kind| *kind == "tool.call.completed");
            let recorded = kinds.iter().position(|kind| *kind == "command.started");
            assert!(recorded < first_result, "{kinds:?}");
            let started = started(&events);
            assert_eq!(started.len(), 1);
            assert_eq!(started[0].script, "sleep 30");
            let ids = call_ids(&events);
            assert_eq!(started[0].call_id.as_str(), ids[0]);
            // Glave's own records: the second call is numbered as if there were none.
            assert_eq!(ids, ["call_4", "call_7"]);
            assert_eq!(endings(&events), [CommandEnding::Stopped { by: None }]);
            assert_eq!(thread.commands().running_now().await, []);
        });
    }

    #[test]
    fn a_resumed_thread_knows_what_still_runs_and_records_what_ended_meanwhile() {
        let dir = folder("bg-resumed");
        block_on!(async {
            let answers = vec![start("sleep 30"), done(), start("sleep 31"), done()];
            let harness = harness(&dir, answers, NoOneToAsk).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "start a server", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            let running = resumed.commands().running_now().await;
            assert_eq!(running.len(), 1);
            assert_eq!(running[0].command, "sleep 30");
            assert!(running[0].started_at.is_some());

            // The person stops it.
            let at = "2026-10-17T10:00:00Z".parse().unwrap();
            let person = Principal::person(PrincipalId::new(at, [9; 10]).unwrap());
            let commands = resumed.commands();
            assert!(
                commands
                    .stop_for(running[0].id, person.clone())
                    .await
                    .unwrap()
            );
            let mut resumed = resumed;
            harness.record_commands(&mut resumed).await.unwrap();
            harness
                .run_turn(&mut resumed, "start another", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();

            // Another glave, on another computer, finds the second one gone.
            let log = MemoryLog::new();
            let here = scope();
            let stored = harness.log().read(&here, thread.id, None, usize::MAX);
            log.append(&here, &stored.await.unwrap()).await.unwrap();
            let elsewhere = harness_on(&dir, vec![], NoOneToAsk, log, 2, settings()).await;
            let thread = elsewhere.resume_thread(scope(), thread.id).await.unwrap();
            assert_eq!(thread.commands().running_now().await, []);
            let events = events(elsewhere.log(), thread.id).await;
            assert_eq!(
                endings(&events),
                [
                    CommandEnding::Stopped { by: Some(person) },
                    CommandEnding::Gone
                ]
            );
        });
    }

    #[test]
    fn archiving_a_thread_stops_its_commands_first() {
        let dir = folder("bg-archived");
        block_on!(async {
            let harness = harness(&dir, vec![start("sleep 30"), done()], NoOneToAsk).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "start a server", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            harness
                .change_standing(&mut thread, &Caller::default(), Change::Archive)
                .await
                .unwrap();
            let events = events(harness.log(), thread.id).await;
            let kinds = kinds(&events);
            assert_eq!(
                kinds[kinds.len() - 2..],
                ["command.ended", "thread.archived"]
            );
            assert_eq!(endings(&events), [CommandEnding::Released]);
        });
    }
}
