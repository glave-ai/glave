use glave_event::{
    At, EventId, Header, Outcome, PlanApproved, Principal, Seq, ThreadId, TurnResult,
};
use glave_ports::{Effect, Message};

use super::*;

fn arguments(value: serde_json::Value) -> Arguments {
    match value {
        serde_json::Value::Object(map) => Arguments::Json(map),
        other => panic!("not an object: {other}"),
    }
}

fn step(step: &str, status: PlanStatus) -> PlanStep {
    PlanStep {
        step: step.to_owned(),
        status,
    }
}

fn event(seq: u64, body: Body) -> Event {
    let at: jiff::Timestamp = "2026-10-26T10:00:00.000Z".parse().unwrap();
    Event {
        header: Header {
            id: EventId::new(at, [1; 10]).unwrap(),
            thread: ThreadId::new(at, [2; 10]).unwrap(),
            seq: Seq::new(seq).unwrap(),
            at: At::from(at),
        },
        body,
    }
}

fn updated(steps: Vec<PlanStep>, call: Option<&str>, proposed: bool) -> Body {
    Body::PlanUpdated(PlanUpdated {
        steps,
        call_id: call.map(|id| CallId::new(id).unwrap()),
        proposed,
        by: None,
    })
}

#[test]
fn the_plan_is_read_from_the_model_s_call_and_says_how_far_along_it_is() {
    let called = called(&arguments(json!({"plan": [
        {"step": "  read the code ", "status": "completed"},
        {"step": "fix the bug", "status": "in_progress"},
        {"step": "run the tests", "status": "pending"}
    ]})))
    .unwrap();
    assert_eq!(called[0].step, "read the code", "trimmed");
    assert_eq!(progress(&called), "1/3 done, now: fix the bug");
    assert_eq!(
        describe(&arguments(
            json!({"plan": [{"step": "a", "status": "pending"}]})
        )),
        "update_plan 0/1 done"
    );
}

#[test]
fn a_plan_it_cannot_hold_is_refused_saying_why() {
    let refused = |value| called(&arguments(value)).unwrap_err();
    assert!(refused(json!({"plan": []})).contains("1 to 64 steps"));
    assert!(refused(json!({"plan": [{"step": " ", "status": "pending"}]})).contains("blank"));
    let long = "x".repeat(MAX_STEP + 1);
    assert!(refused(json!({"plan": [{"step": long, "status": "pending"}]})).contains("short line"));
    let many: Vec<_> = (0..=MAX_STEPS)
        .map(|n| json!({"step": format!("step {n}"), "status": "pending"}))
        .collect();
    assert!(refused(json!({ "plan": many })).contains("1 to 64 steps"));
    assert!(
        refused(json!({"plan": [{"step": "a", "status": "maybe"}]})).contains("unknown variant")
    );
    assert!(called(&Arguments::Malformed("{".into())).is_err());
    // A habit from elsewhere, such as Codex's explanation, is no error.
    let explained = json!({"explanation": "why", "plan": [{"step": "a", "status": "pending"}]});
    assert!(called(&arguments(explained)).is_ok());
}

#[test]
fn the_latest_update_is_the_plan_and_a_proposal_waits_until_approved() {
    let proposal = vec![step("read", PlanStatus::Pending)];
    let edited = vec![step("read the parser", PlanStatus::Pending)];
    let mut events = vec![
        event(1, updated(proposal, Some("call_1"), true)),
        event(2, updated(edited.clone(), None, true)),
    ];
    let planning = Planning::of(&events);
    let plan = planning.proposed().unwrap();
    assert_eq!(plan.steps, edited, "the person's edit is the plan");
    events.push(event(
        3,
        Body::PlanApproved(PlanApproved {
            by: Some(Principal::glave()),
        }),
    ));
    let approved = Planning::of(&events);
    assert!(approved.proposed().is_none());
    assert_eq!(approved.plan.unwrap().steps, edited, "built as edited");
    let ticked = vec![step("read the parser", PlanStatus::Completed)];
    events.push(event(4, updated(ticked.clone(), Some("call_9"), false)));
    assert_eq!(Planning::of(&events).plan.unwrap().steps, ticked);
}

#[test]
fn the_model_is_told_of_the_person_s_edit_where_it_was_made_and_of_nothing_else() {
    let steps = vec![step("read the parser", PlanStatus::Pending)];
    let proposed = event(1, updated(steps.clone(), Some("call_1"), true));
    let edit = event(2, updated(steps, None, true));
    let ended = event(
        3,
        Body::TurnEnded(TurnResult {
            outcome: Outcome::Completed,
            ..TurnResult::begun()
        }),
    );
    let messages = crate::conversation::conversation(&[proposed, ended, edit]);
    let [Message::User { parts }] = messages.as_slice() else {
        panic!("one note, of the edit: {messages:?}");
    };
    let [Part::Text { text }] = parts.as_slice() else {
        panic!("a note in words: {parts:?}");
    };
    assert!(
        text.starts_with("[The person changed the plan. It is now:\n1. [pending] read the parser"),
        "{text}"
    );
}

#[test]
fn a_call_plan_mode_refuses_says_why_in_the_model_s_terms() {
    let request = |effect, caution: Option<&str>| ApprovalRequest {
        thread: ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [0; 10]).unwrap(),
        call_id: CallId::new("call_1").unwrap(),
        tool: "edit".into(),
        effect,
        summary: "edit src/lib.rs".into(),
        secret: false,
        caution: caution.map(str::to_owned),
        outside_sandbox: None,
    };
    let changes = refused(&request(Effect::Changes, None));
    assert!(
        changes.starts_with("Not run: plan mode changes nothing"),
        "{changes}"
    );
    assert!(changes.contains("this changes files") && changes.ends_with("goes in the plan."));
    let risky = refused(&request(Effect::Runs, Some("it pushes to a remote")));
    assert!(risky.contains(", and it pushes to a remote."), "{risky}");
}

#[test]
fn steps_read_as_numbered_lines_with_their_status() {
    let steps = vec![
        step("read", PlanStatus::Completed),
        step("fix", PlanStatus::InProgress),
        step("test", PlanStatus::Pending),
    ];
    assert_eq!(
        listed(&steps),
        "1. [completed] read\n2. [in_progress] fix\n3. [pending] test\n"
    );
}
