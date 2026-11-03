//! The history holds words, never files (`specs/invariants.md`, 26): a real server's bytes,
//! base64 and floods of text go through a real turn, and no event, nor the model, holds them.

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::path::Path;
use std::sync::Arc;

use glave_event::{
    Arguments, BlobRef, ModelRef, Part, RawEvent, Sha256, StopReason, ThreadId, TokenCounts, Usage,
};
use glave_harness::{AllowAll, Cancel, Harness, Parts, Permissions, Settings, Thread, Workspace};
use glave_mcp::{Limits, McpServer};
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::testing::{CountingEntropy, ManualClock, ScriptedModel, Seen, TempDir};
use glave_ports::{
    Blob, BlobError, BlobStore, Chunks, Completion, EventLog as _, InMemoryBlobs, InMemoryLog,
    Message, OrgId, ProjectId, RequestedCall, Scope, Swept, ToolSource, block_on, read_all,
};
use serde_json::{Value, json};

const SERVER: &str = env!("CARGO_BIN_EXE_glave-mcp-test-server");
const MOST_RUN: usize = 8 * 1024;
const MOST_TEXT: usize = 64 * 1024;
const KINDS: [&str; 6] = [
    "image",
    "audio",
    "resource",
    "base64",
    "structured",
    "flood",
];

fn scope() -> Scope {
    let id = |n: u128| uuid::Uuid::from_u128(n);
    Scope::glaves_own(OrgId::from_uuid(id(1)), ProjectId::from_uuid(id(2)))
}

/// The model's answer: a call of the server's `blocks` for `what`, or, with none, done.
fn step(what: Option<&str>) -> Completion {
    let counts = TokenCounts {
        input: 100,
        cache_read: 0,
        cache_write: 0,
        output: 10,
        reasoning: 0,
    };
    let call = what.map(|what| RequestedCall {
        provider_call_id: Some(format!("p_{what}")),
        tool: "mcp__tests__blocks".into(),
        arguments: Arguments::Json(
            json!({ "what": what })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        ),
    });
    Completion {
        model: ModelRef {
            provider: "script".into(),
            name: "test-model".into(),
        },
        response_id: None,
        parts: Vec::new(),
        stop: if call.is_some() {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        },
        tool_calls: call.into_iter().collect(),
        provider_stop: None,
        usage: Usage::new(counts, false, None).unwrap(),
        cost: None,
        served: None,
        upstream: None,
    }
}

/// The turn calling for each of [`KINDS`], then done, on `blobs`: its events and what the model
/// was sent.
async fn run_turn<B: BlobStore>(
    dir: &Path,
    blobs: B,
) -> (
    Harness<ScriptedModel, InMemoryLog, B, AllowAll>,
    Thread,
    Seen,
) {
    let (computers, info) = glave_computer::LocalComputers::here(dir, std::env::vars_os())
        .await
        .unwrap();
    let mut answers: Vec<Completion> = KINDS.iter().map(|kind| step(Some(kind))).collect();
    answers.push(step(None));
    let provider = ScriptedModel::new(answers);
    let seen = provider.seen();
    let mut settings = Settings::new("test-model", "script");
    settings.permissions = Permissions::FullAccess;
    let parts = Parts {
        provider,
        log: InMemoryLog::new(),
        blobs,
        approver: AllowAll,
        clock: Box::new(ManualClock::ticking(1_791_000_000_000)),
        entropy: Box::new(CountingEntropy::starting_at(1)),
        workspace: Workspace::new(computers, info),
    };
    let config = ServerConfig::Stdio {
        command: SERVER.to_owned(),
        args: Vec::new(),
        env: BTreeMap::default(),
    };
    let server = McpServer::start(
        "tests",
        &config,
        &BTreeMap::default(),
        dir,
        Limits::default(),
        &glave_process::Processes::unrecorded(),
    );
    server.started().await.unwrap();
    let server: Arc<dyn ToolSource> = Arc::new(server);
    let harness = Harness::new(parts, settings).with_tools(vec![server]);
    let mut thread = harness.start_thread(scope()).await.unwrap();
    harness
        .run_turn(&mut thread, "fetch them all", &Cancel::new(), &mut |_| {})
        .await
        .unwrap();
    (harness, thread, seen)
}

/// The longest stretch of base64's letters in `text`, by the plainest reading there is: lines
/// of it are counted together.
fn longest_run(text: &str) -> usize {
    let (mut longest, mut run) = (0, 0);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'-' | b'_') {
            run += 1;
            longest = longest.max(run);
        } else if byte != b'\n' && byte != b'\\' {
            run = 0;
        }
    }
    longest
}

