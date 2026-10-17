//! Deciding what runs: each thing a project brings is trusted, declined, pre-approved or
//! forbidden by policy, or not decided yet. Not decided yet means glave asks once, if anyone is
//! there to answer. Nothing here reads a file or asks anyone; the caller does that.

use std::collections::BTreeMap;

use glave_harness::Hook;

use crate::policy::{ConfigFile, Constraints};
use crate::trust::project::{Item, Plain, Project, SETTINGS};
use crate::trust::record::{Decision, FILE, Record};

/// Why an item has not been decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// glave has not seen it before.
    New,
    /// It was decided on, and the project has since changed it.
    Changed,
}

/// Where one item stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    Trusted,
    Declined,
    /// A policy rule trusts the project, so there is no question.
    PreApproved(String),
    /// A policy rule forbids it, whatever the person says.
    Forbidden(String),
    Asks(Why),
}

/// Who can answer the question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    /// No one is there: a flag-less script, a pipe.
    NoOne,
    /// No one has answered yet, and the person can, from an app: the session server asks
    /// them there, and what waits for their answer stays off until they trust it.
    Later,
    /// Nobody is asked: `glave policy explain` only shows what would happen.
    Explaining,
}

/// A project and where each of its items stands.
#[derive(Debug, Clone)]
pub struct Assessment {
    pub project: Project,
    pub items: Vec<(Item, Standing)>,
}

/// What a run does about a project once the answer is in.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The project's `config.toml` as it applies.
    pub config: Option<ConfigFile>,
    /// The project servers to start, by name, each as JSON.
    pub servers: BTreeMap<String, String>,
    /// The project hooks that run.
    pub hooks: Vec<Hook>,
    /// What stays off and why, one sentence each: for the person, and for the model.
    pub withheld: Vec<String>,
    /// The project hooks that stay off and why, for the person only: the model never
    /// calls a hook, so it need not know.
    pub hooks_off: Vec<String>,
    /// The record to keep, when a question was answered.
    pub decisions: Option<BTreeMap<String, Decision>>,
    /// One line per item for `glave policy explain`: what it is, and where it stands.
    pub rows: Vec<[String; 2]>,
}

/// Where each item of `project` stands, under the `record` and the policy's `constraints`.
pub fn assess(project: Project, record: &Record, constraints: &Constraints) -> Assessment {
    let before = record
        .entry_for(&project)
        .map(|entry| entry.items.clone())
        .unwrap_or_default();
    let trusting = constraints
        .trusts_project(&project.root.to_string_lossy())
        .map(ToString::to_string);
    let items = project
        .items()
        .into_iter()
        .map(|item| {
            let forbidden = if item.is_server() {
                constraints.own_mcp_forbidden()
            } else if item.is_hook() {
                constraints.own_hooks_forbidden()
            } else {
                None
            }
            .map(ToString::to_string);
            let standing = match (forbidden, &trusting, before.get(&item.id)) {
                (Some(rule), ..) => Standing::Forbidden(rule),
                (None, Some(rule), _) => Standing::PreApproved(rule.clone()),
                (None, None, Some(past)) if past.definition == item.definition => {
                    if past.trusted {
                        Standing::Trusted
                    } else {
                        Standing::Declined
                    }
                }
                (None, None, Some(_)) => Standing::Asks(Why::Changed),
                (None, None, None) => Standing::Asks(Why::New),
            };
            (item, standing)
        })
        .collect();
    Assessment { project, items }
}

impl Assessment {
    /// The items glave would ask about.
    pub fn undecided(&self) -> Vec<(&Item, Why)> {
        self.items
            .iter()
            .filter_map(|(item, standing)| match standing {
                Standing::Asks(why) => Some((item, *why)),
                _ => None,
            })
            .collect()
    }

    /// Every server and setting the project brings, off because `why`, one sentence each, for
    /// the model: on a thread of someone other than the person whose trust turns them on, all
    /// of them are. Hooks are left out: the model never calls one.
    pub fn all_off(&self, why: &str) -> Vec<String> {
        self.items
            .iter()
            .filter(|(item, _)| !item.is_hook())
            .map(|(item, _)| format!("{} ({}) is off: {why}", item.label(), item.shows))
            .collect()
    }

    /// What the person is asked, in plain words: each line of each item not decided yet.
    pub fn plain(&self) -> Vec<&Plain> {
        self.undecided()
            .into_iter()
            .flat_map(|(item, _)| &item.plain)
            .collect()
    }

