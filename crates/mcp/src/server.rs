//! One MCP server as a tool source. It starts in the background the moment it is made, so
//! glave never waits for a server it has not asked anything yet; its first listing waits for it.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::Arguments;
use glave_ports::{
    BoxFuture, Cancellation, ToolError, ToolInfo, ToolOutput, ToolSource, unless_cancelled,
};
use http::{HeaderName, HeaderValue};
use rmcp::service::RoleClient;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{IntoTransport, StreamableHttpClientTransport, TokioChildProcess};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::McpError;
use crate::config::ServerConfig;
use crate::connection::{CANCELLED, Connection};

/// Variables a server's process starts with, beyond its own configured ones. Nothing else from
/// glave's environment reaches it, so neither do glave's keys.
const PASSED_ON: &[&str] = &[
    "HOME", "LOGNAME", "PATH", "SHELL", "USER", "LANG", "LC_ALL", "TERM", "TMPDIR", "TZ",
];

/// How long a server may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// To start and answer.
    pub startup: Duration,
    /// To answer one call.
    pub call: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            startup: Duration::from_secs(30),
            call: Duration::from_secs(300),
        }
    }
}

#[derive(Clone)]
enum State {
    Starting,
    Ready(Arc<Connection>),
    Failed(McpError),
}

/// An MCP server: starting, started, or failed to start.
pub struct McpServer {
    name: String,
    state: watch::Receiver<State>,
    /// The start in the background, until it is over or the server is closed.
    starting: Mutex<Option<JoinHandle<()>>>,
    call_limit: Duration,
    /// Whether what every tool and resource of the server returns is untrusted, as the person
    /// or a policy marked it.
    untrusted: bool,
}

impl McpServer {
    /// Starts the server `config` describes: a program in `dir`, which gets the few
    /// `PASSED_ON` variables from `environment`, then its own; or a server at a URL.
    pub fn start(
        name: &str,
        config: &ServerConfig,
        environment: &BTreeMap<String, String>,
        dir: &Path,
        limits: Limits,
    ) -> Self {
        if tokio::runtime::Handle::try_current().is_err() {
            let error = McpError::start(name, "glave started it outside its async runtime");
            return Self::failed(name, error, limits);
        }
        let started = match config {
            ServerConfig::Stdio { command, args, env } => {
                let process = process(command, args, env, environment, dir);
                Self::program(name, process, command, limits)
            }
            ServerConfig::Http { url, headers } => Self::http(name, url, headers, limits, false),
        };
        started.unwrap_or_else(|error| Self::failed(name, error, limits))
    }

