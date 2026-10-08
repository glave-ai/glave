//! `glave serve --stdio`, the real binary, driven the way another program drives it: JSON-RPC
//! 2.0 requests on its standard input, one per line, and the answers and a turn's
//! notifications on its standard output.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod serving {
    use std::io::{BufRead as _, BufReader, Write as _};

    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::{Value, json};

    use super::support::*;

    #[test]
    fn another_program_runs_a_turn_over_stdio() {
        let place = place("serve");
        let server = FakeServer::start(vec![Reply::text("Hello from glave.")]).unwrap();
        let mut child = start(&place, &server, &["serve", "--stdio", "--model", "m"]);
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut send = |message: Value| {
            writeln!(input, "{message}").unwrap();
            input.flush().unwrap();
        };
        let mut answer_to = |id: i64, notes: &mut Vec<Value>| loop {
            let message: Value = serde_json::from_str(&output.next().unwrap().unwrap()).unwrap();
            assert_eq!(message["jsonrpc"], "2.0");
            if message["id"] == id {
                return message;
            }
            notes.push(message);
        };
        let mut notes = Vec::new();

        send(json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover"}));
        let discovered = answer_to(1, &mut notes);
        assert_eq!(discovered["result"]["name"], "glave");

        send(json!({"jsonrpc": "2.0", "id": 2, "method": "thread/start"}));
        let thread = answer_to(2, &mut notes)["result"]["thread"]["id"].clone();
        assert!(thread.is_string());

        send(json!({"jsonrpc": "2.0", "id": 3, "method": "turn/start",
                    "params": {"thread": thread, "prompt": "hi"}}));
        let turn = answer_to(3, &mut notes);
        assert_eq!(turn["result"]["turn"]["outcome"], "completed");
        assert!(
            notes.iter().any(|note| note["method"] == "turn/live"
                && note["params"]["update"]["text"] == "Hello from glave."),
            "{notes:?}"
        );
        assert!(notes.iter().any(|note| note["method"] == "thread/event"
            && note["params"]["event"]["type"] == "turn.ended"));

        send(json!({"jsonrpc": "2.0", "id": 4, "method": "thread/explode"}));
        assert_eq!(answer_to(4, &mut notes)["error"]["code"], -32601);

        drop(input);
        let status = child.wait().unwrap();
        assert!(status.success(), "glave ends when its input does");
    }
}
