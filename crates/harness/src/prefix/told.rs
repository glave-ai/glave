//! What the model knows of what can change mid-thread, and the note that tells it what changed
//! since: one `<system-reminder>` that opens the turn's message.

use std::collections::BTreeMap;

use glave_ports::ToolSpec;
use glave_ports::memories::REMEMBER;

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
        for (name, spec) in &now.tools {
            let is_listed = || listed.iter().any(|listed| listed.name == *name);
            if self.tools.get(name).is_some_and(|was| was != spec) && is_listed() {
                notes.push(changed(spec));
            }
        }
        let came: Vec<String> = (now.tools.keys())
            .filter(|name| !self.tools.contains_key(*name))
            .map(|name| format!("`{name}`"))
            .collect();
        if !came.is_empty() {
            notes.push(format!(
                "Tools were connected after this conversation began: {}. They are not \
                 available here yet: they will be once the conversation is compacted, or in a \
                 new thread. If the person asks for them, tell them so.",
                came.join(", ")
            ));
        }
        (!notes.is_empty()).then(|| {
            format!(
                "<system-reminder>\n{}\n</system-reminder>",
                notes.join("\n\n")
            )
        })
    }

    /// What the model knows once it has read the news of `now`: never fewer tools than before.
    pub(super) fn and(&self, now: Self) -> Self {
        let mut tools = self.tools.clone();
        tools.extend(now.tools);
        Self { tools, ..now }
    }
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
