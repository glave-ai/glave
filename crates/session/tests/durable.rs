//! Durable waits (`specs/one-server.md`, "Durable turns"), on every store: a server stopped
//! while a turn waits for the person leaves it waiting; another on the same store lists it
//! waiting, Needs you included, and the answer goes on with the turn to its end, its events
//! those of a run that was never stopped but for ids and times. The same for a question, a
//! denial and a subagent's approval; and with two subagents side by side, one waiting and one
//! running when the server stops, the running one is reported interrupted and never started
//! again. `crashes.rs` stops a turn at every event boundary.

mod crashes;

use std::future::ready;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use glave_event::{Body, CallId, PrincipalId, RawEvent, ThreadId, decode};
use glave_keys::{InMemoryRoot, KeySettings};
use glave_ports::testing::{ScriptedModel, TempDir};
use glave_ports::{
    BoxFuture, Completion, EventLog, InMemoryLeases, InMemoryObjects, InMemoryProjects, Leases,
    Projects, ToolSource,
};
use glave_session::protocol::{Answer, Notification, Reply, Request, Status, Update};
use glave_session::{Answering, Repaired, TurnItem, TurnStream};
use jiff::Timestamp;
use serde_json::json;

use crate::support::*;

/// A store as a server opens it: its log, projects and leases.
pub(crate) type Opened<L> = (L, Arc<dyn Projects>, Arc<dyn Leases>);

/// A store servers open, each as a process of its own would.
pub(crate) trait Store {
    type Log: EventLog + 'static;
    fn open(&self) -> BoxFuture<'_, Opened<Self::Log>>;
}

/// Memory every server shares.
pub(crate) struct Memory {
    pub(crate) log: Shared,
    pub(crate) projects: Arc<InMemoryProjects>,
    pub(crate) leases: Arc<InMemoryLeases>,
}

impl Memory {
    pub(crate) fn new() -> Self {
        let (_, leases) = shared_leases();
        Self {
            log: Shared::default(),
            projects: Arc::new(InMemoryProjects::new()),
            leases,
        }
    }
}

impl Store for Memory {
    type Log = Shared;
    fn open(&self) -> BoxFuture<'_, Opened<Shared>> {
        let projects: Arc<dyn Projects> = Arc::clone(&self.projects) as Arc<dyn Projects>;
        let leases: Arc<dyn Leases> = Arc::clone(&self.leases) as Arc<dyn Leases>;
        Box::pin(ready((self.log.clone(), projects, leases)))
    }
}

/// One SQLite file each server opens.
struct Sqlite {
    database: PathBuf,
    _dir: TempDir,
}

impl Store for Sqlite {
    type Log = glave_store_sqlite::SqliteLog;
    fn open(&self) -> BoxFuture<'_, Opened<Self::Log>> {
        Box::pin(async move {
            let store = sqlite(&self.database).await;
            let projects: Arc<dyn Projects> = Arc::new(store.projects());
            let leases: Arc<dyn Leases> = Arc::new(store.leases());
            (store.log(), projects, leases)
        })
    }
}

/// One Postgres schema each server connects to, under one root key.
struct Postgres {
    config: glave_store_postgres::Config,
    root: Arc<InMemoryRoot>,
    objects: InMemoryObjects,
}

impl Store for Postgres {
    type Log = glave_store_postgres::PostgresLog;
    fn open(&self) -> BoxFuture<'_, Opened<Self::Log>> {
        Box::pin(async move {
            let keys = KeySettings::new(Arc::<InMemoryRoot>::clone(&self.root));
            let objects = Arc::new(self.objects.clone());
            let store =
                glave_store_postgres::Store::connect_with(self.config.clone(), keys, objects)
                    .await
                    .unwrap();
            let projects: Arc<dyn Projects> = Arc::new(store.projects());
            let leases: Arc<dyn Leases> = Arc::new(store.leases());
            (store.log(), projects, leases)
        })
    }
}

