//! Subagents in the terminal: the real binary hands work to a child thread, shows the child's
//! steps under its description, and the model gets the child's report.

#[cfg(test)]
#[path = "support/binary.rs"]
mod support;

#[cfg(test)]
mod subagents {
    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::json;

    use super::support::*;

    #[test]
    fn a_subagent_shows_its_steps_and_its_report_reaches_the_model() {
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
        assert!(shown.contains("agent read the readme"), "{shown}");
        assert!(
            shown.contains("↳ read the readme · read README.md"),
            "{shown}"
        );
        assert!(shown.contains("agent done read the readme"), "{shown}");
        assert!(shown.contains("The readme is a one-line demo."), "{shown}");
        let received = model.received();
        assert_eq!(received.len(), 4);
        let report = said(&received[3].body, "tool");
        assert!(report[0].contains("README.md says: # demo."), "{report:?}");
    }
}
