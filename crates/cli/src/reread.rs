//! The person's own settings, read again (`specs/one-server.md`, "Its settings"): a server that
//! runs for days looks at its `config.toml`, the policy and `mcp.json` at the start of each
//! turn, and when their content's hash changed, reads them again. The models follow from the
//! next turn (the model, the models offered beside it, within the policy), and the person's
//! own servers are reconciled: an added one starts, a removed one stops, a changed one starts
//! again. What only a start applies stays as the start set it until glave restarts, and the
//! log says so. Settings that no longer read leave the last that did in place.

use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_config::policy::{Choices, Locations};
use glave_ports::BoxFuture;
use glave_session::{Apply, Changed, Models, Rereads};

use crate::args::Args;
use crate::gateway::Switch;
use crate::mcp::{self, Servers};
use crate::projects::Projects;
use crate::providers::{Host, Lineup};
use crate::setup::Setup;

/// What the person's settings change as they are read again.
pub struct Rereading {
    args: Args,
    /// The folder glave started in, whose policies are read as at the start.
    folder: PathBuf,
    /// glave's folder: the person's `mcp.json`.
    home: PathBuf,
    /// Every file read again, hashed together.
    files: Vec<PathBuf>,
    /// The hash of their content as last read.
    seen: tokio::sync::Mutex<u64>,
    /// The choices glave started with: what only a start applies.
    start: Choices,
    /// Where model accounts are kept, when the person may connect them here; else the gateway
    /// the models run on alone.
    host: Option<Arc<Host>>,
    switch: Switch,
    servers: Arc<Servers>,
    projects: Arc<Projects>,
}

/// What a [`Rereading`] is made with.
pub struct Parts {
    pub args: Args,
    pub folder: PathBuf,
    pub home: PathBuf,
    pub start: Choices,
    pub host: Option<Arc<Host>>,
    pub switch: Switch,
    pub servers: Arc<Servers>,
    pub projects: Arc<Projects>,
}

impl Rereading {
    /// Reads again what `parts` started with, from the content of its files now.
    pub async fn new(parts: Parts) -> Self {
        let locations = Locations::here(&parts.home, &parts.folder);
        let files = [locations.person]
            .into_iter()
            .chain(locations.policies)
            .chain([parts.home.join(mcp::FILE)])
            .collect::<Vec<_>>();
        let seen = digest(&files).await;
        Self {
            args: parts.args,
            folder: parts.folder,
            home: parts.home,
            files,
            seen: tokio::sync::Mutex::new(seen),
            start: parts.start,
            host: parts.host,
            switch: parts.switch,
            servers: parts.servers,
            projects: parts.projects,
        }
    }

    /// The settings as they read now, and what follows from them.
    async fn read(&self) -> anyhow::Result<Changed> {
        let mut setup = Setup::load(&self.args, &self.folder).await?;
        setup.choices = self.as_started(setup.choices);
        let constraints = &setup.loaded.constraints;
        let through_gateway = self.start.gateway.is_some();
        let mcp = setup.choices.mcp.as_ref();
        let wanted = mcp::wanted(&self.home, mcp, constraints, through_gateway).await?;
        let models = self.models(&setup).await?;
        let own = self.servers.reconcile(wanted).then(|| {
            self.projects.set_taken(self.servers.names());
            self.servers.sources()
        });
        Ok(Changed {
            models: Some(models),
            own,
        })
    }

    /// `now`, with what only a start applies as glave started with it, said in the log when
    /// it differs. The judge stays too: the harness keeps the start's.
    fn as_started(&self, now: Choices) -> Choices {
        let start = &self.start;
        let waits = start.gateway != now.gateway
            || start.provider != now.provider
            || start.store != now.store
            || start.telemetry != now.telemetry
            || start.workers != now.workers
            || start.github != now.github;
        if waits {
            tracing::warn!(
                "glave's settings changed where only a restart applies them (the store, the \
                 gateway, the provider, telemetry, workers or GitHub); restart glave for them"
            );
        }
        Choices {
            gateway: start.gateway.clone(),
            provider: start.provider.clone(),
            store: start.store.clone(),
            telemetry: start.telemetry.clone(),
            workers: start.workers.clone(),
            github: start.github.clone(),
            judge: start.judge.clone(),
            ..now
        }
    }

    /// The models of `setup`, run from now on.
    async fn models(&self, setup: &Setup) -> anyhow::Result<Models> {
        let (_, configured) = crate::providers::endpoint(&setup.choices)?;
        let lineup = Lineup::new(setup, configured);
        if let Some(host) = &self.host {
            return host.relined(lineup).await;
        }
        let built = lineup.build(&[])?;
        self.switch.set(built.gateway);
        Ok(built.models)
    }
}

impl Rereads for Rereading {
    fn reread<'a>(&'a self, apply: &'a Apply<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            {
                let mut seen = self.seen.lock().await;
                let now = digest(&self.files).await;
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

/// The hash of `files`' names and content now; one missing or unreadable counts as such.
async fn digest(files: &[PathBuf]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for file in files {
        file.hash(&mut hasher);
        content(file).await.hash(&mut hasher);
    }
    hasher.finish()
}

/// What `file` holds now, or why it could not be read.
async fn content(file: &Path) -> Result<Vec<u8>, std::io::ErrorKind> {
    tokio::fs::read(file).await.map_err(|error| error.kind())
}
