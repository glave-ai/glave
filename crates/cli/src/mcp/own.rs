//! The person's own MCP servers, from `mcp.json` in glave's folder, for their threads
//! everywhere: started once in their home folder, and reconciled when their settings change
//! (`specs/one-server.md`, "Its settings"): an added server starts, a removed one stops, and a
//! changed one starts again. One a turn still uses is closed once no turn does.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Context as _;
use glave_config::policy::{Chosen, Constraints, check_own_servers};
use glave_mcp::{Limits, McpServer, ServerConfig};
use glave_ports::ToolSource;
use glave_process::Processes;

use super::{FILE, environment, mine_untrusted, turned_on};
use crate::gateway::remote::Remote;
use crate::terminal::Printer;

/// One of the person's servers as their settings describe it: started again when this
/// changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    name: String,
    config: ServerConfig,
    untrusted: bool,
}

/// What the person's settings turn on: the servers of `home`'s `mcp.json` that `chosen`
/// names, or every one, those `constraints` allow, and how far each is trusted. With no
/// settings file there are none. `through_gateway`: the person uses a gateway's catalog.
pub async fn wanted(
    home: &Path,
    chosen: Option<&Chosen<Vec<String>>>,
    constraints: &Constraints,
    through_gateway: bool,
) -> anyhow::Result<Vec<Wanted>> {
    let path = home.join(FILE);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("could not read {}", path.display())),
    };
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let own = glave_mcp::parse(&text, &environment())
        .with_context(|| format!("in {}", path.display()))?;
    let mine: Vec<String> = own.keys().cloned().collect();
    let configs = turned_on(own, chosen, &path)?;
    // Without a gateway, the catalog's servers are the ones in mcp.json it names.
    let catalog = match constraints.mcp_catalog() {
        Some((names, _)) if !through_gateway => names,
        _ => Vec::new(),
    };
    let on: Vec<&str> = mine
        .iter()
        .filter(|name| configs.contains_key(*name) && !catalog.contains(name))
        .map(String::as_str)
        .collect();
    check_own_servers(&on, constraints)?;
    let marks = glave_mcp::marks(&text).with_context(|| format!("in {}", path.display()))?;
    Ok(configs
        .into_iter()
        .map(|(name, config)| {
            let untrusted = mine_untrusted(&name, &config, marks.get(&name), constraints);
            Wanted {
                name,
                config,
                untrusted,
            }
        })
        .collect())
}

/// The servers running for the person, and those replaced while a turn still used them.
#[derive(Default)]
struct Running {
    /// Each with what started it, in the order of their names.
    servers: Vec<(Wanted, Arc<McpServer>)>,
    retired: Vec<Arc<McpServer>>,
}

/// The person's own servers, for their threads everywhere; first the catalog of the gateway
/// they use, when there is one.
pub struct Servers {
    catalog: Option<Arc<McpServer>>,
    running: Mutex<Running>,
    /// Where they start: the person's home folder.
    root: PathBuf,
    reported: AtomicBool,
    /// Where they started, and where each project's servers start too.
    processes: Processes,
}

