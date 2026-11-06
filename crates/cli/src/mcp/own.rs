//! The person's own MCP servers, from `mcp.json` in glave's folder, for their threads
//! everywhere: started once in their home folder, and reconciled when their settings change
//! (`specs/one-server.md`, "Its settings"): an added server starts, a removed one stops, and a
//! changed one starts again. One a turn still uses is closed once no turn does. A remote
//! server is signed in with the person's sign-in to its address, asked for at each request
//! (`connected.rs`), and starts again when they sign in to it or remove that sign-in. The
//! connectors of glave's catalog they connected start beside them, by their ids, each offered
//! to the threads of the place it is used in: everywhere, or one project of theirs alone.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Context as _;
use glave_config::policy::{Chosen, Constraints, check_own_servers};
use glave_harness::OwnTools;
use glave_mcp::{Limits, McpServer, Reachable};
use glave_ports::ProjectId;
use glave_ports::settings::mcp::ServerConfig;
use glave_process::Processes;

use super::{FILE, Offered, environment, mine_untrusted, turned_on};
use crate::connected::{Connected, Signed};
use crate::gateway::remote::Remote;
use crate::terminal::Printer;

/// One of the person's servers as their settings describe it: started again when this
/// changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wanted {
    name: String,
    config: ServerConfig,
    untrusted: bool,
    /// Whether the person is signed in at its address.
    signed_in: bool,
    /// The one project whose threads alone use it: none where every thread of theirs does.
    only: Option<ProjectId>,
}

/// What the person's settings turn on: the servers of `home`'s `mcp.json` that `chosen`
/// names, or every one, those `constraints` allow, how far each is trusted, and whether the
/// person is signed in to it, at one of the addresses `signed` holds; then the connectors
/// `signed` holds that `chosen` names, or every one, that `constraints` offer, in the order of
/// their ids. `through_gateway`: the person uses a gateway's catalog.
pub(crate) async fn wanted(
    home: &Path,
    chosen: Option<&Chosen<Vec<String>>>,
    constraints: &Constraints,
    through_gateway: bool,
    signed: &Signed,
) -> anyhow::Result<Vec<Wanted>> {
    let path = home.join(FILE);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).with_context(|| format!("could not read {}", path.display())),
    };
    let own = if text.is_empty() {
        BTreeMap::new()
    } else {
        glave_ports::settings::mcp::parse(&text, &environment())
            .with_context(|| format!("in {}", path.display()))?
    };
    let connectors = connectors(&own, signed, constraints);
    let mine: Vec<String> = own.keys().cloned().collect();
    // Every connector offered is a name `chosen` may give, connected now or not.
    let offered = Offered::by(constraints);
    let catalog = glave_session::Catalog::built_in().map(|it| it.connectors);
    let ids: Vec<String> = catalog
        .unwrap_or_default()
        .into_iter()
        .filter(|it| offered.allows(&it.id))
        .map(|it| it.id)
        .collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let configs = turned_on(own, chosen, &ids, &path)?;
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
    let marks = if text.is_empty() {
        BTreeMap::new()
    } else {
        glave_ports::settings::mcp::marks(&text)
            .with_context(|| format!("in {}", path.display()))?
    };
    let chose = |id: &str| chosen.is_none_or(|chosen| chosen.value.iter().any(|it| it == id));
    let connectors = connectors.into_iter().filter(|(id, _, _)| chose(id));
    let configs = configs
        .into_iter()
        .map(|(name, config)| (name, config, None));
    Ok(configs
        .chain(connectors)
        .map(|(name, config, only)| {
            let untrusted = mine_untrusted(&name, &config, marks.get(&name), constraints);
            let signed_in = match &config {
                ServerConfig::Http { url, .. } => {
                    glave_mcp::address(url).is_some_and(|url| signed.addresses.contains(&url))
                }
                ServerConfig::Stdio { .. } => false,
            };
            Wanted {
                name,
                config,
                untrusted,
                signed_in,
                only,
            }
        })
        .collect())
}