/// The person every server works for, and their agent, so each records the same.
fn person() -> PrincipalId {
    PrincipalId::new(Timestamp::from_second(1_791_000_000).unwrap(), [1; 10]).unwrap()
}

fn agent() -> PrincipalId {
    PrincipalId::new(Timestamp::from_second(1_791_000_000).unwrap(), [2; 10]).unwrap()
}

/// A server on `store` in `dir`, asking `model`, offering `tools`, where clients answer, ask
/// mode on, subagents offered; `seed` keeps its ids apart from another process's.
pub(crate) async fn server_on<S: Store>(
    dir: &Path,
    store: &S,
    model: ScriptedModel,
    tools: Vec<Arc<dyn ToolSource>>,
    seed: u64,
) -> Tested<S::Log> {
    let (log, projects, leases) = store.open().await;
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    setup.model = Some(model);
    setup.tools = tools;
    setup.seed = seed;
    setup.settings.person = Some(person());
    setup.settings.agent = Some(agent());
    setup.settings.subagents = true;
    setup.projects = Some(projects);
    setup.leases = Some(leases);
    setup.build_on(dir, log).await.0
}

/// A call of `tool` with `arguments`, as the model makes it.
pub(crate) fn calling(tool: &str, arguments: serde_json::Value) -> Completion {
    answer(None, Some((tool, arguments)))
}

/// Several calls in one answer.
pub(crate) fn calling_all(calls: Vec<Completion>) -> Completion {
    let mut all = calls.into_iter();
    let mut first = all.next().unwrap();
    for (n, mut call) in all.enumerate() {
        for requested in &mut call.tool_calls {
            requested.provider_call_id = Some(format!("p_{n}"));
        }
        first.tool_calls.extend(call.tool_calls);
    }
    first
}

/// A subagent's call: its brief names the script it is answered from.
pub(crate) fn subagent(word: &str, task: &str) -> Completion {
    calling(
        "agent",
        json!({"description": task, "prompt": format!("{word}: {task}")}),
    )
}

/// What waits in `thread`, by its events: an approval asked and not decided, or questions not
/// answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Pending {
    Approval(CallId),
    Question(CallId),
}

/// What waits in `events`, if anything does.
fn pending_in(events: &[RawEvent]) -> Option<Pending> {
    let mut asked: Vec<CallId> = Vec::new();
    let mut questions: Vec<CallId> = Vec::new();
    for raw in events {
        match decode(raw.clone()).unwrap().body {
            Body::ApprovalRequested(request) => asked.push(request.call_id),
            Body::ApprovalDecided(decided) => asked.retain(|call| *call != decided.call_id),
            Body::ToolCallRequested(call) if call.tool == "question" => {
                questions.push(call.call_id);
            }
            Body::ToolCallCompleted(done) => questions.retain(|call| *call != done.call_id),
            Body::TurnEnded(_) => {
                asked.clear();
                questions.clear();
            }
            _ => {}
        }
    }
    asked
        .pop()
        .map(Pending::Approval)
        .or_else(|| questions.pop().map(Pending::Question))
}

/// Every thread `root` started, by the parent its summary names.
pub(crate) async fn children_of<L: EventLog + 'static>(
    server: &Tested<L>,
    root: ThreadId,
) -> Vec<ThreadId> {
    let listed = server
        .request(Request::ThreadList {
            project: Some(scope().project()),
            limit: Some(100),
            show: None,
            cursor: None,
        })
        .await
        .unwrap();
    let Reply::Threads { threads, .. } = listed else {
        panic!("threads, not {listed:?}");
    };
    let mut children: Vec<_> = threads
        .into_iter()
        .filter(|thread| thread.parent == Some(root))
        .map(|thread| thread.id)
        .collect();
    children.sort();
    children
}

