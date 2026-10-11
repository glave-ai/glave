//! Constraints, which only narrow: every rule of every policy file applies at once, so a later
//! file can add rules but never lift one. Allow-lists intersect (a value must pass each), deny
//! lists add up, a requirement anywhere is required, and limits take the smallest.

use std::fmt;
use std::path::{Path, PathBuf};

use glave_harness::ApprovalMode;
use globset::Glob;

use crate::policy::files::{JudgeRule, PolicyFile, TaskRules};
use crate::policy::workers::{NetworkRule, WorkerRules, limits};

/// The approval modes there are, cautious first.
const MODES: [ApprovalMode; 2] = [ApprovalMode::Ask, ApprovalMode::Auto];

/// One rule, from one policy file.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub kind: Kind,
    pub from: PathBuf,
}

/// What a rule says.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Models must match one of these globs over `provider/model`.
    ModelsAllow(Vec<String>),
    /// Models must match none of these.
    ModelsDeny(Vec<String>),
    ApprovalModes(Vec<ApprovalMode>),
    Judge(JudgeRule),
    SandboxRequired,
    /// The servers the organization offers.
    McpCatalog(Vec<String>),
    /// People may not add MCP servers of their own.
    McpOwnForbidden,
    PersonPerDayUsd(f64),
    /// Projects at these paths (globs) are trusted without asking.
    ProjectsTrusted(Vec<String>),
    /// People and projects may not run hooks of their own.
    HooksOwnForbidden,
    /// What workers may be.
    Workers(WorkerRules),
    /// How many tasks may run, and schedules be kept.
    Tasks(TaskRules),
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let quoted = |items: &[String]| {
            let items: Vec<String> = items.iter().map(|item| format!("{item:?}")).collect();
            format!("[{}]", items.join(", "))
        };
        match self {
            Self::ModelsAllow(globs) => write!(f, "[models] allow = {}", quoted(globs)),
            Self::ModelsDeny(globs) => write!(f, "[models] deny = {}", quoted(globs)),
            Self::ApprovalModes(modes) => {
                let names: Vec<String> = modes.iter().map(|m| m.name().to_owned()).collect();
                write!(f, "[approval] modes = {}", quoted(&names))
            }
            Self::Judge(rule) => write!(f, "[approval] judge = {:?}", rule.name()),
            Self::SandboxRequired => f.write_str("[sandbox] required = true"),
            Self::McpCatalog(names) => write!(f, "[mcp] catalog = {}", quoted(names)),
            Self::McpOwnForbidden => f.write_str("[mcp] own = false"),
            Self::PersonPerDayUsd(usd) => write!(f, "[budget] person_per_day_usd = {usd}"),
            Self::ProjectsTrusted(globs) => write!(f, "[projects] trusted = {}", quoted(globs)),
            Self::HooksOwnForbidden => f.write_str("[hooks] own = false"),
            Self::Workers(rules) => f.write_str(&rules.describe()),
            Self::Tasks(rules) => {
                let parts: Vec<String> = [
                    ("max_running", rules.max_running),
                    ("max_schedules", rules.max_schedules),
                ]
                .iter()
                .filter_map(|(key, value)| value.map(|value| format!("{key} = {value}")))
                .collect();
                write!(f, "[tasks] {}", parts.join(", "))
            }
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} in {}", self.kind, self.from.display())
    }
}

/// Every rule of every policy file, in the order the files were read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Constraints {
    rules: Vec<Rule>,
}

impl Constraints {
    /// The rules of `files`, organization first.
    pub fn of(files: &[(PathBuf, PolicyFile)]) -> Self {
        let mut rules = Vec::new();
        for (path, file) in files {
            let mut add = |kind| {
                rules.push(Rule {
                    kind,
                    from: path.clone(),
                });
            };
            if let Some(allow) = &file.models.allow {
                add(Kind::ModelsAllow(allow.clone()));
            }
            if !file.models.deny.is_empty() {
                add(Kind::ModelsDeny(file.models.deny.clone()));
            }
            if let Some(modes) = &file.approval.modes {
                add(Kind::ApprovalModes(
                    modes.iter().map(|m| m.mode()).collect(),
                ));
            }
            if let Some(judge) = file.approval.judge.filter(|j| *j != JudgeRule::Allowed) {
                add(Kind::Judge(judge));
            }
            if file.sandbox.required {
                add(Kind::SandboxRequired);
            }
            if let Some(catalog) = &file.mcp.catalog {
                add(Kind::McpCatalog(catalog.clone()));
            }
            if file.mcp.own == Some(false) {
                add(Kind::McpOwnForbidden);
            }
            if let Some(usd) = file.budget.person_per_day_usd {
                add(Kind::PersonPerDayUsd(usd));
            }
            if !file.projects.trusted.is_empty() {
                add(Kind::ProjectsTrusted(file.projects.trusted.clone()));
            }
            if file.hooks.own == Some(false) {
                add(Kind::HooksOwnForbidden);
            }
            if !file.workers.is_empty() {
                add(Kind::Workers(file.workers.clone()));
            }
            if file.tasks != TaskRules::default() {
                add(Kind::Tasks(file.tasks));
            }
        }
        Self { rules }
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// The rule that refuses the model `name` (`provider/model`), if one does.
    pub fn refuses_model(&self, name: &str) -> Option<&Rule> {
        let matches =
            |glob: &String| Glob::new(glob).is_ok_and(|glob| glob.compile_matcher().is_match(name));
        self.rules.iter().find(|rule| match &rule.kind {
            Kind::ModelsAllow(globs) => !globs.iter().any(matches),
            Kind::ModelsDeny(globs) => globs.iter().any(matches),
            _ => false,
        })
    }

    /// The approval modes every rule allows, and the rules that narrowed them.
    pub fn approval_modes(&self) -> (Vec<ApprovalMode>, Vec<&Rule>) {
        let narrowing: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|rule| matches!(rule.kind, Kind::ApprovalModes(_)))
            .collect();
        let allowed = MODES
            .into_iter()
            .filter(|mode| {
                narrowing.iter().all(|rule| match &rule.kind {
                    Kind::ApprovalModes(modes) => modes.contains(mode),
                    _ => true,
                })
            })
            .collect();
        (allowed, narrowing)
    }

