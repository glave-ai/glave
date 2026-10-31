//! What a project chooses for its threads (`specs/one-server.md`, "Each project's own tools,
//! hooks and settings"): its `.glave/config.toml`, as far as it applies, among glave's own
//! levels, nearest first: a flag, the environment, the project, the person's file, glave's
//! defaults. Its constraints only narrow, as everywhere. A model it chooses must be one glave
//! offers, and settings the constraints refuse are set aside whole, each with a sentence.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use glave_config::policy::{self, Choices, Source, WebFile};
use glave_config::trust::{self, Outcome, Project};
use glave_harness::Settled;
use glave_ports::ToolSource;
use glave_ports::settings::Judge;
use glave_web::{Web, WebSettings};

use super::ProjectFolders;

/// What one project chooses, as read at the start of a turn.
pub(super) struct Read {
    pub(super) settled: Settled,
    /// Every choice its threads run with, when its settings fit the constraints.
    pub(super) choices: Option<Choices>,
    /// What of its settings glave sets aside, and why, one sentence each.
    pub(super) aside: Vec<String>,
}

impl Read {
    /// `web_fetch` as the project's own `[web]` sets it up, when it has one.
    pub(super) fn web(&self, root: &Path) -> Option<Arc<dyn ToolSource>> {
        let choices = self.choices.as_ref()?;
        if choices.web.from != Source::File(config_of(root)) {
            return None;
        }
        match web_fetch(&choices.web.value) {
            Ok(web) => Some(Arc::new(web)),
            Err(error) => {
                tracing::warn!(%error, "a project's [web] could not be used");
                None
            }
        }
    }
}

/// `web_fetch` as `web` sets it up, offering nothing when it is off.
pub(super) fn web_fetch(web: &WebFile) -> anyhow::Result<Web> {
    let settings = WebSettings {
        policy: web.policy()?,
        limits: glave_web::Limits::default(),
    };
    Web::new(settings).context("could not set up web_fetch")
}

/// `web_fetch` as `web` sets it up, as a tool source; nothing when it is off.
pub(crate) fn web_offered(web: &WebFile) -> anyhow::Result<Option<Arc<dyn ToolSource>>> {
    let web = web_fetch(web)?;
    Ok(web
        .offers_any()
        .then(|| Arc::new(web) as Arc<dyn ToolSource>))
}

/// A project's `config.toml`, in the project at `root`.
fn config_of(root: &Path) -> std::path::PathBuf {
    root.join(".glave").join(policy::CONFIG)
}

impl ProjectFolders {
    /// What the project at `root` on a person's own computer chooses, from `config`, its
    /// `config.toml` as read there: what needs no trust, since nothing there has been trusted,
    /// whatever a rule says of a path on this computer. One that doesn't parse chooses nothing.
    /// What it says stays out of this server's log, which isn't theirs.
    pub(super) fn settled_there(&self, root: &Path, config: &str) -> Settled {
        let Ok(parsed) = policy::config(config, &config_of(root)) else {
            tracing::warn!("a project's settings on a person's computer don't read");
            return Settled::default();
        };
        let project = Project {
            root: root.to_path_buf(),
            remote: None,
            config: Some(parsed),
            servers: BTreeMap::new(),
        };
        let outcome = Outcome {
            config: project.config_without_trust(),
            servers: BTreeMap::new(),
            hooks: Vec::new(),
            withheld: Vec::new(),
            hooks_off: Vec::new(),
            decisions: None,
            rows: Vec::new(),
        };
        let read = self.read(root, &outcome);
        if !read.aside.is_empty() {
            let aside = read.aside.len();
            tracing::warn!(
                aside,
                "a project's settings on a person's computer were set aside"
            );
        }
        read.settled
    }

    /// What the project at `root`, as `outcome` applies its files, chooses for its threads,
    /// saying in the log what was set aside.
    pub(super) fn settle(&self, root: &Path, outcome: &Outcome) -> Read {
        let read = self.read(root, outcome);
        for line in &read.aside {
            tracing::warn!(project = %root.display(), "{line}");
        }
        read
    }

    /// What the project at `root`, as `outcome` applies its files, chooses for its threads.
    fn read(&self, root: &Path, outcome: &Outcome) -> Read {
        let base = self.base();
        let constraints = &base.loaded.constraints;
        let protections = trust::protections(outcome, &base.choices, constraints);
        let path = config_of(root);
        let config = outcome.config.clone().unwrap_or_default();
        let choices = match base.loaded.with_project(&path, config).settle() {
            Ok(choices) => choices,
            Err(error) => {
                return Read {
                    settled: Settled {
                        protections,
                        ..Settled::default()
                    },
                    choices: None,
                    aside: vec![format!(
                        "this project's settings are set aside, since glave cannot use them: \
                         {error}"
                    )],
                };
            }
        };
        let ours = |from: &Source| *from == Source::File(path.clone());
        let mut aside = Vec::new();
        let mut offered = |model: &str, key: &str| {
            let offered = self.models.offers(model);
            if !offered {
                aside.push(format!(
                    "this project's {key} {model:?} is set aside: glave does not offer that \
                     model here, so its threads use the default"
                ));
            }
            offered
        };
        let model = choices
            .model
            .as_ref()
            .filter(|model| ours(&model.from))
            .map(|model| model.value.clone())
            .filter(|model| offered(model, "model"));
        let judge = Some(&choices.judge)
            .filter(|judge| ours(&judge.from))
            .map(|judge| judge.value.clone())
            .filter(|judge| match judge {
                Judge::Model(model) => offered(model, "judge_model"),
                Judge::Main | Judge::Off => true,
            });
        let settled = Settled {
            protections,
            model,
            judge,
            titles: ours(&choices.titles.from).then_some(choices.titles.value),
            worktree: ours(&choices.worktree.from).then_some(choices.worktree.value),
        };
        Read {
            settled,
            choices: Some(choices),
            aside,
        }
    }
}
