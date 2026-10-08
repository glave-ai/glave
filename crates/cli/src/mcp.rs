//! The MCP servers in glave's settings, `mcp.json` in glave's folder: started in the background
//! with the session, waited for before the first turn, and closed when glave ends.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context as _;
use glave_mcp::{Limits, McpServer};
use glave_ports::ToolSource;

use crate::terminal::Printer;

/// The settings file, in glave's folder.
pub const FILE: &str = "mcp.json";

/// The servers of one session.
pub struct Servers {
    servers: Vec<Arc<McpServer>>,
    reported: AtomicBool,
}

impl Servers {
    /// Starts every server in `home`'s settings, in `root`. With no settings file there are none.
    pub async fn start(home: &Path, root: &Path) -> anyhow::Result<Self> {
        let path = home.join(FILE);
        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("could not read {}", path.display()));
            }
        };
        let environment: BTreeMap<String, String> = std::env::vars_os()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        let configs = if text.is_empty() {
            BTreeMap::new()
        } else {
            glave_mcp::parse(&text, &environment)
                .with_context(|| format!("in {}", path.display()))?
        };
        let servers = configs
            .iter()
            .map(|(name, config)| {
                Arc::new(McpServer::start(
                    name,
                    config,
                    &environment,
                    root,
                    Limits::default(),
                ))
            })
            .collect();
        Ok(Self {
            servers,
            reported: AtomicBool::new(false),
        })
    }

    /// The servers as tool sources, in the order of their names.
    pub fn sources(&self) -> Vec<Arc<dyn ToolSource>> {
        self.servers
            .iter()
            .map(|server| Arc::clone(server) as Arc<dyn ToolSource>)
            .collect()
    }

    /// The servers' names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.servers.iter().map(|server| server.name()).collect()
    }

    /// Before the first turn only: waits for every server to start, and says why any did not.
    pub async fn report(&self, printer: &mut Printer) {
        if self.reported.swap(true, Ordering::SeqCst) {
            return;
        }
        let starting: Vec<&str> = self
            .servers
            .iter()
            .filter(|server| server.starting())
            .map(|server| server.name())
            .collect();
        if !starting.is_empty() {
            printer.status(&format!("starting MCP servers: {}", starting.join(", ")));
        }
        // They start side by side, so this waits only as long as the slowest.
        for server in &self.servers {
            if let Err(err) = server.started().await {
                printer.warning(&err.to_string());
            }
        }
    }

    pub async fn close(&self) {
        for server in &self.servers {
            server.close().await;
        }
    }
}
