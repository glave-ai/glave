//! The person's own settings, read again (`specs/one-server.md`, "Its settings"): a server that
//! runs for days looks at its `config.toml`, the policy and `mcp.json` at the start of each
//! turn, and when their content's hash changed, reads them again. From the next turn, threads
//! run with what they choose: the models (and the judge, with the models it names), the mode,
//! the network, hooks, `[web]`, titles and memory; a new thread takes
//! `worktree`; and the person's own servers are reconciled: an added one starts, a removed one
//! stops, a changed one starts again, and so does a remote one the person signs in to or whose
//! sign-in they remove. A constraint added meanwhile narrows the next turn whatever the
//! person's file says. What only a start applies stays as the start set it until glave
//! restarts, and the log says so. Settings that no longer read leave the last that did.

use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

use glave_computer::person_sandbox;
use glave_config::policy::{Choices, Locations};
use glave_harness::Defaults;
use glave_ports::computer::SandboxLevel;
use glave_ports::settings::{Hooks, Judge};
use glave_ports::{BoxFuture, ToolSource};
use glave_session::{Apply, Changed, Models, Rereads, Runs};

use crate::args::Args;
use crate::gateway::GatewaySwitch;
use crate::mcp::{self, Servers};
use crate::project_folders::ProjectFolders;
use crate::providers::{Lineup, ModelAccounts};
use crate::setup::Setup;

/// What the person's settings change as they are read again.
pub(crate) struct Rereading {
    args: Args,
    /// Where the person's `config.toml` and the policies are, as glave started.
    locations: Locations,
    /// glave's folder: the person's `mcp.json`, and what the sandbox protects.
    home: PathBuf,
    /// Every file read again, hashed together.
    files: Vec<PathBuf>,
    /// The hash of their content as last read.
    seen: tokio::sync::Mutex<u64>,
    /// The choices glave started with: what only a start applies.
    start: Choices,
    /// The most a sandbox here allows, as glave found it when it started; none where none
    /// can run.
    here: Option<SandboxLevel>,
    /// The judge as last run: one the models in use serve.
    judge: std::sync::Mutex<Judge>,
    /// Where model accounts are kept, when the person may connect them here; else the gateway
    /// the models run on alone.
    accounts: Option<Arc<ModelAccounts>>,
    switch: GatewaySwitch,
    servers: Arc<Servers>,
    projects: Arc<ProjectFolders>,
}

/// What a [`Rereading`] is made with.
pub(crate) struct Parts {
    pub args: Args,
    pub folder: PathBuf,
    pub home: PathBuf,
    pub start: Choices,
    pub here: Option<SandboxLevel>,
    pub accounts: Option<Arc<ModelAccounts>>,
    pub switch: GatewaySwitch,
    pub servers: Arc<Servers>,
    pub projects: Arc<ProjectFolders>,
}

impl Rereading {
    /// Reads again what `parts` started with, from the content of its files now.
    pub(crate) async fn new(parts: Parts) -> Self {
        let locations = Locations::here(&parts.home, &parts.folder);
        let files = [locations.person.clone()]
            .into_iter()
            .chain(locations.policies.iter().cloned())
            .chain([parts.home.join(mcp::FILE)])
            .collect::<Vec<_>>();
        let seen = digest(&files, &parts.servers.signed_in().await).await;
        Self {
            args: parts.args,
            locations,
            home: parts.home,
            files,
            seen: tokio::sync::Mutex::new(seen),
            judge: std::sync::Mutex::new(parts.start.judge.value.clone()),
            start: parts.start,
            here: parts.here,
            accounts: parts.accounts,
            switch: parts.switch,
            servers: parts.servers,
            projects: parts.projects,
        }
    }

    /// The settings as they read now, and what follows from them. How threads run follows
    /// even where the models no longer line up, or the person's servers no longer read.
    async fn read(&self) -> anyhow::Result<Changed> {
        let setup = reloaded(&self.args, &self.locations, &self.start).await?;
        let mut runs = runs(&setup, self.here, &self.home)?;
        let own = self.own_servers(&setup).await;
        let models = match self.models(&setup).await {
            Ok(models) => Some(models),
            Err(error) => {
                tracing::warn!(
                    "glave's models changed but cannot be used, so the last ones stay: {error:#}"
                );
                runs.defaults.judge = kept(&runs.defaults.judge, self.last_judge());
                None
            }
        };
        *self.judge.lock().unwrap_or_else(PoisonError::into_inner) = runs.defaults.judge.clone();
        self.projects.set_own(setup.loaded, setup.choices);
        Ok(Changed {
            models,
            own,
            runs: Some(runs),
        })
    }