    fn program(
        name: &str,
        process: Command,
        command: &str,
        limits: Limits,
    ) -> Result<Self, McpError> {
        let (transport, _) = TokioChildProcess::builder(process)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| McpError::start(name, format!("could not run {command}: {err}")))?;
        Ok(Self::connect(name, transport, limits))
    }

    /// A gateway's catalog at `url` (its `/mcp`), sent `headers` with every request: one
    /// server whose tools are named `<server>__<tool>` after the servers that offer them, so
    /// the model sees each as `mcp__<server>__<tool>`, as if the server were the person's own.
    pub fn catalog(
        name: &str,
        url: &str,
        headers: &BTreeMap<String, String>,
        limits: Limits,
    ) -> Self {
        Self::http(name, url, headers, limits, true)
            .unwrap_or_else(|error| Self::failed(name, error, limits))
    }

    /// A server over Streamable HTTP, sent `headers` with every request. A header's value is
    /// never shown, since it is often a key.
    fn http(
        name: &str,
        url: &str,
        headers: &BTreeMap<String, String>,
        limits: Limits,
        catalog: bool,
    ) -> Result<Self, McpError> {
        let mut sent = HashMap::new();
        for (header, value) in headers {
            let header_name = HeaderName::try_from(header.as_str())
                .map_err(|_| McpError::start(name, format!("{header:?} is not a header name")))?;
            let header_value = HeaderValue::try_from(value.as_str()).map_err(|_| {
                McpError::start(name, format!("the value of header {header} is not valid"))
            })?;
            sent.insert(header_name, header_value);
        }
        let config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(sent);
        let transport = StreamableHttpClientTransport::from_config(config);
        Ok(Self::open(name, transport, limits, catalog))
    }

    /// Starts talking MCP over `transport`.
    pub fn connect<T, E, A>(name: &str, transport: T, limits: Limits) -> Self
    where
        T: IntoTransport<RoleClient, E, A> + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
        A: 'static,
    {
        Self::open(name, transport, limits, false)
    }

    fn open<T, E, A>(name: &str, transport: T, limits: Limits, catalog: bool) -> Self
    where
        T: IntoTransport<RoleClient, E, A> + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
        A: 'static,
    {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            let error = McpError::start(name, "glave started it outside its async runtime");
            return Self::failed(name, error, limits);
        };
        let (sender, state) = watch::channel(State::Starting);
        let owned = name.to_owned();
        let starting = runtime.spawn(async move {
            let opened = Connection::open(&owned, transport, limits.startup, catalog).await;
            sender.send_replace(match opened {
                Ok(connection) => State::Ready(Arc::new(connection)),
                Err(err) => State::Failed(err),
            });
        });
        Self {
            name: name.to_owned(),
            state,
            starting: Mutex::new(Some(starting)),
            call_limit: limits.call,
            untrusted: false,
        }
    }

    /// Marks what the server returns as untrusted, as a web page is, when `untrusted`: the
    /// person or a policy says the server brings in content from outside. Tools that say they
    /// reach an open world (`openWorldHint`) are untrusted whatever this says.
    #[must_use]
    pub fn untrusted(mut self, untrusted: bool) -> Self {
        self.untrusted = untrusted;
        self
    }

    /// Whether the server is marked untrusted.
    pub fn is_untrusted(&self) -> bool {
        self.untrusted
    }

    fn failed(name: &str, error: McpError, limits: Limits) -> Self {
        Self {
            name: name.to_owned(),
            state: watch::channel(State::Failed(error)).1,
            starting: Mutex::new(None),
            call_limit: limits.call,
            untrusted: false,
        }
    }

    /// The longest one call may take.
    pub(crate) fn call_limit(&self) -> Duration {
        self.call_limit
    }

    /// The server's name, as configured.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether it is still starting.
    pub fn starting(&self) -> bool {
        matches!(*self.state.borrow(), State::Starting)
    }

    /// Waits until the server has started, or failed to.
    pub async fn started(&self) -> Result<(), McpError> {
        self.connection().await.map(|_| ())
    }

    /// The connection, once started.
    pub(crate) async fn connection(&self) -> Result<Arc<Connection>, McpError> {
        let mut state = self.state.clone();
        let settled = state
            .wait_for(|state| !matches!(state, State::Starting))
            .await
            .map(|state| state.clone());
        match settled {
            Ok(State::Ready(connection)) => Ok(connection),
            Ok(State::Failed(error)) => Err(error),
            Ok(State::Starting) | Err(_) => Err(McpError::start(&self.name, "it was closed")),
        }
    }

    /// Stops the server: a start still under way is abandoned, and a started server has its
    /// input closed, then is killed if it has not exited within a few seconds.
    pub async fn close(&self) {
        let starting = self
            .starting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(starting) = starting {
            starting.abort();
            // Aborted or finished, it is over; dropping a start kills its process.
            let _ = starting.await;
        }
        let state = self.state.borrow().clone();
        if let State::Ready(connection) = state {
            connection.close().await;
        }
    }

    /// The server's tools exactly as it lists them, once started: for a gateway that serves
    /// them on, annotations and all.
    pub async fn listing(&self) -> Result<Vec<rmcp::model::Tool>, McpError> {
        let connection = self.connection().await?;
        connection
            .listing()
            .await
            .map_err(|message| McpError::start(&self.name, message))
    }

    /// Calls the server's `tool` with `arguments`, within the call limit, giving back the
    /// server's own result: for a gateway that serves its tools on. Why it failed otherwise.
    pub async fn forward(
        &self,
        tool: &str,
        arguments: Option<rmcp::model::JsonObject>,
    ) -> Result<rmcp::model::CallToolResult, String> {
        self.forward_with(tool, arguments, None).await
    }

    /// Calls the server's `tool` as [`McpServer::forward`] does, sending `meta` as the
    /// request's `_meta`: who and where the call is for, when glave's own server needs it.
    pub async fn forward_with(
        &self,
        tool: &str,
        arguments: Option<rmcp::model::JsonObject>,
        meta: Option<rmcp::model::JsonObject>,
    ) -> Result<rmcp::model::CallToolResult, String> {
        let connection = self.connection().await.map_err(|err| err.to_string())?;
        connection
            .forward(tool, arguments, meta, self.call_limit)
            .await
    }

    /// Reads the resource at `uri`, sending `meta` as the request's `_meta`.
    pub async fn read_with(
        &self,
        uri: &str,
        meta: Option<rmcp::model::JsonObject>,
        cancel: &dyn Cancellation,
    ) -> Result<Vec<rmcp::model::ResourceContents>, String> {
        let connection = self.connection().await.map_err(|err| err.to_string())?;
        connection.read(uri, meta, cancel, self.call_limit).await
    }

    async fn list(&self) -> Result<Vec<ToolInfo>, ToolError> {
        let connection = self.connection().await.map_err(|err| ToolError {
            source_name: self.name.clone(),
            message: err.to_string(),
        })?;
        connection.list().await
    }

    async fn run(
        &self,
        name: &str,
        arguments: &Arguments,
        cancel: &dyn Cancellation,
    ) -> ToolOutput {
        match unless_cancelled(cancel, self.connection()).await {
            Some(Ok(connection)) => {
                let mut output = connection
                    .call(name, arguments, cancel, self.call_limit)
                    .await;
                output.untrusted |= self.untrusted;
                output
            }
            Some(Err(err)) => ToolOutput::text(true, err.to_string()),
            None => ToolOutput::text(true, CANCELLED),
        }
    }
}

/// The program for a server, with only the environment it should have. If glave goes
/// without closing it, the server goes too.
fn process(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
    dir: &Path,
) -> Command {
    let mut process = Command::new(command);
    process
        .args(args)
        .current_dir(dir)
        .env_clear()
        .kill_on_drop(true);
    for variable in PASSED_ON {
        if let Some(value) = environment.get(*variable) {
            process.env(variable, value);
        }
    }
    process.envs(env);
    process
}

impl Drop for McpServer {
    fn drop(&mut self) {
        if let Some(starting) = self
            .starting
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            starting.abort();
        }
    }
}

/// The most of a server's instructions the model is given.
const INSTRUCTION_CHARS: usize = 4_000;

impl ToolSource for McpServer {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        Box::pin(self.list())
    }

    fn instructions(&self) -> Option<String> {
        let State::Ready(connection) = &*self.state.borrow() else {
            return None;
        };
        let text = connection.instructions()?;
        let total = text.chars().count();
        let text = if total > INSTRUCTION_CHARS {
            let kept: String = text.chars().take(INSTRUCTION_CHARS).collect();
            format!("{kept}\n[cut short: {total} characters]")
        } else {
            text
        };
        Some(format!("## {}\n\n{text}", self.name))
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
