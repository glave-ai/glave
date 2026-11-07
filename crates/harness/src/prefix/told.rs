//! What the model knows of what can change mid-thread, and the note that tells it what changed
//! since: one `<system-reminder>` that opens the turn's message.

use std::collections::{BTreeMap, BTreeSet};

use glave_ports::ToolSpec;
use glave_ports::memories::REMEMBER;

use crate::toolset::{TOOL_SEARCH, connector_called, connector_of, wrapper};

/// What the model is told when it is no longer signed in to any git host.
const SIGNED_OUT: &str = "No account is connected any more: git and the forge tools no longer \
     sign in by themselves.";

/// Opens the line the system prompt says the person's accounts with (`prompt::signed_in`).
const SIGNED_IN: &str = "Accounts the person connected";

/// What the model is told when plan mode ends without a plan built.
const PLAN_OFF: &str = "Plan mode is off: what it said above no longer applies, and you may \
     change things as the person's mode allows.";

/// What the model is told when memory is turned off.
const MEMORY_OFF: &str = "Memory is off now: its tools are refused, and what is said above about \
     saving to it no longer applies.";

/// What the model is told when memory is turned on again, its tools listed.
const MEMORY_ON: &str = "Memory is on again: its tools work.";

/// What the model is told when memory is turned on in a thread that began without it.
const MEMORY_LATER: &str = "Memory was turned on after this conversation began: its tools \
     come once the conversation is compacted, or in a new thread.";

/// What the model knows of what can change mid-thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Told {
    /// The accounts the person connected, as the prompt says them.
    pub(super) signed_in: Option<String>,
    /// Whether the thread is in plan mode.
    pub(super) planning: bool,
    /// Whether the thread reads memory.
    pub(super) memory: bool,
    /// Every tool of a service it was told of, by name: those the prefix lists, then those it
    /// was told came. glave's own, offered by what the thread is, are never among them.
    pub(super) tools: BTreeMap<String, ToolSpec>,
    /// Those of a service someone connected, on offer now, by name.
    pub(super) connected: BTreeSet<String>,
    /// The connectors whose servers could not list their tools: one that answers again is not
    /// a connection turned on, and its tools wait for a new prefix (#1199).
    pub(super) gone: BTreeSet<String>,
}

impl Told {
    /// The tools of services among `specs`, by name.
    pub(super) fn services(specs: &[ToolSpec]) -> BTreeMap<String, ToolSpec> {
        (specs.iter())
            .filter(|spec| !crate::toolset::is_glaves(&spec.name))
            .map(|spec| (spec.name.clone(), spec.clone()))
            .collect()
    }

    /// The accounts `system`, a prefix's prompt, names, as the prompt says them.
    pub(super) fn signed_in_by(system: &str) -> Option<String> {
        let (_, from) = system.split_once(SIGNED_IN)?;
        let line = from.split("\n\n").next().unwrap_or(from);
        Some(format!("{SIGNED_IN}{line}"))
    }

    /// The note that tells a model who knows this what `now` changes, if it changes anything;
    /// `listed` is the prefix's specs.
    pub(super) fn news(&self, now: &Self, listed: &[ToolSpec]) -> Option<String> {
        let mut notes = Vec::new();
        if self.signed_in != now.signed_in {
            notes.push(now.signed_in.as_deref().unwrap_or(SIGNED_OUT).to_owned());
        }
        if self.planning != now.planning {
            let plan_mode = crate::plan::PLAN_MODE.trim_start();
            notes.push((if now.planning { plan_mode } else { PLAN_OFF }).to_owned());
        }
        let lists_memory = listed.iter().any(|spec| spec.name == REMEMBER);
        match (self.memory, now.memory) {
            (true, false) => notes.push(MEMORY_OFF.to_owned()),
            (false, true) if lists_memory => notes.push(MEMORY_ON.to_owned()),
            (false, true) => notes.push(MEMORY_LATER.to_owned()),
            _ => {}
        }
        // A connector's own tool says what it holds, which a search shows anew.
        let real = |name: &str| connector_called(name).is_none() && name != TOOL_SEARCH;
        for (name, spec) in now.tools.iter().filter(|(name, _)| real(name)) {
            let is_listed = || listed.iter().any(|listed| listed.name == *name);
            if self.tools.get(name).is_some_and(|was| was != spec) && is_listed() {
                notes.push(changed(spec));
            }
        }
        let came = self.came(now, listed);
        if !came.usable.is_empty() {
            notes.push(format!(
                "Tools were connected after this conversation began, and your tool list has \
                 them now: {}.",
                came.usable.join(", ")
            ));
        }
        if !came.relisted.is_empty() {
            notes.push(format!(
                "Your tool list now holds connected tools by their connector: {}. Each calls \
                 its connector's tools by name, and {TOOL_SEARCH} finds them.",
                came.relisted.join(", ")
            ));
        }
        if !came.back.is_empty() {
            let back: Vec<String> = (came.back.iter()).map(|name| format!("`{name}`")).collect();
            notes.push(format!(
                "Tool servers that were not available when this conversation began answer \
                 again: {}. Their tools are not available here yet: they will be once the \
                 conversation is compacted, or in a new thread. If the person asks for them, \
                 tell them so.",
                back.join(", ")
            ));
        }
        if !came.waiting.is_empty() {
            let waiting: Vec<String> = (came.waiting.iter())
                .map(|name| format!("`{name}`"))
                .collect();
            notes.push(format!(
                "Tools were connected after this conversation began: {}. They are not \
                 available here yet: they will be once the conversation is compacted, or in a \
                 new thread. If the person asks for them, tell them so.",
                waiting.join(", ")
            ));
        }
        (!notes.is_empty()).then(|| {
            format!(
                "<system-reminder>\n{}\n</system-reminder>",
                notes.join("\n\n")
            )
        })
    }

