//! A thread's prefix: the system prompt and tools its model calls open with, the same bytes
//! from its first call on, so the provider's prompt cache keeps matching and the conversation
//! is never billed again in full (owner, 2026-11-05; `specs/invariants.md`, 27). Only a
//! compaction, which restarts the conversation, and another model, which shares none of the
//! cache, make a new one. What changes meanwhile reaches the model at the end of the
//! conversation instead, in one note that opens the turn's message, as Claude Code and OpenCode
//! send a `<system-reminder>` and Codex its context updates: accounts connected, plan mode on or
//! off, a listed tool that changed (a skill installed), and tools that came after the prefix,
//! which wait for a compaction or a new thread. A tool whose source stops answering stays listed.

use std::collections::BTreeMap;

use glave_event::{Part, Permissions};
use glave_ports::{BlobStore, EventLog, ModelProvider, ToolSpec};

use crate::approval::Approver;
use crate::harness::{Harness, Thread};
use crate::toolset::ToolSet;

/// What the model is told when it is no longer signed in to any git host.
const SIGNED_OUT: &str = "No account is connected any more: git and the forge tools no longer \
     sign in by themselves.";

/// What the model is told when plan mode ends without a plan built.
const PLAN_OFF: &str = "Plan mode is off: what it said above no longer applies, and you may \
     change things as the person's mode allows.";

/// What the model is told when memory is turned off.
const MEMORY_OFF: &str = "Memory is off now: its tools are refused, and what is said above about \
     saving to it no longer applies.";

/// What the model is told when memory is turned on again, its tools listed.
const MEMORY_ON: &str = "Memory is on again: its tools work.";

/// The prefix a thread's model calls open with.
#[derive(Debug, Clone)]
pub(crate) struct Prefix {
    /// The model it was made for.
    model: String,
    /// Its system prompt and tools, as its first call sent them.
    tools: ToolSet,
    /// What the model has been told of what can change: by the prefix, then by notes.
    told: Told,
}

/// What the model knows of what can change mid-thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Told {
    /// The accounts the person connected, as the prompt says them.
    signed_in: Option<String>,
    /// Whether the thread is in plan mode.
    planning: bool,
    /// Whether the thread reads memory.
    memory: bool,
    /// Every tool it was told of, by name: those the prefix lists, then those it was told came.
    tools: BTreeMap<String, ToolSpec>,
}

impl Told {
    /// The note that tells a model who knows this what `now` changes, if it changes anything.
    fn news(&self, now: &Self, listed: &ToolSet) -> Option<String> {
        let mut notes = Vec::new();
        if self.signed_in != now.signed_in {
            notes.push(now.signed_in.as_deref().unwrap_or(SIGNED_OUT).to_owned());
        }
        if self.planning != now.planning {
            let plan_mode = crate::plan::PLAN_MODE.trim_start();
            notes.push((if now.planning { plan_mode } else { PLAN_OFF }).to_owned());
        }
        if self.memory && !now.memory {
            notes.push(MEMORY_OFF.to_owned());
        } else if !self.memory && now.memory && !listed.memory.is_empty() {
            notes.push(MEMORY_ON.to_owned());
        }
        for (name, spec) in &now.tools {
            let is_listed = || listed.specs.iter().any(|listed| listed.name == *name);
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

    /// What the model knows once it has read the news of `now`: never less than before.
    fn and(&self, now: Self) -> Self {
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

impl Prefix {
    /// The tools of a turn whose model calls open with this prefix, called through what is
    /// on offer `now`, with the news of what `told` changes.
    fn behind(&self, now: ToolSet, told: Told) -> ToolSet {
        let mut set = self.tools.clone();
        set.route(now);
        set.news = (self.told.news(&told, &self.tools)).map(|news| (news, self.told.and(told)));
        set
    }
}

/// The note that opens `offer`'s turn's message, if anything changed since the model was told.
pub(crate) fn news(offer: &ToolSet) -> Option<Part> {
    let (text, _) = offer.news.as_ref()?;
    Some(Part::Text { text: text.clone() })
}

/// Remembers that `thread`'s model was told what `offer`'s news says, once it is recorded.
pub(crate) fn heard(thread: &mut Thread, offer: &ToolSet) {
    if let (Some(prefix), Some((_, told))) = (thread.prefix.as_mut(), offer.news.as_ref()) {
        prefix.told = told.clone();
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The tools of `thread`'s next turn: its prefix, made now from what is on offer when it
    /// has none for its model, and called through what is on offer now.
    pub(crate) async fn tools(&self, thread: &mut Thread) -> ToolSet {
        let (now, told) = self.on_offer(thread).await;
        let model = self.model_for(thread);
        if let Some(prefix) = thread
            .prefix
            .as_ref()
            .filter(|prefix| prefix.model == model)
        {
            return prefix.behind(now, told);
        }
        let tools = now.clone();
        thread.prefix = Some(Prefix { model, tools, told });
        now
    }

    /// The tools on offer now in `thread`, from every source: the harness's, then its
    /// project's; and what they would tell the model of what can change.
    async fn on_offer(&self, thread: &Thread) -> (ToolSet, Told) {
        let sources = self.sources_for(thread);
        let mut set = ToolSet::gather(&sources, &self.system_for(thread)).await;
        let mut signed_in = None;
        if let Some(credentials) = self.workspace_for(thread).credentials() {
            signed_in = crate::prompt::signed_in(&credentials.hosts().await);
        }
        if let Some(note) = &signed_in {
            set.system = format!("{}\n\n{note}", set.system);
        }
        if let Some((memory, _)) = self.memory_for(thread) {
            match memory.tools().await {
                Ok(tools) => set.offer_memory(tools),
                Err(err) => set.goes_without(err),
            }
        }
        if self.approver.can_answer(thread.id) {
            set.offer_question();
        }
        if self.settings.subagents {
            set.offer_agent();
        }
        // Plan mode proposes its plan with it, and a thread with a plan keeps it current, so
        // both have it whatever the settings say.
        let planning = self.permissions_for(thread) == Permissions::Plan;
        if self.settings.plans || planning || thread.plan.plan.is_some() {
            set.offer_plan();
        }
        // In every thread a person writes to, so that a goal starting changes no tools.
        if !thread.delegated() {
            set.offer_goal();
        }
        if planning {
            set.system.push_str(crate::plan::PLAN_MODE);
        }
        let tools = (set.specs.iter())
            .map(|spec| (spec.name.clone(), spec.clone()))
            .collect();
        let told = Told {
            signed_in,
            planning,
            memory: self.memory_for(thread).is_some(),
            tools,
        };
        (set, told)
    }
}
