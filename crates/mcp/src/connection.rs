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
    CallToolRequest, CallToolRequestParams, CallToolResult, ClientRequest, JsonObject, MetaObject,
    ProtocolVersion, ReadResourceRequestParams, RequestMetaObject, Resource, ResourceContents,
    ServerResult, Tool,
};
use rmcp::service::{
    ClientLifecycleMode, ClientServiceExt as _, Peer, PeerRequestOptions, RoleClient,
    RunningService, ServiceError,
};
use rmcp::transport::IntoTransport;
use tokio::task::JoinHandle;

use crate::McpError;
use crate::client::{Client, subscribe, watch_tools};
use crate::names::{catalog, exposed};
use crate::output::output;
use crate::reason::{failed, reason};

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
    /// The model's name for each tool, to the server's and whether the tool says it reaches an
    /// open world (`openWorldHint`), from the latest listing.
    named: Mutex<HashMap<String, (String, bool)>>,
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
        // Subscribed before the connection is handed out, so a change made right after is heard.
        let subscription = if announces {
            let Ok(subscription) = tokio::time::timeout(limit, subscribe(&peer)).await else {
                let _ = service.cancel().await;
                return Err(McpError::start(
                    name,
                    format!("it did not answer within {limit:?}"),
                ));
            };
            subscription
        } else {
            None
        };
        let watching = subscription
            .map(|subscription| tokio::spawn(watch_tools(subscription, Arc::clone(&stale))));
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
                message: failed(&self.name, &err),
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
            // MCP's flag for a tool that reaches beyond the server, such as a web search: what
            // it returns is untrusted, as a web page is.
            let open_world = tool
                .annotations
                .as_ref()
                .and_then(|hints| hints.open_world_hint)
                .unwrap_or(false);
            named.insert(model_name, (tool.name.to_string(), open_world));
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
        meta: Option<JsonObject>,
        limit: Duration,
    ) -> Result<CallToolResult, String> {
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        params.meta = meta.map(|meta| RequestMetaObject(MetaObject(meta)));
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
        meta: Option<JsonObject>,
        cancel: &dyn Cancellation,
        limit: Duration,
    ) -> Result<Vec<ResourceContents>, String> {
        let mut params = ReadResourceRequestParams::new(uri);
        params.meta = meta.map(|meta| RequestMetaObject(MetaObject(meta)));
        let read = tokio::time::timeout(limit, self.peer.read_resource(params));
        match unless_cancelled(cancel, read).await {
            Some(Ok(Ok(result))) => Ok(result.contents),
            Some(Ok(Err(err))) => Err(format!("{}: {err}", self.name)),
            Some(Err(_)) => Err(format!("{} did not answer within {limit:?}.", self.name)),
            None => Err(CANCELLED.to_owned()),
        }
    }

    /// Calls the tool the model knows as `name`, for at most `limit`: `Ok` with what the
    /// server answered, a result or an error in its own words, untrusted when the tool
    /// reaches an open world; `Err` with glave's own words when it gave no answer.
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: &Arguments,
        cancel: &dyn Cancellation,
        limit: Duration,
    ) -> Result<ToolOutput, ToolOutput> {
        let Some((tool, open_world)) = self
            .named
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
        else {
            return Err(ToolOutput::text(
                true,
                format!("There is no tool named {name:?}."),
            ));
        };
        let mut output = self.send(tool, arguments, cancel, limit).await?;
        output.untrusted |= open_world;
        Ok(output)
    }

    /// Sends one call of the server's `tool`, for at most `limit`: `Ok` with the server's
    /// answer, `Err` with glave's words when there is none.
    async fn send(
        &self,
        tool: String,
        arguments: &Arguments,
        cancel: &dyn Cancellation,
        limit: Duration,
    ) -> Result<ToolOutput, ToolOutput> {
        let Arguments::Json(arguments) = arguments else {
            return Err(ToolOutput::text(true, "The arguments were not valid JSON."));
        };
        let params = CallToolRequestParams::new(tool).with_arguments(arguments.clone());
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let sent = self
            .peer
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await;
        let mut handle = match sent {
            Ok(handle) => handle,
            Err(err) => return Err(ToolOutput::text(true, failed(&self.name, &err))),
        };
        let waited = unless_cancelled(cancel, tokio::time::timeout(limit, &mut handle.rx)).await;
        let glave = |text: String| Err(ToolOutput::text(true, text));
        match waited {
            Some(Ok(Ok(Ok(ServerResult::CallToolResult(result))))) => Ok(output(&result)),
            Some(Ok(Ok(Ok(ServerResult::InputRequiredResult(_))))) => {
                glave("The server asked for more input, which glave cannot give it yet.".to_owned())
            }
            Some(Ok(Ok(Ok(_)))) => glave("The server answered something else.".to_owned()),
            // The server's error, its message in its own words.
            Some(Ok(Ok(Err(err @ ServiceError::McpError(_))))) => {
                Ok(ToolOutput::text(true, failed(&self.name, &err)))
            }
            Some(Ok(Ok(Err(err)))) => glave(failed(&self.name, &err)),
            Some(Ok(Err(_))) => glave(format!("{} stopped answering.", self.name)),
            Some(Err(_)) => {
                let _ = handle.cancel(Some("timed out".to_owned())).await;
                glave(format!("{} did not answer within {limit:?}.", self.name))
            }
            None => {
                let _ = handle
                    .cancel(Some("cancelled by the person".to_owned()))
                    .await;
                glave(CANCELLED.to_owned())
            }
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.stop_watching();
    }
}
