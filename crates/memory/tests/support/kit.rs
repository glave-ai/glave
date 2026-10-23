//! Shared helpers for memory's tests: a sealed SQLite store in memory, memory over it with
//! given rules, scripted answers, and a harness wired to both and the scripted model.

use std::future::{Future, ready};
use std::sync::Arc;

use glave_event::{
    Arguments, Body, ModelRef, Part, PrincipalId, StopReason, ThreadId, TokenCounts, Usage, decode,
};
use glave_harness::{ApprovalRequest, Approver, Decision, Harness, Parts, Settings, Workspace};
use glave_keys::{InMemoryRoot, KeySettings};
use glave_memory::{GlaveMemory, Memories, Personal, Rules};
pub(crate) use glave_ports::block_on;
use glave_ports::memories::ProjectMemory;
use glave_ports::testing::{CountingEntropy, ManualClock, TempDir};
pub(crate) use glave_ports::testing::{ScriptedModel, Seen};
use glave_ports::{Clock, Completion, EventLog, OrgId, ProjectId, RequestedCall, Scope};
use glave_store_sqlite::{SqliteBlobs, SqliteLog, Store};
use jiff::Timestamp;

/// 2026-10-03, in milliseconds.
pub(crate) const START: i64 = 1_791_000_000_000;

pub(crate) fn uuid(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0192_f5c0_0000_7000_8000_0000_0000_0000 | n)
}

pub(crate) fn scope(org: u128, project: u128) -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(uuid(org)),
        ProjectId::from_uuid(uuid(project)),
    )
}

pub(crate) fn person() -> PrincipalId {
    PrincipalId::try_from(uuid(50)).unwrap()
}

pub(crate) fn agent() -> PrincipalId {
    PrincipalId::try_from(uuid(51)).unwrap()
}

/// The person's personal project, where their personal memory lives.
pub(crate) fn inbox() -> Scope {
    scope(1, 100)
}

/// A shared project, an isolated one, and one where memory is off.
pub(crate) fn shared() -> Scope {
    scope(1, 2)
}
pub(crate) fn isolated() -> Scope {
    scope(1, 3)
}
pub(crate) fn off() -> Scope {
    scope(1, 4)
}

pub(crate) fn rules() -> Rules {
    let mut rules = Rules {
        personal: Some(Personal {
            scope: inbox(),
            person: person(),
            on: true,
        }),
        ..Rules::default()
    };
    rules
        .projects
        .insert(isolated().project(), ProjectMemory::Isolated);
    rules.projects.insert(off().project(), ProjectMemory::Off);
    rules
}

/// The kit's clock as the keys read it: what it reads now, without moving it on.
struct Reading(ManualClock);

impl Clock for Reading {
    fn now(&self) -> Timestamp {
        Timestamp::from_millisecond(self.0.millis()).unwrap()
    }
}

/// A sealed store in memory, its clock shared with whatever is wired to it.
pub(crate) struct Kit {
    pub store: Store,
    pub clock: ManualClock,
}

impl Kit {
    pub(crate) fn new() -> Self {
        let clock = ManualClock::ticking(START);
        let mut keys = KeySettings::new(Arc::new(InMemoryRoot::new().unwrap()));
        keys.clock = Arc::new(Reading(clock.clone()));
        Self {
            store: Store::open_in_memory(keys).unwrap(),
            clock,
        }
    }

    pub(crate) fn memories(&self, rules: Rules, seed: u64) -> Memories<SqliteLog> {
        Memories::new(
            self.store.log(),
            self.store.keyring(),
            rules,
            Box::new(self.clock.clone()),
            Box::new(CountingEntropy::starting_at(seed << 32)),
        )
    }

    pub(crate) async fn memory(&self, rules: Rules, seed: u64) -> Arc<GlaveMemory<SqliteLog>> {
        Arc::new(
            GlaveMemory::start(self.memories(rules, seed))
                .await
                .unwrap(),
        )
    }

    /// The output of each tool call's result in `thread` of `scope`, as its log keeps it.
    pub(crate) async fn results(&self, scope: Scope, thread: ThreadId) -> Vec<String> {
        let events = (self.store.log())
            .read(&scope, thread, None, usize::MAX)
            .await
            .unwrap();
        events
            .into_iter()
            .filter_map(|raw| match decode(raw).ok()?.body {
                Body::ToolCallCompleted(done) => Some(format!("{:?}", done.output)),
                _ => None,
            })
            .collect()
    }

    /// A harness on this store, with `memory` when given, and what the model is sent.
    pub(crate) async fn harness<A: Approver>(
        &self,
        dir: &std::path::Path,
        answers: Vec<Completion>,
        approver: A,
        memory: Option<Arc<GlaveMemory<SqliteLog>>>,
        settings: Settings,
    ) -> (Harness<ScriptedModel, SqliteLog, SqliteBlobs, A>, Seen) {
        let (computers, info) = glave_computer::LocalComputers::here(dir, std::env::vars_os())
            .await
            .unwrap();
        let provider = ScriptedModel::new(answers);
        let seen = provider.seen();
        let parts = Parts {
            provider,
            log: self.store.log(),
            blobs: self.store.blobs(),
            approver,
            clock: Box::new(self.clock.clone()),
            entropy: Box::new(CountingEntropy::starting_at(7 << 40)),
            workspace: Workspace::new(computers, info),
        };
        let mut harness = Harness::new(parts, settings);
        if let Some(memory) = memory {
            harness = harness.with_memory(memory);
        }
        (harness, seen)
    }
}

pub(crate) fn settings() -> Settings {
    let mut settings = Settings::new("test-model", "script");
    settings.person = Some(person());
    settings.agent = Some(agent());
    settings.instructions = Some("# AGENTS.md\n\nUse tabs.".into());
    settings
}

pub(crate) fn answer(text: Option<&str>, calls: Vec<(&str, serde_json::Value)>) -> Completion {
    let counts = TokenCounts {
        input: 100,
        cache_read: 0,
        cache_write: 0,
        output: 10,
        reasoning: 0,
    };
    let stop = if calls.is_empty() {
        StopReason::EndTurn
    } else {
        StopReason::ToolUse
    };
    Completion {
        model: ModelRef {
            provider: "script".into(),
            name: "test-model".into(),
        },
        response_id: None,
        parts: text
            .map(|t| Part::Text { text: t.into() })
            .into_iter()
            .collect(),
        tool_calls: calls
            .into_iter()
            .map(|(tool, args)| RequestedCall {
                provider_call_id: None,
                tool: tool.into(),
                arguments: Arguments::Json(args.as_object().unwrap().clone()),
            })
            .collect(),
        stop,
        provider_stop: None,
        usage: Usage::new(counts, false, None).unwrap(),
        cost: None,
        served: None,
        upstream: None,
    }
}

pub(crate) fn said(text: &str) -> Completion {
    answer(Some(text), Vec::new())
}

pub(crate) fn folder(name: &str) -> TempDir {
    TempDir::new(&format!("memory-{name}")).unwrap()
}

/// Every text part of a user message, in order.
pub(crate) fn user_texts(message: &glave_ports::Message) -> Vec<String> {
    match message {
        glave_ports::Message::User { parts } => parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Says yes to everything it is asked.
pub(crate) struct Yes;
impl Approver for Yes {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::AllowOnce)
    }
}