/// The connectors of glave's catalog `signed` holds that `constraints` offer, by their ids, as
/// remote servers, each with the one project it is used in alone, in the order of their ids:
/// none the person's `own` servers already name, or are at the address of.
fn connectors(
    own: &BTreeMap<String, ServerConfig>,
    signed: &Signed,
    constraints: &Constraints,
) -> Vec<(String, ServerConfig, Option<ProjectId>)> {
    let Ok(catalog) = glave_session::Catalog::built_in() else {
        return Vec::new();
    };
    let offered = Offered::by(constraints);
    let theirs: BTreeSet<String> = own
        .values()
        .filter_map(|config| match config {
            ServerConfig::Http { url, .. } => glave_mcp::address(url),
            ServerConfig::Stdio { .. } => None,
        })
        .collect();
    let kept = |id: &str| signed.connectors.iter().find(|it| it.id == id);
    let mut connectors: Vec<(String, ServerConfig, Option<ProjectId>)> = catalog
        .connectors
        .into_iter()
        .filter(|it| offered.allows(&it.id) && !own.contains_key(&it.id))
        .filter(|it| glave_mcp::address(&it.url).is_some_and(|url| !theirs.contains(&url)))
        .filter_map(|it| {
            let only = kept(&it.id)?.only;
            let config = ServerConfig::Http {
                url: it.url,
                headers: BTreeMap::new(),
            };
            Some((it.id, config, only))
        })
        .collect();
    connectors.sort_by(|one, other| one.0.cmp(&other.0));
    connectors
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
pub(crate) struct Servers {
    catalog: Option<Arc<McpServer>>,
    running: Mutex<Running>,
    /// Where they start: the person's home folder.
    root: PathBuf,
    reported: AtomicBool,
    /// Where they started, and where each project's servers start too.
    processes: Processes,
    /// The person's sign-ins to remote servers, where they may sign in here.
    connected: Option<Arc<Connected>>,
    /// The addresses a remote one may be at.
    reachable: Reachable,
}

impl Servers {
    /// Starts `wanted` in `root`, after the catalog of the `remote` gateway, when there is one,
    /// remote ones signed in with what the person `connected`, each only at an address
    /// `reachable` allows.
    pub(crate) fn start(
        wanted: Vec<Wanted>,
        root: &Path,
        remote: Option<&Remote>,
        processes: &Processes,
        connected: Option<Arc<Connected>>,
        reachable: Reachable,
    ) -> Self {
        let servers = Self {
            catalog: remote.map(|remote| Arc::new(remote.catalog())),
            running: Mutex::default(),
            root: root.to_owned(),
            reported: AtomicBool::new(false),
            processes: processes.clone(),
            connected,
            reachable,
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

    /// `wanted` started, in the person's home folder; a remote one signed in as the person,
    /// and web search's telling the person what each call searched.
    fn started(&self, wanted: &Wanted) -> Arc<McpServer> {
        let name = &wanted.name;
        let limits = Limits {
            reachable: self.reachable,
            ..Limits::default()
        };
        let server = match (&wanted.config, &self.connected) {
            (ServerConfig::Http { url, headers }, Some(connected)) => {
                let server = McpServer::remote(name, url, headers, connected.signer(url), limits);
                match connected.search_at(url) {
                    Some(search) => server.searching_with(search.tool),
                    None => server,
                }
            }
            (config, _) => McpServer::start(
                name,
                config,
                &environment(),
                &self.root,
                limits,
                &self.processes,
            ),
        };
        Arc::new(server.untrusted(wanted.untrusted))
    }

    /// Offers the person the connectors `offered` names from now on.
    pub(crate) fn offer(&self, offered: Offered) {
        if let Some(connected) = &self.connected {
            connected.offer(offered);
        }
    }

    /// The addresses the person is signed in to, and the connectors they connected.
    pub(crate) async fn signed(&self) -> Signed {
        match &self.connected {
            Some(connected) => connected.signed().await,
            None => Signed::default(),
        }
    }

    /// Runs `wanted` from now on: keeps the servers that stay the same, starts those added or
    /// changed, and puts away those removed or changed. Whether anything changed.
    pub(crate) fn reconcile(&self, wanted: Vec<Wanted>) -> bool {
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
    pub(crate) async fn tidy(&self) {
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
    pub(crate) fn processes(&self) -> &Processes {
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

    /// The servers as tool sources, for the threads of each place: the catalog first, then
    /// those used there in the order of their names, then their resources. A place with
    /// connectors of its own has them beside those used everywhere; every other place has
    /// only those.
    pub(crate) fn tools(&self) -> OwnTools {
        let running = self.lock();
        let used_in = |place: Option<ProjectId>| -> Vec<Arc<McpServer>> {
            let here = (running.servers.iter())
                .filter(|(wanted, _)| wanted.only.is_none() || wanted.only == place)
                .map(|(_, server)| Arc::clone(server));
            self.catalog.iter().cloned().chain(here).collect()
        };
        let places: BTreeSet<ProjectId> = (running.servers.iter())
            .filter_map(|(wanted, _)| wanted.only)
            .collect();
        let everywhere = OwnTools::everywhere(super::sources(&used_in(None)));
        places.into_iter().fold(everywhere, |tools, place| {
            tools.with_place(place, super::sources(&used_in(Some(place))))
        })
    }

    /// The servers' names, in order.
    pub(crate) fn names(&self) -> Vec<String> {
        self.all()
            .iter()
            .map(|server| server.name().to_owned())
            .collect()
    }

    /// Before the first turn only: says which servers are still starting, which the turn waits
    /// for. Each turn's `turn.started` says why one did not.
    pub(crate) fn report(&self, printer: &mut Printer) {
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
    }

    /// Closes every server side by side, so one slow to exit holds up no other.
    pub(crate) async fn close(&self) {
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
        let start =
            |chosen| runtime.block_on(wanted(&home, chosen, &forbidden, false, &Signed::default()));
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
            .block_on(wanted(&home, None, &offered, false, &Signed::default()))
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

    #[test]
    fn connected_connectors_start_beside_the_person_s_servers_unless_theirs_or_not_offered() {
        let home = glave_ports::testing::TempDir::new("mcp-connectors").unwrap();
        let servers = r#"{"mcpServers": {
            "linear": {"command": "/nonexistent/linear"},
            "mine": {"url": "https://mcp.notion.com/mcp"}
        }}"#;
        std::fs::write(home.join(FILE), servers).unwrap();
        let project = ProjectId::from_uuid(uuid::Uuid::from_u128(7));
        let signed = Signed {
            addresses: BTreeSet::from(["https://mcp.sentry.dev/mcp".to_owned()]),
            connectors: ["sentry", "deepwiki", "linear", "notion"]
                .map(|id| glave_session::KeptConnector {
                    id: id.to_owned(),
                    only: (id == "deepwiki").then_some(project),
                })
                .into(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let names = |policy: &str, chosen: Option<&Chosen<Vec<String>>>| {
            let rules = constraints(policy);
            let wanted = runtime.block_on(wanted(&home, chosen, &rules, false, &signed));
            let wanted = wanted.unwrap();
            let signed_in: Vec<bool> = wanted.iter().map(|it| it.signed_in).collect();
            let only: Vec<Option<ProjectId>> = wanted.iter().map(|it| it.only).collect();
            let names: Vec<String> = wanted.into_iter().map(|it| it.name).collect();
            (names, signed_in, only)
        };
        let (all, signed_in, only) = names("version = 1\n", None);
        assert_eq!(
            all,
            ["linear", "mine", "deepwiki", "sentry"],
            "a name or an address of mcp.json's is its own"
        );
        assert_eq!(signed_in, [false, false, false, true]);
        assert_eq!(
            only,
            [None, None, Some(project), None],
            "used where it was put"
        );
        let chosen = Chosen {
            value: vec!["sentry".to_owned()],
            from: glave_config::policy::Source::Default,
        };
        assert_eq!(names("version = 1\n", Some(&chosen)).0, ["sentry"]);
        let unconnected = Chosen {
            value: vec!["todoist".to_owned()],
            from: glave_config::policy::Source::Default,
        };
        let none: Vec<String> = Vec::new();
        assert_eq!(
            names("version = 1\n", Some(&unconnected)).0,
            none,
            "a connector chosen but not connected is no typo"
        );
        let narrowed = "version = 1\n[mcp]\ncatalog = [\"linear\", \"mine\", \"deepwiki\"]\n";
        assert_eq!(names(narrowed, None).0, ["linear", "mine", "deepwiki"]);
    }

    /// A server named `name` running `command`, which does not exist.
    fn server(name: &str, command: &str) -> Wanted {
        Wanted {
            name: name.to_owned(),
            config: ServerConfig::Stdio {
                command: format!("/nonexistent/{command}"),
                args: Vec::new(),
                env: BTreeMap::new(),
            },
            untrusted: false,
            signed_in: false,
            only: None,
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
            let servers = Servers::start(
                started.clone(),
                &root,
                None,
                &unrecorded,
                None,
                Reachable::default(),
            );
            let first = |servers: &Servers| Arc::clone(&servers.lock().servers[0].1);
            let a = first(&servers);
            assert!(
                !servers.reconcile(started),
                "the same settings change nothing"
            );
            let held = servers.tools();
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
