//! A small MCP server for tests, on standard input and output, or with `--http` over
//! Streamable HTTP on a free local port that it prints first. Its tools: `echo` (read-only),
//! `fail`, `slow` (waits until the call is cancelled), `env` (says whether a variable reached
//! it), `header` (says what a request header was) and `flood` (more output than a model should
//! be given). Built with the `test-server` feature.

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;

use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
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
        Tool::new("header", "A request header's value.", schema(&name)),
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
            "header" => {
                let name = argument(&request, "name");
                let value = context
                    .extensions
                    .get::<http::request::Parts>()
                    .and_then(|parts| parts.headers.get(name.as_str()))
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("(none)")
                    .to_owned();
                CallToolResult::success(vec![ContentBlock::text(format!("{name}: {value}"))])
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
    let http = std::env::args().nth(1).as_deref() == Some("--http");
    let served = runtime.block_on(async {
        if http {
            serve_http().await
        } else {
            let running = TestServer.serve(rmcp::transport::stdio()).await?;
            running.waiting().await?;
            Ok(())
        }
    });
    if served.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Serves until killed, after printing its URL.
async fn serve_http() -> Result<(), Box<dyn std::error::Error>> {
    let service = StreamableHttpService::new(
        || Ok(TestServer),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let mut out = std::io::stdout();
    writeln!(out, "http://{}/mcp", listener.local_addr()?)?;
    out.flush()?;
    let mut connections = tokio::task::JoinSet::new();
    loop {
        let (stream, _) = listener.accept().await?;
        let service = TowerToHyperService::new(service.clone());
        connections.spawn(async move {
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
        while connections.try_join_next().is_some() {}
    }
}
