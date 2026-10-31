//! The tool catalog at `/mcp`, end to end: two MCP servers (glave's test server, as `docs` and
//! as `tracker`) served as one, reached the way glave reaches it, as a catalog source signed in
//! with the person's token. Tools are merged under `<server>__<tool>`, each caller sees and
//! calls only what their policy allows, the servers' annotations pass through, and every call
//! is recorded in the caller's stream without its content.

use std::collections::BTreeMap;

use glave_event::{Arguments, Body, Part};
use glave_gateway::Budgets;
use glave_mcp::{Limits, McpServer};
use glave_ports::{NeverCancelled, ToolOutput, ToolSource as _};
use serde_json::json;

use crate::world::{World, block_on};

fn catalog(world: &World, subject: &str) -> McpServer {
    let headers = BTreeMap::from([(
        "Authorization".to_owned(),
        format!("Bearer {}", world.token(subject)),
    )]);
    McpServer::catalog(
        "gateway",
        &format!("{}/mcp", world.url),
        &headers,
        Limits::default(),
    )
}

fn text(output: &ToolOutput) -> String {
    output
        .parts
        .iter()
        .map(|part| match part {
            Part::Text { text } => text.clone(),
            _ => String::new(),
        })
        .collect()
}

fn arguments(value: &serde_json::Value) -> Arguments {
    Arguments::Json(value.as_object().unwrap().clone())
}

#[test]
fn two_servers_become_one_catalog_named_by_server_and_tool() {
    block_on!(async {
        let world = World::start(Vec::new(), Budgets::default(), true).await;
        let alice = catalog(&world, "alice");
        let tools = alice.tools().await.unwrap();
        let names: Vec<&str> = tools.iter().map(|tool| tool.spec.name.as_str()).collect();
        assert!(names.contains(&"mcp__docs__echo"), "{names:?}");
        assert!(names.contains(&"mcp__tracker__echo"), "{names:?}");
        assert_eq!(names.len(), 26, "thirteen tools from each: {names:?}");
        let echo = tools
            .iter()
            .find(|tool| tool.spec.name == "mcp__docs__echo")
            .unwrap();
        assert_eq!(
            echo.effect,
            glave_ports::Effect::Reads,
            "readOnlyHint passed through"
        );
        let listed = alice.listing().await.unwrap();
        let lookup = listed
            .iter()
            .find(|tool| tool.name == "docs__lookup")
            .unwrap();
        let hints = lookup.annotations.as_ref().unwrap();
        assert_eq!(
            hints.open_world_hint,
            Some(true),
            "openWorldHint passed through"
        );
        let found = alice
            .call(
                "mcp__docs__lookup",
                &arguments(&json!({"text": "rust"})),
                &NeverCancelled,
            )
            .await;
        assert!(
            found.untrusted,
            "glave labels what an open-world catalog tool returns untrusted"
        );
        let said = alice
            .call(
                "mcp__docs__echo",
                &arguments(&json!({"text": "hello"})),
                &NeverCancelled,
            )
            .await;
        assert!(!said.is_error, "{said:?}");
        assert_eq!(text(&said), "hello");
        let recorded = world.recorded("alice").await;
        let [Body::GatewayToolCalled(_), Body::GatewayToolCalled(called)] = recorded.as_slice()
        else {
            panic!("{recorded:?}");
        };
        assert_eq!(
            (called.server.as_str(), called.tool.as_str()),
            ("docs", "echo")
        );
        assert!(!called.is_error);
        alice.close().await;
    });
}

#[test]
fn each_caller_sees_and_calls_only_the_tools_their_policy_allows() {
    block_on!(async {
        let world = World::start(Vec::new(), Budgets::default(), true).await;
        let bob = catalog(&world, "bob");
        let names: Vec<String> = bob
            .tools()
            .await
            .unwrap()
            .into_iter()
            .map(|tool| tool.spec.name)
            .collect();
        assert!(
            names.iter().all(|name| name.starts_with("mcp__docs__")),
            "{names:?}"
        );
        let listed = bob.listing().await.unwrap();
        assert!(
            listed
                .iter()
                .all(|tool| !tool.name.starts_with("tracker__"))
        );
        // Asked for by name anyway, the call is refused, and nothing reaches the server.
        let refused = bob
            .forward("tracker__echo", json!({"text": "x"}).as_object().cloned())
            .await
            .unwrap();
        assert_eq!(refused.is_error, Some(true));
        let said = serde_json::to_string(&refused.content).unwrap();
        assert!(said.contains("not allowed"), "{said}");
        assert!(world.recorded("bob").await.is_empty());
        let failed = bob
            .call("mcp__docs__fail", &arguments(&json!({})), &NeverCancelled)
            .await;
        assert!(failed.is_error);
        let recorded = world.recorded("bob").await;
        assert!(
            matches!(recorded.as_slice(), [Body::GatewayToolCalled(called)] if called.is_error),
            "{recorded:?}"
        );
        bob.close().await;
    });
}