impl Servers {
    /// Starts `wanted` in `root`, after the catalog of the `remote` gateway, when there is one.
    pub fn start(
        wanted: Vec<Wanted>,
        root: &Path,
        remote: Option<&Remote>,
        processes: &Processes,
    ) -> Self {
        let servers = Self {
            catalog: remote.map(|remote| Arc::new(remote.catalog())),
            running: Mutex::default(),
            root: root.to_owned(),
            reported: AtomicBool::new(false),
            processes: processes.clone(),
        };
        servers.lock().servers = wanted
            .into_iter()
            .map(|wanted| {
                let server = servers.started(&wanted);
                (wanted, server)
            })
            .collect();
        servers
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Running> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `wanted` started, in the person's home folder.
    fn started(&self, wanted: &Wanted) -> Arc<McpServer> {
        let (name, limits) = (&wanted.name, Limits::default());
        let server = McpServer::start(
            name,
            &wanted.config,
            &environment(),
            &self.root,
            limits,
            &self.processes,
        );
        Arc::new(server.untrusted(wanted.untrusted))
    }

    /// Runs `wanted` from now on: keeps the servers that stay the same, starts those added or
    /// changed, and puts away those removed or changed. Whether anything changed.
    pub fn reconcile(&self, wanted: Vec<Wanted>) -> bool {
        let mut running = self.lock();
        let before = std::mem::take(&mut running.servers);
        let unchanged = before.len() == wanted.len()
            && before
                .iter()
                .zip(&wanted)
                .all(|((had, _), want)| had == want);
        if unchanged {
            running.servers = before;
            return false;
        }
        let (mut kept, gone): (Vec<_>, Vec<_>) = before
            .into_iter()
            .partition(|(had, _)| wanted.contains(had));
        running
            .retired
            .extend(gone.into_iter().map(|(_, server)| server));
        running.servers = wanted
            .into_iter()
            .map(|want| {
                if let Some(at) = kept.iter().position(|(had, _)| *had == want) {
                    kept.swap_remove(at)
                } else {
                    let server = self.started(&want);
                    (want, server)
                }
            })
            .collect();
        true
    }

    /// Closes the servers put away that no turn uses any more.
    pub async fn tidy(&self) {
        let idle: Vec<Arc<McpServer>> = {
            let mut running = self.lock();
            let (busy, idle) = std::mem::take(&mut running.retired)
                .into_iter()
                .partition(|server| Arc::strong_count(server) > 1);
            running.retired = busy;
            idle
        };
        super::close(&idle).await;
    }

    /// Where glave's servers start: the person's, and each project's.
    pub fn processes(&self) -> &Processes {
        &self.processes
    }

    /// Every server running, the catalog first.
    fn all(&self) -> Vec<Arc<McpServer>> {
        let running = self.lock();
        self.catalog
            .iter()
            .cloned()
            .chain(running.servers.iter().map(|(_, server)| Arc::clone(server)))
            .collect()
    }

    /// The servers as tool sources, the catalog first, then in the order of their names,
    /// then their resources.
    pub fn sources(&self) -> Vec<Arc<dyn ToolSource>> {
        super::sources(&self.all())
    }

    /// The servers' names, in order.
    pub fn names(&self) -> Vec<String> {
        self.all()
            .iter()
            .map(|server| server.name().to_owned())
            .collect()
    }

    /// Before the first turn only: waits for every server to start, and says why any did not.
    pub async fn report(&self, printer: &mut Printer) {
        if self.reported.swap(true, Ordering::SeqCst) {
            return;
        }
        let all = self.all();
        let starting: Vec<&str> = all
            .iter()
            .filter(|server| server.starting())
            .map(|server| server.name())
            .collect();
        if !starting.is_empty() {
            printer.status(&format!("starting MCP servers: {}", starting.join(", ")));
        }
        // They start side by side, so this waits only as long as the slowest.
        for server in &all {
            if let Err(err) = server.started().await {
                printer.warning(&err.to_string());
            }
        }
    }

    /// Closes every server side by side, so one slow to exit holds up no other.
    pub async fn close(&self) {
        let retired = std::mem::take(&mut self.lock().retired);
        super::close(&[self.all(), retired].concat()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::tests::constraints;

    #[test]
    fn a_policy_that_forbids_people_s_own_servers_stops_them_starting() {
        let home = glave_ports::testing::TempDir::new("mcp-policy").unwrap();
        let servers = r#"{"mcpServers": {"mine": {"command": "/nonexistent/server"}}}"#;
        std::fs::write(home.join(FILE), servers).unwrap();
        let forbidden = constraints("version = 1\n[mcp]\nown = false\n");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let start = |chosen| runtime.block_on(wanted(&home, chosen, &forbidden, false));
        let refused = start(None).err().unwrap().to_string();
        assert!(
            refused.contains("mine") && refused.contains("[mcp] own = false"),
            "{refused}"
        );
        let none = Chosen {
            value: Vec::new(),
            from: glave_config::policy::Source::Default,
        };
        assert_eq!(start(Some(&none)).unwrap(), Vec::new());
        let offered = constraints("version = 1\n[mcp]\nown = false\ncatalog = [\"mine\"]\n");
        let started = runtime
            .block_on(wanted(&home, None, &offered, false))
            .unwrap();
        assert_eq!(
            started
                .iter()
                .map(|wanted| wanted.name.as_str())
                .collect::<Vec<_>>(),
            ["mine"],
            "without a gateway, a server of mcp.json the catalog names is the catalog's"
        );
    }

    /// A server named `name` running `command`, which does not exist.
    fn server(name: &str, command: &str) -> Wanted {
        Wanted {
            name: name.to_owned(),
            config: ServerConfig::Stdio {
                command: format!("/nonexistent/{command}"),
                args: Vec::new(),
                env: std::collections::BTreeMap::new(),
            },
            untrusted: false,
        }
    }

    #[test]
    fn reconciling_keeps_what_stays_starts_what_is_new_and_closes_what_went() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let root = std::env::temp_dir();
            let unrecorded = Processes::unrecorded();
            let started = vec![server("a", "a"), server("b", "b")];
            let servers = Servers::start(started.clone(), &root, None, &unrecorded);
            let first = |servers: &Servers| Arc::clone(&servers.lock().servers[0].1);
            let a = first(&servers);
            assert!(
                !servers.reconcile(started),
                "the same settings change nothing"
            );
            let held = servers.sources();
            assert!(servers.reconcile(vec![server("a", "a"), server("b", "b2"), server("c", "c")]));
            assert_eq!(servers.names(), ["a", "b", "c"]);
            assert!(Arc::ptr_eq(&a, &first(&servers)), "a stays as it was");
            servers.tidy().await;
            assert_eq!(
                servers.lock().retired.len(),
                1,
                "the old b, which a turn still holds"
            );
            drop(held);
            servers.tidy().await;
            assert!(
                servers.lock().retired.is_empty(),
                "closed once no turn holds it"
            );
            servers.close().await;
        });
    }
}
