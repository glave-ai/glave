//! The MCP servers in glave's settings, `mcp.json` in glave's folder, and the ones a trusted
//! project's `.mcp.json` brings: started in the background with the session, waited for before
//! the first turn, and closed when glave ends. A `config.toml` may turn on only some of them,
//! and a policy may forbid them.
//!
//! The organization's catalog (`[mcp] catalog`) comes from its gateway when `[gateway] url` is
//! set: one source, first, so a catalog tool wins over a server of the person's with the same
//! name. Without a gateway, the servers of `mcp.json` the catalog names are the catalog's, and
//! only the others are the person's own, which `[mcp] own = false` forbids.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, bail};
use glave_mcp::{Limits, McpServer, Resources, ServerConfig};
use glave_ports::ToolSource;

use crate::gateway::remote::Remote;
use crate::terminal::Printer;
use glave_config::policy::{Chosen, Constraints, check_own_servers};

/// The settings file, in glave's folder.
pub const FILE: &str = "mcp.json";

/// The environment servers' `${VAR}` are filled from.
pub fn environment() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// The servers of one session.
pub struct Servers {
    started: Vec<Arc<McpServer>>,
    reported: AtomicBool,
    /// Project servers left out because the person's own has the same name.
    shadowed: Vec<String>,
}

impl Servers {
    /// Starts the servers in `home`'s settings and the trusted `project` servers (a
    /// `.mcp.json`'s text), in `root`: those `chosen`, or every one; and first the catalog of
    /// the `remote` gateway, when there is one. With no settings file there are none of the
    /// person's. Fails when `constraints` forbid the person's own.
    pub async fn start(
        home: &Path,
        root: &Path,
        chosen: Option<&Chosen<Vec<String>>>,
        constraints: &Constraints,
        project: Option<&str>,
        remote: Option<&Remote>,
    ) -> anyhow::Result<Self> {
        let path = home.join(FILE);
        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("could not read {}", path.display()));
            }
        };
        let environment = environment();
        let own = if text.is_empty() {
            BTreeMap::new()
        } else {
            glave_mcp::parse(&text, &environment)
                .with_context(|| format!("in {}", path.display()))?
        };
        let mine: Vec<String> = own.keys().cloned().collect();
        let mut configs = own;
        let mut shadowed = Vec::new();
        if let Some(project) = project {
            for (name, config) in glave_mcp::parse(project, &environment)
                .context("in the project's trusted .mcp.json servers")?
            {
                match configs.entry(name) {
                    Entry::Occupied(taken) => shadowed.push(taken.key().clone()),
                    Entry::Vacant(free) => {
                        free.insert(config);
                    }
                }
            }
        }
        let configs = turned_on(configs, chosen, &path)?;
        // Without a gateway, the catalog's servers are the ones in mcp.json it names.
        let catalog = match (remote, constraints.mcp_catalog()) {
            (None, Some((names, _))) => names,
            _ => Vec::new(),
        };
        let on: Vec<&str> = mine
            .iter()
            .filter(|name| configs.contains_key(*name) && !catalog.contains(name))
            .map(String::as_str)
            .collect();
        check_own_servers(&on, constraints)?;
        let marks = Marks {
            mine: if text.is_empty() {
                BTreeMap::new()
            } else {
                glave_mcp::marks(&text).with_context(|| format!("in {}", path.display()))?
            },
            project: match project {
                Some(project) => glave_mcp::marks(project)?,
                None => BTreeMap::new(),
            },
        };
        let offered = remote.map(|remote| Arc::new(remote.catalog()));
        let servers = offered
            .into_iter()
            .chain(configs.iter().map(|(name, config)| {
                let untrusted = marks.untrusted(name, config, &mine, constraints);
                let server = McpServer::start(name, config, &environment, root, Limits::default());
                Arc::new(server.untrusted(untrusted))
            }))
            .collect();
        Ok(Self {
            started: servers,
            reported: AtomicBool::new(false),
            shadowed,
        })
    }

    /// The servers as tool sources, in the order of their names, then their resources.
    pub fn sources(&self) -> Vec<Arc<dyn ToolSource>> {
        sources(&self.started)
    }

    /// The servers' names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.started.iter().map(|server| server.name()).collect()
    }

    /// Before the first turn only: waits for every server to start, and says why any did not.
    pub async fn report(&self, printer: &mut Printer) {
        if self.reported.swap(true, Ordering::SeqCst) {
            return;
        }
        let starting: Vec<&str> = self
            .started
            .iter()
            .filter(|server| server.starting())
            .map(|server| server.name())
            .collect();
        for name in &self.shadowed {
            printer.warning(&format!(
                "the project's MCP server {name:?} is not started: your mcp.json has one of the same name"
            ));
        }
        if !starting.is_empty() {
            printer.status(&format!("starting MCP servers: {}", starting.join(", ")));
        }
        // They start side by side, so this waits only as long as the slowest.
        for server in &self.started {
            if let Err(err) = server.started().await {
                printer.warning(&err.to_string());
            }
        }
    }

    /// Closes every server side by side, so one slow to exit holds up no other.
    pub async fn close(&self) {
        close(&self.started).await;
    }
}

/// `started` as tool sources, in order, then their resources.
pub fn sources(started: &[Arc<McpServer>]) -> Vec<Arc<dyn ToolSource>> {
    let mut sources: Vec<Arc<dyn ToolSource>> = started
        .iter()
        .map(|server| Arc::clone(server) as Arc<dyn ToolSource>)
        .collect();
    sources.push(Arc::new(Resources::new(started.to_vec())));
    sources
}

