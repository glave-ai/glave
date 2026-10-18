//! The MCP servers in glave's settings, `mcp.json` in glave's folder: the person's own, for
//! their threads everywhere, started once in their home folder in the background with the
//! session, waited for before the first turn, reconciled when the settings change, and closed
//! when glave ends (`own.rs`); and how a trusted
//! project's `.mcp.json` servers start in its folder (`projects.rs`). A `config.toml` may turn
//! on only some of them, and a policy may forbid them.
//!
//! The organization's catalog (`[mcp] catalog`) comes from its gateway when `[gateway] url` is
//! set: one source, first, so a catalog tool wins over a server of the person's with the same
//! name. Without a gateway, the servers of `mcp.json` the catalog names are the catalog's, and
//! only the others are the person's own, which `[mcp] own = false` forbids.

mod own;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_config::policy::{Chosen, Constraints};
use glave_mcp::{Limits, McpServer, Resources, ServerConfig};
use glave_ports::ToolSource;
use glave_process::{Owner, Processes};

pub use own::{Servers, wanted};

/// The settings file, in glave's folder.
pub const FILE: &str = "mcp.json";

/// The environment servers' `${VAR}` are filled from.
pub fn environment() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
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
    processes: &Processes,
) -> anyhow::Result<Vec<Arc<McpServer>>> {
    let processes = processes.owned_by(Owner::Project(root.to_string_lossy().into_owned()));
    let text = glave_config::trust::mcp_json(servers);
    let environment = environment();
    let configs = glave_mcp::parse(&text, &environment)
        .context("in the project's trusted .mcp.json servers")?;
    let marks = glave_mcp::marks(&text)?;
    Ok(configs
        .iter()
        .map(|(name, config)| {
            let untrusted = project_untrusted(name, marks.get(name).copied(), constraints);
            let limits = Limits::default();
            let server = McpServer::start(name, config, &environment, root, limits, &processes);
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

/// Whether what the person's own server `name` returns is untrusted, as a web page is. A
/// policy's `[mcp] untrusted` always wins. Otherwise their own mark in `mcp.json`
/// (`marked`) holds; unmarked, one on this computer is trusted and a remote one is not, unless
/// a policy's `[mcp] trusted` vouches for it. A project's servers are untrusted unless a
/// policy vouches for them (`project_untrusted`): a repository cannot vouch for itself, and the
/// person who wants to trust one copies it into `mcp.json`.
fn mine_untrusted(
    name: &str,
    config: &ServerConfig,
    marked: Option<&bool>,
    constraints: &Constraints,
) -> bool {
    if constraints.mcp_untrusted(name).is_some() {
        return true;
    }
    let vouched = constraints.mcp_trusted(name).is_some();
    match marked {
        Some(&marked) => marked,
        None => config.is_remote() && !vouched,
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
            "mcp (from {}) names servers with no settings in {}: {}",
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
        let mine = glave_mcp::parse(mine_text, &env).unwrap();
        let theirs = glave_mcp::parse(project_text, &env).unwrap();
        let (mine_marks, their_marks) = (
            glave_mcp::marks(mine_text).unwrap(),
            glave_mcp::marks(project_text).unwrap(),
        );
        let untrusted = |rules: &Constraints| -> Vec<&str> {
            let own = mine.iter().filter(|(name, config)| {
                mine_untrusted(name, config, mine_marks.get(*name), rules)
            });
            let project = theirs.iter().filter(|(name, _)| {
                project_untrusted(name, their_marks.get(*name).copied(), rules)
            });
            let mut names: Vec<&str> = own.chain(project).map(|(name, _)| name.as_str()).collect();
            names.sort_unstable();
            names
        };
        let none = constraints("version = 1\n");
        assert_eq!(untrusted(&none), ["claims", "doubted", "remote", "repo"]);
        let policy = constraints(
            "version = 1\n[mcp]\ntrusted = [\"repo\", \"remote\", \"doubted\"]\nuntrusted = [\"local\"]\n",
        );
        assert_eq!(untrusted(&policy), ["claims", "doubted", "local"]);
    }
}
