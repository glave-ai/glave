//! Deciding what runs: each thing a project brings is trusted, declined, pre-approved or
//! forbidden by policy, or not decided yet. Not decided yet means glave asks once, if anyone is
//! there to answer. Nothing here reads a file or asks anyone; the caller does that.

use std::collections::BTreeMap;

use crate::policy::{ConfigFile, Constraints};
use crate::trust::project::{Item, Project, SETTINGS};
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
    /// No one is there: a flag-less script, a pipe, the session server.
    NoOne,
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
    /// What stays off and why, one sentence each: for the person, and for the model.
    pub withheld: Vec<String>,
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
            let forbidden = constraints
                .own_mcp_forbidden()
                .filter(|_| item.is_server())
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

    /// What the run does, given `answer` to the question if there was one.
    pub fn apply(self, answer: Answer) -> Outcome {
        let mut servers = BTreeMap::new();
        let mut withheld = Vec::new();
        let mut rows = Vec::new();
        let mut decisions = BTreeMap::new();
        let mut settings_on = false;
        for (item, standing) in &self.items {
            let (on, text, reason) = verdict(standing, answer);
            rows.push([format!("{} ({})", item.label(), item.shows), text]);
            if on {
                match item.id.strip_prefix("mcp:") {
                    Some(name) => {
                        servers.insert(name.to_owned(), item.definition.clone());
                    }
                    None => settings_on = true,
                }
            } else {
                withheld.push(format!(
                    "{} ({}) is off: {reason}",
                    item.label(),
                    item.shows
                ));
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
        Outcome {
            config,
            servers,
            withheld,
            decisions: answered.then_some(decisions),
            rows,
        }
    }
}

/// Whether an item is on, how `glave policy explain` says where it stands, and, when it is
/// off, why.
fn verdict(standing: &Standing, answer: Answer) -> (bool, String, String) {
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
            format!("{rule} does not allow servers a project brings"),
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
        (Standing::Asks(why), Answer::Explaining) => {
            let text = match why {
                Why::New => "off until trusted: glave will ask once",
                Why::Changed => "off until trusted: it changed, so glave will ask once more",
            };
            (false, text.to_owned(), String::new())
        }
    }
}
