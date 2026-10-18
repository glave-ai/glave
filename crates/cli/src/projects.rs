//! Each project as this glave reads it (`specs/policy.md`, "Project trust"; `specs/one-server.md`,
//! "Each project's own tools, hooks and settings"): what a thread opens with when it works in
//! a folder of its own (your own AGENTS.md, then the folder's, then what its project brings
//! that stays off, and why); what its project chooses for it at the start of each turn
//! (`settle.rs`); what it turns on for it once you trust it (its servers, started in its
//! folder, its hooks, and settings that loosen its commands); and, for an app, what it asks
//! you to trust, with your yes kept in `trust.toml` exactly as the terminal keeps it. No
//! project is the server's own: the folder glave started in is one project among the others.

mod settle;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use glave_config::policy::{Choices, Loaded, Locations};
use glave_config::trust::{self, Answer, Outcome, Project};
use glave_harness::{AGENTS_MD, Brought, Hooks, Projects as Host, Settled, agents_md};
use glave_mcp::McpServer;
use glave_memory::GlaveMemory;
use glave_ports::{BoxFuture, SpaceId, ToolSource};
use glave_session::ProjectTrust;
use glave_session::protocol::{ApiError, TrustAsk, TrustItem, code};

use crate::data::DataLog;
use crate::gateway::Switch;
use crate::setup::off_note;

pub use settle::web_fetch;

/// The projects whose servers run at once for their threads; past this, the one used least
/// recently is closed, and started again when its next turn needs it.
const MOST_PROJECTS: usize = 16;

/// Why everything a project brings is off on a thread of someone other than the person on
/// this computer, as the model is told.
const FOR_OTHERS: &str = "a project's own tools and settings run only for the person on the \
     computer glave runs on, never for anyone else's threads";

/// The servers started for one project's threads.
struct Kept {
    root: PathBuf,
    /// What was trusted, by name, each as JSON: started again when it changes.
    servers: BTreeMap<String, String>,
    started: Vec<Arc<McpServer>>,
    sources: Vec<Arc<dyn ToolSource>>,
}

impl Kept {
    /// Whether a thread may still use these servers: its turn's tools hold their sources.
    fn in_use(&self) -> bool {
        self.sources
            .last()
            .is_some_and(|resources| Arc::strong_count(resources) > 1)
    }
}

/// The servers started for projects' threads.
#[derive(Default)]
struct Running {
    /// Each project's, most recently used last.
    kept: Vec<Kept>,
    /// Replaced or put away while a thread still held them: closed once none does.
    retired: Vec<Kept>,
}

impl Running {
    /// Puts `kept` away, and takes out what no thread uses any more, to close.
    fn retire(&mut self, kept: impl IntoIterator<Item = Kept>) -> Vec<Kept> {
        self.retired.extend(kept);
        let (busy, idle) = std::mem::take(&mut self.retired)
            .into_iter()
            .partition(Kept::in_use);
        self.retired = busy;
        idle
    }
}

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
    pub models: Switch,
    /// Memory, when it is on: each project's settings choose its space's.
    pub memory: Option<Arc<GlaveMemory<DataLog>>>,
}

/// Every project as this glave reads it: see the module's documentation.
pub struct Projects {
    /// glave's folder: the person's AGENTS.md and the trust record.
    home: PathBuf,
    loaded: Loaded,
    choices: Choices,
    taken: Vec<String>,
    models: Switch,
    memory: Option<Arc<GlaveMemory<DataLog>>>,
    /// Who answers what a project asks: the person from an app, or no one.
    answer: Answer,
    /// Projects' servers, most recently used last; and those that were replaced or put away
    /// while a turn could still be using them, closed once none is.
    running: Mutex<Running>,
    /// Held while a yes is kept, so two answers at once never lose one.
    keeping: tokio::sync::Mutex<()>,
}

impl Projects {
    /// The projects of a glave in `home` that runs with `own`. `later`: the person can answer
    /// what a project asks from an app; else no one can.
    pub fn new(home: PathBuf, own: Own, later: bool) -> Self {
        Self {
            home,
            loaded: own.loaded,
            choices: own.choices,
            taken: own.taken,
            models: own.models,
            memory: own.memory,
            answer: if later { Answer::Later } else { Answer::NoOne },
            running: Mutex::new(Running::default()),
            keeping: tokio::sync::Mutex::new(()),
        }
    }

    /// Closes every project's servers. Call when glave ends, once no turn runs.
    pub async fn close(&self) {
        let running = std::mem::take(&mut *self.lock());
        for kept in running.kept.into_iter().chain(running.retired) {
            crate::mcp::close(&kept.started).await;
        }
    }