    fn last_judge(&self) -> Judge {
        self.judge
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The person's own servers as `setup` turns them on, when they changed. A rule that now
    /// forbids them puts them all away; a `mcp.json` that no longer reads leaves them running.
    async fn own_servers(&self, setup: &Setup) -> Option<Vec<Arc<dyn ToolSource>>> {
        let constraints = &setup.loaded.constraints;
        let through_gateway = self.start.gateway.is_some();
        let mcp = setup.choices.mcp.as_ref();
        let signed_in = self.servers.signed_in().await;
        let wanted = mcp::wanted(&self.home, mcp, constraints, through_gateway, &signed_in);
        let wanted = match wanted.await {
            Ok(wanted) => wanted,
            Err(error) if constraints.own_mcp_forbidden().is_some() => {
                tracing::warn!("your own MCP servers are put away: {error:#}");
                Vec::new()
            }
            Err(error) => {
                tracing::warn!(
                    "your MCP servers changed but cannot be used, so the last ones stay: \
                     {error:#}"
                );
                return None;
            }
        };
        self.servers.reconcile(wanted).then(|| {
            self.projects.set_taken(self.servers.names());
            self.servers.sources()
        })
    }

    /// The models of `setup`, run from now on.
    async fn models(&self, setup: &Setup) -> anyhow::Result<Models> {
        let (_, configured) = crate::providers::endpoint(&setup.choices)?;
        let lineup = Lineup::new(setup, configured);
        if let Some(accounts) = &self.accounts {
            return accounts.relined(lineup).await;
        }
        let built = lineup.build(&[], &[])?;
        self.switch.set(built.gateway);
        Ok(built.models)
    }
}

/// The judge to run, `chosen` now, while the models stay as they were, with `last`: one that
/// names a model goes with the models it names, so it stays `last` (the main model where that
/// was off); off, or the main model, needs none.
fn kept(chosen: &Judge, last: Judge) -> Judge {
    match (chosen, last) {
        (Judge::Model(_), Judge::Off) => Judge::Main,
        (Judge::Model(_), last) => last,
        (chosen, _) => chosen.clone(),
    }
}

/// The person's settings at `locations` as a server that started with `start` runs them
/// from now on: within the constraints as they are now. Where no sandbox runs, a rule that now
/// requires one leaves every command asking: the mode is narrowed below full access.
async fn reloaded(args: &Args, locations: &Locations, start: &Choices) -> anyhow::Result<Setup> {
    let (mut setup, aside) = Setup::reload(args, locations).await?;
    for line in aside {
        tracing::warn!(
            "the policy does not allow what your settings chose, so glave's default applies: {line}"
        );
    }
    setup.choices = as_started(start, setup.choices);
    Ok(setup)
}

/// `now`, with what only a start applies as glave started with it (`start`), said in the log
/// when it differs.
fn as_started(start: &Choices, now: Choices) -> Choices {
    let waits = start.gateway != now.gateway
        || start.provider != now.provider
        || start.store != now.store
        || start.telemetry != now.telemetry
        || start.workers != now.workers
        || start.computers != now.computers
        || start.github != now.github
        || start.origin != now.origin
        || start.acme != now.acme
        || start.forwarded != now.forwarded
        || start.router_floor != now.router_floor;
    if waits {
        tracing::warn!(
            "glave's settings changed where only a restart applies them (the store, the \
             gateway, the provider, telemetry, workers, people's computers, which providers it allows, GitHub or where people reach glave); \
             restart glave for them"
        );
    }
    Choices {
        gateway: start.gateway.clone(),
        provider: start.provider.clone(),
        store: start.store.clone(),
        telemetry: start.telemetry.clone(),
        workers: start.workers.clone(),
        computers: start.computers.clone(),
        github: start.github.clone(),
        origin: start.origin.clone(),
        acme: start.acme.clone(),
        router_floor: start.router_floor.clone(),
        signup: start.signup.clone(),
        forwarded: start.forwarded.clone(),
        unsandboxed: start.unsandboxed.clone(),
        ..now
    }
}

/// How threads run with `setup`'s choices, where neither they nor their projects choose: in
/// the most a sandbox here allows (`here`), with `glave_home`, glave's folder, protected in it.
fn runs(setup: &Setup, here: Option<SandboxLevel>, glave_home: &Path) -> anyhow::Result<Runs> {
    let choices = &setup.choices;
    let sandbox = here.map(|level| crate::sandboxing::sandbox_at(level, choices, Some(glave_home)));
    Ok(Runs {
        defaults: Defaults {
            permissions: choices.permissions.value,
            highest: setup.loaded.constraints.highest_permissions().0,
            judge: choices.judge.value.clone(),
            titles: choices.titles.value,
            sandbox,
            home_sandbox: choices
                .computers
                .as_ref()
                .filter(|computers| computers.value.sandbox != Some(false))
                .map(|_| person_sandbox(choices.network.value)),
            ask_network: setup.loaded.constraints.network_ask_forbidden().is_none(),
            hooks: Hooks::new(setup.hooks.clone())?,
            web: crate::project_folders::web_offered(&choices.web.value)?,
        },
        worktree: choices.worktree.value,
    })
}

impl Rereads for Rereading {
    fn reread<'a>(&'a self, apply: &'a Apply<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            {
                let mut seen = self.seen.lock().await;
                let now = digest(&self.files, &self.servers.signed_in().await).await;
                if now != *seen {
                    *seen = now;
                    match self.read().await {
                        Ok(changed) => apply(changed),
                        Err(error) => tracing::warn!(
                            "glave's settings changed but cannot be used, so the last ones stay: \
                             {error:#}"
                        ),
                    }
                }
            }
            self.servers.tidy().await;
        })
    }
}

/// The hash of `files`' names and content now, one missing or unreadable counting as such,
/// and of the addresses the person is `signed_in` to, which their servers follow too.
async fn digest(files: &[PathBuf], signed_in: &BTreeSet<String>) -> u64 {
    let mut hasher = DefaultHasher::new();
    for file in files {
        file.hash(&mut hasher);
        content(file).await.hash(&mut hasher);
    }
    signed_in.hash(&mut hasher);
    hasher.finish()
}

/// What `file` holds now, or why it could not be read.
async fn content(file: &Path) -> Result<Vec<u8>, std::io::ErrorKind> {
    tokio::fs::read(file).await.map_err(|error| error.kind())
}

#[cfg(test)]
mod tests;
