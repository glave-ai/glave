//! Shared helpers for the session tests: a scripted model, a ticking clock, counting
//! entropy, and a server wired to them over a real temporary folder.

use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use glave_event::{Arguments, ModelRef, Part, StopReason, TokenCounts, Usage};
use glave_harness::{Clock, Entropy, Settings, Workspace};
use glave_ports::{
    Completion, Delta, EventLog, MemoryBlobs, MemoryLog, ModelProvider, ModelRequest, OrgId,
    ProviderError, RequestedCall, Scope, SpaceId,
};
use glave_session::{Answering, Server, Wiring};
use jiff::Timestamp;
use tokio::sync::Notify;

/// A model that answers from a script, optionally waiting at a gate before each answer.
pub struct Script {
    answers: Mutex<VecDeque<Completion>>,
    gate: Option<Arc<Notify>>,
}

impl ModelProvider for Script {
    fn complete(
        &self,
        _request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        let next = self.answers.lock().unwrap().pop_front();
        if let Some(Part::Text { text }) = next.as_ref().and_then(|c| c.parts.first()) {
            on_delta(Delta::Text(text.clone()));
        }
        let gate = self.gate.clone();
        async move {
            if let Some(gate) = gate {
                gate.notified().await;
            }
            next.ok_or_else(|| ProviderError::Protocol("script exhausted".into()))
        }
    }
}

pub struct Ticking(AtomicI64);
impl Clock for Ticking {
    fn now(&self) -> Timestamp {
        Timestamp::from_millisecond(self.0.fetch_add(1, Ordering::SeqCst)).unwrap()
    }
}

pub struct Counting(AtomicU64);
impl Entropy for Counting {
    fn bytes(&self) -> [u8; 10] {
        let n = self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes();
        [n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7], 0, 0]
    }
}

pub fn answer(text: Option<&str>, call: Option<(&str, serde_json::Value)>) -> Completion {
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
        tool_calls: call
            .map(|(tool, args)| RequestedCall {
                provider_call_id: None,
                tool: tool.into(),
                arguments: Arguments::Json(args.as_object().unwrap().clone()),
            })
            .into_iter()
            .collect(),
        stop: if text.is_some() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        },
        provider_stop: None,
        usage: Usage::new(TokenCounts::default(), false, None).unwrap(),
        cost: None,
    }
}

pub fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn scope() -> Scope {
    Scope {
        org: OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        space: SpaceId::from_uuid(uuid::Uuid::from_u128(2)),
    }
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

pub async fn server_on<L: EventLog + 'static>(
    dir: &PathBuf,
    answers: Vec<Completion>,
    gate: Option<Arc<Notify>>,
    log: L,
    answering: Answering,
    seed: u64,
) -> Server<Script, L, MemoryBlobs> {
    let wiring = Wiring {
        provider: Script {
            answers: Mutex::new(answers.into()),
            gate,
        },
        log,
        blobs: MemoryBlobs::new(),
        clock: Box::new(Ticking(AtomicI64::new(
            1_791_000_000_000 + 1_000_000 * i64::try_from(seed).unwrap(),
        ))),
        entropy: Box::new(Counting(AtomicU64::new(seed << 32))),
        workspace: Workspace::open(dir, std::env::vars_os()).await.unwrap(),
    };
    Server::new(
        wiring,
        Settings::new("test-model", "script"),
        scope(),
        answering,
    )
}

pub async fn server(
    dir: &PathBuf,
    answers: Vec<Completion>,
    answering: Answering,
) -> Server<Script, MemoryLog, MemoryBlobs> {
    server_on(dir, answers, None, MemoryLog::new(), answering, 1).await
}