/// Closes `started` side by side, so one slow to exit holds up no other.
pub async fn close(started: &[Arc<McpServer>]) {
    let mut closing = tokio::task::JoinSet::new();
    for server in started {
        let server = Arc::clone(server);
        closing.spawn(async move { server.close().await });
    }
    while closing.join_next().await.is_some() {}
}

/// Starts the trusted `servers` of the project at `root` (by name, each as JSON), in its
/// folder, for the threads that work in it.
pub fn start_project(
    servers: &BTreeMap<String, String>,
    root: &Path,
    constraints: &Constraints,
) -> anyhow::Result<Vec<Arc<McpServer>>> {
    let text = glave_config::trust::mcp_json(servers);
    let environment = environment();
    let configs = glave_mcp::parse(&text, &environment)
        .context("in the project's trusted .mcp.json servers")?;
    let marks = glave_mcp::marks(&text)?;
    Ok(configs
        .iter()
        .map(|(name, config)| {
            let untrusted = project_untrusted(name, marks.get(name).copied(), constraints);
            let server = McpServer::start(name, config, &environment, root, Limits::default());
            Arc::new(server.untrusted(untrusted))
        })
        .collect())
}

/// Whether what a project's server `name` returns is untrusted: unless a policy's `[mcp]
/// trusted` vouches for it and neither a policy nor the project (`marked`) says otherwise. A
/// repository cannot vouch for itself.
fn project_untrusted(name: &str, marked: Option<bool>, constraints: &Constraints) -> bool {
    constraints.mcp_untrusted(name).is_some()
        || marked == Some(true)
        || constraints.mcp_trusted(name).is_none()
}

/// The `"untrusted"` marks of the person's `mcp.json` and of the project's `.mcp.json`.
struct Marks {
    mine: BTreeMap<String, bool>,
    project: BTreeMap<String, bool>,
}

impl Marks {
    /// Whether what the server `name` returns is untrusted, as a web page is. A policy's
    /// `[mcp] untrusted` always wins. The person's own servers (`mine`) follow their own mark;
    /// unmarked, one on this computer is trusted and a remote one is not. A project's servers
    /// are untrusted unless a policy's `[mcp] trusted` vouches for them: a repository cannot
    /// vouch for itself, and the person who wants to trust one copies it into `mcp.json`.
    fn untrusted(
        &self,
        name: &str,
        config: &ServerConfig,
        mine: &[String],
        constraints: &Constraints,
    ) -> bool {
        if !mine.iter().any(|own| own == name) {
            return project_untrusted(name, self.project.get(name).copied(), constraints);
        }
        if constraints.mcp_untrusted(name).is_some() {
            return true;
        }
        let vouched = constraints.mcp_trusted(name).is_some();
        match self.mine.get(name) {
            Some(&marked) => marked,
            None => config.is_remote() && !vouched,
        }
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
            "mcp (from {}) names servers with no settings in {} or the project's trusted \
             .mcp.json: {}",
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
    use crate::gateway::tests::constraints;

    #[test]
    fn a_policy_can_mark_servers_untrusted() {
        let rules = constraints("version = 1\n[mcp]\nuntrusted = [\"search\"]\n");
        let rule = rules.mcp_untrusted("search").unwrap().to_string();
        assert!(rule.starts_with("[mcp] untrusted = [\"search\"]"), "{rule}");
        assert!(rules.mcp_untrusted("docs").is_none());
    }

    #[test]
    fn project_and_remote_servers_are_untrusted_unless_vouched_for() {
        let env = BTreeMap::new();
        let mine_text = r#"{"mcpServers": {
            "local": {"command": "local-mcp"},
            "remote": {"url": "https://mcp.example.com/mcp"},
            "vouched": {"url": "https://docs.example.com/mcp", "untrusted": false},
            "doubted": {"command": "search-mcp", "untrusted": true}
        }}"#;
        let project_text = r#"{"mcpServers": {
            "repo": {"command": "repo-mcp"},
            "claims": {"command": "claims-mcp", "untrusted": false}
        }}"#;
        let mut configs = glave_mcp::parse(mine_text, &env).unwrap();
        let mine: Vec<String> = configs.keys().cloned().collect();
        configs.extend(glave_mcp::parse(project_text, &env).unwrap());
        let marks = Marks {
            mine: glave_mcp::marks(mine_text).unwrap(),
            project: glave_mcp::marks(project_text).unwrap(),
        };
        let untrusted = |rules: &Constraints| -> Vec<&str> {
            configs
                .iter()
                .filter(|(name, config)| marks.untrusted(name, config, &mine, rules))
                .map(|(name, _)| name.as_str())
                .collect()
        };
        let none = constraints("version = 1\n");
        assert_eq!(untrusted(&none), ["claims", "doubted", "remote", "repo"]);
        let policy = constraints(
            "version = 1\n[mcp]\ntrusted = [\"repo\", \"remote\", \"doubted\"]\nuntrusted = [\"local\"]\n",
        );
        assert_eq!(untrusted(&policy), ["claims", "doubted", "local"]);
    }

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
        let start =
            |chosen| runtime.block_on(Servers::start(&home, &home, chosen, &forbidden, None, None));
        let refused = start(None).err().unwrap().to_string();
        assert!(
            refused.contains("mine") && refused.contains("[mcp] own = false"),
            "{refused}"
        );
        let none = Chosen {
            value: Vec::new(),
            from: glave_config::policy::Source::Default,
        };
        assert_eq!(start(Some(&none)).unwrap().names(), Vec::<&str>::new());
        let offered = constraints("version = 1\n[mcp]\nown = false\ncatalog = [\"mine\"]\n");
        let started = runtime
            .block_on(Servers::start(&home, &home, None, &offered, None, None))
            .unwrap();
        assert_eq!(
            started.names(),
            ["mine"],
            "without a gateway, a server of mcp.json the catalog names is the catalog's"
        );
    }
}
