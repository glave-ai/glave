//! Constraints, which only narrow: every rule of every policy file applies at once, so a later
//! file can add rules but never lift one. Allow-lists intersect (a value must pass each), deny
//! lists add up, a requirement anywhere is required, and limits take the smallest.

use std::fmt;
use std::path::{Path, PathBuf};

use glave_event::{Money, Permissions};
use globset::Glob;

use crate::policy::files::{JudgeRule, PolicyFile, TaskRules};
use crate::policy::memory::MemoryRules;
use crate::policy::placement::Placement;
use crate::policy::workers::{NetworkRule, WorkerRules, limits};

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
    /// The most anyone may choose.
    PermissionsHighest(Permissions),
    Judge(JudgeRule),
    /// Commands go without the network, which only the sandbox can keep from them.
    NetworkOff,
    /// No command without the network may ask the person to run with it.
    NetworkAskForbidden,
    /// The servers the organization offers.
    McpCatalog(Vec<String>),
    /// People may not add MCP servers of their own.
    McpOwnForbidden,
    /// What these MCP servers return is untrusted, as web pages are.
    McpUntrusted(Vec<String>),
    /// These MCP servers are vouched for, though remote or a project's.
    McpTrusted(Vec<String>),
    /// Each person's budget per UTC day, in US dollars.
    PersonPerDayUsd(Money),
    /// Projects at these paths (globs) are trusted without asking.
    ProjectsTrusted(Vec<String>),
    /// People and projects may not run hooks of their own.
    HooksOwnForbidden,
    /// What workers may be.
    Workers(WorkerRules),
    /// How many tasks may run, and schedules be kept.
    Tasks(TaskRules),
    /// What memory may be.
    Memory(MemoryRules),
    /// Where threads may work.
    Placement(Vec<Placement>),
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
            Self::PermissionsHighest(preset) => {
                write!(f, "[permissions] highest = {:?}", preset.name())
            }
            Self::Judge(rule) => write!(f, "[approval] judge = {:?}", rule.name()),
            Self::NetworkOff => f.write_str("[sandbox] network = false"),
            Self::NetworkAskForbidden => f.write_str("[sandbox] ask_network = false"),
            Self::McpCatalog(names) => write!(f, "[mcp] catalog = {}", quoted(names)),
            Self::McpOwnForbidden => f.write_str("[mcp] own = false"),
            Self::McpUntrusted(names) => write!(f, "[mcp] untrusted = {}", quoted(names)),
            Self::McpTrusted(names) => write!(f, "[mcp] trusted = {}", quoted(names)),
            Self::PersonPerDayUsd(usd) => {
                write!(f, "[budget] person_per_day_usd = {}", usd.decimal())
            }
            Self::ProjectsTrusted(globs) => write!(f, "[projects] trusted = {}", quoted(globs)),
            Self::HooksOwnForbidden => f.write_str("[hooks] own = false"),
            Self::Workers(rules) => f.write_str(&rules.describe()),
            Self::Memory(rules) => f.write_str(&rules.describe()),
            Self::Placement(places) => {
                let names: Vec<String> = places.iter().map(|p| p.name().to_owned()).collect();
                write!(f, "[placement] allowed = {}", quoted(&names))
            }
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
            if let Some(highest) = file.permissions.highest {
                add(Kind::PermissionsHighest(highest));
            }
            if let Some(judge) = file.approval.judge.filter(|j| *j != JudgeRule::Allowed) {
                add(Kind::Judge(judge));
            }
            if file.sandbox.network == Some(false) {
                add(Kind::NetworkOff);
            }
            if file.sandbox.ask_network == Some(false) {
                add(Kind::NetworkAskForbidden);
            }
            if let Some(catalog) = &file.mcp.catalog {
                add(Kind::McpCatalog(catalog.clone()));
            }
            if file.mcp.own == Some(false) {
                add(Kind::McpOwnForbidden);
            }
            if !file.mcp.untrusted.is_empty() {
                add(Kind::McpUntrusted(file.mcp.untrusted.clone()));
            }
            if !file.mcp.trusted.is_empty() {
                add(Kind::McpTrusted(file.mcp.trusted.clone()));
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
            if !file.memory.is_empty() {
                add(Kind::Memory(file.memory.clone()));
            }
            if let Some(allowed) = &file.placement.allowed {
                add(Kind::Placement(allowed.clone()));
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

    /// The most anyone may choose, and the first rule that keeps it there: the lowest
    /// `[permissions] highest`, and Workspace where a rule takes the network from commands,
    /// which only the sandbox can keep. Full access, and no rule, where none narrows it.
    pub fn highest_permissions(&self) -> (Permissions, Option<&Rule>) {
        let caps = self.rules.iter().filter_map(|rule| match rule.kind {
            Kind::PermissionsHighest(preset) => Some((preset, rule)),
            Kind::NetworkOff => Some((Permissions::Workspace, rule)),
            _ => None,
        });
        caps.fold((Permissions::FullAccess, None), |(lowest, first), (preset, rule)| {
            if preset < lowest {
                (preset, Some(rule))
            } else {
                (lowest, first)
            }
        })
    }

    /// The presets anyone may choose, the most cautious first.
    pub fn permissions_allowed(&self) -> Vec<Permissions> {
        let (highest, _) = self.highest_permissions();
        Permissions::ALL
            .into_iter()
            .filter(|preset| *preset <= highest)
            .collect()
    }

    /// The judge rule in force, `required` or `off`; an error when files ask for both.
    pub fn judge(&self) -> crate::Result<Option<(JudgeRule, &Path)>> {
        let mut found: Option<&Rule> = None;
        for rule in &self.rules {
            if !matches!(rule.kind, Kind::Judge(_)) {
                continue;
            }
            match found {
                Some(first) if first.kind != rule.kind => {
                    crate::error::refuse!(
                        "{first} and {rule} contradict each other: nothing meets both"
                    )
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

    /// The rule that requires the sandbox, if one does: one that leaves no Full access, so
    /// commands never run without the sandbox and without asking.
    pub fn sandbox_required(&self) -> Option<&Rule> {
        self.highest_permissions().1
    }

    /// The first rule that takes the network from commands.
    pub fn network_off(&self) -> Option<&Rule> {
        self.first(|kind| matches!(kind, Kind::NetworkOff))
    }

    /// The first rule that forbids a command without the network to ask for it.
    pub fn network_ask_forbidden(&self) -> Option<&Rule> {
        self.first(|kind| matches!(kind, Kind::NetworkAskForbidden))
    }

    /// The first rule that forbids people's own MCP servers.
    pub fn own_mcp_forbidden(&self) -> Option<&Rule> {
        self.first(|kind| matches!(kind, Kind::McpOwnForbidden))
    }

    /// The first rule that marks the MCP server `name` untrusted.
    pub fn mcp_untrusted(&self, name: &str) -> Option<&Rule> {
        self.first(
            |kind| matches!(kind, Kind::McpUntrusted(names) if names.iter().any(|n| n == name)),
        )
    }

    /// The first rule that vouches for the MCP server `name`. Like `trusts_project`, it lifts a
    /// default rather than adding a limit; it never lifts an `untrusted` mark.
    pub fn mcp_trusted(&self, name: &str) -> Option<&Rule> {
        self.first(
            |kind| matches!(kind, Kind::McpTrusted(names) if names.iter().any(|n| n == name)),
        )
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
            .min_by_key(|(usd, _)| usd.micros)
            .map(|(_, rule)| rule)
    }

    /// The smallest daily budget per person.
    pub fn person_per_day(&self) -> Option<Money> {
        self.person_per_day_usd().and_then(|rule| match rule.kind {
            Kind::PersonPerDayUsd(usd) => Some(usd),
            _ => None,
        })
    }

    /// The servers every `[mcp] catalog` rule offers (each narrows the one before), and the
    /// first such rule; `None` when no rule names a catalog.
    pub fn mcp_catalog(&self) -> Option<(Vec<String>, &Rule)> {
        let mut found: Option<(Vec<String>, &Rule)> = None;
        for rule in &self.rules {
            if let Kind::McpCatalog(names) = &rule.kind {
                found = Some(match found {
                    None => (names.clone(), rule),
                    Some((kept, first)) => (
                        kept.into_iter()
                            .filter(|name| names.contains(name))
                            .collect(),
                        first,
                    ),
                });
            }
        }
        found
    }

    /// The rules glave cannot enforce yet, with why. glave refuses to run under any of them,
    /// rather than quietly not keeping them.
    pub fn not_enforced(&self) -> Vec<(&Rule, &'static str)> {
        let budget = self.person_per_day_usd().map(|rule| {
            (
                rule,
                "glave keeps a daily budget only as a gateway serving many (glave gateway \
                 serve); in glave itself it keeps budgets for one run",
            )
        });
        self.rules
            .iter()
            .filter_map(|rule| match rule.kind {
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

    /// The rules a gateway serving many cannot enforce yet: the daily budget it keeps.
    pub fn not_enforced_by_the_gateway(&self) -> Vec<(&Rule, &'static str)> {
        self.not_enforced()
            .into_iter()
            .filter(|(rule, _)| !matches!(rule.kind, Kind::PersonPerDayUsd(_)))
            .collect()
    }

    /// The limits every `[tasks]` rule sets at once: the smallest of each.
    pub fn task_limits(&self) -> TaskRules {
        let smallest = |a: Option<usize>, b: Option<usize>| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let mut limits = TaskRules::default();
        for rule in &self.rules {
            if let Kind::Tasks(tasks) = &rule.kind {
                limits.max_running = smallest(limits.max_running, tasks.max_running);
                limits.max_schedules = smallest(limits.max_schedules, tasks.max_schedules);
            }
        }
        limits
    }

    /// The limits every `[workers]` rule sets at once, and a rule that takes the network from
    /// commands takes it from workers too: theirs is set when they start, so none asks.
    pub fn worker_limits(&self) -> glave_ports::computer::Limits {
        let mut limits = limits(self.rules.iter().filter_map(|rule| match &rule.kind {
            Kind::Workers(rules) => Some(rules),
            _ => None,
        }));
        if self.network_off().is_some() {
            limits.network = Some(glave_ports::computer::Network::Off);
        }
        limits
    }

    fn first(&self, test: impl Fn(&Kind) -> bool) -> Option<&Rule> {
        self.rules.iter().find(|rule| test(&rule.kind))
    }
}
