//! A connection to one MCP server through rmcp: opened with the newest protocol the server
//! knows, its tools listed for the model and kept until the server says they changed, and calls
//! made with a time limit and cancelled when the turn is.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::Arguments;
use glave_ports::{
    Cancellation, Effect, ToolError, ToolInfo, ToolOutput, ToolSpec, unless_cancelled,
};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, ClientRequest, JsonObject,
    ProtocolVersion, ReadResourceRequestParams, Resource, ResourceContents, ServerResult, Tool,
};
use rmcp::service::{
    ClientLifecycleMode, ClientServiceExt as _, Peer, PeerRequestOptions, RoleClient,
    RunningService,
};
use rmcp::transport::IntoTransport;
use tokio::task::JoinHandle;

use crate::McpError;
use crate::client::{Client, watch_tools};
use crate::names::{catalog, exposed};
use crate::output::output;
use crate::reason::reason;

/// What the model is told when the person cancels the turn mid-call.
pub(crate) const CANCELLED: &str = "[stopped: the person cancelled the turn]";

/// An open connection.
pub(crate) struct Connection {
    name: String,
    /// Whether the server is a gateway's catalog, whose tools carry their own server's name.
    catalog: bool,
    peer: Peer<RoleClient>,
    /// The running connection, until it is closed.
    service: Mutex<Option<RunningService<RoleClient, Client>>>,
    /// The latest listing, kept until the server says its tools changed.
    listed: Mutex<Option<Vec<ToolInfo>>>,
    /// Set when the server says its tools changed.
    stale: Arc<AtomicBool>,
    /// The model's name for each tool, to the server's, from the latest listing.
    named: Mutex<HashMap<String, String>>,
    /// Hearing about changes from a server on the newer protocol, while connected.
    watching: Mutex<Option<JoinHandle<()>>>,
}

