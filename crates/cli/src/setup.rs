//! The settings a run uses: its flags and the environment over the person's `config.toml`,
//! kept within the policy (`specs/policy.md`); what the folder it runs in asks the person here
//! to trust; and `glave policy explain`, which shows them. A project's own settings are its
//! threads' alone (`project_folders.rs`), never the server's.

use std::path::Path;
use std::process::ExitCode;

use glave_ports::computer::Sandbox;
use glave_ports::settings::Hook;

use crate::args::Args;
use crate::{config, home};
use glave_config::policy::{
    self, Choices, ConfigFile, Loaded, Locations, ProjectTrust, ServerFile,
};
use glave_config::trust::{self, Asking, Outcome, Project, Record};

use crate::data::Data;
use crate::trust::Trust;

/// Why the sandbox cannot run, as `glave policy explain` says it without trying.
pub(crate) const NO_SANDBOX: &str = "this computer has no sandbox glave can use (macOS's Seatbelt, or \
                          bubblewrap on Linux)";

/// What this run uses: the files read, and the choices within the constraints.
pub(crate) struct Setup {
    pub loaded: Loaded,
    pub choices: Choices,
    /// The hooks that run for the person's threads everywhere: their own. A project's trusted
    /// hooks run for its own threads alone, in its own folder (`project_folders.rs`).
    pub hooks: Vec<Hook>,
    /// The most a sandbox on this computer allows, which each mode narrows, once the
    /// workspace found it (`sandboxing.rs`); none where none can run, or before it looked.
    pub sandbox: Option<Sandbox>,
}

impl Setup {
    /// Reads the files for this run in `folder`, the project's own aside, and keeps its choices
    /// within the policy.
    pub(crate) async fn load(args: &Args, folder: &Path) -> anyhow::Result<Self> {
        let loaded = read(args, folder, None).await?;
        let choices = served(loaded.settle()?, args);
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
        let hooks = mine.clone();
        Ok(Self {
            loaded,
            choices,
            hooks,
            sandbox: None,
        })
    }

    /// The files at `locations` read again by a server that already runs (`reread.rs`), as
    /// [`Setup::load`] reads them, but it cannot refuse to start: a choice the constraints
    /// refuse now gives way as glave's default would, and the person's hooks a rule forbids
    /// are left out. What gave way, one sentence each.
    pub(crate) async fn reload(
        args: &Args,
        locations: &Locations,
    ) -> anyhow::Result<(Self, Vec<String>)> {
        let loaded = read_at(args, locations, None).await?;
        let (choices, mut aside) = loaded.settle_giving_way()?;
        let choices = served(choices, args);
        let mine = &loaded.person_hooks;
        let hooks = match loaded.constraints.own_hooks_forbidden() {
            Some(rule) if !mine.is_empty() => {
                aside.push(format!(
                    "your own hooks are off: {rule} does not allow them"
                ));
                Vec::new()
            }
            _ => mine.clone(),
        };
        let setup = Self {
            loaded,
            choices,
            hooks,
            sandbox: None,
        };
        Ok((setup, aside))
    }
}

/// `choices`, their origin as glave serves it over HTTP where `args` say it does: this
/// computer's `http://localhost`, named without a port, at the one it listens on.
fn served(mut choices: Choices, args: &Args) -> Choices {
    if let (Some(origin), Some(address)) = (choices.origin.as_mut(), args.http) {
        origin.value = origin.value.serving_at(address.port());
    }
    choices
}

/// Asks the person about the project around `folder` once, if `asking` allows and there is a
/// question, keeping the answer where every server reads it; then what it brings that stays
/// off, and why, to tell the person.
pub(crate) async fn ask_here(
    folder: &Path,
    setup: &Setup,
    asking: Asking,
    trust: &Trust,
) -> anyhow::Result<Vec<String>> {
    let outcome = resolve(folder, &setup.loaded.constraints, asking, trust).await?;
    Ok([outcome.withheld, outcome.hooks_off].concat())
}