    /// What of the settings of the project around `folder` glave sets aside, and why, one
    /// sentence each, to tell the person.
    pub async fn set_aside(&self, folder: &Path) -> Vec<String> {
        let root = self.root(folder).await;
        match self.outcome(&root).await {
            Ok(outcome) => self.settle(&root, &outcome).aside,
            Err(_) => Vec::new(),
        }
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
    async fn assessed(&self, root: &Path) -> glave_config::Result<trust::Assessment> {
        let project = Project::read(root, &crate::mcp::environment()).await?;
        trust::assessed(&self.home, project, &self.loaded.constraints).await
    }

    /// What the project at `root` runs now, and what stays off and why.
    async fn outcome(&self, root: &Path) -> glave_config::Result<Outcome> {
        Ok(self.assessed(root).await?.apply(self.answer))
    }

    /// What the model is told stays off of the project whose folder is `folder`.
    async fn off(&self, folder: &Path) -> Option<String> {
        match self.outcome(&self.root(folder).await).await {
            Ok(outcome) => off_note(&outcome.withheld),
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

    /// The tool sources of the project at `root` for `servers`: kept from an earlier turn
    /// while they are the same, else started now. What they replace, and a project's past
    /// [`MOST_PROJECTS`], is closed once no thread's turn holds it any more.
    async fn tools(
        &self,
        root: &Path,
        servers: BTreeMap<String, String>,
    ) -> Vec<Arc<dyn ToolSource>> {
        let (sources, closing) = {
            let mut running = self.lock();
            let mut closing = Vec::new();
            if let Some(at) = running.kept.iter().position(|kept| kept.root == root) {
                let kept = running.kept.remove(at);
                if kept.servers == servers {
                    let sources = kept.sources.clone();
                    running.kept.push(kept);
                    return sources;
                }
                closing.push(kept);
            }
            let sources = if servers.is_empty() {
                Vec::new()
            } else {
                match crate::mcp::start_project(&servers, root, &self.loaded.constraints) {
                    Ok(started) => {
                        let sources = crate::mcp::sources(&started);
                        running.kept.push(Kept {
                            root: root.to_owned(),
                            servers,
                            started,
                            sources: sources.clone(),
                        });
                        sources
                    }
                    Err(error) => {
                        tracing::warn!(%error, "a project's trusted servers could not start");
                        Vec::new()
                    }
                }
            };
            let over = running.kept.len().saturating_sub(MOST_PROJECTS);
            closing.extend(running.kept.drain(..over));
            let closing = running.retire(closing);
            (sources, closing)
        };
        for kept in closing {
            crate::mcp::close(&kept.started).await;
        }
        sources
    }

    /// What the project at `root`, the space `space`'s, chooses and turns on now beyond what
    /// glave runs for the person's threads everywhere.
    async fn bring(&self, root: &Path, space: SpaceId) -> glave_config::Result<Brought> {
        let outcome = self.outcome(root).await?;
        let read = self.settle(root, &outcome);
        let mcp = read.choices.as_ref().unwrap_or(&self.choices).mcp.as_ref();
        let chosen = mcp.map(|chosen| &chosen.value);
        let servers: BTreeMap<String, String> = outcome
            .servers
            .iter()
            .filter(|(name, _)| !self.taken.contains(name))
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
        let setting = choices
            .and_then(|choices| {
                glave_config::policy::memory_settings(choices, &self.loaded.constraints).ok()
            })
            .map_or_else(|| memories.rules().default, |settings| settings.space);
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
fn failed(error: &glave_config::Error) -> ApiError {
    tracing::warn!(%error, "a project or the trust record could not be read");
    ApiError::new(
        code::INTERNAL,
        "glave could not read what this project brings, so it stays off; try again, or open the \
         project in a terminal to see why",
    )
}

impl Host for Projects {
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

impl ProjectTrust for Projects {
    fn ask<'a>(&'a self, folder: &'a Path) -> BoxFuture<'a, Result<Option<TrustAsk>, ApiError>> {
        Box::pin(async move {
            let root = self.root(folder).await;
            // A project whose files do not read runs nothing, so it has nothing to trust.
            let Ok(project) = Project::read(&root, &crate::mcp::environment()).await else {
                return Ok(None);
            };
            let assessment = trust::assessed(&self.home, project, &self.loaded.constraints)
                .await
                .map_err(|error| failed(&error))?;
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
            trust::trust_asked(&self.home, project, &self.loaded.constraints, ask)
                .await
                .map_err(|error| failed(&error))
        })
    }
}
