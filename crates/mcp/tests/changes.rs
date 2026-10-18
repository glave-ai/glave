//! Tool listings are kept until a server says its tools changed, and that is heard on MCP
//! 2026-07-28 (a subscription), on the older protocol (a notification), and over HTTP.

#[cfg(test)]
mod changes {
    use std::collections::BTreeMap;
    use std::io::{BufRead as _, BufReader};
    use std::path::Path;
    use std::process::{Command, Stdio};

    use glave_event::{Arguments, Part};
    use glave_mcp::{Limits, McpServer, ServerConfig};
    use glave_ports::testing::Running;
    use glave_ports::{NeverCancelled, ToolSource as _, block_on};

    const SERVER: &str = env!("CARGO_BIN_EXE_glave-mcp-test-server");

    fn launch(config: &ServerConfig) -> McpServer {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        McpServer::start(
            "t",
            config,
            &BTreeMap::new(),
            dir,
            Limits::default(),
            &glave_process::Processes::unrecorded(),
        )
    }

    fn program(args: &[&str]) -> ServerConfig {
        ServerConfig::Stdio {
            command: SERVER.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: BTreeMap::new(),
        }
    }

    async fn call(server: &McpServer, tool: &str) -> String {
        let none = Arguments::Json(serde_json::Map::new());
        let output = server.call(tool, &none, &NeverCancelled).await;
        match output.parts.as_slice() {
            [Part::Text { text }] => text.clone(),
            other => panic!("not one text part: {other:?}"),
        }
    }

    async fn offers_extra(server: &McpServer) -> bool {
        let tools = server.tools().await.unwrap();
        tools.iter().any(|tool| tool.spec.name == "mcp__t__extra")
    }

    /// Listed once however often it is asked; after `grow`, listed again with `extra`.
    async fn changes_are_heard(server: &McpServer) {
        for _ in 0..3 {
            assert!(!offers_extra(server).await);
        }
        assert_eq!(call(server, "mcp__t__listed").await, "1", "listed once");
        assert_eq!(call(server, "mcp__t__grow").await, "grown");
        // The server sends the change before its answer, but over HTTP on another stream, so it
        // may land a moment later. No clock: a change never heard hangs, and nextest stops it.
        while !offers_extra(server).await {
            tokio::task::yield_now().await;
        }
        assert_eq!(call(server, "mcp__t__extra").await, "extra here");
        assert_eq!(
            call(server, "mcp__t__listed").await,
            "2",
            "listed again once"
        );
        server.close().await;
    }

    #[test]
    fn on_mcp_2026_a_change_is_heard_on_a_subscription() {
        block_on!(async { changes_are_heard(&launch(&program(&[]))).await });
    }

    #[test]
    fn on_the_older_protocol_a_change_is_heard_as_a_notification() {
        block_on!(async { changes_are_heard(&launch(&program(&["--legacy"]))).await });
    }

    #[test]
    fn over_http_a_change_is_heard_too() {
        let child = Command::new(SERVER)
            .arg("--http")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut listening = Running::new(child);
        let mut url = String::new();
        BufReader::new(listening.stdout.take().unwrap())
            .read_line(&mut url)
            .unwrap();
        let config = ServerConfig::Http {
            url: url.trim().to_owned(),
            headers: BTreeMap::new(),
        };
        block_on!(async { changes_are_heard(&launch(&config)).await });
    }
}
