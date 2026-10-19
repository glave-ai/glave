//! Each project as this glave reads it (`specs/policy.md`, "Project trust"; `specs/one-server.md`,
//! "Each project's own tools, hooks and settings"): what a thread opens with when it works in
//! a folder of its own (your own AGENTS.md, then the folder's, then what its project brings
//! that stays off, and why); what its project chooses for it at the start of each turn
//! (`settle.rs`); what it turns on for it once you trust it (its servers, started in its
//! folder, its hooks, and settings that loosen its commands); and, for an app, what it asks
//! you to trust, with your yes kept in the trust ledger exactly as the terminal keeps it. No
//! project is the server's own: the folder glave started in is one project among the others.

mod servers;
mod settle;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use glave_config::policy::memory_settings;
use glave_config::policy::{Choices, Loaded, Locations};
use glave_config::trust::{self, Answer, Outcome, Project};
use glave_harness::{AGENTS_MD, Brought, Hooks, Settled, agents_md};
use glave_memory::{GlaveMemory, Personal, SpaceMemory};
use glave_ports::{BoxFuture, Clock, SpaceId, ToolSource};
use glave_process::Processes;
use glave_session::ProjectTrust;
use glave_session::protocol::{ApiError, TrustAsk, TrustItem, code};
use tokio::task::JoinHandle;

use crate::data::DataLog;
use crate::gateway::GatewaySwitch;
use crate::setup::off_note;
use crate::trust::Trust;
use servers::Running;

pub use settle::web_offered;

/// How often glave looks for projects' servers to close.
const TIDY_EVERY: Duration = Duration::from_secs(60);

/// Why everything a project brings is off on a thread of someone other than the person on
/// this computer, as the model is told.
const FOR_OTHERS: &str = "a project's own tools and settings run only for the person on the \
     computer glave runs on, never for anyone else's threads";

/// What glave runs with itself, which each project's settings go among.
pub struct Own {
    /// The levels glave's choices came from, and the constraints.
    pub loaded: Loaded,
    /// glave's own settled choices, its sandbox as it runs: what a project's settings change.
    pub choices: Choices,
    /// The servers glave runs for the person's threads everywhere, by name: a project's
    /// server of the same name gives way to theirs.
    pub taken: Vec<String>,
    /// The models a thread may be sent to now.
    pub models: GatewaySwitch,
    /// Memory, when it is on: each project's settings choose its space's.
    pub memory: Option<Arc<GlaveMemory<DataLog>>>,
    /// What tells how long a project's servers have gone unused.
    pub clock: Box<dyn Clock>,
    /// Where a project's servers start, each owned by its project.
    pub processes: Processes,
    /// What the person decided about each project.
    pub trust: Trust,
}

/// The levels glave's own choices came from, the constraints, and those choices: what each
/// project's settings go among, as the person's settings read last.
struct Base {
    loaded: Loaded,
    choices: Choices,
}

/// Every project as this glave reads it: see the module's documentation.
pub struct ProjectFolders {
    /// glave's folder: the person's AGENTS.md.
    home: PathBuf,
    /// What the person decided about each project.
    trust: Trust,
    base: RwLock<Arc<Base>>,
    /// The names of the person's own servers, as they run now.
    taken: RwLock<Vec<String>>,
    models: GatewaySwitch,
    memory: Option<Arc<GlaveMemory<DataLog>>>,
    /// The person's own memory as memory started: read in shared spaces while the settings
    /// allow it.
    personal: Option<Personal>,
    clock: Box<dyn Clock>,
    processes: Processes,
    /// Who answers what a project asks: the person from an app, or no one.
    answer: Answer,
    /// Projects' servers, most recently used last; and those that were replaced or put away
    /// while a turn could still be using them, closed once none is.
    running: Mutex<Running>,
    /// What closes idle servers every [`TIDY_EVERY`], once [`ProjectFolders::tidy`] starts it.
    tidying: Mutex<Option<JoinHandle<()>>>,
    /// Held while a yes is kept, so two answers at once never lose one.
    keeping: tokio::sync::Mutex<()>,
}

