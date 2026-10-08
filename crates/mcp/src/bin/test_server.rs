//! A small MCP server on standard input and output, for tests: `echo` (read-only), `fail`, `slow`
//! (waits until the call is cancelled), `env` (says whether a variable reached it) and `flood`
//! (more output than a model should be given). Built with the `test-server` feature.

use std::process::ExitCode;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt as _};
use serde_json::json;

struct TestServer;

fn schema(value: &serde_json::Value) -> Arc<JsonObject> {
    Arc::new(value.as_object().cloned().unwrap_or_default())
}

fn tools() -> Vec<Tool> {
    let text = json!({"type": "object", "properties": {"text": {"type": "string"}}});
    let name = json!({"type": "object", "properties": {"name": {"type": "string"}}});
    let none = json!({"type": "object"});
    let mut echo = Tool::new("echo", "Say the text back.", schema(&text));
    echo.annotations = Some(ToolAnnotations::new().read_only(true));
    vec![
        echo,
        Tool::new("fail", "Always fails.", schema(&none)),
        Tool::new("slow", "Waits until cancelled.", schema(&none)),
        Tool::new("env", "Whether a variable is set.", schema(&name)),
        Tool::new("flood", "Far too much text.", schema(&none)),
    ]
}

fn argument(request: &CallToolRequestParams, name: &str) -> String {
    request
        .arguments
        .as_ref()
        .and_then(|arguments| arguments.get(name))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned()
}

impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult {
            tools: tools(),
            ..Default::default()
        }))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let result = match request.name.as_ref() {
            "echo" => CallToolResult::success(vec![ContentBlock::text(argument(&request, "text"))]),
            "fail" => {
                CallToolResult::error(vec![ContentBlock::text("it failed, as it always does")])
            }
            "slow" => {
                context.ct.cancelled().await;
                CallToolResult::error(vec![ContentBlock::text("cancelled")])
            }
            "env" => {
                let name = argument(&request, "name");
                let set = std::env::var_os(&name).is_some();
                CallToolResult::success(vec![ContentBlock::text(format!("{name} set: {set}"))])
            }
            "flood" => CallToolResult::success(vec![ContentBlock::text("x".repeat(250_000))]),
            other => return Err(ErrorData::invalid_params(format!("no tool {other}"), None)),
        };
        Ok(CallToolResponse::Complete(result))
    }
}

fn main() -> ExitCode {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return ExitCode::FAILURE;
    };
    let served = runtime.block_on(async {
        let running = TestServer.serve(rmcp::transport::stdio()).await?;
        running.waiting().await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    });
    if served.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
