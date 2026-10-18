//! A server over Streamable HTTP: the test server on a local port, reached with the headers
//! its settings give, and one that is not there.

#[cfg(test)]
mod http {
    use std::collections::BTreeMap;
    use std::io::{BufRead as _, BufReader};
    use std::path::Path;
    use std::process::{Command, Stdio};

    use glave_event::{Arguments, Part};
    use glave_mcp::{Limits, McpError, McpServer, ServerConfig};
    use glave_ports::block_on;
    use glave_ports::testing::{AlreadyCancelled, FAKE_SECRET, Running};
    use glave_ports::{NeverCancelled, ToolOutput, ToolSource as _, conformance};
    use serde_json::json;

    /// The test server serving HTTP, killed when this is dropped.
    struct Served {
        _running: Running,
        url: String,
    }

    impl Served {
        fn start() -> Self {
            let child = Command::new(env!("CARGO_BIN_EXE_glave-mcp-test-server"))
                .arg("--http")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut running = Running::new(child);
            let mut url = String::new();
            BufReader::new(running.stdout.take().unwrap())
                .read_line(&mut url)
                .unwrap();
            Self {
                _running: running,
                url: url.trim().to_owned(),
            }
        }
    }

    fn at(url: &str, headers: &[(&str, &str)]) -> ServerConfig {
        ServerConfig::Http {
            url: url.to_owned(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    fn launch(config: &ServerConfig) -> McpServer {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        McpServer::start(
            "web",
            config,
            &BTreeMap::new(),
            dir,
            Limits::default(),
            &glave_process::Processes::unrecorded(),
        )
    }

    fn arguments(value: serde_json::Value) -> Arguments {
        match value {
            serde_json::Value::Object(map) => Arguments::Json(map),
            other => panic!("not an object: {other}"),
        }
    }

    fn text(output: &ToolOutput) -> String {
        match output.parts.as_slice() {
            [Part::Text { text }] => text.clone(),
            other => panic!("not one text part: {other:?}"),
        }
    }

    fn start_error(outcome: Result<(), McpError>) -> String {
        match outcome {
            Err(McpError::Start { server, message }) => {
                assert_eq!(server, "web");
                message
            }
            other => panic!("it started: {other:?}"),
        }
    }

    #[test]
    fn an_http_server_passes_the_conformance_suite_and_gets_its_headers() {
        let listening = Served::start();
        block_on!(async {
            let headers = [("Authorization", "Bearer test-token"), ("X-Team", "glave")];
            let server = launch(&at(&listening.url, &headers));
            assert_eq!(conformance::tool_source(&server).await, Ok(()));
            let mut seen = Vec::new();
            for name in ["authorization", "x-team"] {
                let asked = arguments(json!({ "name": name }));
                seen.push(text(
                    &server
                        .call("mcp__web__header", &asked, &NeverCancelled)
                        .await,
                ));
            }
            assert_eq!(seen, ["authorization: Bearer test-token", "x-team: glave"]);
            server.close().await;
        });
    }

    #[test]
    fn a_cancelled_call_over_http_stops_and_the_server_keeps_working() {
        let listening = Served::start();
        block_on!(async {
            let server = launch(&at(&listening.url, &[]));
            server.tools().await.unwrap();
            let none = arguments(json!({}));
            let stopped = server
                .call("mcp__web__slow", &none, &AlreadyCancelled)
                .await;
            assert_eq!(
                stopped,
                ToolOutput::text(true, "[stopped: the person cancelled the turn]")
            );
            let echo = arguments(json!({"text": "still here"}));
            let said = server.call("mcp__web__echo", &echo, &NeverCancelled).await;
            assert_eq!(text(&said), "still here");
            server.close().await;
        });
    }

    #[test]
    fn a_server_that_is_not_there_says_why() {
        block_on!(async {
            // Nothing listens on the discard port.
            let server = launch(&at("http://127.0.0.1:9/mcp", &[]));
            let why = start_error(server.started().await);
            assert!(
                why.starts_with("could not reach http://127.0.0.1:9/mcp: "),
                "{why}"
            );
        });
    }

    #[test]
    fn a_bad_header_is_named_and_its_value_never_shown() {
        block_on!(async {
            let server = launch(&at("http://127.0.0.1:9/mcp", &[("bad header", "x")]));
            let why = start_error(server.started().await);
            assert_eq!(why, "\"bad header\" is not a header name");
            let secret = format!("{FAKE_SECRET}\nvalue");
            let server = launch(&at("http://127.0.0.1:9/mcp", &[("Authorization", &secret)]));
            let why = start_error(server.started().await);
            assert_eq!(why, "the value of header Authorization is not valid");
        });
    }
}