impl ProjectFolders {
    /// The projects of a glave in `home` that runs with `own`. `later`: the person can answer
    /// what a project asks from an app; else no one can.
    pub fn new(home: PathBuf, own: Own, later: bool) -> Self {
        Self {
            home,
            trust: own.trust,
            base: RwLock::new(Arc::new(Base {
                loaded: own.loaded,
                choices: own.choices,
            })),
            taken: RwLock::new(own.taken),
            models: own.models,
            personal: own
                .memory
                .as_ref()
                .and_then(|memory| memory.memories().rules().personal),
            memory: own.memory,
            clock: own.clock,
            processes: own.processes,
            answer: if later { Answer::Later } else { Answer::NoOne },
            running: Mutex::new(Running::default()),
            tidying: Mutex::new(None),
            keeping: tokio::sync::Mutex::new(()),
        }
    }

    /// Closes the servers of `projects` no turn has used for 30 minutes, every
    /// [`TIDY_EVERY`], until [`ProjectFolders::close`].
    pub fn tidy(projects: &Arc<Self>) {
        let weak = Arc::downgrade(projects);
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(TIDY_EVERY).await;
                let Some(projects) = weak.upgrade() else {
                    break;
                };
                projects.sweep().await;
            }
        });
        let mut tidying = projects
            .tidying
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(earlier) = tidying.replace(task) {
            earlier.abort();
        }
    }

    /// Closes every project's servers no turn of the project has used for 30 minutes, and
    /// those put away that no turn holds any more.
    pub async fn sweep(&self) {
        let closing = self.lock().idle(self.clock.now());
        servers::close(closing).await;
    }

    /// Closes every project's servers, and stops closing idle ones. Call when glave ends, once
    /// no turn runs.
    pub async fn close(&self) {
        let tidying = self
            .tidying
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(tidying) = tidying {
            tidying.abort();
            let _ = tidying.await;
        }
        let running = std::mem::take(&mut *self.lock());
        servers::close(running.kept.into_iter().chain(running.retired)).await;
    }

    /// What of the project around `folder` glave sets aside, and why, one sentence each, to
    /// tell the person: settings it cannot use, and servers that give way to the person's own.
    pub async fn set_aside(&self, folder: &Path) -> Vec<String> {
        let root = self.root(folder).await;
        match self.outcome(&root).await {
            Ok(outcome) => [
                self.settle(&root, &outcome).aside,
                self.taken_from(&outcome),
            ]
            .concat(),
            Err(_) => Vec::new(),
        }
    }

    /// Settles each project's settings among `loaded`'s levels and constraints, and glave's
    /// own `choices`, from its threads' next turn on, and runs memory by them from its next
    /// read or write: the person's settings as read again.
    pub fn set_own(&self, loaded: Loaded, choices: Choices) {
        self.remember(&loaded, &choices);
        let base = Arc::new(Base { loaded, choices });
        *self.base.write().unwrap_or_else(PoisonError::into_inner) = base;
    }

    /// Runs memory, when it runs, by the person's `choices` within `loaded`'s constraints.
    /// Memory that was off when glave started stays off until it restarts, and so does the
    /// person's own.
    fn remember(&self, loaded: &Loaded, choices: &Choices) {
        let settings = match memory_settings(choices, &loaded.constraints) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!("memory's settings cannot be used, so the last ones stay: {error}");
                return;
            }
        };
        let Some(memory) = &self.memory else {
            if settings.on {
                tracing::warn!("memory was off when glave started; restart glave to turn it on");
            }
            return;
        };
        let on = settings.on;
        let default = if on { settings.space } else { SpaceMemory::Off };
        let personal = self.personal.filter(|_| on && settings.personal);
        let memories = memory.memories();
        memories.set_person(personal, default, settings.retention_days);
    }

    fn base(&self) -> Arc<Base> {
        Arc::clone(&self.base.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Gives way to the person's own servers named `taken` from now on, as they run now.
    pub fn set_taken(&self, taken: Vec<String>) {
        *self.taken.write().unwrap_or_else(PoisonError::into_inner) = taken;
    }

    /// Whether one of the person's own servers is named `name`.
    fn is_taken(&self, name: &str) -> bool {
        let taken = self.taken.read().unwrap_or_else(PoisonError::into_inner);
        taken.iter().any(|taken| taken == name)
    }

    /// Why each of `outcome`'s servers that shares a name with one of the person's own is off.
    fn taken_from(&self, outcome: &Outcome) -> Vec<String> {
        outcome
            .servers
            .keys()
            .filter(|name| self.is_taken(name))
            .map(|name| {
                format!(
                    "MCP server {name} is off: the person's own server of the same name runs \
                     instead"
                )
            })
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Running> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The project around `folder` as trust names it, looked for off the async runtime, since
    /// it reads the disk.
    async fn root(&self, folder: &Path) -> PathBuf {
        let (home, looked) = (self.home.clone(), folder.to_owned());
        tokio::task::spawn_blocking(move || project_root(&home, &looked))
            .await
            .unwrap_or_else(|_| folder.to_owned())
    }

    /// Where each item of the project at `root` stands now.
    async fn assessed(&self, root: &Path) -> anyhow::Result<trust::Assessment> {
        let project = Project::read(root, &crate::mcp::environment()).await?;
        let record = self.trust.record().await?;
        Ok(trust::assess(
            project,
            &record,
            &self.base().loaded.constraints,
        ))
    }

    /// What the project at `root` runs now, and what stays off and why.
    async fn outcome(&self, root: &Path) -> anyhow::Result<Outcome> {
        Ok(self.assessed(root).await?.apply(self.answer))
    }

    /// What the model is told stays off of the project whose folder is `folder`.
    async fn off(&self, folder: &Path) -> Option<String> {
        match self.outcome(&self.root(folder).await).await {
            Ok(outcome) => {
                off_note(&[outcome.withheld.clone(), self.taken_from(&outcome)].concat())
            }
            // A project file that does not parse turns nothing on; say so rather than fail.
            Err(error) => off_note(&[format!("the project's own settings are off: {error}")]),
        }
    }

    /// What the model is told stays off of the project whose folder is `folder` on a thread
    /// of someone other than the person on this computer: all of it.
    async fn off_for_others(&self, folder: &Path) -> Option<String> {
        match self.assessed(&self.root(folder).await).await {
            Ok(assessment) => off_note(&assessment.all_off(FOR_OTHERS)),
            Err(error) => off_note(&[format!("the project's own settings are off: {error}")]),
        }
    }

    /// The tool sources of the project at `root` for its `trusted` servers, kept or started now
    /// (`servers.rs`), closing what they replace.
    async fn tools(
        &self,
        root: &Path,
        trusted: BTreeMap<String, String>,
    ) -> Vec<Arc<dyn ToolSource>> {
        let now = self.clock.now();
        let base = self.base();
        let constraints = &base.loaded.constraints;
        let processes = &self.processes;
        let (sources, closing) = self
            .lock()
            .sources(root, trusted, now, constraints, processes);
        servers::close(closing).await;
        sources
    }

    /// What the project at `root`, the space `space`'s, chooses and turns on now beyond what
    /// glave runs for the person's threads everywhere.
    async fn bring(&self, root: &Path, space: SpaceId) -> anyhow::Result<Brought> {
        let outcome = self.outcome(root).await?;
        let read = self.settle(root, &outcome);
        let base = self.base();
        let mcp = read.choices.as_ref().unwrap_or(&base.choices).mcp.as_ref();
        let chosen = mcp.map(|chosen| &chosen.value);
        let servers: BTreeMap<String, String> = outcome
            .servers
            .iter()
            .filter(|(name, _)| !self.is_taken(name))
            .filter(|(name, _)| chosen.is_none_or(|chosen| chosen.contains(name)))
            .map(|(name, definition)| (name.clone(), definition.clone()))
            .collect();
        let hooks = Hooks::new(outcome.hooks.clone()).unwrap_or_else(|error| {
            tracing::warn!(%error, "a project's trusted hooks could not be used");
            Hooks::default()
        });
        self.remember_in(space, read.choices.as_ref());
        Ok(Brought {
            web: read.web(root),
            settled: read.settled,
            tools: self.tools(root, servers).await,
            hooks,
        })
    }

    /// Gives `space`'s memory the setting its project's `choices` choose, or the person's when
    /// they could not be read.
    fn remember_in(&self, space: SpaceId, choices: Option<&Choices>) {
        let Some(memory) = &self.memory else {
            return;
        };
        let memories = memory.memories();
        let constraints = &self.base().loaded.constraints;
        // Memory the person turned off stays off in every space.
        let setting = choices
            .and_then(|choices| memory_settings(choices, constraints).ok())
            .map_or_else(
                || memories.rules().default,
                |settings| {
                    if settings.on {
                        settings.space
                    } else {
                        SpaceMemory::Off
                    }
                },
            );
        memories.set_space(space, setting);
    }
}

/// The project around `folder` as trust names it: its repository's root, or the folder
/// itself, links resolved.
fn project_root(home: &Path, folder: &Path) -> PathBuf {
    let root = Locations::here(home, folder).root;
    std::fs::canonicalize(&root).unwrap_or(root)
}

/// A failure reading a project, as the session API reports it: in plain words, the files it
/// names kept to the server's log.
fn failed(error: &dyn std::fmt::Display) -> ApiError {
    tracing::warn!(%error, "a project or the trust record could not be read");
    ApiError::new(
        code::INTERNAL,
        "glave could not read what this project brings, so it stays off; try again, or open the \
         project in a terminal to see why",
    )
}

impl glave_harness::ProjectFolders for ProjectFolders {
    fn for_folder<'a>(
        &'a self,
        root: &'a Path,
        project: Option<&'a Path>,
        trusting: bool,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let personal = self.home.join(AGENTS_MD);
            let agents = match agents_md(root, Some(&personal)).await {
                Ok(read) => read.map(|read| read.text),
                Err(error) => {
                    tracing::warn!(%error, "a thread's AGENTS.md could not be read");
                    None
                }
            };
            let off = match project {
                Some(folder) if trusting => self.off(folder).await,
                Some(folder) => self.off_for_others(folder).await,
                None => None,
            };
            [agents, off]
                .into_iter()
                .flatten()
                .reduce(|all, part| format!("{all}\n\n{part}"))
        })
    }

    fn settled<'a>(&'a self, project: &'a Path) -> BoxFuture<'a, Settled> {
        Box::pin(async move {
            let root = self.root(project).await;
            match self.outcome(&root).await {
                Ok(outcome) => self.settle(&root, &outcome).settled,
                Err(_) => Settled::default(),
            }
        })
    }

    fn brought<'a>(&'a self, project: &'a Path, space: SpaceId) -> BoxFuture<'a, Brought> {
        Box::pin(async move {
            let root = self.root(project).await;
            self.bring(&root, space).await.unwrap_or_else(|error| {
                tracing::warn!(%error, "what a project brings could not be read");
                Brought::default()
            })
        })
    }
}

