//! A connection to one MCP server through rmcp: opened with the newest protocol the server
//! knows, its tools listed for the model, and calls made with a time limit and cancelled when
//! the turn is.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use glave_event::Arguments;
use glave_ports::{
    Cancellation, Effect, ToolError, ToolInfo, ToolOutput, ToolSpec, unless_cancelled,
};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientRequest, ProtocolVersion, ServerResult,
};
use rmcp::service::{
    ClientLifecycleMode, ClientServiceExt as _, Peer, PeerRequestOptions, RoleClient,
    RunningService,
};
use rmcp::transport::IntoTransport;

use crate::McpError;
use crate::names::exposed;
use crate::output::output;
use crate::reason::reason;

/// An open connection.
pub(crate) struct Connection {
    name: String,
    peer: Peer<RoleClient>,
    /// The running connection, until it is closed.
    service: Mutex<Option<RunningService<RoleClient, ()>>>,
    /// The model's name for each tool, to the server's, from the latest listing.
    named: Mutex<HashMap<String, String>>,
}

impl Connection {
    /// Connects over `transport` within `limit`, speaking MCP 2026-07-28 without a handshake,
    /// or the 2025-11-25 handshake with servers that predate it.
    pub(crate) async fn open<T, E, A>(
        name: &str,
        transport: T,
        limit: Duration,
    ) -> Result<Self, McpError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let lifecycle = ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::V_2025_11_25),
        };
        let started = tokio::time::timeout(limit, ().serve_with_lifecycle(transport, lifecycle));
        let service = match started.await {
            Ok(Ok(service)) => service,
            Ok(Err(err)) => return Err(McpError::start(name, reason(&err))),
            Err(_) => {
                return Err(McpError::start(
                    name,
                    format!("it did not answer within {limit:?}"),
                ));
            }
        };
        Ok(Self {
            name: name.to_owned(),
            peer: service.peer().clone(),
            service: Mutex::new(Some(service)),
            named: Mutex::new(HashMap::new()),
        })
    }

    /// Closes the connection: a server process has its input closed, and is killed if it has
    /// not exited within a few seconds.
    pub(crate) async fn close(&self) {
        let service = self
            .service
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(service) = service {
            // Best effort: the process is gone either way when this returns.
            let _ = service.cancel().await;
        }
    }

    /// The server's tools as the model sees them. A tool that says it only reads may run
    /// beside others and without asking; any other may change or run anything.
    pub(crate) async fn list(&self) -> Result<Vec<ToolInfo>, ToolError> {
        let tools = self.peer.list_all_tools().await.map_err(|err| ToolError {
            source_name: self.name.clone(),
            message: err.to_string(),
        })?;
        let mut named = HashMap::new();
        let mut infos = Vec::with_capacity(tools.len());
        for tool in tools {
            let model_name = exposed(&self.name, &tool.name);
            let read_only = tool
                .annotations
                .as_ref()
                .and_then(|hints| hints.read_only_hint)
                .unwrap_or(false);
            let description = tool
                .description
                .as_deref()
                .or(tool.title.as_deref())
                .unwrap_or(&tool.name)
                .to_owned();
            infos.push(ToolInfo {
                spec: ToolSpec {
                    name: model_name.clone(),
                    description,
                    parameters: serde_json::Value::Object((*tool.input_schema).clone()),
                },
                effect: if read_only {
                    Effect::Reads
                } else {
                    Effect::Runs
                },
                parallel: read_only,
            });
            named.insert(model_name, tool.name.to_string());
        }
        *self.named.lock().unwrap_or_else(PoisonError::into_inner) = named;
        Ok(infos)
    }

    /// Calls the tool the model knows as `name`, for at most `limit`.
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: &Arguments,
        cancel: &dyn Cancellation,
        limit: Duration,
    ) -> ToolOutput {
        let Some(tool) = self
            .named
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
        else {
            return ToolOutput::text(true, format!("There is no tool named {name:?}."));
        };
        let Arguments::Json(arguments) = arguments else {
            return ToolOutput::text(true, "The arguments were not valid JSON.");
        };
        let params = CallToolRequestParams::new(tool).with_arguments(arguments.clone());
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let sent = self
            .peer
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await;
        let mut handle = match sent {
            Ok(handle) => handle,
            Err(err) => return ToolOutput::text(true, format!("{}: {err}", self.name)),
        };
        let waited = unless_cancelled(cancel, tokio::time::timeout(limit, &mut handle.rx)).await;
        match waited {
            Some(Ok(Ok(Ok(ServerResult::CallToolResult(result))))) => output(&result),
            Some(Ok(Ok(Ok(ServerResult::InputRequiredResult(_))))) => ToolOutput::text(
                true,
                "The server asked for more input, which glave cannot give it yet.",
            ),
            Some(Ok(Ok(Ok(_)))) => ToolOutput::text(true, "The server answered something else."),
            Some(Ok(Ok(Err(err)))) => ToolOutput::text(true, format!("{}: {err}", self.name)),
            Some(Ok(Err(_))) => ToolOutput::text(true, format!("{} stopped answering.", self.name)),
            Some(Err(_)) => {
                let _ = handle.cancel(Some("timed out".to_owned())).await;
                ToolOutput::text(
                    true,
                    format!("{} did not answer within {limit:?}.", self.name),
                )
            }
            None => {
                let _ = handle
                    .cancel(Some("cancelled by the person".to_owned()))
                    .await;
                ToolOutput::text(true, "[stopped: the person cancelled the turn]")
            }
        }
    }
}
