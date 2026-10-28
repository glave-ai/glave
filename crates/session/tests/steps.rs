//! Steps (`specs/session.md`, "Steps"): a thread's runs of tool calls, each told in a line of
//! the server's, and each turn's reply, read from its log.

use glave_session::protocol::{Reply, Request};
use serde_json::json;

use crate::support::*;

#[test]
fn a_turn_s_runs_are_told_from_what_its_calls_recorded_and_its_last_words_are_the_reply() {
    let dir = folder("steps");
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    block_on!(async {
        let script = vec![
            answer(None, Some(("read", json!({"path": "a.txt"})))),
            answer(None, Some(("bash", json!({"command": "exit 3"})))),
            answer(Some("It fails; looking closer."), None),
        ];
        let mut setup = Setup::new(script);
        setup.settings.titles = false;
        let (server, _seen) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "why does it fail?").await.unwrap();
        let answered = server.request(Request::ThreadSteps { thread }).await;
        server.shutdown().await;
        let Ok(Reply::Steps { steps }) = answered else {
            panic!("steps, not {answered:?}")
        };
        let lines: Vec<&str> = steps.runs.iter().map(|run| &*run.sentence).collect();
        assert_eq!(lines, ["Ran 1 command, read 1 file (1 failed)"]);
        assert_eq!(steps.runs.first().map(|run| run.calls.len()), Some(2));
        assert_eq!(steps.replies.len(), 1, "the turn's last words");
    });
}