/// What waits in `root` or one of its subagents, and where.
pub(crate) async fn waiting<L: EventLog + 'static>(
    server: &Tested<L>,
    root: ThreadId,
) -> Option<(ThreadId, Pending)> {
    let mut threads = vec![root];
    threads.extend(children_of(server, root).await);
    for thread in threads {
        if let Some(pending) = pending_in(&events(server, thread).await) {
            return Some((thread, pending));
        }
    }
    None
}

/// The answer a test gives what waits: `deny` for an approval, or yes to it.
pub(crate) fn answering(thread: ThreadId, pending: Pending, deny: bool) -> Request {
    match pending {
        Pending::Approval(approval) => Request::ApprovalDecide {
            thread,
            approval,
            decision: if deny {
                Answer::Deny
            } else {
                Answer::AllowOnce
            },
        },
        Pending::Question(question) => Request::QuestionAnswer {
            thread,
            question,
            answers: vec!["Postgres".into()],
        },
    }
}

/// Follows `stream` until a wait is announced.
async fn until_asked(stream: &mut TurnStream) {
    while let Some(item) = stream.next().await {
        if let TurnItem::Note(note) = item
            && let Notification::Live { update, .. } = note.as_ref()
            && matches!(
                update,
                Update::ApprovalPending { .. } | Update::Asked { .. }
            )
        {
            return;
        }
    }
    panic!("nothing waited");
}

/// The events of `threads` as runs compare them: each event's type and payload, but for ids
/// (a thread's, a workspace's), which each run makes anew.
async fn compared<L: EventLog + 'static>(
    server: &Tested<L>,
    threads: &[ThreadId],
) -> Vec<(String, serde_json::Value)> {
    let mut seen = Vec::new();
    for thread in threads {
        for event in events(server, *thread).await {
            let mut data: serde_json::Value = serde_json::from_str(event.data.get()).unwrap();
            but_for_ids(&mut data);
            seen.push((event.kind.as_str().to_owned(), data));
        }
    }
    seen
}

/// `value` with every id in it, a string that reads as a UUID, the same placeholder; the
/// person's and the agent's are the same in every run, and kept.
fn but_for_ids(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            let kept = [person().to_string(), agent().to_string()];
            if uuid::Uuid::parse_str(text).is_ok() && !kept.contains(text) {
                *text = "id".into();
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(but_for_ids),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(but_for_ids),
        _ => {}
    }
}

/// One run of `script` saying `prompt` in `dir` on `store`: the turn waits, the answer `deny`
/// says is given (by a second server, after the first is stopped, when `restart`), and the
/// turn goes on to its end. Its events as runs compare them, and the last server.
async fn run<S: Store>(
    dir: &Path,
    store: &S,
    script: impl Fn() -> Vec<(&'static str, Vec<Completion>)>,
    prompt: &str,
    restart: bool,
    deny: bool,
) -> (Vec<(String, serde_json::Value)>, Tested<S::Log>) {
    let first = server_on(dir, store, ScriptedModel::keyed(script()), Vec::new(), 1).await;
    let thread = start(&first, false).await;
    let mut stream = stream_turn(&first, thread, prompt, None).await.unwrap();
    until_asked(&mut stream).await;
    let server = if restart {
        first.stop(Duration::ZERO).await;
        drop((stream, first));
        let second = server_on(dir, store, ScriptedModel::keyed(script()), Vec::new(), 2).await;
        assert_eq!(
            second.repair_left_open().await.unwrap(),
            Repaired::default(),
            "a waiting turn is not closed as the server starts"
        );
        needs_you(&second, thread).await;
        second
    } else {
        first
    };
    let (at, pending) = waiting(&server, thread).await.expect("it waits");
    server.request(answering(at, pending, deny)).await.unwrap();
    // A stop waits for the work going on with the turn to end.
    server.stop(Duration::from_secs(3_600)).await;
    let mut threads = vec![thread];
    threads.extend(children_of(&server, thread).await);
    let ended = events(&server, thread).await;
    assert!(
        ended
            .last()
            .is_some_and(|event| event.kind.as_str() == "turn.ended"),
        "the turn went on to its end"
    );
    (compared(&server, &threads).await, server)
}

/// Right after a restart, the waiting thread is listed waiting, as Needs you shows it.
async fn needs_you<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) {
    let listed = server
        .request(Request::ThreadList {
            project: Some(scope().project()),
            limit: None,
            show: None,
            cursor: None,
        })
        .await
        .unwrap();
    let Reply::Threads { threads, .. } = listed else {
        panic!("threads, not {listed:?}");
    };
    let summary = threads.iter().find(|summary| summary.id == thread).unwrap();
    assert_eq!(summary.status, Status::Waiting, "Needs you lists it");
}