    /// The judge rule in force, `required` or `off`; an error when files ask for both.
    pub fn judge(&self) -> anyhow::Result<Option<(JudgeRule, &Path)>> {
        let mut found: Option<&Rule> = None;
        for rule in &self.rules {
            if !matches!(rule.kind, Kind::Judge(_)) {
                continue;
            }
            match found {
                Some(first) if first.kind != rule.kind => {
                    anyhow::bail!("{first} and {rule} contradict each other: nothing meets both")
                }
                Some(_) => {}
                None => found = Some(rule),
            }
        }
        Ok(found.and_then(|rule| match rule.kind {
            Kind::Judge(judge) => Some((judge, rule.from.as_path())),
            _ => None,
        }))
    }

    /// The first rule that requires the sandbox.
    pub fn sandbox_required(&self) -> Option<&Rule> {
        self.first(|kind| matches!(kind, Kind::SandboxRequired))
    }

    /// The first rule that forbids people's own MCP servers.
    pub fn own_mcp_forbidden(&self) -> Option<&Rule> {
        self.first(|kind| matches!(kind, Kind::McpOwnForbidden))
    }

    /// The first rule that forbids people's and projects' own hooks.
    pub fn own_hooks_forbidden(&self) -> Option<&Rule> {
        self.first(|kind| matches!(kind, Kind::HooksOwnForbidden))
    }

    /// The first rule that trusts the project at `root` without asking. Unlike the others
    /// this one lifts a question rather than adding a limit, so any file may add to it; it
    /// never lifts a deny.
    pub fn trusts_project(&self, root: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| match &rule.kind {
            Kind::ProjectsTrusted(globs) => globs
                .iter()
                .any(|glob| Glob::new(glob).is_ok_and(|g| g.compile_matcher().is_match(root))),
            _ => false,
        })
    }

    /// The smallest daily budget per person, and the rule that set it.
    pub fn person_per_day_usd(&self) -> Option<&Rule> {
        self.rules
            .iter()
            .filter_map(|rule| match rule.kind {
                Kind::PersonPerDayUsd(usd) => Some((usd, rule)),
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, rule)| rule)
    }

    /// The rules glave cannot enforce yet, with why. glave refuses to run under any of them,
    /// rather than quietly not keeping them.
    pub fn not_enforced(&self) -> Vec<(&Rule, &'static str)> {
        let budget = self.person_per_day_usd().map(|rule| {
            (
                rule,
                "the gateway keeps budgets for one run, not per day, yet",
            )
        });
        self.rules
            .iter()
            .filter_map(|rule| match rule.kind {
                Kind::McpCatalog(_) => Some((
                    rule,
                    "glave cannot start servers an organization offers yet",
                )),
                Kind::Workers(WorkerRules {
                    network: Some(NetworkRule::Allowlist),
                    ..
                }) => Some((
                    rule,
                    "an allow-list of hosts is not enforced on workers yet",
                )),
                _ => None,
            })
            .chain(budget)
            .collect()
    }

    /// The limits every `[tasks]` rule sets at once: the smallest of each.
    pub fn task_limits(&self) -> glave_session::TaskLimits {
        let smallest = |a: Option<usize>, b: Option<usize>| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let mut limits = glave_session::TaskLimits::default();
        for rule in &self.rules {
            if let Kind::Tasks(tasks) = &rule.kind {
                limits.max_running = smallest(limits.max_running, tasks.max_running);
                limits.max_schedules = smallest(limits.max_schedules, tasks.max_schedules);
            }
        }
        limits
    }

    /// The limits every `[workers]` rule sets at once.
    pub fn worker_limits(&self) -> glave_computer::Limits {
        limits(self.rules.iter().filter_map(|rule| match &rule.kind {
            Kind::Workers(rules) => Some(rules),
            _ => None,
        }))
    }

    fn first(&self, test: impl Fn(&Kind) -> bool) -> Option<&Rule> {
        self.rules.iter().find(|rule| test(&rule.kind))
    }
}
