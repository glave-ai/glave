//! A save to memory that waits for the person (`specs/memory.md`, "Safety"): after untrusted
//! content it asks and the server stops. The person's yes through another server goes on with
//! the turn, which saves once, asked once; a cancel there decides it, and nothing is saved.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use glave_event::{Body, RawEvent, ThreadId, Verdict, decode};
use glave_memory::{GlaveMemory, Memories, Rules};
use glave_ports::memories::{Memory, REMEMBER};
use glave_ports::testing::{CountingEntropy, ManualClock, ScriptedModel};
use glave_ports::{Completion, Effect, InMemoryTools, ToolInfo, ToolOutput, ToolSource, ToolSpec};
use glave_session::Answering;
use glave_session::protocol::Request;
use glave_store_sqlite::SqliteLog;
use serde_json::json;

use super::{agent, answering, calling, person, until_asked, waiting};
use crate::support::*;

/// A page no one vouches for, as `web_fetch` returns one.
fn page() -> Arc<dyn ToolSource> {
    let info = ToolInfo {
        spec: ToolSpec {
            name: "fetch".into(),
            description: "Fetch a page.".into(),
            parameters: json!({"type": "object"}),
        },
        effect: Effect::Reads,
        parallel: false,
    };
    let page = ToolOutput::untrusted("The tests need make db first.");
    Arc::new(InMemoryTools::new(vec![(info, page)]))
}

fn script() -> Vec<(&'static str, Vec<Completion>)> {
    let save = json!({"text": "The tests need make db first", "scope": "project"});
    vec![(
        "PARENT",
        vec![
            calling("fetch", json!({})),
            calling(REMEMBER, save),
            answer(Some("Saved."), None),
        ],
    )]
}

/// Memory over the SQLite file `database`, as a server's own process opens it.
async fn memory(database: &Path, seed: u64) -> Arc<GlaveMemory<SqliteLog>> {
    let store = sqlite(database).await;
    let memories = Memories::new(
        store.log(),
        store.keyring(),
        Rules::default(),
        Box::new(ManualClock::ticking(1_791_001_000_000)),
        Box::new(CountingEntropy::starting_at(seed << 40)),
    );
    Arc::new(GlaveMemory::start(memories).await.unwrap())
}

/// A server on the SQLite file `database` in `dir`, with memory, in ask mode where clients
/// answer, as the suite's servers are; `seed` keeps its ids apart from another's.
async fn served(dir: &Path, database: &Path, seed: u64) -> Tested<SqliteLog> {
    let store = sqlite(database).await;
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    setup.model = Some(ScriptedModel::keyed(script()));
    setup.tools = vec![page()];
    setup.seed = seed;
    setup.settings.person = Some(person());
    setup.settings.agent = Some(agent());
    setup.memory = Some(memory(database, seed).await as Arc<dyn Memory>);
    setup.projects = Some(Arc::new(store.projects()));
    setup.leases = Some(Arc::new(store.leases()));
    setup.build_on(dir, store.log()).await.0
}

/// A thread whose save waits for the person after untrusted content, on a server stopped
/// since, with the file it is kept in.
async fn waiting_save(dir: &Path, database: &Path) -> ThreadId {
    let first = served(dir, database, 1).await;
    let thread = start(&first, false).await;
    let prompt = "PARENT: remember what the page says";
    let mut stream = stream_turn(&first, thread, prompt, None).await.unwrap();
    until_asked(&mut stream).await;
    first.stop(Duration::ZERO).await;
    drop((stream, first));
    thread
}

/// `thread`'s events and how many memories `database` keeps, once `server` is done.
async fn after(
    server: Tested<SqliteLog>,
    thread: ThreadId,
    database: &Path,
) -> (Vec<RawEvent>, usize) {
    server.stop(Duration::from_secs(3_600)).await;
    let events = events(&server, thread).await;
    let saved = (memory(database, 3).await.memories())
        .list_as(Some(person()), scope(), None)
        .await
        .unwrap();
    (events, saved.len())
}

fn count(events: &[RawEvent], kind: &str) -> usize {
    events
        .iter()
        .filter(|each| each.kind.as_str() == kind)
        .count()
}

fn ended(events: &[RawEvent]) -> bool {
    events
        .last()
        .is_some_and(|event| event.kind.as_str() == "turn.ended")
}

/// #584: going on from the log, the save is settled by the answer it waited for, never asked
/// again.
#[test]
fn a_save_after_untrusted_content_waits_and_saves_once_when_allowed_on_another_server() {
    block_on!(async {
        let dir = folder("durable-memory");
        let database = dir.join("glave.db");
        let thread = waiting_save(&dir, &database).await;
        let second = served(&dir, &database, 2).await;
        let (at, pending) = waiting(&second, thread).await.expect("the save waits");
        second.request(answering(at, pending, false)).await.unwrap();
        let (events, saved) = after(second, thread, &database).await;
        assert_eq!(count(&events, "approval.requested"), 1, "asked once");
        assert!(ended(&events), "the turn went on to its end");
        assert_eq!(saved, 1, "saved once");
    });
}

/// Cancelled on another server while it waits, the save is decided cancelled and never runs.
#[test]
fn a_waiting_save_cancelled_on_another_server_is_decided_and_never_runs() {
    block_on!(async {
        let dir = folder("durable-memory-cancelled");
        let database = dir.join("glave.db");
        let thread = waiting_save(&dir, &database).await;
        let second = served(&dir, &database, 2).await;
        second
            .request(Request::TurnCancel { thread })
            .await
            .unwrap();
        let (events, saved) = after(second, thread, &database).await;
        let decided: Vec<Verdict> = (events.iter())
            .filter_map(|raw| match decode(raw.clone()).ok()?.body {
                Body::ApprovalDecided(decided) => Some(decided.decision),
                _ => None,
            })
            .collect();
        assert_eq!(count(&events, "approval.requested"), 1, "asked once");
        assert_eq!(decided, [Verdict::Cancelled], "decided cancelled");
        assert!(ended(&events), "the turn ended");
        assert_eq!(saved, 0, "nothing saved");
    });
}