/// What the model is told of `withheld`, a project's tools and settings that stay off with
/// why, so it does not look for them; nothing when nothing is off.
pub(crate) fn off_note(withheld: &[String]) -> Option<String> {
    (!withheld.is_empty()).then(|| {
        format!(
            "# Project tools that are off\n\nThis project brings tools or settings that glave \
             did not turn on:\n{}\nDo not look for them or try to run them yourself. If the \
             task needs them, say so to the person.",
            withheld
                .iter()
                .map(|line| format!("- {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

/// `glave policy explain`: prints every setting and constraint as a thread in this folder's
/// project would run with them, and fails when glave would refuse to run here.
pub(crate) async fn explain(args: &Args) -> anyhow::Result<ExitCode> {
    let folder = std::env::current_dir()?;
    let home = home::folder()?;
    let policies = policy::policies(&Locations::here(&home, &folder)).await?;
    let constraints = policy::Constraints::of(&policies);
    let store = Data::here().await?;
    let person = crate::identity::load_or_make(&store).await?.person;
    let trust = Trust::open(&store, person).await?;
    let outcome = resolve(&folder, &constraints, Asking::Explaining, &trust).await?;
    let mut loaded = read(args, &folder, Some(&outcome)).await?;
    // Where people have computers of their own, their commands run there.
    if loaded.choices.computers.is_none() && !glave_computer::sandbox_available() {
        loaded.no_sandbox_here = Some(NO_SANDBOX.to_owned());
    }
    // The provider names models as the gateway sees them; without a model there is none.
    let model = loaded
        .choices
        .model
        .as_ref()
        .map(|model| model.value.clone());
    let provider = match (model, &loaded.choices.provider) {
        (Some(_), Some(cloud)) => Some(cloud.value.name().to_owned()),
        (Some(model), None) => Some(config::endpoint(Some(model), None)?.provider.provider),
        (None, _) => None,
    };
    let (text, runs) = policy::explain(&loaded, provider.as_deref());
    print!("{text}");
    Ok(if runs {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// What the project around `folder` runs, asking the person when `asking` lets and keeping
/// their answer in `trust`.
async fn resolve(
    folder: &Path,
    constraints: &policy::Constraints,
    asking: Asking,
    trust: &Trust,
) -> anyhow::Result<Outcome> {
    let home = home::folder()?;
    let root = Locations::here(&home, folder).root;
    let project = Project::read(&root).await?;
    let record = trust.record().await?;
    let outcome = trust::resolve(&record, project.clone(), constraints, asking);
    if let Some(decisions) = &outcome.decisions {
        trust
            .keep(Record::deciding(&project, decisions.clone()))
            .await?;
    }
    Ok(outcome)
}

/// The files for a run in `folder`, with the project's settings as `project` applies them,
/// when they count: `glave policy explain` shows a thread of that project.
async fn read(args: &Args, folder: &Path, project: Option<&Outcome>) -> anyhow::Result<Loaded> {
    let home = home::folder()?;
    read_at(args, &Locations::here(&home, folder), project).await
}

/// The files at `locations`, as [`read`] reads them.
async fn read_at(
    args: &Args,
    locations: &Locations,
    project: Option<&Outcome>,
) -> anyhow::Result<Loaded> {
    let policies = policy::policies(locations).await?;
    let config = project.and_then(|outcome| outcome.config.clone());
    let mut loaded = policy::load(locations, &policies, config, flags(args), environment()).await?;
    loaded.project = project
        .filter(|outcome| !outcome.rows.is_empty())
        .map(|outcome| ProjectTrust {
            root: locations.root.clone(),
            rows: outcome.rows.clone(),
        });
    Ok(loaded)
}

/// The choices made with flags. A prompt given on the command line is answered once, so its
/// thread gets no title from a model.
fn flags(args: &Args) -> ConfigFile {
    let one_shot = args.action == crate::args::Action::Run && !args.prompt.is_empty();
    ConfigFile {
        model: args.model.clone(),
        permissions: args.permissions,
        network: args.network,
        worktree: args.worktree.then_some(true),
        titles: one_shot.then_some(false),
        on: args.on,
        server: args.server.clone().map(|url| ServerFile { url }),
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
        store: crate::data::environment(),
        ..ConfigFile::default()
    }
}
