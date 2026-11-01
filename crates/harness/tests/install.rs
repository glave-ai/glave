//! `install`: the person chooses where what the agent adds goes, through the card a service's
//! question uses, and the turn goes on from their answer; where no one can answer, it is not
//! offered.

use glave_event::{Body, Outcome, ServiceAnswer, StopReason};
use glave_harness::{Cancel, HarnessError, NoOneToAsk, Own};
use glave_ports::{Caller, InMemoryLog, block_on};
use serde_json::json;

use crate::network::InTheLog;
use crate::support::*;

const SKILL: &str = "---\nname: tidy\ndescription: Tidies the changelog.\n---\n\nSort it.\n";

fn adding() -> Vec<glave_ports::Completion> {
    let call = json!({"kind": "skill", "name": "tidy", "skill": SKILL});
    vec![
        answer(None, vec![("install", call)], StopReason::ToolUse),
        answer(Some("Added."), vec![], StopReason::EndTurn),
    ]
}

#[test]
fn the_person_chooses_where_a_skill_goes_and_the_turn_goes_on() {
    let (dir, own) = (folder("install-turn"), folder("install-own"));
    block_on!(async {
        let mut settings = settings();
        settings.own = Some(Own {
            folder: own.to_path_buf(),
            servers: true,
        });
        let (harness, seen) =
            harness_seeing(&dir, adding(), InTheLog, InMemoryLog::new(), 1, settings).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let ran = harness
            .run_turn(
                &mut thread,
                "make that a skill",
                &Cancel::new(),
                &mut |_| {},
            )
            .await;
        assert!(matches!(ran, Err(HarnessError::Waits(_))), "{ran:?}");
        let offered = seen.lock().unwrap()[0]
            .tools
            .iter()
            .any(|t| t.name == "install");
        assert!(offered);

        let events = events(harness.log(), thread.id).await;
        let (call, at, asked) = events
            .iter()
            .find_map(|event| match &event.body {
                Body::ServiceAsked(asked) => {
                    Some((asked.call_id.clone(), event.header.seq, asked.clone()))
                }
                _ => None,
            })
            .expect("it asked");
        assert_eq!(
            asked.message,
            "Add the skill “tidy” (6 lines): Tidies the changelog."
        );
        assert_eq!(asked.ask, "glave asks you, not the model");
        assert_eq!(
            asked.note,
            "Nothing changes until you choose. The model is told what you chose."
        );
        assert!(!own.join("skills").exists(), "nothing before the answer");

        let everywhere = ServiceAnswer::Answered {
            values: json!({"place": "everywhere"}).as_object().unwrap().clone(),
        };
        harness
            .answer_service(scope(), thread.id, &call, at, everywhere, None)
            .await
            .unwrap();
        let ended = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(ended.outcome, Outcome::Completed);
        let written = std::fs::read_to_string(own.join("skills/tidy/SKILL.md")).unwrap();
        assert_eq!(written, SKILL);
        assert!(!dir.join(".agents").exists(), "not in the project");
        let told = format!("{:?}", seen.lock().unwrap().last().unwrap().messages);
        assert!(told.contains("to the person's own skills"), "{told}");
    });
}

#[test]
fn with_no_one_to_choose_it_is_not_offered() {
    let dir = folder("install-alone");
    block_on!(async {
        let script = vec![answer(Some("Done."), vec![], StopReason::EndTurn)];
        let (harness, seen) =
            harness_seeing(&dir, script, NoOneToAsk, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(
                &mut thread,
                "make that a skill",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        let offered = seen.lock().unwrap()[0]
            .tools
            .iter()
            .any(|t| t.name == "install");
        assert!(!offered);
    });
}
