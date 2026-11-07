//! A thread's prefix: the system prompt and tools its model calls open with, the same bytes
//! from its first call on, so the provider's prompt cache keeps matching and the conversation
//! is never billed again in full (owner, 2026-11-05; `specs/invariants.md`, 27). Only a
//! compaction, which restarts the conversation, another model, which shares none of the cache,
//! and a connection of the person's turned on for the thread's place, whose tools it lacks
//! (owner, 2026-11-06), make a new one, once; a thread read again from the log keeps the one
//! its log recorded.
//! What changes meanwhile reaches the model at the end of the conversation instead, in one note
//! that opens the turn's message, as Claude Code and OpenCode send a `<system-reminder>` and
//! Codex its context updates (`told.rs`). A tool whose source stops answering stays listed.
//! A connection turned on is the one change that makes a new prefix mid-thread, once, so that
//! it works at once (owner, 2026-11-06): the note says so.

mod kept;
mod told;

use glave_event::{Part, Permissions};
use glave_ports::{BlobStore, EventLog, ModelProvider, ToolSpec};

use crate::approval::Approver;
use crate::harness::{Harness, Thread};
use crate::toolset::ToolSet;

pub(crate) use kept::Kept;
pub(crate) use told::Told;

/// Where a thread's prefix stands. Boxed, so a thread held across the turn's many awaits
/// stays small.
#[derive(Debug, Clone, Default)]
pub(crate) enum Prefixed {
    /// None yet, or none since a compaction: its next call makes one.
    #[default]
    None,
    /// As its log kept it, until its next call takes it up.
    Kept(Box<Kept>),
    /// The one its calls open with.
    Made(Box<Prefix>),
}

/// The prefix a thread's model calls open with.
#[derive(Debug, Clone)]
pub(crate) struct Prefix {
    /// The model it was made for.
    model: String,
    /// Its system prompt, as its first call sent it.
    system: String,
    /// Its tools, as its first call listed them.
    specs: Vec<ToolSpec>,
    /// What the model has been told of what can change: by the prefix, then by notes.
    told: Told,
}

impl Prefix {
    /// The tools of a turn whose model calls open with this prefix, called through what is
    /// on offer `now`, with the news of what `told` changes.
    fn behind(&self, now: ToolSet, told: Told) -> ToolSet {
        let mut set = now.listed_as(self.system.clone(), self.specs.clone());
        set.news =
            (self.told.news(&told, &self.specs)).map(|news| Box::new((news, self.told.and(told))));
        set
    }
}

/// The note that opens `offer`'s turn's message, if anything changed since the model was told.
pub(crate) fn news(offer: &ToolSet) -> Option<Part> {
    let (text, _) = offer.news.as_deref()?;
    Some(Part::Text { text: text.clone() })
}

/// Remembers that `thread`'s model was told what `offer`'s news says, once it is recorded.
pub(crate) fn heard(thread: &mut Thread, offer: &ToolSet) {
    if let (Prefixed::Made(prefix), Some((_, told))) = (&mut thread.prefix, offer.news.as_deref()) {
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
    /// The tools of `thread`'s next turn: its prefix, kept or made now from what is on offer
    /// when it has none for its model, and called through what is on offer now.
    pub(crate) async fn tools(&self, thread: &mut Thread) -> ToolSet {
        // Boxed: what every source says can be large, and turns hold this across awaits.
        let (now, told) = Box::pin(self.on_offer(thread)).await;
        let model = self.model_for(thread);
        let mut prefix = match std::mem::take(&mut thread.prefix) {
            Prefixed::Made(prefix) if prefix.model == model => prefix,
            Prefixed::Kept(kept) if kept.model == model => Box::new(kept.prefix()),
            _ => Box::new(Prefix {
                model,
                system: now.system.clone(),
                specs: now.specs.clone(),
                told: told.clone(),
            }),
        };
        if prefix.told.connection_came(&told, &prefix.specs) {
            prefix.system.clone_from(&now.system);
            prefix.specs.clone_from(&now.specs);
        }
        let set = prefix.behind(now, told);
        thread.prefix = Prefixed::Made(prefix);
        set
    }

    /// `given`, a subagent's tools from its parent, behind the prefix the child's log kept,
    /// when it is going on with the same model: a child keeps its own first prefix.
    pub(crate) fn kept_for(&self, thread: &Thread, given: ToolSet) -> ToolSet {
        match &thread.prefix {
            Prefixed::Kept(kept) if kept.model == self.model_for(thread) => kept.tools(given),
            _ => given,
        }
    }

    /// The tools on offer now in `thread`, from every source: the harness's, the person's own
    /// used where it works, then its project's; and what they would tell the model of what can
    /// change.
    async fn on_offer(&self, thread: &Thread) -> (ToolSet, Told) {
        let (own, away) = self.own_for(thread);
        let sources = self.sources_with(thread, &own.sources);
        let mut set = ToolSet::gather(&sources, &self.system_for(thread)).await;
        set.away = away;
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
        set.frame(self.context_window(&self.model_for(thread)));
        let told = Told {
            signed_in,
            planning,
            memory: self.memory_for(thread).is_some(),
            tools: Told::services(&set.specs),
            connected: set.connected_names(),
        };
        (set, told)
    }
}