/// A turn stopped at `script`'s wait goes on to its end on another server, its events those of
/// a run that never stopped.
async fn survives<S: Store>(
    name: &str,
    store: &S,
    script: impl Fn() -> Vec<(&'static str, Vec<Completion>)> + Copy,
    deny: bool,
) {
    let dir = folder(name);
    let (uninterrupted, _) = run(&dir, &Memory::new(), script, "PARENT: go", false, deny).await;
    let (restarted, _) = run(&dir, store, script, "PARENT: go", true, deny).await;
    assert_eq!(
        restarted, uninterrupted,
        "{name}: the same events but ids and times"
    );
}

fn an_approval() -> Vec<(&'static str, Vec<Completion>)> {
    vec![(
        "PARENT",
        vec![
            calling("bash", json!({"command": "touch ran.txt"})),
            answer(Some("Done."), None),
        ],
    )]
}

fn a_question() -> Vec<(&'static str, Vec<Completion>)> {
    let asks = json!({"questions": [{"question": "Which database?"}]});
    vec![(
        "PARENT",
        vec![
            calling("question", asks),
            answer(Some("Postgres it is."), None),
        ],
    )]
}

fn a_subagents_approval() -> Vec<(&'static str, Vec<Completion>)> {
    vec![
        (
            "PARENT",
            vec![subagent("CHILD-A", "touch it"), answer(Some("Done."), None)],
        ),
        (
            "CHILD-A",
            vec![
                calling("bash", json!({"command": "touch child.txt"})),
                answer(Some("Touched child.txt."), None),
            ],
        ),
    ]
}

/// Every scenario on stores `make` gives, one for each.
async fn suite<S: Store, F: Future<Output = S>>(store: &str, make: impl Fn(&str) -> F) {
    survives(
        &format!("{store}-approval"),
        &make("approval").await,
        an_approval,
        false,
    )
    .await;
    survives(
        &format!("{store}-question"),
        &make("question").await,
        a_question,
        false,
    )
    .await;
    survives(
        &format!("{store}-denial"),
        &make("denial").await,
        an_approval,
        true,
    )
    .await;
    let child = make("subagent").await;
    survives(
        &format!("{store}-subagent"),
        &child,
        a_subagents_approval,
        false,
    )
    .await;
    side_by_side(&format!("{store}-side-by-side"), &make("side").await).await;
}

/// Child A waits for an approval while child B runs, and the server stops: B is reported
/// interrupted, the parent waits with A, and once A is answered the parent takes A's report and
/// B's, B never started again.
async fn side_by_side<S: Store>(name: &str, store: &S) {
    let dir = folder(name);
    let gate = Arc::new(tokio::sync::Notify::new());
    let script = || {
        vec![
            (
                "PARENT",
                vec![
                    calling_all(vec![
                        subagent("CHILD-A", "touch a"),
                        subagent("CHILD-B", "think"),
                    ]),
                    answer(Some("Both reported."), None),
                ],
            ),
            (
                "CHILD-A",
                vec![
                    calling("bash", json!({"command": "touch a.txt"})),
                    answer(Some("Touched a.txt."), None),
                ],
            ),
            ("CHILD-B", vec![answer(Some("Thought."), None)]),
        ]
    };
    let model = ScriptedModel::keyed(script()).gated_for("CHILD-B", Arc::clone(&gate));
    let first_seen = model.seen();
    let first = server_on(&dir, store, model, Vec::new(), 1).await;
    let thread = start(&first, false).await;
    let mut stream = stream_turn(&first, thread, "PARENT: both", None)
        .await
        .unwrap();
    until_asked(&mut stream).await;
    first.stop(Duration::ZERO).await;
    drop((stream, first));

    let model = ScriptedModel::keyed(script());
    let second_seen = model.seen();
    let second = server_on(&dir, store, model, Vec::new(), 2).await;
    assert_eq!(
        second.repair_left_open().await.unwrap(),
        Repaired::default()
    );
    needs_you(&second, thread).await;
    let (at, pending) = waiting(&second, thread).await.expect("child A waits");
    second.request(answering(at, pending, false)).await.unwrap();
    second.stop(Duration::from_secs(3_600)).await;

    let children = children_of(&second, thread).await;
    assert_eq!(children.len(), 2, "two subagents, none started again");
    let asked_b = |seen: &Seen| {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|request| format!("{:?}", request.messages.first()).contains("CHILD-B"))
            .count()
    };
    assert_eq!(
        asked_b(&first_seen),
        1,
        "B ran once, until the server stopped"
    );
    assert_eq!(asked_b(&second_seen), 0, "B never ran again");
    let outputs: Vec<String> = events(&second, thread)
        .await
        .into_iter()
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::ToolCallCompleted(done) => Some(format!("{:?}", done.output)),
            _ => None,
        })
        .collect();
    assert_eq!(outputs.len(), 2, "{outputs:?}");
    assert!(outputs[0].contains("Touched a.txt."), "{outputs:?}");
    assert!(outputs[1].contains("interrupted"), "{outputs:?}");
    let ended = events(&second, thread).await;
    let last = decode(ended.last().unwrap().clone()).unwrap().body;
    assert!(
        matches!(last, Body::TurnEnded(ended) if ended.outcome == glave_event::Outcome::Completed)
    );
    assert!(dir.join("a.txt").exists());
}

