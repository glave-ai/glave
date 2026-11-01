//! A connection to one MCP server through rmcp: opened with the newest protocol the server
//! knows, its tools listed for the model and kept until the server says they changed, and calls
//! made within their limits and cancelled when the turn is (`call.rs`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::Arguments;
use glave_ports::{
    Called, Cancellation, Effect, NeverCancelled, ServiceAsks, ToolError, ToolInfo, ToolOutput,
    ToolSpec, spoken, unless_cancelled,
};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, GetPromptRequestParams, GetPromptResult, JsonObject,
    MetaObject, Prompt, ProtocolVersion, ReadResourceRequestParams, RequestMetaObject, Resource,
    ResourceContents, Tool,
};
use rmcp::service::{ClientLifecycleMode, ClientServiceExt as _, Peer, RoleClient, RunningService};
use rmcp::transport::IntoTransport;
use tokio::task::JoinHandle;

use crate::McpError;
use crate::call::{Answerer, Ran, Unanswered, Waits, call};
use crate::client::{Client, subscribe, watch_tools};
use crate::names::{catalog, exposed};
use crate::output::output;
use crate::probe::Probed;
use crate::reason::{failed, reason};

/// What the model is told when the person cancels the turn mid-call.
pub(crate) const CANCELLED: &str = "[stopped: the person cancelled the turn]";

/// What glave tells a server as it connects.
#[derive(Default)]
pub(crate) struct Opening {
    /// Whether the server is a gateway's catalog, whose tools carry their own server's name.
    pub(crate) catalog: bool,
    /// The project folder it works for, told when it asks (MCP's roots).
    pub(crate) root: Option<PathBuf>,
}

