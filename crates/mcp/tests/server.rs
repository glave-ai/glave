//! A real MCP server process as a tool source: started with only the environment it should
//! have, listed, called, cancelled, timed out and closed.

#[cfg(test)]
mod server {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::time::Duration;

    use glave_event::{Arguments, Part};
    use glave_mcp::{Limits, McpError, McpServer, ServerConfig};
    use glave_ports::{BoxFuture, Cancellation, Effect, ToolOutput, ToolSource as _, conformance};
    use serde_json::json;

    const SERVER: &str = env!("CARGO_BIN_EXE_glave-mcp-test-server");

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// glave's own environment, as the server sees it before the allowlist: with a key in it.
    fn environment() -> BTreeMap<String, String> {
        let mut environment: BTreeMap<String, String> = std::env::vars().collect();
        environment.insert("GLAVE_API_KEY".into(), "not-a-real-key".into());
        environment
    }

    fn stdio(command: &str, args: &[&str], env: &[(&str, &str)]) -> ServerConfig {
        ServerConfig::Stdio {
            command: command.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: env
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    fn launch(name: &str, config: &ServerConfig, limits: Limits) -> McpServer {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        McpServer::start(name, config, &environment(), dir, limits)
    }

    async fn start(env: &[(&str, &str)]) -> McpServer {
        let server = launch("tests", &stdio(SERVER, &[], env), Limits::default());
        server.started().await.unwrap();
        server
    }

    struct Never;

    impl Cancellation for Never {
        fn is_cancelled(&self) -> bool {
            false
        }
        fn cancelled(&self) -> BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }

    struct Already;

    impl Cancellation for Already {
        fn is_cancelled(&self) -> bool {
            true
        }
        fn cancelled(&self) -> BoxFuture<'_, ()> {
            Box::pin(std::future::ready(()))
        }
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

    #[test]
    fn passes_the_tool_source_conformance_suite() {
        block_on(async {
            let server = start(&[]).await;
            assert_eq!(conformance::tool_source(&server).await, Ok(()));
            server.close().await;
        });
    }

    #[test]
    fn tools_are_named_for_their_server_and_marked_by_what_they_do() {
        block_on(async {
            let server = start(&[]).await;
            let tools = server.tools().await.unwrap();
            let named: Vec<(&str, Effect, bool)> = tools
                .iter()
                .map(|tool| (tool.spec.name.as_str(), tool.effect, tool.parallel))
                .collect();
            assert_eq!(
                named,
                [
                    ("mcp__tests__echo", Effect::Reads, true),
                    ("mcp__tests__fail", Effect::Runs, false),
                    ("mcp__tests__slow", Effect::Runs, false),
                    ("mcp__tests__env", Effect::Runs, false),
                    ("mcp__tests__header", Effect::Runs, false),
                    ("mcp__tests__grow", Effect::Runs, false),
                    ("mcp__tests__listed", Effect::Runs, false),
                    ("mcp__tests__flood", Effect::Runs, false),
                ]
            );
            assert_eq!(tools[0].spec.description, "Say the text back.");
            assert_eq!(
                server.instructions().as_deref(),
                Some("## tests\n\nCall echo to repeat text; it costs nothing.")
            );
            assert_eq!(
                tools[0].spec.parameters["properties"]["text"]["type"],
                "string"
            );
            server.close().await;
        });
    }

    #[test]
    fn calls_reach_the_server_and_its_answers_come_back() {
        block_on(async {
            let server = start(&[]).await;
            server.tools().await.unwrap();
            let echo = arguments(json!({"text": "hello"}));
            let said = server.call("mcp__tests__echo", &echo, &Never).await;
            assert_eq!(said, ToolOutput::text(false, "hello"));
            let failed = server
                .call("mcp__tests__fail", &arguments(json!({})), &Never)
                .await;
            assert!(failed.is_error);
            assert_eq!(text(&failed), "it failed, as it always does");
            let flood = server
                .call("mcp__tests__flood", &arguments(json!({})), &Never)
                .await;
            let flood = text(&flood);
            assert!(flood.starts_with("xxxx"), "{flood:.20}");
            assert!(flood.ends_with("[cut short: 250000 characters, only the first 100000 shown]"));
            server.close().await;
        });
    }

    #[test]
    fn the_server_gets_its_own_settings_and_none_of_glaves_secrets() {
        block_on(async {
            let server = start(&[("TRACKER_TOKEN", "for-the-tracker")]).await;
            server.tools().await.unwrap();
            let mut seen = Vec::new();
            for name in ["GLAVE_API_KEY", "TRACKER_TOKEN", "PATH"] {
                let asked = arguments(json!({ "name": name }));
                seen.push(text(&server.call("mcp__tests__env", &asked, &Never).await));
            }
            assert_eq!(
                seen,
                [
                    "GLAVE_API_KEY set: false",
                    "TRACKER_TOKEN set: true",
                    "PATH set: true"
                ]
            );
            server.close().await;
        });
    }

    #[test]
    fn a_cancelled_call_stops_and_the_server_keeps_working() {
        block_on(async {
            let server = start(&[]).await;
            server.tools().await.unwrap();
            let none = arguments(json!({}));
            let stopped = server.call("mcp__tests__slow", &none, &Already).await;
            assert_eq!(
                stopped,
                ToolOutput::text(true, "[stopped: the person cancelled the turn]")
            );
            let echo = arguments(json!({"text": "still here"}));
            let said = server.call("mcp__tests__echo", &echo, &Never).await;
            assert_eq!(text(&said), "still here");
            server.close().await;
        });
    }

    #[test]
    fn a_call_that_takes_too_long_is_given_up_on() {
        block_on(async {
            let limits = Limits {
                call: Duration::from_millis(200),
                ..Limits::default()
            };
            let server = launch("tests", &stdio(SERVER, &[], &[]), limits);
            server.tools().await.unwrap();
            let none = arguments(json!({}));
            let late = server.call("mcp__tests__slow", &none, &Never).await;
            assert_eq!(
                late,
                ToolOutput::text(true, "tests did not answer within 200ms.")
            );
            server.close().await;
        });
    }

    #[test]
    fn calls_after_closing_fail_with_a_reason() {
        block_on(async {
            let server = start(&[]).await;
            server.tools().await.unwrap();
            server.close().await;
            let echo = arguments(json!({"text": "hello"}));
            let said = server.call("mcp__tests__echo", &echo, &Never).await;
            assert!(said.is_error);
            assert!(text(&said).starts_with("tests: "), "{said:?}");
            assert!(server.tools().await.is_err());
        });
    }

    #[test]
    fn a_server_starts_in_the_background_and_its_first_listing_waits_for_it() {
        block_on(async {
            let server = launch("tests", &stdio(SERVER, &[], &[]), Limits::default());
            let tools = server.tools().await.unwrap();
            assert_eq!(tools.len(), 8);
            server.close().await;
        });
    }

    fn start_error(server: &McpServer, outcome: Result<(), McpError>) -> String {
        match outcome {
            Err(McpError::Start {
                server: name,
                message,
            }) => {
                assert_eq!(name, server.name());
                message
            }
            other => panic!("{} started: {other:?}", server.name()),
        }
    }

    #[test]
    fn a_server_that_cannot_start_says_why() {
        block_on(async {
            let missing = launch(
                "gone",
                &stdio("/nonexistent/glave-no-such-server", &[], &[]),
                Limits::default(),
            );
            let why = start_error(&missing, missing.started().await);
            assert!(why.starts_with("could not run /nonexistent/"), "{why}");
            let listed = missing.tools().await.unwrap_err();
            assert!(listed.to_string().contains("could not run"), "{listed}");
            let mute = launch("mute", &stdio("true", &[], &[]), Limits::default());
            let why = start_error(&mute, mute.started().await);
            assert_eq!(why, "it exited or closed the connection before answering");
        });
    }

    #[test]
    fn a_server_that_never_answers_is_given_up_on_and_stopped() {
        block_on(async {
            let limits = Limits {
                startup: Duration::from_millis(200),
                ..Limits::default()
            };
            let silent = launch("silent", &stdio("sleep", &["60"], &[]), limits);
            let why = start_error(&silent, silent.started().await);
            assert_eq!(why, "it did not answer within 200ms");
            silent.close().await;
        });
    }

    #[test]
    fn closing_a_server_that_is_still_starting_stops_it() {
        block_on(async {
            let silent = launch("silent", &stdio("sleep", &["60"], &[]), Limits::default());
            silent.close().await;
            let why = start_error(&silent, silent.started().await);
            assert_eq!(why, "it was closed");
        });
    }
}
