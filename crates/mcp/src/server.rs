//! One MCP server as a tool source. It starts in the background the moment it is made, so
//! glave never waits for a server it has not asked anything yet; its first listing waits for it.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::Arguments;
use glave_ports::{
    BoxFuture, Cancellation, ToolError, ToolInfo, ToolOutput, ToolSource, unless_cancelled,
};
use rmcp::service::RoleClient;
use rmcp::transport::{IntoTransport, TokioChildProcess};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::McpError;
use crate::config::ServerConfig;
use crate::connection::Connection;

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
}

impl McpServer {
    /// Starts the server `config` describes, in `dir`. A program gets the few `PASSED_ON`
    /// variables from `environment`, then its own.
    pub fn start(
        name: &str,
        config: &ServerConfig,
        environment: &BTreeMap<String, String>,
        dir: &Path,
        limits: Limits,
    ) -> Self {
        match config {
            ServerConfig::Stdio { command, args, env } => {
                let mut process = Command::new(command);
                // If glave goes without closing it, the server goes too.
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
                let spawned = TokioChildProcess::builder(process)
                    .stderr(Stdio::null())
                    .spawn();
                match spawned {
                    Ok((transport, _)) => Self::connect(name, transport, limits),
                    Err(err) => Self::failed(
                        name,
                        McpError::start(name, format!("could not run {command}: {err}")),
                        limits,
                    ),
                }
            }
            ServerConfig::Http { .. } => Self::failed(
                name,
                McpError::start(name, "glave cannot reach MCP servers over HTTP yet"),
                limits,
            ),
        }
    }

    /// Starts talking MCP over `transport`.
    pub fn connect<T, E, A>(name: &str, transport: T, limits: Limits) -> Self
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
            let opened = Connection::open(&owned, transport, limits.startup).await;
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
        }
    }

    fn failed(name: &str, error: McpError, limits: Limits) -> Self {
        Self {
            name: name.to_owned(),
            state: watch::channel(State::Failed(error)).1,
            starting: Mutex::new(None),
            call_limit: limits.call,
        }
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

    async fn connection(&self) -> Result<Arc<Connection>, McpError> {
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
                connection
                    .call(name, arguments, cancel, self.call_limit)
                    .await
            }
            Some(Err(err)) => ToolOutput::text(true, err.to_string()),
            None => ToolOutput::text(true, "[stopped: the person cancelled the turn]"),
        }
    }
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

impl ToolSource for McpServer {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        Box::pin(self.list())
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
