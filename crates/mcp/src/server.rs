//! One MCP server as a tool source. It starts in the background the moment it is made, so
//! glave never waits for a server it has not asked anything yet; its first listing waits for it.
//! One that fails to start is started again at a later ask, in the background (`again.rs`). How
//! it serves as a tool source is `server/source.rs`.

mod source;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use glave_event::Arguments;
use glave_ports::auth::Auth;
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::{
    BoxFuture, Called, Cancellation, Clock, ToolError, ToolInfo, ToolOutput, unless_cancelled,
};
use glave_process::{Child, ProcessError, Processes, Program};
use http::{HeaderName, HeaderValue};
use rmcp::service::RoleClient;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{IntoTransport, StreamableHttpClientTransport};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::McpError;
use crate::again::Again;
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

/// One start of a server, from its transport to its connection or why it failed.
type Start = Arc<dyn Fn() -> BoxFuture<'static, State> + Send + Sync>;

/// What a server says while it starts again, which nothing waits for.
const STARTING_AGAIN: &str = "it failed to start, and is starting again";

/// An MCP server: starting, started, or failed to start.
pub struct McpServer {
    name: String,
    state: Arc<watch::Sender<State>>,
    /// How it starts again after a failure, when it can.
    start: Option<Start>,
    /// Its failed starts, and when it is next started again.
    again: Mutex<Again>,
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
    /// The person's name for the account of the service it is signed in to, when they named
    /// one: each call, and an approval of it, names it.
    account: Option<String>,
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
                let (command, args, env) = (command.clone(), args.clone(), env.clone());
                let (environment, dir) = (environment.clone(), dir.to_owned());
                let program = move || program(&command, &args, &env, &environment, &dir);
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
        program: impl Fn() -> Program + Send + Sync + 'static,
        processes: Processes,
        root: Option<PathBuf>,
        limits: Limits,
    ) -> Self {
        let owned = name.to_owned();
        let transport = move || {
            let (mut program, processes, owned) = (program(), processes.clone(), owned.clone());
            async move {
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
            }
        };
        let opening = Opening {
            catalog: false,
            root,
        };
        Self::open(name, transport, limits, opening, true)
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
        let transport = move || {
            let transport =
                StreamableHttpClientTransport::with_client(client.clone(), config.clone());
            async { Ok((transport, None)) }
        };
        Ok(Self::open(name, transport, limits, opening, true))
    }

    /// Starts talking MCP over `transport`.
    pub fn connect<T, E, A>(name: &str, transport: T, limits: Limits) -> Self
    where
        T: IntoTransport<RoleClient, E, A> + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
        A: 'static,
    {
        // Given once, so started once.
        let given = Mutex::new(Some(transport));
        let owned = name.to_owned();
        let transport = move || {
            let taken = given.lock().unwrap_or_else(PoisonError::into_inner).take();
            let given = taken.ok_or_else(|| McpError::start(&owned, "it was started once"));
            async { given.map(|transport| (transport, None)) }
        };
        Self::open(name, transport, limits, Opening::default(), false)
    }

    /// Talks MCP over the transport `transport` makes, in the background, with the server's
    /// process when it has one: kept while connected, and stopped if it never connects. One that
    /// fails is started again with a new transport, when `again` says it may be.
    fn open<T, E, A, F>(
        name: &str,
        transport: impl Fn() -> F + Send + Sync + 'static,
        limits: Limits,
        opening: Opening,
        again: bool,
    ) -> Self
    where
        T: IntoTransport<RoleClient, E, A> + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
        A: 'static,
        F: Future<Output = Result<(T, Option<Child>), McpError>> + Send + 'static,
    {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            let error = McpError::start(name, "glave started it outside its async runtime");
            return Self::failed(name, error, limits);
        };
        let owned = name.to_owned();
        let process = Arc::new(Mutex::new(None));
        let held = Arc::clone(&process);
        let start: Start = Arc::new(move || {
            let (name, opening, held) = (owned.clone(), opening.clone(), Arc::clone(&held));
            let transport = transport();
            Box::pin(async move {
                let opened = match transport.await {
                    Ok((transport, child)) => {
                        let opened = Connection::open(&name, transport, limits.startup, opening);
                        let opened = opened.await;
                        if opened.is_ok() {
                            *held.lock().unwrap_or_else(PoisonError::into_inner) = child;
                        }
                        opened
                    }
                    Err(error) => Err(error),
                };
                match opened {
                    Ok(connection) => State::Ready(Arc::new(connection)),
                    Err(err) => State::Failed(err),
                }
            })
        });
        let state = Arc::new(watch::Sender::new(State::Starting));
        let starting = begin(&runtime, &state, &start);
        Self {
            name: name.to_owned(),
            state,
            start: again.then_some(start),
            again: Mutex::new(Again::default()),
            starting: Mutex::new(Some(starting)),
            process,
            waits: limits.waits(),
            untrusted: false,
            search: None,
            account: None,
        }
    }

    /// Times its starts again by `clock`, not the computer's.
    #[must_use]
    pub fn timed_by(mut self, clock: Arc<dyn Clock>) -> Self {
        *self.again.get_mut().unwrap_or_else(PoisonError::into_inner) = Again::new(clock);
        self
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

    /// Names `account`, the person's name for the account of the service it is signed in to,
    /// in each call's line, so an approval says which account would act: `mcp__stripe__refund
    /// (Acme) {…}`.
    #[must_use]
    pub fn account(mut self, account: Option<String>) -> Self {
        self.account = account;
        self
    }

    /// Whether the server is marked untrusted.
    pub fn is_untrusted(&self) -> bool {
        self.untrusted
    }

    fn failed(name: &str, error: McpError, limits: Limits) -> Self {
        Self {
            name: name.to_owned(),
            state: Arc::new(watch::Sender::new(State::Failed(error))),
            start: None,
            again: Mutex::new(Again::default()),
            starting: Mutex::new(None),
            process: Arc::new(Mutex::new(None)),
            waits: limits.waits(),
            untrusted: false,
            search: None,
            account: None,
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

    /// Waits until the server has started, or failed to: the start under way, one started
    /// again included.
    pub async fn started(&self) -> Result<(), McpError> {
        match self.settled().await {
            State::Ready(_) => Ok(()),
            State::Failed(error) => Err(error),
            State::Starting => Err(McpError::start(&self.name, "it was closed")),
        }
    }

    /// How the start under way ends.
    async fn settled(&self) -> State {
        let mut state = self.state.subscribe();
        let settled = state
            .wait_for(|state| !matches!(state, State::Starting))
            .await
            .map(|state| state.clone());
        settled.unwrap_or(State::Starting)
    }

    fn again(&self) -> MutexGuard<'_, Again> {
        self.again.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The connection, once started: its first start waited for, a start again never, and a
    /// failed one started again when it is due.
    pub(crate) async fn connection(&self) -> Result<Arc<Connection>, McpError> {
        let now = self.state.borrow().clone();
        let state = match now {
            State::Starting if self.again().retrying() => {
                return Err(McpError::start(&self.name, STARTING_AGAIN));
            }
            State::Starting => self.settled().await,
            settled => settled,
        };
        match state {
            State::Ready(connection) => Ok(connection),
            State::Failed(error) => {
                self.start_again_if_due();
                Err(error)
            }
            State::Starting => Err(McpError::start(&self.name, "it was closed")),
        }
    }

    /// Starts the server again in the background, when it failed and its wait is over.
    fn start_again_if_due(&self) {
        let (Some(start), Ok(runtime)) = (&self.start, tokio::runtime::Handle::try_current())
        else {
            return;
        };
        // Decided and begun under the one lock, so two asks at once start it once.
        let mut again = self.again();
        let failed = matches!(*self.state.borrow(), State::Failed(_));
        if !failed || !again.failed() {
            return;
        }
        let starting = begin(&runtime, &self.state, start);
        drop(again);
        // The start before is over, since the server had failed.
        *self.starting.lock().unwrap_or_else(PoisonError::into_inner) = Some(starting);
    }

    /// Stops the server: a start still under way is abandoned, and a started server has its
    /// input closed, then is stopped with everything it started if it has not exited within a
    /// few seconds.
    pub async fn close(&self) {
        self.again().close();
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
        let closed = McpError::start(&self.name, "it was closed");
        self.state.send_if_modified(|state| {
            let starting = matches!(state, State::Starting);
            if starting {
                *state = State::Failed(closed);
            }
            starting
        });
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

/// Begins `start` in the background on `runtime`, how it ends told through `state`.
fn begin(
    runtime: &tokio::runtime::Handle,
    state: &Arc<watch::Sender<State>>,
    start: &Start,
) -> JoinHandle<()> {
    state.send_replace(State::Starting);
    let (state, start) = (Arc::clone(state), Arc::clone(start));
    runtime.spawn(async move {
        state.send_replace(start().await);
    })
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