impl ProjectTrust for ProjectFolders {
    fn ask<'a>(&'a self, folder: &'a Path) -> BoxFuture<'a, Result<Option<TrustAsk>, ApiError>> {
        Box::pin(async move {
            let root = self.root(folder).await;
            // A project whose files do not read runs nothing, so it has nothing to trust.
            let Ok(project) = Project::read(&root, &crate::mcp::environment()).await else {
                return Ok(None);
            };
            let constraints = &self.base().loaded.constraints;
            let record = self.trust.record().await.map_err(|error| failed(&error))?;
            let assessment = trust::assess(project, &record, constraints);
            if assessment.undecided().is_empty() {
                return Ok(None);
            }
            let items = assessment
                .plain()
                .into_iter()
                .map(|plain| TrustItem {
                    text: plain.text.to_owned(),
                    detail: plain.detail.clone(),
                })
                .collect();
            Ok(Some(TrustAsk {
                ask: assessment.ask_id(),
                items,
            }))
        })
    }

    fn trust<'a>(
        &'a self,
        folder: &'a Path,
        ask: &'a str,
    ) -> BoxFuture<'a, Result<bool, ApiError>> {
        Box::pin(async move {
            let root = self.root(folder).await;
            let _keeping = self.keeping.lock().await;
            let project = Project::read(&root, &crate::mcp::environment())
                .await
                .map_err(|error| failed(&error))?;
            let record = self.trust.record().await.map_err(|error| failed(&error))?;
            let constraints = &self.base().loaded.constraints;
            let Some(entry) = trust::trust_asked(&record, project, constraints, ask) else {
                return Ok(false);
            };
            self.trust
                .keep(entry)
                .await
                .map_err(|error| failed(&error))?;
            Ok(true)
        })
    }
}
