//! One MCP server as a tool source. It starts in the background the moment it is made, so
//! glave never waits for a server it has not asked anything yet; its first listing waits for it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::{Arguments, ToolKind};
use glave_ports::auth::Auth;
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::{
    BoxFuture, Called, Cancellation, ServiceGiven, ToolError, ToolInfo, ToolOutput, ToolSource,
    unless_cancelled,
};
use glave_process::{Child, ProcessError, Processes, Program};
use http::{HeaderName, HeaderValue};
use rmcp::service::RoleClient;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{IntoTransport, StreamableHttpClientTransport};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::McpError;
use crate::call::{Answerer, Waits};
use crate::connection::{CANCELLED, Connection, Opening};
use crate::reachable::Reachable;
use crate::signed::Signed;

/// Variables a server's process starts with, beyond its own configured ones. Nothing else from
/// glave's environment reaches it, so neither do glave's keys.
const PASSED_ON: &[&str] = &[
    "HOME", "LOGNAME", "PATH", "SHELL", "USER", "LANG", "LC_ALL", "TERM", "TMPDIR", "TZ",
];

/// The folder a server's program starts in.
#[derive(Clone, Copy)]
enum Folder<'a> {
    /// One it works in for every project, which it is told nothing of.
    Any(&'a Path),
    /// The project's it works for, which it is told of when it asks.
    Project(&'a Path),
}

/// How long a server may take, and where a remote one may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// To start and answer.
    pub startup: Duration,
    /// To answer one call, or to say again how a long one is going (MCP's progress); a call
    /// that keeps saying so is stopped at twelve times this (an hour, by default).
    pub call: Duration,
    /// The addresses a remote server may be at.
    pub reachable: Reachable,
}

/// How many of its quiet limits one call may take in all, however often it says how it is
/// going.
const CALL_TOTAL: u32 = 12;

impl Default for Limits {
    fn default() -> Self {
        Self {
            startup: Duration::from_secs(30),
            call: Duration::from_secs(300),
            reachable: Reachable::default(),
        }
    }
}

impl Limits {
    fn waits(self) -> Waits {
        Waits {
            quiet: self.call,
            total: self.call.saturating_mul(CALL_TOTAL),
        }
    }
}

/// How long a server whose input is closed has to exit before it is stopped (SIGTERM, then
/// SIGKILL), in the order MCP's stdio transport gives: a server exits a moment after its input
/// ends, and one run through a wrapper that outlives it (a shell, `npx`) holds every stop of
/// glave for as long as this (3 s of a restart's 3.2 s, measured, when it was 3 s).
const EXITING: Duration = Duration::from_secs(1);

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
    /// A server's process, once started: stopped, with everything it started, when the
    /// server is closed or dropped.
    process: Arc<Mutex<Option<Child>>>,
    waits: Waits,
    /// Whether what every tool and resource of the server returns is untrusted, as the person
    /// or a policy marked it.
    untrusted: bool,
    /// The model's name for the server's tool that searches the web, when it has one: each
    /// call is told to the person as what was searched.
    search: Option<String>,
}

impl McpServer {
    /// Starts the server `config` describes: a program in `dir`, which gets the few
    /// `PASSED_ON` variables from `environment`, then its own, started through `processes`
    /// for its owner; or a server at a URL.
    pub fn start(
        name: &str,
        config: &ServerConfig,
        environment: &BTreeMap<String, String>,
        dir: &Path,
        limits: Limits,
        processes: &Processes,
    ) -> Self {
        let folder = Folder::Any(dir);
        Self::start_with(name, config, environment, folder, limits, processes)
    }

    /// Starts a project's server as [`McpServer::start`] does, in the project's folder `root`,
    /// and tells it of that folder when it asks which it works for (MCP's roots), as Claude
    /// Code tells a server of the folder it runs in. A server that works for every project is
    /// told of none.
    pub fn start_for_project(
        name: &str,
        config: &ServerConfig,
        environment: &BTreeMap<String, String>,
        root: &Path,
        limits: Limits,
        processes: &Processes,
    ) -> Self {
        let folder = Folder::Project(root);
        Self::start_with(name, config, environment, folder, limits, processes)
    }