#[test]
fn in_memory_waits_survive_a_restart() {
    block_on!(suite("memory", |_| ready(Memory::new())));
}

#[test]
fn on_sqlite_waits_survive_a_restart() {
    block_on!(suite("sqlite", |name| {
        let dir = folder(&format!("durable-sqlite-{name}"));
        ready(Sqlite {
            database: dir.join("glave.db"),
            _dir: dir,
        })
    }));
}

#[test]
fn on_postgres_waits_survive_a_restart() {
    let Ok(url) = std::env::var("GLAVE_TEST_POSTGRES_URL") else {
        eprintln!("skipped: set GLAVE_TEST_POSTGRES_URL to run the Postgres tests");
        return;
    };
    block_on!(async {
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let schemas = std::sync::Mutex::new(Vec::new());
        let admin = &client;
        let schemas = &schemas;
        let url = &url;
        suite("postgres", |name| {
            let schema = format!("glave_test_durable_{name}_{}", std::process::id());
            async move {
                admin
                    .batch_execute(&format!(
                        "DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema};"
                    ))
                    .await
                    .unwrap();
                let mut config: glave_store_postgres::Config = url.parse().unwrap();
                config.options(format!("-c search_path={schema}"));
                schemas.lock().unwrap().push(schema);
                Postgres {
                    config,
                    root: Arc::new(InMemoryRoot::new().unwrap()),
                    objects: InMemoryObjects::new(),
                }
            }
        })
        .await;
        let made = std::mem::take(&mut *schemas.lock().unwrap());
        for schema in made {
            admin
                .batch_execute(&format!("DROP SCHEMA {schema} CASCADE;"))
                .await
                .unwrap();
        }
        drop(client);
        let _ = connection.await;
    });
}
