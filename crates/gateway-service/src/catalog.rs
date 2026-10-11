//! The tool catalog, served over MCP at `/mcp`: the organization's MCP servers as one, each
//! reached with credentials only the gateway holds. A tool is named `<server>__<tool>`, the
//! double underscore glave, Claude Code and Codex already put between a server and its tools,
//! so names stay within what every model API accepts (letters, digits, `_` and `-`) and never
//! collide across servers. Everything else about a tool, its annotations included
//! (`openWorldHint`, `readOnlyHint`), is passed through as its server lists it.
//!
//! Each caller sees and may call only the tools their policy allows, and every call is
//! recorded in their stream: which tool, how long, whether it failed. Never what it was sent
//! or what it answered.

use std::sync::Arc;
use std::time::Instant;

use glave_event::GatewayToolCalled;
use glave_mcp::McpServer;
use glave_ports::EventLog;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, DiscoverResult,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler};

use crate::service::{Service, Verified};

/// Between a server's name and its tool's.
pub(crate) const SEPARATOR: &str = "__";

/// The catalog, as one MCP server.
pub(crate) struct Catalog<L> {
    service: Arc<Service<L>>,
}

impl<L> Clone for Catalog<L> {
    fn clone(&self) -> Self {
        Self {
            service: Arc::clone(&self.service),
        }
    }
}

impl<L> Catalog<L> {
    pub(crate) fn new(service: Arc<Service<L>>) -> Self {
        Self { service }
    }
}

/// The caller the HTTP layer signed in for this request.
fn caller(context: &RequestContext<RoleServer>) -> Result<Verified, ErrorData> {
    context
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<Verified>())
        .cloned()
        .ok_or_else(|| ErrorData::invalid_request("no sign-in", None))
}

impl<L: EventLog + 'static> Catalog<L> {
    /// The server a catalog tool's name belongs to, and the tool's own name there. The
    /// longest server name that fits wins, so `a__b` and `a` never mix.
    fn find(&self, name: &str) -> Option<(&Arc<McpServer>, String)> {
        self.service
            .catalog
            .iter()
            .filter_map(|server| {
                let tool = name.strip_prefix(server.name())?.strip_prefix(SEPARATOR)?;
                Some((server, tool.to_owned()))
            })
            .max_by_key(|(server, _)| server.name().len())
    }

    async fn listing(&self, verified: &Verified) -> ListToolsResult {
        let mut tools = Vec::new();
        for server in &self.service.catalog {
            let listed = match server.listing().await {
                Ok(listed) => listed,
                Err(error) => {
                    self.service.notice(&format!("catalog: {error}"));
                    continue;
                }
            };
            for mut tool in listed {
                let refused =
                    self.service
                        .policy
                        .refuses_tool(&verified.caller, server.name(), &tool.name);
                if refused.is_some() {
                    continue;
                }
                tool.name = format!("{}{SEPARATOR}{}", server.name(), tool.name).into();
                tools.push(tool);
            }
        }
        ListToolsResult {
            tools,
            ..Default::default()
        }
    }

    async fn forward(
        &self,
        verified: &Verified,
        request: CallToolRequestParams,
    ) -> Result<CallToolResult, ErrorData> {
        let Some((server, tool)) = self.find(&request.name) else {
            return Err(ErrorData::invalid_params(
                format!("the catalog has no tool {:?}", request.name),
                None,
            ));
        };
        if let Some(rule) = self
            .service
            .policy
            .refuses_tool(&verified.caller, server.name(), &tool)
        {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "{} is not allowed: {rule}",
                request.name
            ))]));
        }
        let started = Instant::now();
        let forwarded = server.forward(&tool, request.arguments).await;
        let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let result = forwarded.unwrap_or_else(|error| {
            CallToolResult::error(vec![ContentBlock::text(self.service.redact(&error))])
        });
        let record = GatewayToolCalled {
            server: server.name().to_owned(),
            tool,
            is_error: result.is_error.unwrap_or(false),
            millis,
        };
        if let Err(error) = self.service.meter.record(verified.person, &record).await {
            let person = verified.person;
            self.service.notice(&format!(
                "a tool call by {person} was not recorded: {error}"
            ));
        }
        Ok(result)
    }
}

impl<L: EventLog + 'static> ServerHandler for Catalog<L> {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(DiscoverResult::from_server_info(
            self.supported_protocol_versions().into_owned(),
            self.get_info(),
        )))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let verified = caller(&context)?;
        Ok(self.listing(&verified).await)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let verified = caller(&context)?;
        self.forward(&verified, request)
            .await
            .map(CallToolResponse::Complete)
    }
}
