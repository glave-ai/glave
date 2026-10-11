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

/// The tools of one turn.
#[derive(Clone)]
pub(crate) struct ToolSet {
    pub(crate) specs: Vec<ToolSpec>,
    /// The system prompt for the turn: glave's own, then any notes from the sources.
    pub(crate) system: String,
    by_name: HashMap<String, (usize, ToolInfo)>,
    /// Sources that could not list their tools, with why.
    pub(crate) missing: Vec<String>,
    /// Whether `question` is on offer: only when someone can answer.
    pub(crate) questions: bool,
    /// Whether `agent` is on offer.
    pub(crate) subagents: bool,
    /// Whether `update_goal` is on offer: only in a goal's turns.
    pub(crate) goal: bool,
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
            questions: false,
            subagents: false,
            goal: false,
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

    /// Offers `question` too, after every other tool.
    pub(crate) fn offer_question(&mut self) {
        self.questions = true;
        self.specs.push(crate::question::spec());
    }

    /// Whether `name` is `question`, while it is on offer.
    pub(crate) fn is_question(&self, name: &str) -> bool {
        self.questions && name.trim().eq_ignore_ascii_case(crate::question::QUESTION)
    }

    /// Offers `agent` too.
    pub(crate) fn offer_agent(&mut self) {
        self.subagents = true;
        self.specs.push(crate::subagent::spec());
    }

    /// Whether `name` is `agent`, while it is on offer.
    pub(crate) fn is_agent(&self, name: &str) -> bool {
        self.subagents && name.trim().eq_ignore_ascii_case(crate::subagent::AGENT)
    }

    /// Offers memory's tools too: `remember`, `edit` and `forget`. They are first-party, so the
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
    }

    /// Whether `name` is a memory tool on offer.
    pub(crate) fn is_memory(&self, name: &str) -> bool {
        let name = name.trim().to_ascii_lowercase();
        self.memory.contains(&name)
    }

    /// Offers `update_goal` too.
    pub(crate) fn offer_goal(&mut self) {
        self.goal = true;
        self.specs.push(crate::goal::spec());
    }

    /// Whether `name` is `update_goal`, while it is on offer.
    pub(crate) fn is_goal(&self, name: &str) -> bool {
        self.goal && name.trim().eq_ignore_ascii_case(crate::goal::UPDATE_GOAL)
    }

    /// The tools a subagent has: these, less `agent` and `question`, and for an explorer
    /// only those that read.
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
            system: self.system.clone(),
            by_name,
            missing: self.missing.clone(),
            questions: false,
            subagents: false,
            goal: false,
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
