//! The settings a run uses: its flags and the environment over the `config.toml` files, kept
//! within the policy (`specs/policy.md`), and `glave policy explain`, which shows them.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use glave_harness::Hook;

use crate::args::Args;
use crate::policy::{self, Choices, ConfigFile, Loaded, Locations, ProjectTrust};
use crate::trust::{self, Asking, Outcome, Project};
use crate::{config, home, mcp};

/// What this run uses: the files read, and the choices within the constraints.
pub struct Setup {
    pub loaded: Loaded,
    pub choices: Choices,
    /// The project's `.mcp.json` servers that are trusted, to start with the person's own.
    pub project_servers: BTreeMap<String, String>,
    /// What the project brings that stays off, and why.
    pub withheld: Vec<String>,
    /// The hooks that run: the person's own, then the project's trusted ones.
    pub hooks: Vec<Hook>,
    /// The project's hooks that stay off, and why: told to the person, never the model.
    pub hooks_off: Vec<String>,
}

impl Setup {
    /// Reads the files for this run in `folder` and keeps its choices within the policy.
    /// Asks the person about the project once, if `asking` allows and there is a question.
    pub async fn load(args: &Args, folder: &Path, asking: Asking) -> anyhow::Result<Self> {
        let (loaded, outcome) = read(args, folder, asking).await?;
        let choices = loaded.settle()?;
        if args.network && choices.sandbox.value.is_none() {
            anyhow::bail!("--network only applies with a sandbox: pass --sandbox, or set one");
        }
        let mine = &loaded.person_hooks;
        if let Some(rule) = loaded.constraints.own_hooks_forbidden()
            && !mine.is_empty()
        {
            anyhow::bail!(
                "{} has hooks of your own, and {rule} does not allow them: remove its [hooks]",
                loaded
                    .configs
                    .last()
                    .map_or_else(String::new, |path| path.display().to_string())
            );
        }
        let hooks = [mine.clone(), outcome.hooks].concat();
        Ok(Self {
            loaded,
            choices,
            project_servers: outcome.servers,
            withheld: outcome.withheld,
            hooks,
            hooks_off: outcome.hooks_off,
        })
    }

    /// What to tell the person before the first turn, if anything.
    pub fn notice(&self) -> Option<String> {
        let lines = [self.withheld.as_slice(), self.hooks_off.as_slice()].concat();
        (!lines.is_empty()).then(|| lines.join("\nglave: "))
    }

    /// What to tell the model, so it does not look for tools that are not there.
    pub fn model_note(&self) -> Option<String> {
        (!self.withheld.is_empty()).then(|| {
            format!(
                "# Project tools that are off\n\nThis project brings tools or settings that glave \
                 did not turn on:\n{}\nDo not look for them or try to run them yourself. If the \
                 task needs them, say so to the person.",
                self.withheld
                    .iter()
                    .map(|line| format!("- {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
    }
}

/// `glave policy explain`: prints every setting and constraint, and fails when glave would
/// refuse to run here.
pub async fn explain(args: &Args) -> anyhow::Result<ExitCode> {
    let (loaded, _) = read(args, &std::env::current_dir()?, Asking::Explaining).await?;
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

async fn read(args: &Args, folder: &Path, asking: Asking) -> anyhow::Result<(Loaded, Outcome)> {
    let home = home::folder()?;
    let locations = Locations::here(&home, folder);
    let policies = policy::policies(&locations).await?;
    let constraints = policy::Constraints::of(&policies);
    let project = Project::read(&locations.root, &mcp::environment()).await?;
    let outcome = trust::resolve(&home, project, &constraints, asking).await?;
    let mut loaded = policy::load(
        &locations,
        &policies,
        outcome.config.clone(),
        flags(args),
        environment(),
    )
    .await?;
    loaded.project = (!outcome.rows.is_empty()).then(|| ProjectTrust {
        root: locations.root.clone(),
        rows: outcome.rows.clone(),
    });
    Ok((loaded, outcome))
}

/// The choices made with flags.
fn flags(args: &Args) -> ConfigFile {
    ConfigFile {
        model: args.model.clone(),
        approval: args.approval.map(Into::into),
        sandbox: args.sandbox,
        worktree: args.worktree.then_some(true),
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