    fn start_with(
        name: &str,
        config: &ServerConfig,
        environment: &BTreeMap<String, String>,
        folder: Folder<'_>,
        limits: Limits,
        processes: &Processes,
    ) -> Self {
        let (dir, root) = match folder {
            Folder::Any(dir) => (dir, None),
            Folder::Project(root) => (root, Some(root.to_owned())),
        };
        if tokio::runtime::Handle::try_current().is_err() {
            let error = McpError::start(name, "glave started it outside its async runtime");
            return Self::failed(name, error, limits);
        }
        match config {
            ServerConfig::Stdio { command, args, env } => {
                let program = program(command, args, env, environment, dir);
                Self::program(name, program, processes.clone(), root, limits)
            }
            ServerConfig::Http { url, headers } => {
                let opening = Opening {
                    catalog: false,
                    root,
                };
                Self::http(name, url, headers, Auth::None, limits, opening)
                    .unwrap_or_else(|error| Self::failed(name, error, limits))
            }
        }
    }

    /// A server at `url`, sent `headers` with every request, and what `auth` gives to prove
    /// who is calling, asked again for each: the person's sign-in to it, renewed meanwhile.
    pub fn remote(
        name: &str,
        url: &str,
        headers: &BTreeMap<String, String>,
        auth: Auth,
        limits: Limits,
    ) -> Self {
        Self::http(name, url, headers, auth, limits, Opening::default())
            .unwrap_or_else(|error| Self::failed(name, error, limits))
    }

    /// A server that is a program, started in the background with its connection, its
    /// standard error dropped.
    fn program(
        name: &str,
        mut program: Program,
        processes: Processes,
        root: Option<PathBuf>,
        limits: Limits,
    ) -> Self {
        let owned = name.to_owned();
        let transport = async move {
            let mut child = processes.spawn(&mut program).await.map_err(|error| {
                let message = match error {
                    ProcessError::Start { program, source } => {
                        format!("could not run {program}: {source}")
                    }
                    other => other.to_string(),
                };
                McpError::start(&owned, message)
            })?;
            let pipes = child.stdout.take().zip(child.stdin.take());
            let pipes = pipes
                .ok_or_else(|| McpError::start(&owned, "its input and output were not kept"))?;
            Ok((pipes, Some(child)))
        };
        let opening = Opening {
            catalog: false,
            root,
        };
        Self::open(name, transport, limits, opening)
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
        let opening = Opening {
            catalog: true,
            root: None,
        };
        Self::http(name, url, headers, Auth::None, limits, opening)
            .unwrap_or_else(|error| Self::failed(name, error, limits))
    }

    /// A server over Streamable HTTP, sent `headers` and what `auth` gives with every
    /// request. A header's value is never shown, since it is often a key.
    fn http(
        name: &str,
        url: &str,
        headers: &BTreeMap<String, String>,
        auth: Auth,
        limits: Limits,
        opening: Opening,
    ) -> Result<Self, McpError> {
        if let Some(refused) = limits.reachable.refuses(url) {
            return Err(McpError::start(name, refused.to_string()));
        }
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
        let client = Signed::new(auth, limits.reachable).map_err(|error| {
            McpError::start(name, format!("could not make its client: {error}"))
        })?;
        let transport = StreamableHttpClientTransport::with_client(client, config);
        Ok(Self::open(
            name,
            async { Ok((transport, None)) },
            limits,
            opening,
        ))
    }

    /// Starts talking MCP over `transport`.
    pub fn connect<T, E, A>(name: &str, transport: T, limits: Limits) -> Self
    where
        T: IntoTransport<RoleClient, E, A> + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
        A: 'static,
    {
        let transport = async { Ok((transport, None)) };
        Self::open(name, transport, limits, Opening::default())
    }