    /// What `now` offers that the model was never told of, sorted by whether `listed`, a
    /// prefix's tools, reaches it.
    fn came(&self, now: &Self, listed: &[ToolSpec]) -> Came {
        let lists = |name: &str| listed.iter().any(|spec| spec.name == name);
        let searching = lists(TOOL_SEARCH);
        let mut came = Came::default();
        for name in now.tools.keys().filter(|n| !self.tools.contains_key(*n)) {
            let called = connector_called(name);
            let connector =
                called.or_else(|| connector_of(name).filter(|_| now.connected.contains(name)));
            let Some(connector) = connector else {
                if lists(name) {
                    came.usable.push(format!("`{name}`"));
                } else {
                    came.waiting.push(name.clone());
                }
                continue;
            };
            let known = self.knows(connector);
            // A server that answers again keeps the prefix as it is, whose prompt says it is
            // not available.
            if !known && !lists(name) && self.gone.contains(connector) {
                came.back.insert(connector.to_owned());
                continue;
            }
            let through = searching && lists(&wrapper(connector));
            match (called.is_some(), known) {
                (true, true) if lists(name) => came.relisted.push(format!("`{name}`")),
                (true, false) if lists(name) => {
                    let count = (now.connected.iter())
                        .filter(|tool| connector_of(tool) == Some(connector))
                        .count();
                    let line = format!("`{name}` ({count} tools, which {TOOL_SEARCH} finds)");
                    came.usable.push(line);
                }
                // A known connector's tools are listed by their own names; a new tool of one
                // listed as one tool is found by the search.
                (true, true) => {}
                (false, true) if through => {}
                (false, _) if lists(name) => came.usable.push(format!("`{name}`")),
                (_, true) => came.waiting.push(name.clone()),
                (_, false) => {
                    came.waiting.push(name.clone());
                    came.connection = true;
                }
            }
        }
        came
    }

    /// Whether the model was told of `connector`: of its tool, or of any of its tools.
    fn knows(&self, connector: &str) -> bool {
        self.tools.contains_key(&wrapper(connector))
            || (self.tools.keys()).any(|name| connector_of(name) == Some(connector))
    }

    /// Whether a connector the model was never told of came, which `listed`, a prefix's tools,
    /// can't reach: then the thread's prefix is made again, once, so that a connection turned
    /// on works at once.
    pub(super) fn connection_came(&self, now: &Self, listed: &[ToolSpec]) -> bool {
        self.came(now, listed).connection
    }

    /// What the model knows once it has read the news of `now`: never fewer tools than before.
    pub(super) fn and(&self, now: Self) -> Self {
        let mut tools = self.tools.clone();
        tools.extend(now.tools);
        Self { tools, ..now }
    }
}

/// What came since the model was last told, by what it can do with it.
#[derive(Debug, Default)]
struct Came {
    /// What it can call now, as the note names it.
    usable: Vec<String>,
    /// Connectors' tools now listed in place of their tools, which it knew by name.
    relisted: Vec<String>,
    /// What waits for a new prefix, by name.
    waiting: Vec<String>,
    /// The connectors of servers that answer again, whose tools wait for a new prefix.
    back: BTreeSet<String>,
    /// Whether a connector it was never told of is among what waits.
    connection: bool,
}

/// What the model is told of a listed tool whose description or parameters changed.
fn changed(spec: &ToolSpec) -> String {
    format!(
        "The tool `{}` changed after this conversation began. It now reads: {}\nIts parameters: {}",
        spec.name, spec.description, spec.parameters
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accounts_a_prompt_names_are_read_back_as_it_says_them() {
        let line = "Accounts the person connected: git signs in to github.com by itself.";
        let system = format!("You are glave.\n\n{line}\n\nMemory: save what matters.");
        assert_eq!(Told::signed_in_by(&system).as_deref(), Some(line));
        assert_eq!(Told::signed_in_by("You are glave."), None);
    }

    #[test]
    fn glaves_own_tools_are_never_told_of_as_connected() {
        let spec = |name: &str| ToolSpec {
            name: name.into(),
            description: String::new(),
            parameters: serde_json::json!({}),
        };
        let now = Told {
            tools: Told::services(&[spec("question"), spec("update_plan"), spec("mcp__a__b")]),
            ..Told::default()
        };
        let news = Told::default().news(&now, &[]).unwrap();
        assert!(news.contains("`mcp__a__b`"), "{news}");
        assert!(
            !news.contains("question") && !news.contains("update_plan"),
            "{news}"
        );
    }
}
