//! The MCP servers in glave's settings, `mcp.json` in glave's folder: started in the background
//! with the session, waited for before the first turn, and closed when glave ends. A
//! `config.toml` may turn on only some of them, and a policy may forbid them all.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, bail};
use glave_mcp::{Limits, McpServer, Resources, ServerConfig};
use glave_ports::ToolSource;

use crate::policy::{Chosen, Constraints, check_own_servers};
use crate::terminal::Printer;

/// The settings file, in glave's folder.
pub const FILE: &str = "mcp.json";

/// The servers of one session.
pub struct Servers {
    servers: Vec<Arc<McpServer>>,
    reported: AtomicBool,
}

impl Servers {
    /// Starts the servers in `home`'s settings, in `root`: those `chosen`, or every one. With no
    /// settings file there are none. Fails when `constraints` forbid any of them.
    pub async fn start(
        home: &Path,
        root: &Path,
        chosen: Option<&Chosen<Vec<String>>>,
        constraints: &Constraints,
    ) -> anyhow::Result<Self> {
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
        let configs = turned_on(configs, chosen, &path)?;
        let names: Vec<&str> = configs.keys().map(String::as_str).collect();
        check_own_servers(&names, constraints)?;
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

    /// The servers as tool sources, in the order of their names, then their resources.
    pub fn sources(&self) -> Vec<Arc<dyn ToolSource>> {
        let mut sources: Vec<Arc<dyn ToolSource>> = self
            .servers
            .iter()
            .map(|server| Arc::clone(server) as Arc<dyn ToolSource>)
            .collect();
        sources.push(Arc::new(Resources::new(self.servers.clone())));
        sources
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

    /// Closes every server side by side, so one slow to exit holds up no other.
    pub async fn close(&self) {
        let mut closing = tokio::task::JoinSet::new();
        for server in &self.servers {
            let server = Arc::clone(server);
            closing.spawn(async move { server.close().await });
        }
        while closing.join_next().await.is_some() {}
    }
}

/// The servers of `configs` that `chosen` names, or all of them when it names none. A name
/// with no server in the settings file is an error, so a typo never quietly turns one off.
fn turned_on(
    mut configs: BTreeMap<String, ServerConfig>,
    chosen: Option<&Chosen<Vec<String>>>,
    path: &Path,
) -> anyhow::Result<BTreeMap<String, ServerConfig>> {
    let Some(chosen) = chosen else {
        return Ok(configs);
    };
    let missing: Vec<&str> = chosen
        .value
        .iter()
        .filter(|name| !configs.contains_key(*name))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        bail!(
            "mcp (from {}) names servers {} has no settings for: {}",
            chosen.from,
            path.display(),
            missing.join(", ")
        );
    }
    configs.retain(|name, _| chosen.value.contains(name));
    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::tests::constraints;

    #[test]
    fn a_policy_that_forbids_people_s_own_servers_stops_them_starting() {
        let home = std::env::temp_dir().join(format!("glave-mcp-policy-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let servers = r#"{"mcpServers": {"mine": {"command": "/nonexistent/server"}}}"#;
        std::fs::write(home.join(FILE), servers).unwrap();
        let forbidden = constraints("version = 1\n[mcp]\nown = false\n");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let start = |chosen| runtime.block_on(Servers::start(&home, &home, chosen, &forbidden));
        let refused = start(None).err().unwrap().to_string();
        assert!(
            refused.contains("mine") && refused.contains("[mcp] own = false"),
            "{refused}"
        );
        let none = Chosen {
            value: Vec::new(),
            from: crate::policy::Source::Default,
        };
        assert_eq!(start(Some(&none)).unwrap().names(), Vec::<&str>::new());
    }
}