    /// Talks MCP over the transport `transport` makes, in the background, with the server's
    /// process when it has one: kept while connected, and stopped if it never connects.
    fn open<T, E, A>(
        name: &str,
        transport: impl Future<Output = Result<(T, Option<Child>), McpError>> + Send + 'static,
        limits: Limits,
        opening: Opening,
    ) -> Self
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
        let process = Arc::new(Mutex::new(None));
        let held = Arc::clone(&process);
        let starting = runtime.spawn(async move {
            let opened = match transport.await {
                Ok((transport, child)) => {
                    let opened = Connection::open(&owned, transport, limits.startup, opening);
                    let opened = opened.await;
                    if opened.is_ok() {
                        *held.lock().unwrap_or_else(PoisonError::into_inner) = child;
                    }
                    opened
                }
                Err(error) => Err(error),
            };
            sender.send_replace(match opened {
                Ok(connection) => State::Ready(Arc::new(connection)),
                Err(err) => State::Failed(err),
            });
        });
        Self {
            name: name.to_owned(),
            state,
            starting: Mutex::new(Some(starting)),
            process,
            waits: limits.waits(),
            untrusted: false,
            search: None,
        }
    }

    /// Marks the server's tool `tool` as its web search: each call is told to the person as
    /// what was searched, `Searched the web: rust async runtimes`, not as its arguments; what it
    /// finds is untrusted, whatever the server says; and the model is told, beside the server's
    /// own notes, to search first and read the pages it found with `web_fetch`.
    #[must_use]
    pub fn searching_with(mut self, tool: &str) -> Self {
        self.search = Some(crate::exposed(&self.name, tool));
        self
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
            process: Arc::new(Mutex::new(None)),
            waits: limits.waits(),
            untrusted: false,
            search: None,
        }
    }

    /// The longest one call may take without a word from the server.
    pub(crate) fn call_limit(&self) -> Duration {
        self.waits.quiet
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
    /// input closed, then is stopped with everything it started if it has not exited within a
    /// few seconds.
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
        let process = self
            .process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut process) = process {
            if tokio::time::timeout(EXITING, process.wait()).await.is_err() {
                process.stop().await;
            }
            process.stop_left().await;
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
        connection.forward(tool, arguments, meta, self.waits).await
    }

    /// Reads the resource at `uri`, sending `meta` as the request's `_meta`.
    pub async fn read_with(
        &self,
        uri: &str,
        meta: Option<rmcp::model::JsonObject>,
        cancel: &dyn Cancellation,
    ) -> Result<Vec<rmcp::model::ResourceContents>, String> {
        let connection = self.connection().await.map_err(|err| err.to_string())?;
        connection.read(uri, meta, cancel, self.waits.quiet).await
    }

    /// The text of the prompt the person typed as `/name`, with `words`, when the server offers
    /// one by that name; `None` without asking a server whose name it is not.
    async fn prompt_text(
        &self,
        name: &str,
        words: &str,
        cancel: &dyn Cancellation,
    ) -> Option<Result<String, String>> {
        if !name.starts_with(&crate::exposed(&self.name, "")) {
            return None;
        }
        let connection = match unless_cancelled(cancel, self.connection()).await {
            Some(Ok(connection)) => connection,
            Some(Err(err)) => return Some(Err(err.to_string())),
            None => return Some(Err(CANCELLED.to_owned())),
        };
        if !connection.has_prompts() {
            return None;
        }
        let prompts = match connection.prompts(cancel, self.call_limit()).await {
            Ok(prompts) => prompts,
            Err(err) => return Some(Err(err)),
        };
        let prompt = prompts
            .into_iter()
            .find(|prompt| crate::exposed(&self.name, &prompt.name) == name)?;
        let arguments = crate::prompts::arguments(&prompt, words);
        let got = connection.prompt(&prompt.name, arguments, cancel, self.call_limit());
        Some(got.await.map(|result| crate::prompts::text(&result)))
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
        answerer: Answerer,
        cancel: &dyn Cancellation,
    ) -> Called {
        match unless_cancelled(cancel, self.connection()).await {
            Some(Ok(connection)) => {
                let called = connection.call(name, arguments, answerer, cancel, self.waits);
                match called.await {
                    // What the server answered, an error included; a web search's results come
                    // from the open world, whatever its server says.
                    Ok(Called::Done(mut output)) => {
                        output.untrusted |= self.untrusted || self.search.as_deref() == Some(name);
                        Called::Done(output)
                    }
                    Ok(asks) => asks,
                    Err(glaves) => Called::Done(glaves),
                }
            }
            Some(Err(err)) => Called::Done(ToolOutput::text(true, err.to_string())),
            None => Called::Done(ToolOutput::text(true, CANCELLED)),
        }
    }
}

