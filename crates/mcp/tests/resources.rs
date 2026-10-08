//! MCP resources as two read-only tools, against the test server's two resources.

#[cfg(test)]
mod resources {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Arc;

    use glave_event::{Arguments, Part};
    use glave_mcp::{Limits, McpServer, Resources, ServerConfig};
    use glave_ports::{BoxFuture, Cancellation, Effect, ToolOutput, ToolSource as _, conformance};
    use serde_json::json;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
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

    fn served() -> Resources {
        let config = ServerConfig::Stdio {
            command: env!("CARGO_BIN_EXE_glave-mcp-test-server").to_owned(),
            args: Vec::new(),
            env: BTreeMap::new(),
        };
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let server = McpServer::start("tests", &config, &BTreeMap::new(), dir, Limits::default());
        Resources::new(vec![Arc::new(server)])
    }

    async fn call(resources: &Resources, tool: &str, arguments: serde_json::Value) -> ToolOutput {
        let serde_json::Value::Object(map) = arguments else {
            panic!("not an object");
        };
        resources.call(tool, &Arguments::Json(map), &Never).await
    }

    fn text(output: &ToolOutput) -> &str {
        match output.parts.as_slice() {
            [Part::Text { text }] => text,
            other => panic!("not one text part: {other:?}"),
        }
    }

    #[test]
    fn two_read_only_tools_pass_the_conformance_suite() {
        block_on(async {
            let resources = served();
            let tools = resources.tools().await.unwrap();
            let named: Vec<(&str, Effect)> = tools
                .iter()
                .map(|tool| (tool.spec.name.as_str(), tool.effect))
                .collect();
            assert_eq!(
                named,
                [
                    ("list_mcp_resources", Effect::Reads),
                    ("read_mcp_resource", Effect::Reads)
                ]
            );
            assert_eq!(conformance::tool_source(&resources).await, Ok(()));
        });
    }

    #[test]
    fn without_servers_that_have_resources_nothing_is_offered() {
        block_on(async {
            let none = Resources::new(Vec::new());
            assert!(none.tools().await.unwrap().is_empty());
        });
    }

    #[test]
    fn resources_are_listed_with_their_server_uri_and_type() {
        block_on(async {
            let resources = served();
            let listed = call(&resources, "list_mcp_resources", json!({})).await;
            assert!(!listed.is_error);
            assert_eq!(
                text(&listed),
                "tests test://greeting greeting (text/plain): A friendly line.\n\
                 tests test://logo logo (image/png)\n"
            );
            let chosen = call(&resources, "list_mcp_resources", json!({"server": "tests"})).await;
            assert_eq!(text(&chosen), text(&listed));
        });
    }

    #[test]
    fn text_is_read_and_anything_else_is_named() {
        block_on(async {
            let resources = served();
            let read = |uri: &str| json!({"server": "tests", "uri": uri});
            let greeting = call(&resources, "read_mcp_resource", read("test://greeting")).await;
            assert_eq!(greeting, ToolOutput::text(false, "hello from a resource"));
            let logo = call(&resources, "read_mcp_resource", read("test://logo")).await;
            assert_eq!(text(&logo), "[binary resource: test://logo, image/png]");
            let missing = call(&resources, "read_mcp_resource", read("test://nothing")).await;
            assert!(missing.is_error);
            assert!(
                text(&missing).contains("no resource test://nothing"),
                "{missing:?}"
            );
        });
    }

    #[test]
    fn mistakes_are_named() {
        block_on(async {
            let resources = served();
            let unknown = json!({"server": "elsewhere", "uri": "test://greeting"});
            let wrong = call(&resources, "read_mcp_resource", unknown).await;
            assert_eq!(
                wrong,
                ToolOutput::text(
                    true,
                    "No server named \"elsewhere\" offers resources. Those that do: tests."
                )
            );
            let bare = call(
                &resources,
                "read_mcp_resource",
                json!({"uri": "test://logo"}),
            )
            .await;
            assert_eq!(
                text(&bare),
                "Give the server's name and the resource's uri."
            );
        });
    }
}
