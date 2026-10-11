//! Shared helpers for memory's tests: a sealed SQLite store in memory, memory over it with
//! given rules, a scripted model, and a harness wired to both.

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use glave_event::{Arguments, ModelRef, Part, PrincipalId, StopReason, TokenCounts, Usage};
use glave_harness::{
    ApprovalRequest, Approver, Clock, Decision, Entropy, Harness, Parts, Settings, Workspace,
};
use glave_keys::{KeySettings, MemoryRoot};
use glave_memory::{GlaveMemory, Memories, Personal, Rules, SpaceMemory};
use glave_ports::{
    Completion, Delta, ModelProvider, ModelRequest, OrgId, ProviderError, RequestedCall, Scope,
    SpaceId,
};
use glave_store_sqlite::{SqliteBlobs, SqliteLog, Store};
use jiff::Timestamp;

pub struct Ticking(pub Arc<AtomicI64>);
impl Clock for Ticking {
    fn now(&self) -> Timestamp {
        let ms = self.0.fetch_add(1, Ordering::SeqCst);
        Timestamp::from_millisecond(ms).unwrap()
    }
}

pub struct Counting(pub AtomicU64);
impl Entropy for Counting {
    fn bytes(&self) -> [u8; 10] {
        let n = self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes();
        [n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7], 0, 0]
    }
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// 2026-10-03, in milliseconds.
pub const START: i64 = 1_791_000_000_000;

pub fn uuid(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0192_f5c0_0000_7000_8000_0000_0000_0000 | n)
}

pub fn scope(org: u128, space: u128) -> Scope {
    Scope {
        org: OrgId::from_uuid(uuid(org)),
        space: SpaceId::from_uuid(uuid(space)),
    }
}

pub fn person() -> PrincipalId {
    PrincipalId::try_from(uuid(50)).unwrap()
}

pub fn agent() -> PrincipalId {
    PrincipalId::try_from(uuid(51)).unwrap()
}

/// The person's Inbox, where their personal memory lives.
pub fn inbox() -> Scope {
    scope(1, 100)
}

/// A shared project space, an isolated one, and one where memory is off.
pub fn shared() -> Scope {
    scope(1, 2)
}
pub fn isolated() -> Scope {
    scope(1, 3)
}
pub fn off() -> Scope {
    scope(1, 4)
}

pub fn rules() -> Rules {
    let mut rules = Rules {
        personal: Some(Personal {
            scope: inbox(),
            person: person(),
        }),
        ..Rules::default()
    };
    rules.spaces.insert(isolated().space, SpaceMemory::Isolated);
    rules.spaces.insert(off().space, SpaceMemory::Off);
    rules
}

/// A sealed store in memory, its clock shared with whatever is wired to it.
pub struct Kit {
    pub store: Store,
    pub clock: Arc<AtomicI64>,
}

impl Kit {
    pub fn new() -> Self {
        let clock = Arc::new(AtomicI64::new(START));
        let mut keys = KeySettings::new(Arc::new(MemoryRoot::new().unwrap()));
        let ticking = Arc::clone(&clock);
        keys.clock =
            Arc::new(move || Timestamp::from_millisecond(ticking.load(Ordering::SeqCst)).unwrap());
        Self {
            store: Store::open_in_memory(keys).unwrap(),
            clock,
        }
    }

    pub fn memories(&self, rules: Rules, seed: u64) -> Memories<SqliteLog> {
        Memories::new(
            self.store.log(),
            self.store.keyring(),
            rules,
            Box::new(Ticking(Arc::clone(&self.clock))),
            Box::new(Counting(AtomicU64::new(seed << 32))),
        )
    }

    pub async fn memory(&self, rules: Rules, seed: u64) -> Arc<GlaveMemory<SqliteLog>> {
        Arc::new(
            GlaveMemory::start(self.memories(rules, seed))
                .await
                .unwrap(),
        )
    }

    /// A harness on this store, with `memory` when given, and what the model is sent.
    pub async fn harness<A: Approver>(
        &self,
        dir: &std::path::Path,
        answers: Vec<Completion>,
        approver: A,
        memory: Option<Arc<GlaveMemory<SqliteLog>>>,
        settings: Settings,
    ) -> (Harness<Script, SqliteLog, SqliteBlobs, A>, Seen) {
        let (computers, info) = glave_computer::LocalComputers::here(dir, std::env::vars_os())
            .await
            .unwrap();
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let parts = Parts {
            provider: Script {
                answers: Mutex::new(answers.into()),
                seen: Arc::clone(&seen),
            },
            log: self.store.log(),
            blobs: self.store.blobs(),
            approver,
            clock: Box::new(Ticking(Arc::clone(&self.clock))),
            entropy: Box::new(Counting(AtomicU64::new(7 << 40))),
            workspace: Workspace::new(computers, info),
        };
        let mut harness = Harness::new(parts, settings);
        if let Some(memory) = memory {
            harness = harness.with_memory(memory);
        }
        (harness, seen)
    }
}

pub fn settings() -> Settings {
    let mut settings = Settings::new("test-model", "script");
    settings.person = Some(person());
    settings.agent = Some(agent());
    settings.instructions = Some("# AGENTS.md\n\nUse tabs.".into());
    settings
}

pub type Seen = Arc<Mutex<Vec<ModelRequest>>>;

pub struct Script {
    pub answers: Mutex<VecDeque<Completion>>,
    pub seen: Seen,
}

impl ModelProvider for Script {
    fn complete(
        &self,
        request: &ModelRequest,
        _on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        self.seen.lock().unwrap().push(request.clone());
        let next = self.answers.lock().unwrap().pop_front();
        ready(next.ok_or_else(|| ProviderError::Protocol("script exhausted".into())))
    }
}

pub fn answer(text: Option<&str>, calls: Vec<(&str, serde_json::Value)>) -> Completion {
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
    }
}

pub fn said(text: &str) -> Completion {
    answer(Some(text), Vec::new())
}

pub fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-memory-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every text part of a user message, in order.
pub fn user_texts(message: &glave_ports::Message) -> Vec<String> {
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
pub struct Yes;
impl Approver for Yes {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::AllowOnce)
    }
}
