//! The settings a run uses: its flags and the environment over the `config.toml` files, kept
//! within the policy (`specs/policy.md`), and `glave policy explain`, which shows them.

use std::path::Path;
use std::process::ExitCode;

use crate::args::Args;
use crate::policy::{self, Choices, ConfigFile, Loaded, Locations};
use crate::{config, home};

/// What this run uses: the files read, and the choices within the constraints.
pub struct Setup {
    pub loaded: Loaded,
    pub choices: Choices,
}

impl Setup {
    /// Reads the files for this run in `folder` and keeps its choices within the policy.
    pub async fn load(args: &Args, folder: &Path) -> anyhow::Result<Self> {
        let loaded = read(args, folder).await?;
        let choices = loaded.settle()?;
        if args.network && choices.sandbox.value.is_none() {
            anyhow::bail!("--network only applies with a sandbox: pass --sandbox, or set one");
        }
        Ok(Self { loaded, choices })
    }

    /// What to tell the person before the first turn, if anything.
    pub fn notice(&self) -> Option<String> {
        self.loaded.untrusted.as_ref().map(|path| {
            format!(
                "not using {}: glave cannot trust a project's settings yet",
                path.display()
            )
        })
    }
}

/// `glave policy explain`: prints every setting and constraint, and fails when glave would
/// refuse to run here.
pub async fn explain(args: &Args) -> anyhow::Result<ExitCode> {
    let loaded = read(args, &std::env::current_dir()?).await?;
    // The provider names models as the gateway sees them; without a model there is none.
    let model = loaded
        .choices
        .model
        .as_ref()
        .map(|model| model.value.clone());
    let provider = match model {
        Some(model) => Some(config::endpoint(Some(model))?.provider.provider),
        None => None,
    };
    let (text, runs) = policy::explain(&loaded, provider.as_deref());
    print!("{text}");
    Ok(if runs {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

async fn read(args: &Args, folder: &Path) -> anyhow::Result<Loaded> {
    let locations = Locations::here(&home::folder()?, folder);
    policy::load(&locations, flags(args), environment()).await
}

/// The choices made with flags.
fn flags(args: &Args) -> ConfigFile {
    ConfigFile {
        model: args.model.clone(),
        approval: args.approval.map(Into::into),
        sandbox: args.sandbox,
        ..ConfigFile::default()
    }
}

/// The choices made in the environment. An empty variable chooses nothing.
fn environment() -> ConfigFile {
    let set = |name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    };
    ConfigFile {
        model: set("GLAVE_MODEL"),
        judge_model: set("GLAVE_JUDGE_MODEL"),
        ..ConfigFile::default()
    }
}
