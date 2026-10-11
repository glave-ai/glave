//! MCP resources as two tools, as Codex and Claude Code offer them: one lists what the servers
//! offer, the other reads one. Both only read, so neither asks the person. They are on offer
//! only while a server has resources.

use std::fmt::Write as _;
use std::sync::Arc;

use glave_event::Arguments;
use glave_ports::{
    BoxFuture, Cancellation, Effect, ToolError, ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use rmcp::model::ResourceContents;
use serde_json::json;

use crate::connection::Connection;
use crate::output::cut;
use crate::server::McpServer;

const LIST: &str = "list_mcp_resources";
const READ: &str = "read_mcp_resource";

/// The resources of a set of servers, as tools.
pub struct Resources {
    servers: Vec<Arc<McpServer>>,
}

impl Resources {
    pub fn new(servers: Vec<Arc<McpServer>>) -> Self {
        Self { servers }
    }

    /// The started servers that offer resources, by name.
    async fn offering(&self) -> Vec<(&McpServer, Arc<Connection>)> {
        let mut offering = Vec::new();
        for server in &self.servers {
            if let Ok(connection) = server.connection().await
                && connection.has_resources()
            {
                offering.push((server.as_ref(), connection));
            }
        }
        offering
    }

    async fn specs(&self) -> Result<Vec<ToolInfo>, ToolError> {
        if self.offering().await.is_empty() {
            return Ok(Vec::new());
        }
        let server = json!({"type": "string", "description": "The server's name."});
        let list = ToolSpec {
            name: LIST.to_owned(),
            description: "List the resources MCP servers offer: files, schemas, records and \
                          other context, each with its server and URI."
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {"server": server},
            }),
        };
        let read = ToolSpec {
            name: READ.to_owned(),
            description: "Read one MCP resource, by its server and URI.".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {"server": server, "uri": {"type": "string"}},
                "required": ["server", "uri"],
            }),
        };
        Ok([list, read]
            .into_iter()
            .map(|spec| ToolInfo {
                spec,
                effect: Effect::Reads,
                parallel: true,
            })
            .collect())
    }

    async fn run(
        &self,
        name: &str,
        arguments: &Arguments,
        cancel: &dyn Cancellation,
    ) -> ToolOutput {
        let Arguments::Json(arguments) = arguments else {
            return ToolOutput::text(true, "The arguments were not valid JSON.");
        };
        // Some models fill an optional field with "", meaning they have no choice to make.
        let asked = |key: &str| {
            arguments
                .get(key)
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
        };
        let offering = self.offering().await;
        if let Some(chosen) = asked("server")
            && !offering.iter().any(|(server, _)| server.name() == chosen)
        {
            let names: Vec<&str> = offering.iter().map(|(server, _)| server.name()).collect();
            let message = format!(
                "No server named {chosen:?} offers resources. Those that do: {}.",
                names.join(", ")
            );
            return ToolOutput::text(true, message);
        }
        match name {
            LIST => list(&offering, asked("server"), cancel).await,
            READ => {
                let (Some(chosen), Some(uri)) = (asked("server"), asked("uri")) else {
                    return ToolOutput::text(
                        true,
                        "Give the server's name and the resource's uri.",
                    );
                };
                let Some((server, connection)) =
                    offering.iter().find(|(server, _)| server.name() == chosen)
                else {
                    return ToolOutput::text(true, format!("No server named {chosen:?}."));
                };
                match connection.read(uri, cancel, server.call_limit()).await {
                    Ok(contents) => {
                        let mut output = ToolOutput::text(false, cut(contents_text(&contents)));
                        output.untrusted = server.is_untrusted();
                        output
                    }
                    Err(err) => ToolOutput::text(true, err),
                }
            }
            other => ToolOutput::text(true, format!("There is no tool named {other:?}.")),
        }
    }
}

/// One line per resource: its server, URI, name, type and description.
async fn list(
    offering: &[(&McpServer, Arc<Connection>)],
    chosen: Option<&str>,
    cancel: &dyn Cancellation,
) -> ToolOutput {
    let mut text = String::new();
    for (server, connection) in offering {
        if chosen.is_some_and(|chosen| chosen != server.name()) {
            continue;
        }
        let resources = match connection.resources(cancel, server.call_limit()).await {
            Ok(resources) => resources,
            Err(err) => return ToolOutput::text(true, err),
        };
        for resource in resources {
            let _ = write!(text, "{} {} {}", server.name(), resource.uri, resource.name);
            if let Some(mime) = &resource.mime_type {
                let _ = write!(text, " ({mime})");
            }
            if let Some(description) = &resource.description {
                let _ = write!(text, ": {description}");
            }
            text.push('\n');
        }
    }
    if text.is_empty() {
        text.push_str("(no resources)");
    }
    ToolOutput::text(false, cut(text))
}

/// A resource's contents as text: text as it is, anything else named.
fn contents_text(contents: &[ResourceContents]) -> String {
    let texts: Vec<String> = contents
        .iter()
        .map(|content| match content {
            ResourceContents::TextResourceContents { text, .. } => text.clone(),
            ResourceContents::BlobResourceContents { uri, mime_type, .. } => {
                let mime = mime_type.as_deref().unwrap_or("unknown type");
                format!("[binary resource: {uri}, {mime}]")
            }
            _ => "[a resource glave cannot show]".to_owned(),
        })
        .collect();
    if texts.is_empty() {
        "(empty)".to_owned()
    } else {
        texts.join("\n")
    }
}

impl ToolSource for Resources {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        Box::pin(self.specs())
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        Box::pin(self.run(name, arguments, cancel))
    }
}