/// An open connection.
pub(crate) struct Connection {
    name: String,
    /// Whether the server is a gateway's catalog, whose tools carry their own server's name.
    catalog: bool,
    peer: Peer<RoleClient>,
    /// glave's side, which answers what the server asks for inside a call.
    client: Arc<Client>,
    /// The running connection, until it is closed.
    service: Mutex<Option<RunningService<RoleClient, Arc<Client>>>>,
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
    /// or the 2025-11-25 handshake with servers that predate it or are slow to say
    /// (`probe.rs`), as `opening` says.
    pub(crate) async fn open<T, E, A>(
        name: &str,
        transport: T,
        limit: Duration,
        opening: Opening,
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
        let client = Arc::new(Client::new(Arc::clone(&stale), opening.root.as_deref()));
        let transport = Probed::new(transport.into_transport());
        let serving = Arc::clone(&client).serve_with_lifecycle(transport, lifecycle);
        let started = tokio::time::timeout(limit, serving);
        let service = match started.await {
            Ok(Ok(service)) => service,
            Ok(Err(err)) => return Err(McpError::start(name, reason(&err))),
            Err(_) => {
                return Err(McpError::start(
                    name,
                    format!("it did not answer within {}", spoken(limit)),
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
                    format!("it did not answer within {}", spoken(limit)),
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
            catalog: opening.catalog,
            peer,
            client,
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

    /// Calls `tool` with `arguments` within `waits`, giving back the server's own result, for a
    /// gateway that serves its tools on, with no way to reach the person: what the server asks is
    /// declined. A call that is dropped leaves the server to finish it.
    pub(crate) async fn forward(
        &self,
        tool: &str,
        arguments: Option<JsonObject>,
        meta: Option<JsonObject>,
        waits: Waits,
    ) -> Result<CallToolResult, String> {
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        params.meta = meta.map(|meta| RequestMetaObject(MetaObject(meta)));
        let called = call(
            &self.name,
            &self.peer,
            &self.client,
            params,
            Answerer::NoOne,
            &NeverCancelled,
            waits,
        );
        match called.await {
            Ok(Ran::Done(result, _)) => Ok(result),
            // With no one to answer, it never asks.
            Ok(Ran::Asks(..)) => Err(format!("{} answered something else.", self.name)),
            Err(Unanswered::Refused(why) | Unanswered::Failed(why)) => Err(why),
            Err(Unanswered::Cancelled) => Err(CANCELLED.to_owned()),
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

    /// Whether the server offers prompts.
    pub(crate) fn has_prompts(&self) -> bool {
        self.peer
            .peer_info()
            .is_some_and(|info| info.capabilities.prompts.is_some())
    }

    /// The server's prompts, within `limit` unless cancelled first.
    pub(crate) async fn prompts(
        &self,
        cancel: &dyn Cancellation,
        limit: Duration,
    ) -> Result<Vec<Prompt>, String> {
        let listed = tokio::time::timeout(limit, self.peer.list_all_prompts());
        match unless_cancelled(cancel, listed).await {
            Some(Ok(Ok(prompts))) => Ok(prompts),
            Some(Ok(Err(err))) => Err(failed(&self.name, &err)),
            Some(Err(_)) => Err(unanswered(&self.name, limit)),
            None => Err(CANCELLED.to_owned()),
        }
    }

    /// The server's prompt `name` with `arguments`, within `limit` unless cancelled first.
    pub(crate) async fn prompt(
        &self,
        name: &str,
        arguments: JsonObject,
        cancel: &dyn Cancellation,
        limit: Duration,
    ) -> Result<GetPromptResult, String> {
        let params = GetPromptRequestParams::new(name).with_arguments(arguments);
        let got = tokio::time::timeout(limit, self.peer.get_prompt(params));
        match unless_cancelled(cancel, got).await {
            Some(Ok(Ok(result))) => Ok(result),
            Some(Ok(Err(err))) => Err(failed(&self.name, &err)),
            Some(Err(_)) => Err(unanswered(&self.name, limit)),
            None => Err(CANCELLED.to_owned()),
        }
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
            Some(Err(_)) => Err(unanswered(&self.name, limit)),
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
            Some(Err(_)) => Err(unanswered(&self.name, limit)),
            None => Err(CANCELLED.to_owned()),
        }
    }

    /// Calls the tool the model knows as `name`, within `waits`, asking `answerer` what the server
    /// asks: `Ok` with what the server answered, a result or an error in its own words, untrusted
    /// when the tool reaches an open world, or what it asks; `Err` with glave's own words when it
    /// gave no answer.
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: &Arguments,
        answerer: Answerer,
        cancel: &dyn Cancellation,
        waits: Waits,
    ) -> Result<Called, ToolOutput> {
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
        let Arguments::Json(arguments) = arguments else {
            return Err(ToolOutput::text(true, "The arguments were not valid JSON."));
        };
        let params = CallToolRequestParams::new(tool).with_arguments(arguments.clone());
        let called = call(
            &self.name,
            &self.peer,
            &self.client,
            params,
            answerer,
            cancel,
            waits,
        );
        let mut output = match called.await {
            Ok(Ran::Done(result, asking)) => asking.told(&self.name, output(&result)),
            Ok(Ran::Asks(form, asking)) => {
                let state = serde_json::to_value(&asking)
                    .map_err(|err| ToolOutput::text(true, format!("{}: {err}", self.name)))?;
                return Ok(Called::Asks(ServiceAsks {
                    service: self.name.clone(),
                    message: form.message,
                    fields: form.fields,
                    state,
                }));
            }
            Err(Unanswered::Refused(why)) => ToolOutput::text(true, why),
            Err(Unanswered::Failed(why)) => return Err(ToolOutput::text(true, why)),
            Err(Unanswered::Cancelled) => return Err(ToolOutput::text(true, CANCELLED)),
        };
        output.untrusted |= open_world;
        Ok(Called::Done(output))
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.stop_watching();
    }
}

/// What a person reads when `server` did not answer within `limit`.
pub(crate) fn unanswered(server: &str, limit: Duration) -> String {
    format!("{server} did not answer within {}.", spoken(limit))
}