/// Fails for a string in `value` the history must not hold.
fn check(value: &Value, seen: &str) {
    match value {
        Value::String(text) => {
            assert!(
                text.len() <= MOST_TEXT,
                "{seen}: {} bytes of text",
                text.len()
            );
            assert!(
                longest_run(text) <= MOST_RUN,
                "{seen}: {} bytes of base64",
                longest_run(text)
            );
        }
        Value::Array(items) => items.iter().for_each(|item| check(item, seen)),
        Value::Object(fields) => fields.values().for_each(|field| check(field, seen)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Every event `thread` holds.
async fn events(log: &InMemoryLog, thread: ThreadId) -> Vec<RawEvent> {
    log.read(&scope(), thread, None, usize::MAX).await.unwrap()
}

/// Each result the model was sent in its last request, as text.
fn results(seen: &Seen) -> Vec<String> {
    let requests = seen.lock().unwrap();
    let last = requests.last().unwrap();
    last.messages
        .iter()
        .filter_map(|message| match message {
            Message::Tool { output, .. } => Some(
                output
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect()
}

/// The blob the line ending `text` names.
fn named(text: &str) -> Sha256 {
    let from = text.rfind("blob:").unwrap().saturating_add("blob:".len());
    Sha256::new(text.get(from..from.saturating_add(64)).unwrap()).unwrap()
}

/// What the line ending `said` names: its media type and bytes.
async fn kept<B: BlobStore>(blobs: &B, said: &str) -> (String, Vec<u8>) {
    let found = blobs.get(&scope(), &named(said)).await.unwrap().unwrap();
    let Blob {
        reference,
        mut bytes,
    } = found;
    let bytes = read_all(bytes.as_mut(), u64::MAX).await.unwrap();
    (reference.media_type, bytes)
}

#[test]
fn no_history_event_holds_base64_over_8_kb_or_text_over_64_kb() {
    block_on!(async {
        let dir = TempDir::new("history").unwrap();
        let (harness, thread, seen) = run_turn(dir.path(), InMemoryBlobs::new()).await;

        for event in events(harness.log(), thread.id).await {
            let data: Value = serde_json::from_str(event.data.get()).unwrap();
            check(&data, event.kind.as_str());
        }
        let results = results(&seen);
        assert_eq!(results.len(), KINDS.len());
        for (kind, said) in KINDS.iter().zip(&results) {
            check(&Value::String(said.clone()), kind);
            assert!(said.contains("saved as blob:"), "{kind}: {said:.200}");
        }

        let blobs = harness.blobs();
        let told = |at: usize| results.get(at).cloned().unwrap();
        // The server called the image text/plain; its first bytes say otherwise.
        assert!(told(0).starts_with("Here is the chart.\n[image, "));
        let (image, png) = kept(blobs, &told(0)).await;
        assert_eq!(image, "image/png");
        assert!(png.starts_with(b"\x89PNG") && png.len() == 20_008);
        assert_eq!(kept(blobs, &told(1)).await.0, "audio/wav");
        assert_eq!(
            kept(blobs, &told(2)).await.1,
            png,
            "the same bytes, kept once"
        );
        assert!(told(3).starts_with("The file: [image, "));
        assert!(told(3).ends_with("] and that is all."), "{}", told(3));
        assert_eq!(kept(blobs, &told(3)).await.1, png);
        assert_eq!(
            kept(blobs, &told(4)).await.1,
            png,
            "escaped in structured JSON"
        );
        // The whole flood is kept, the model shown its start and the line.
        assert!(told(5).starts_with("All work and no play."));
        assert!(told(5).len() < 9 * 1024);
        let (text, flood) = kept(blobs, &told(5)).await;
        assert_eq!(text, "text/plain");
        assert_eq!(flood.len(), "All work and no play. ".len() * 4_000);
    });
}

/// A store with no room for what a tool gives: a turn's own configuration is all it keeps.
#[derive(Default)]
struct Refusing(InMemoryBlobs);

impl BlobStore for Refusing {
    async fn put(
        &self,
        scope: &Scope,
        thread: Option<ThreadId>,
        media_type: &str,
        limit: u64,
        bytes: Box<dyn Chunks>,
    ) -> Result<BlobRef, BlobError> {
        if media_type == "application/json" {
            self.0.put(scope, thread, media_type, limit, bytes).await
        } else {
            Err(BlobError::Storage("the disk is full".into()))
        }
    }

    fn get(
        &self,
        _scope: &Scope,
        _sha256: &Sha256,
    ) -> impl Future<Output = Result<Option<Blob>, BlobError>> + Send {
        ready(Ok(None))
    }

    fn refer(
        &self,
        _scope: &Scope,
        _sha256: &Sha256,
        _thread: ThreadId,
    ) -> impl Future<Output = Result<bool, BlobError>> + Send {
        ready(Ok(false))
    }

    fn release(
        &self,
        _scope: &Scope,
        _thread: ThreadId,
    ) -> impl Future<Output = Result<(), BlobError>> + Send {
        ready(Ok(()))
    }

    fn sweep(&self, _org: OrgId) -> impl Future<Output = Result<Swept, BlobError>> + Send {
        ready(Ok(Swept::default()))
    }
}

#[test]
fn what_cannot_be_kept_is_said_so_and_stays_out() {
    block_on!(async {
        let dir = TempDir::new("history-refused").unwrap();
        let (harness, thread, seen) = run_turn(dir.path(), Refusing::default()).await;
        for event in events(harness.log(), thread.id).await {
            let data: Value = serde_json::from_str(event.data.get()).unwrap();
            check(&data, event.kind.as_str());
        }
        for said in results(&seen) {
            assert!(said.contains("could not be kept: "), "{said:.200}");
        }
    });
}