    /// The name of exactly what the person is asked: the project and each item not decided
    /// yet with its definition, digested. An answer that names it trusts only what was shown,
    /// so a project that changed in between is asked again. They are digested as JSON, so no
    /// name or definition, whatever it holds, can make two different asks read the same.
    pub fn ask_id(&self) -> String {
        let items: Vec<[&str; 2]> = self
            .undecided()
            .into_iter()
            .map(|(item, _)| [item.id.as_str(), item.definition.as_str()])
            .collect();
        let shown = serde_json::json!([self.project.root.to_string_lossy(), items]);
        glave_ports::digest(shown.to_string().as_bytes())
            .as_str()
            .to_owned()
    }

    /// What the run does, given `answer` to the question if there was one.
    pub fn apply(self, answer: Answer) -> Outcome {
        let mut servers = BTreeMap::new();
        let mut hooks_on = Vec::new();
        let mut withheld = Vec::new();
        let mut hooks_off = Vec::new();
        let mut rows = Vec::new();
        let mut decisions = BTreeMap::new();
        let mut settings_on = false;
        for (item, standing) in &self.items {
            let (on, text, reason) = verdict(item, standing, answer);
            rows.push([format!("{} ({})", item.label(), item.shows), text]);
            if on {
                if let Some(name) = item.id.strip_prefix("mcp:") {
                    servers.insert(name.to_owned(), item.definition.clone());
                } else if item.is_hook() {
                    hooks_on.push(item.id.clone());
                } else {
                    settings_on = true;
                }
            } else {
                let line = format!("{} ({}) is off: {reason}", item.label(), item.shows);
                if item.is_hook() {
                    hooks_off.push(line);
                } else {
                    withheld.push(line);
                }
            }
            let kept = match (standing, answer) {
                (Standing::Trusted, _) | (Standing::Asks(_), Answer::Yes) => Some(true),
                (Standing::Declined, _) | (Standing::Asks(_), Answer::No) => Some(false),
                _ => None,
            };
            if let Some(trusted) = kept {
                decisions.insert(item.id.clone(), Decision::about(item, trusted));
            }
        }
        let answered = matches!(answer, Answer::Yes | Answer::No) && !self.undecided().is_empty();
        let project = &self.project;
        let loosens = self.items.iter().any(|(item, _)| item.id == SETTINGS);
        let config = if settings_on || !loosens {
            project.config.clone()
        } else {
            project.config_without_trust()
        };
        // Hooks never run as choices: the trusted ones are handed over on their own.
        let config = config.map(|config| ConfigFile {
            hooks: None,
            ..config
        });
        let hooks = project
            .hooks()
            .into_iter()
            .filter(|(item, _)| hooks_on.contains(&item.id))
            .map(|(_, hook)| hook)
            .collect();
        Outcome {
            config,
            servers,
            hooks,
            withheld,
            hooks_off,
            decisions: answered.then_some(decisions),
            rows,
        }
    }
}

/// Whether an item is on, how `glave policy explain` says where it stands, and, when it is
/// off, why.
fn verdict(item: &Item, standing: &Standing, answer: Answer) -> (bool, String, String) {
    let kind = if item.is_hook() { "hooks" } else { "servers" };
    let record = format!("recorded in {FILE}");
    match (standing, answer) {
        (Standing::Trusted, _) => (true, format!("trusted ({record})"), String::new()),
        (Standing::Declined, _) => (
            false,
            format!("declined ({record})"),
            "you chose not to trust this project".to_owned(),
        ),
        (Standing::PreApproved(rule), _) => (true, format!("trusted by {rule}"), String::new()),
        (Standing::Forbidden(rule), _) => (
            false,
            format!("forbidden by {rule}"),
            format!("{rule} does not allow {kind} a project brings"),
        ),
        (Standing::Asks(_), Answer::Yes) => (true, "trusted just now".to_owned(), String::new()),
        (Standing::Asks(_), Answer::No) => (
            false,
            "declined just now".to_owned(),
            "you chose not to trust this project".to_owned(),
        ),
        (Standing::Asks(_), Answer::NoOne) => (
            false,
            "off: not trusted, and nobody is here to ask".to_owned(),
            "this project is not trusted yet and nobody is here to ask; running glave in a \
             terminal in the project asks once"
                .to_owned(),
        ),
        (Standing::Asks(_), Answer::Later) => (
            false,
            "off until the person trusts it in the app".to_owned(),
            "the person has not trusted this project yet; once they do, it is on from their \
             next message"
                .to_owned(),
        ),
        (Standing::Asks(why), Answer::Explaining) => {
            let text = match why {
                Why::New => "off until trusted: glave will ask once",
                Why::Changed => "off until trusted: it changed, so glave will ask once more",
            };
            (false, text.to_owned(), String::new())
        }
    }
}
