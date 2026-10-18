//! A client of `glave serve --stdio` for the binary's end-to-end tests of the session API, and
//! a look through the home for what must never be written.

use std::io::{BufRead as _, BufReader, Lines, Write as _};
use std::path::Path;
use std::process::{ChildStdin, ChildStdout};

use glave_ports::testing::Running;
use glave_provider_openai::fake::FakeServer;
use serde_json::{Value, json};

use crate::support::{Place, start_with, text};

/// `glave serve --stdio` with `args`, logging everything into the home, and a client of it;
/// the server is stopped when this drops.
pub struct Served {
    child: Running,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    next: i64,
    seen: String,
}

impl Served {
    pub fn start(place: &Place, server: &FakeServer, args: &[&str]) -> Self {
        // Logged in full, into the home a test looks through for what must never be written.
        let log = place.home.join("glave.log");
        let log = log.to_str().unwrap();
        let args = [&["serve", "--stdio", "--log", log], args].concat();
        let mut child = start_with(place, server, &args, &[("GLAVE_LOG", "trace")]);
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            child,
            input,
            output,
            next: 0,
            seen: String::new(),
        }
    }

    /// Calls `method` and waits for its answer.
    pub fn call(&mut self, method: &str, params: &Value) -> Value {
        let id = self.send(method, params);
        loop {
            let message = self.read();
            if message["id"] == id {
                return message;
            }
        }
    }

    /// The next message the server sends.
    fn read(&mut self) -> Value {
        let line = self.output.next().unwrap().unwrap();
        self.seen.push_str(&line);
        serde_json::from_str(&line).unwrap()
    }

    /// Calls `method` and waits for its answer, answering each approval it waits for with
    /// what `decide` returns for its notification (`allow_once`, `deny`); and how many asked.
    /// Before each answer the models are listed, which reads the server's settings again
    /// while the turn waits, as another client's request would.
    pub fn call_deciding(
        &mut self,
        method: &str,
        params: &Value,
        mut decide: impl FnMut(&Value) -> &'static str,
    ) -> (Value, usize) {
        let id = self.send(method, params);
        let mut asked = 0;
        loop {
            let message = self.read();
            if message["id"] == id {
                return (message, asked);
            }
            let update = &message["params"]["update"];
            if message["method"] == "turn/live" && update["kind"] == "approval_pending" {
                asked += 1;
                let decision = decide(&message);
                let listed = self.send("model/list", &json!({}));
                while self.read()["id"] != listed {}
                let thread = &message["params"]["thread"];
                let answer = json!({"thread": thread, "approval": update["approval"],
                                    "decision": decision});
                self.send("approval/decide", &answer);
            }
        }
    }

    /// Sends a request for `method`; its id.
    fn send(&mut self, method: &str, params: &Value) -> i64 {
        self.next += 1;
        let id = self.next;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.input, "{message}").unwrap();
        self.input.flush().unwrap();
        id
    }

    pub fn models(&mut self) -> Vec<String> {
        let listed = self.call("model/list", &json!({}));
        listed["result"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Ends the server, and everything it said, on either stream.
    pub fn end(self) -> String {
        drop(self.input);
        let output = self.child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", text(&output));
        format!("{}{}", self.seen, text(&output))
    }
}

/// Whether any file in `folder` holds `needle` as written.
pub fn holds(folder: &Path, needle: &str) -> bool {
    std::fs::read_dir(folder).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        if path.is_dir() {
            return holds(&path, needle);
        }
        let bytes = std::fs::read(&path).unwrap_or_default();
        bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
    })
}