impl Connection {
    /// Connects over `transport` within `limit`, speaking MCP 2026-07-28 without a handshake,
    /// or the 2025-11-25 handshake with servers that predate it.
    pub(crate) async fn open<T, E, A>(
        name: &str,
        transport: T,
        limit: Duration,
        catalog: bool,
    ) -> Result<Self, McpError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let lifecycle = ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::V_2025_11_25),
        };
        let stale = Arc::new(AtomicBool::new(false));
        let client = Client::new(Arc::clone(&stale));
        let started =
            tokio::time::timeout(limit, client.serve_with_lifecycle(transport, lifecycle));
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
        let peer = service.peer().clone();
        let announces = peer.peer_info().is_some_and(|info| {
            info.capabilities
                .tools
                .as_ref()
                .is_some_and(|tools| tools.list_changed == Some(true))
        });
        let watching =
            announces.then(|| tokio::spawn(watch_tools(peer.clone(), Arc::clone(&stale))));
        Ok(Self {
            name: name.to_owned(),
            catalog,
            peer,
            service: Mutex::new(Some(service)),
            listed: Mutex::new(None),
            stale,
            named: Mutex::new(HashMap::new()),
            watching: Mutex::new(watching),
        })
    }

    /// Closes the connection: a server process has its input closed, and is killed if it has
    /// not exited within a few seconds.
    pub(crate) async fn close(&self) {
        // A closed server offers nothing.
        *self.listed.lock().unwrap_or_else(PoisonError::into_inner) = None;
        if let Some(watching) = self.stop_watching() {
            // Once it is over, its subscription has told the server it is done.
            let _ = watching.await;
        }
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

    /// Stops hearing about changes, giving back the task to wait for.
    fn stop_watching(&self) -> Option<JoinHandle<()>> {
        let watching = self
            .watching
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        watching.abort();
        Some(watching)
    }

    /// The server's tools as the model sees them, listed again only once the server says they
    /// changed. A tool that says it only reads may run beside others and without asking; any
    /// other may change or run anything.
    pub(crate) async fn list(&self) -> Result<Vec<ToolInfo>, ToolError> {
        // Cleared before listing, so a change during the listing is listed next time.
        let changed = self.stale.swap(false, Ordering::SeqCst);
        if !changed
            && let Some(listed) = self
                .listed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        {
            return Ok(listed);
        }
        let tools = self.peer.list_all_tools().await.map_err(|err| {
            self.stale.store(true, Ordering::SeqCst);
            ToolError {
                source_name: self.name.clone(),
                message: err.to_string(),
            }
        })?;
        let mut named = HashMap::new();
        let mut infos = Vec::with_capacity(tools.len());
        for tool in tools {
            let model_name = if self.catalog {
                catalog(&tool.name)
            } else {
                exposed(&self.name, &tool.name)
            };
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
        *self.listed.lock().unwrap_or_else(PoisonError::into_inner) = Some(infos.clone());
        Ok(infos)
    }

    /// The server's tools exactly as it lists them, for a gateway that serves them on.
    pub(crate) async fn listing(&self) -> Result<Vec<Tool>, String> {
        self.peer
            .list_all_tools()
            .await
            .map_err(|err| format!("{}: {err}", self.name))
    }

    /// Calls `tool` with `arguments` for at most `limit`, giving back the server's own result,
    /// for a gateway that serves its tools on. A call that is dropped leaves the server to
    /// finish it.
    pub(crate) async fn forward(
        &self,
        tool: &str,
        arguments: Option<JsonObject>,
        limit: Duration,
    ) -> Result<CallToolResult, String> {
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let sent = self
            .peer
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
            .map_err(|err| format!("{}: {err}", self.name))?;
        match tokio::time::timeout(limit, sent.rx).await {
            Ok(Ok(Ok(ServerResult::CallToolResult(result)))) => Ok(result),
            Ok(Ok(Ok(_))) => Err(format!("{} answered something else.", self.name)),
            Ok(Ok(Err(err))) => Err(format!("{}: {err}", self.name)),
            Ok(Err(_)) => Err(format!("{} stopped answering.", self.name)),
            Err(_) => Err(format!("{} did not answer within {limit:?}.", self.name)),
        }
    }

    /// What the server says about using its tools, if anything.
    pub(crate) fn instructions(&self) -> Option<String> {
        let info = self.peer.peer_info()?;
        let text = info.instructions.as_deref()?.trim();
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// Whether the server offers resources.
    pub(crate) fn has_resources(&self) -> bool {
        self.peer
            .peer_info()
            .is_some_and(|info| info.capabilities.resources.is_some())
    }

    /// The server's resources, within `limit` unless cancelled first.
    pub(crate) async fn resources(
        &self,
        cancel: &dyn Cancellation,
        limit: Duration,
    ) -> Result<Vec<Resource>, String> {
        let listed = tokio::time::timeout(limit, self.peer.list_all_resources());
        match unless_cancelled(cancel, listed).await {
            Some(Ok(Ok(resources))) => Ok(resources),
            Some(Ok(Err(err))) => Err(format!("{}: {err}", self.name)),
            Some(Err(_)) => Err(format!("{} did not answer within {limit:?}.", self.name)),
            None => Err(CANCELLED.to_owned()),
        }
    }

    /// The contents of the resource at `uri`, within `limit` unless cancelled first.
    pub(crate) async fn read(
        &self,
        uri: &str,
        cancel: &dyn Cancellation,
        limit: Duration,
    ) -> Result<Vec<ResourceContents>, String> {
        let params = ReadResourceRequestParams::new(uri);
        let read = tokio::time::timeout(limit, self.peer.read_resource(params));
        match unless_cancelled(cancel, read).await {
            Some(Ok(Ok(result))) => Ok(result.contents),
            Some(Ok(Err(err))) => Err(format!("{}: {err}", self.name)),
            Some(Err(_)) => Err(format!("{} did not answer within {limit:?}.", self.name)),
            None => Err(CANCELLED.to_owned()),
        }
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
                ToolOutput::text(true, CANCELLED)
            }
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.stop_watching();
    }
}
