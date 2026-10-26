//! Subagents in the terminal: the real binary hands work to a child thread, says when it
//! starts and ends, each on a line of its own, and the model gets the child's report.

use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::support::*;

#[test]
fn a_subagent_shows_as_it_starts_and_ends_and_its_report_reaches_the_model() {
    let place = place("subagents");
    let model = FakeServer::start(vec![
            Reply::tool_call(
                "call_1",
                "agent",
                &json!({"description": "read the readme", "prompt": "Read README.md.", "subagent_type": "explore"}),
            ),
            Reply::tool_call("call_2", "read", &json!({"path": "README.md"})),
            Reply::text("README.md says: # demo."),
            Reply::text("The readme is a one-line demo."),
        ])
        .unwrap();
    let output = glave(
        &place,
        &model,
        &["--model", "m", "what is in the readme?"],
        "",
    );
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    let lines: Vec<&str> = shown.lines().map(str::trim).collect();
    assert!(lines.contains(&"agent read the readme"), "{shown}");
    assert!(lines.contains(&"agent done"), "{shown}");
    assert!(
        !shown.contains("README.md says"),
        "its words are its own: {shown}"
    );
    assert!(shown.contains("The readme is a one-line demo."), "{shown}");
    let received = model.received();
    assert_eq!(received.len(), 4);
    let report = said(&received[3].body, "tool");
    assert!(report[0].contains("README.md says: # demo."), "{report:?}");
}