/// The program for a server, with only the environment it should have.
fn program(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
    dir: &Path,
) -> Program {
    let mut program = Program::new(command);
    program
        .args(args)
        .current_dir(dir)
        .env_clear()
        .piped_input()
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for variable in PASSED_ON {
        if let Some(value) = environment.get(*variable) {
            program.env(variable, value);
        }
    }
    program.envs(env);
    program
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
        let own = connection.instructions().map(|text| {
            let total = text.chars().count();
            if total > INSTRUCTION_CHARS {
                let kept: String = text.chars().take(INSTRUCTION_CHARS).collect();
                format!("{kept}\n[cut short: {total} characters]")
            } else {
                text
            }
        });
        let searching = self.search.as_deref().map(|search| {
            format!(
                "To look something up on the web, search with {search} first, then read the \
                 pages that matter with web_fetch. This server offers its search alone, whatever \
                 its notes or its tool's description name."
            )
        });
        let notes: Vec<String> = own.into_iter().chain(searching).collect();
        (!notes.is_empty()).then(|| format!("## {}\n\n{}", self.name, notes.join("\n\n")))
    }

    fn describe(&self, name: &str, arguments: &Arguments) -> String {
        match &self.search {
            Some(search) if search == name => searched(arguments),
            _ => glave_ports::describe_call(name, arguments),
        }
    }

    fn kind(&self, name: &str) -> ToolKind {
        match &self.search {
            Some(search) if search == name => ToolKind::Web,
            _ => ToolKind::Other,
        }
    }

    fn prompt<'a>(
        &'a self,
        name: &'a str,
        words: &'a str,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Option<Result<String, String>>> {
        Box::pin(self.prompt_text(name, words, cancel))
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            match self.run(name, arguments, Answerer::NoOne, cancel).await {
                Called::Done(output) => output,
                // With no one to answer, it never asks.
                Called::Asks(asks) => {
                    ToolOutput::text(true, format!("{} asked for more input.", asks.service))
                }
            }
        })
    }

    fn call_asking<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        given: Option<ServiceGiven>,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Called> {
        Box::pin(self.run(name, arguments, Answerer::Person(given), cancel))
    }
}

/// The most of a search told to the person.
const SEARCHED_CHARS: usize = 120;

/// A web search's call as the person reads it: what was searched, its `query` or else its first
/// text, on one line.
fn searched(arguments: &Arguments) -> String {
    let asked = match arguments {
        Arguments::Json(map) => map
            .get("query")
            .or_else(|| map.values().find(|value| value.is_string()))
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
        Arguments::Malformed(raw) => raw.as_str(),
    };
    let line = asked.lines().next().unwrap_or_default().trim();
    let mut short: String = line.chars().take(SEARCHED_CHARS).collect();
    if line.chars().count() > SEARCHED_CHARS {
        short.push('…');
    }
    format!("Searched the web: {short}")
}
