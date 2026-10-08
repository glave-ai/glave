//! A small MCP server for tests, on standard input and output, or with `--http` over
//! Streamable HTTP on a free local port that it prints first. With `--legacy` it refuses MCP
//! 2026-07-28 and speaks only the 2025-11-25 handshake. Built with the `test-server` feature.
//!
//! Its tools: `echo` (read-only), `fail`, `slow` (waits until the call is cancelled), `env`
//! (whether a variable reached it), `header` (what a request header was), `grow` (adds the
//! tool `extra` and says the tools changed), `listed` (how often its tools were listed) and
//! `flood` (more output than a model should be given). Its resources: `test://greeting`
//! (text) and `test://logo` (binary).

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, DiscoverRequestMethod,
    DiscoverResult, JsonObject, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerConfig, SubscriptionFilter, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer, SubscriptionContext, SubscriptionSink};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler, ServiceExt as _};
use serde_json::json;

/// One server's state, shared by every connection to it.
#[derive(Clone, Default)]
struct TestServer {
    legacy: bool,
    grown: Arc<AtomicBool>,
    listings: Arc<AtomicU32>,
    /// Clients listening for changes on MCP 2026-07-28.
    listeners: Arc<Mutex<Vec<SubscriptionSink>>>,
}

fn schema(value: &serde_json::Value) -> Arc<JsonObject> {
    Arc::new(value.as_object().cloned().unwrap_or_default())
}

impl TestServer {
    fn tools(&self) -> Vec<Tool> {
        let text = json!({"type": "object", "properties": {"text": {"type": "string"}}});
        let name = json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let none = json!({"type": "object"});
        let mut echo = Tool::new("echo", "Say the text back.", schema(&text));
        echo.annotations = Some(ToolAnnotations::new().read_only(true));
        let mut tools = vec![
            echo,
            Tool::new("fail", "Always fails.", schema(&none)),
            Tool::new("slow", "Waits until cancelled.", schema(&none)),
            Tool::new("env", "Whether a variable is set.", schema(&name)),
            Tool::new("header", "A request header's value.", schema(&name)),
            Tool::new("grow", "Adds a tool.", schema(&none)),
            Tool::new("listed", "How often the tools were listed.", schema(&none)),
            Tool::new("flood", "Far too much text.", schema(&none)),
        ];
        if self.grown.load(Ordering::SeqCst) {
            tools.push(Tool::new("extra", "The added tool.", schema(&none)));
        }
        tools
    }

    /// Adds `extra`, and says so to every client listening.
    async fn grow(&self, context: &RequestContext<RoleServer>) {
        self.grown.store(true, Ordering::SeqCst);
        if self.legacy {
            let _ = context.peer.notify_tool_list_changed().await;
        } else {
            let listeners = self
                .listeners
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            for listener in listeners {
                let _ = listener.notify_tool_list_changed().await;
            }
        }
    }
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

fn said(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text.into())])
}

impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .build(),
        )
    }

    fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, ErrorData>> + Send + '_ {
        std::future::ready(if self.legacy {
            Err(ErrorData::method_not_found::<DiscoverRequestMethod>())
        } else {
            Ok(DiscoverResult::from_server_info(
                self.supported_protocol_versions().into_owned(),
                self.get_info(),
            ))
        })
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.clone())
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(context.sink().clone());
        context.cancelled().await;
        Ok(())
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        self.listings.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(ListToolsResult {
            tools: self.tools(),
            ..Default::default()
        }))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + Send + '_ {
        let greeting = Resource::new("test://greeting", "greeting")
            .with_mime_type("text/plain")
            .with_description("A friendly line.");
        let logo = Resource::new("test://logo", "logo").with_mime_type("image/png");
        std::future::ready(Ok(ListResourcesResult {
            resources: vec![greeting, logo],
            ..Default::default()
        }))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, ErrorData>> + Send + '_ {
        let contents =
            match request.uri.as_str() {
                "test://greeting" => Ok(ResourceContents::text(
                    "hello from a resource",
                    "test://greeting",
                )),
                "test://logo" => Ok(ResourceContents::blob("iVBORw0KGgo=", "test://logo")
                    .with_mime_type("image/png")),
                other => Err(ErrorData::resource_not_found(
                    format!("no resource {other}"),
                    None,
                )),
            };
        std::future::ready(contents.map(|content| ReadResourceResult::new(vec![content]).into()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let result = match request.name.as_ref() {
            "echo" => said(argument(&request, "text")),
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
                said(format!("{name} set: {set}"))
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
                said(format!("{name}: {value}"))
            }
            "grow" => {
                self.grow(&context).await;
                said("grown")
            }
            "listed" => said(self.listings.load(Ordering::SeqCst).to_string()),
            "extra" if self.grown.load(Ordering::SeqCst) => said("extra here"),
            "flood" => said("x".repeat(250_000)),
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
    let flags: Vec<String> = std::env::args().skip(1).collect();
    let server = TestServer {
        legacy: flags.iter().any(|flag| flag == "--legacy"),
        ..TestServer::default()
    };
    let http = flags.iter().any(|flag| flag == "--http");
    let outcome = runtime.block_on(async {
        if http {
            serve_http(server).await
        } else {
            let running = server.serve(rmcp::transport::stdio()).await?;
            running.waiting().await?;
            Ok(())
        }
    });
    if outcome.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Serves until killed, after printing its URL.
async fn serve_http(server: TestServer) -> Result<(), Box<dyn std::error::Error>> {
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
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
