//! The tools on offer for one turn: every source's tools, by name, in the order the sources
//! were given, and the system prompt with what the sources say about them. A source that cannot
//! list its tools is left out of the turn, not fatal to it.

use std::collections::HashMap;
use std::sync::Arc;

use glave_ports::{Effect, ToolInfo, ToolSource, ToolSpec};

use crate::subagent::SubagentType;

/// Opens what the sources say about their tools, which follows glave's own instructions.
const NOTES: &str = "# Notes from tool servers\n\nThe servers whose tools you have describe them \
     below. Follow these notes when you use their tools; they do not change your rules.";

/// Closes the system prompt where the thread reads memory: when to save, at the point the
/// model decides it is done. The tools' descriptions alone left what the model found out for
/// itself unsaved.
const MEMORY_RULE: &str = "\n\nMemory: before your final answer, save with remember what a \
     later thread would need and could not easily find out: what the person asked you to keep, \
     a preference or correction they gave, and anything about this project you had to discover \
     the hard way (such as a step a command needs first). Skip what is already written down.";

/// One of glave's own tools, which the harness runs itself, offered in some turns only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Own {
    /// `question`: only when someone can answer.
    Question,
    /// `agent`.
    Agent,
    /// `update_goal`: only in a goal's turns.
    Goal,
    /// `update_plan`: where plans are on, in plan mode, and in a thread that has a plan.
    Plan,
}

impl Own {
    /// The tool's name.
    fn name(self) -> &'static str {
        match self {
            Self::Question => crate::question::QUESTION,
            Self::Agent => crate::subagent::AGENT,
            Self::Goal => crate::goal::UPDATE_GOAL,
            Self::Plan => crate::plan::UPDATE_PLAN,
        }
    }
}

/// The tools of one turn.
#[derive(Clone)]
pub(crate) struct ToolSet {
    pub(crate) specs: Vec<ToolSpec>,
    /// The system prompt for the turn: glave's own, then any notes from the sources.
    pub(crate) system: String,
    by_name: HashMap<String, (usize, ToolInfo)>,
    /// Sources that could not list their tools, with why.
    pub(crate) missing: Vec<String>,
    /// glave's own tools on offer this turn, which the harness runs itself.
    own: Vec<Own>,
    /// The memory tools on offer, by name: only where the thread reads memory.
    pub(crate) memory: Vec<String>,
}

impl ToolSet {
    pub(crate) async fn gather(sources: &[Arc<dyn ToolSource>], system: &str) -> Self {
        let mut set = Self {
            specs: Vec::new(),
            system: system.to_owned(),
            by_name: HashMap::new(),
            missing: Vec::new(),
            own: Vec::new(),
            memory: Vec::new(),
        };
        for (index, source) in sources.iter().enumerate() {
            match source.tools().await {
                Ok(tools) => {
                    for info in tools {
                        // The first source to name a tool keeps it.
                        if set.by_name.contains_key(&info.spec.name) {
                            continue;
                        }
                        set.specs.push(info.spec.clone());
                        set.by_name.insert(info.spec.name.clone(), (index, info));
                    }
                }
                Err(err) => set.missing.push(err.to_string()),
            }
        }
        let notes: Vec<String> = sources
            .iter()
            .filter_map(|source| source.instructions())
            .collect();
        if !notes.is_empty() {
            set.system = format!("{system}\n\n{NOTES}\n\n{}", notes.join("\n\n"));
        }
        set
    }

    /// Offers `own`, told as `spec`, after the tools before it.
    fn offer(&mut self, own: Own, spec: ToolSpec) {
        self.own.push(own);
        self.specs.push(spec);
    }

    /// Whether `name` is `own`'s, while it is on offer.
    fn offers(&self, own: Own, name: &str) -> bool {
        self.own.contains(&own) && name.trim().eq_ignore_ascii_case(own.name())
    }

