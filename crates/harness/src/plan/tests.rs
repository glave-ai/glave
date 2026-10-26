use glave_event::{At, EventId, Header, Seq, ThreadId};

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

fn updated(steps: Vec<PlanStep>, call: &str) -> Body {
    Body::PlanUpdated(PlanUpdated {
        steps,
        call_id: Some(CallId::new(call).unwrap()),
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
fn the_latest_update_is_the_plan() {
    let first = vec![step("read", PlanStatus::InProgress)];
    let ticked = vec![step("read", PlanStatus::Completed)];
    let events = [
        event(1, updated(first, "call_1")),
        event(2, updated(ticked.clone(), "call_2")),
    ];
    assert_eq!(Planning::of(&events).plan.unwrap().steps, ticked);
    assert!(
        Planning::of(&[]).plan.is_none(),
        "none until the model keeps one"
    );
}