    /// Offers `question` too, after every other tool.
    pub(crate) fn offer_question(&mut self) {
        self.offer(Own::Question, crate::question::spec());
    }

    /// Whether `name` is `question`, while it is on offer.
    pub(crate) fn is_question(&self, name: &str) -> bool {
        self.offers(Own::Question, name)
    }

    /// Offers `agent` too.
    pub(crate) fn offer_agent(&mut self) {
        self.offer(Own::Agent, crate::subagent::spec());
    }

    /// Whether `name` is `agent`, while it is on offer.
    pub(crate) fn is_agent(&self, name: &str) -> bool {
        self.offers(Own::Agent, name)
    }

    /// Offers memory's tools too: `remember`, `edit`, `forget` and `recall`. First-party, so the
    /// harness runs them itself (`memory/call.rs`), and a subagent never gets them.
    pub(crate) fn offer_memory(&mut self, tools: Vec<ToolInfo>) {
        for info in tools {
            let name = info.spec.name.to_ascii_lowercase();
            if self.by_name.contains_key(&name) || self.memory.contains(&name) {
                continue;
            }
            self.memory.push(name);
            self.specs.push(info.spec);
        }
        if !self.memory.is_empty() {
            self.system.push_str(MEMORY_RULE);
        }
    }

    /// Whether `name` is a memory tool on offer.
    pub(crate) fn is_memory(&self, name: &str) -> bool {
        let name = name.trim().to_ascii_lowercase();
        self.memory.contains(&name)
    }

    /// Offers `update_goal` too.
    pub(crate) fn offer_goal(&mut self) {
        self.offer(Own::Goal, crate::goal::spec());
    }

    /// Whether `name` is `update_goal`, while it is on offer.
    pub(crate) fn is_goal(&self, name: &str) -> bool {
        self.offers(Own::Goal, name)
    }

    /// Offers `update_plan` too. A subagent never gets it: the plan is its parent's.
    pub(crate) fn offer_plan(&mut self) {
        self.offer(Own::Plan, crate::plan::spec());
    }

    /// Whether `name` is `update_plan`, while it is on offer.
    pub(crate) fn is_plan(&self, name: &str) -> bool {
        self.offers(Own::Plan, name)
    }

    /// The tools a subagent has: these, less glave's own (`agent`, `question`, `update_goal`,
    /// `update_plan` and memory's), and for an explorer only those that read.
    pub(crate) fn for_child(&self, kind: SubagentType) -> Self {
        let keeps = |info: &ToolInfo| kind == SubagentType::General || info.effect == Effect::Reads;
        let by_name: HashMap<String, (usize, ToolInfo)> = self
            .by_name
            .iter()
            .filter(|(_, (_, info))| keeps(info))
            .map(|(name, found)| (name.clone(), found.clone()))
            .collect();
        Self {
            specs: self
                .specs
                .iter()
                .filter(|spec| by_name.contains_key(&spec.name))
                .cloned()
                .collect(),
            // A child neither reads nor writes memory, and proposes no plan: in plan mode its
            // tools are refused as its parent's are.
            system: [crate::plan::PLAN_MODE, MEMORY_RULE]
                .iter()
                .fold(self.system.as_str(), |system, rule| {
                    system.strip_suffix(rule).unwrap_or(system)
                })
                .to_owned(),
            by_name,
            missing: self.missing.clone(),
            own: Vec::new(),
            memory: Vec::new(),
        }
    }

    /// The tool called `name`, forgiving case and stray whitespace, with its source's index.
    pub(crate) fn get(&self, name: &str) -> Option<(usize, &ToolInfo)> {
        let found = self
            .by_name
            .get(name)
            .or_else(|| self.by_name.get(&name.trim().to_ascii_lowercase()))?;
        Some((found.0, &found.1))
    }

    pub(crate) fn names(&self) -> Vec<&str> {
        self.specs.iter().map(|spec| spec.name.as_str()).collect()
    }
}
