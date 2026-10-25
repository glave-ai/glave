//! A turn stopped at every event boundary (`specs/one-server.md`, "The pull requests", 5): its
//! server dies after the first `n` appends, for every `n` a whole run makes, and a server
//! started on the same store finds each turn waiting or closes it `interrupted`; answering
//! what waits then takes the turn to its end. Alone and with two subagents side by side, no
//! call ever runs twice.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use glave_event::{Arguments, RawEvent, Seq, ThreadId};
use glave_ports::testing::ScriptedModel;
use glave_ports::{
    BoxFuture, Completion, Effect, EventLog, InMemoryLeases, InMemoryProjects, InMemoryTools,
    Leases, Located, LogError, Projects, Scope, ThreadQuery, ThreadView, ToolInfo, ToolOutput,
    ToolSource, ToolSpec,
};
use glave_session::protocol::{Reply, Request};
use serde_json::json;

use super::{Memory, Opened, Store, answering, calling, calling_all, server_on, subagent, waiting};
use crate::support::*;

/// A log that takes `left` appends, then refuses every other, as a store whose glave died
/// does: nothing more lands.
#[derive(Clone)]
struct Dying {
    log: Shared,
    left: Arc<AtomicUsize>,
    died: Arc<AtomicBool>,
}

impl Dying {
    fn after(log: &Shared, appends: usize) -> Self {
        Self {
            log: log.clone(),
            left: Arc::new(AtomicUsize::new(appends)),
            died: Arc::default(),
        }
    }
}

impl EventLog for Dying {
    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send {
        let alive = self
            .left
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        if !alive {
            self.died.store(true, Ordering::SeqCst);
        }
        let appended = self.log.append(scope, events);
        async move {
            if alive {
                appended.await
            } else {
                Err(LogError::Storage("this glave stopped".into()))
            }
        }
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        self.log.read(scope, thread, after, limit)
    }

    fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<glave_ports::search::Found>, LogError>> + Send {
        self.log.search(scope, query, limit)
    }

    fn locate(
        &self,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Located>, LogError>> + Send {
        self.log.locate(thread)
    }

    fn thread_views(
        &self,
        scope: &Scope,
        query: &ThreadQuery,
    ) -> impl Future<Output = Result<Vec<ThreadView>, LogError>> + Send {
        self.log.thread_views(scope, query)
    }

    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        self.log.thread_view(scope, thread)
    }

    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        self.log.rebuild_thread_view(scope, thread)
    }
}

/// The server that dies: the store's log, dying after its appends, and its projects; its
/// leases its own, so they are gone with it, as a dead server's lapse.
struct Doomed {
    log: Dying,
    projects: Arc<InMemoryProjects>,
    leases: Arc<InMemoryLeases>,
}

impl Store for Doomed {
    type Log = Dying;
    fn open(&self) -> BoxFuture<'_, Opened<Dying>> {
        let projects: Arc<dyn Projects> = Arc::clone(&self.projects) as Arc<dyn Projects>;
        let leases: Arc<dyn Leases> = Arc::clone(&self.leases) as Arc<dyn Leases>;
        Box::pin(std::future::ready((self.log.clone(), projects, leases)))
    }
}

/// A tool every call of which needs the person, and which counts its runs.
fn deploys() -> Arc<InMemoryTools> {
    let info = ToolInfo {
        spec: ToolSpec {
            name: "deploy".into(),
            description: "Deploy a part.".into(),
            parameters: json!({"type": "object", "properties": {"what": {"type": "string"}}}),
        },
        effect: Effect::Runs,
        parallel: false,
    };
    Arc::new(InMemoryTools::new(vec![(
        info,
        ToolOutput::text(false, "deployed"),
    )]))
}

fn deploy(what: &str) -> Completion {
    calling("deploy", json!({ "what": what }))
}

/// One turn alone: a deploy, a question, another deploy, then its answer.
fn alone() -> Vec<(&'static str, Vec<Completion>)> {
    let asks = json!({"questions": [{"question": "Which database?"}]});
    vec![(
        "PARENT",
        vec![
            deploy("one"),
            calling("question", asks),
            deploy("two"),
            answer(Some("Done."), None),
        ],
    )]
}

/// Two subagents side by side, each deploying its part, then a deploy of the parent's own.
fn with_children() -> Vec<(&'static str, Vec<Completion>)> {
    vec![
        (
            "PARENT",
            vec![
                calling_all(vec![
                    subagent("CHILD-A", "deploy a"),
                    subagent("CHILD-B", "deploy b"),
                ]),
                deploy("p"),
                answer(Some("Done."), None),
            ],
        ),
        (
            "CHILD-A",
            vec![deploy("a"), answer(Some("Deployed a."), None)],
        ),
        (
            "CHILD-B",
            vec![deploy("b"), answer(Some("Deployed b."), None)],
        ),
    ]
}

/// How many times each deploy ran, by what it deployed.
fn runs(tools: &InMemoryTools) -> Vec<(String, usize)> {
    let mut counted: Vec<(String, usize)> = Vec::new();
    for (_, arguments) in tools.calls() {
        let Arguments::Json(map) = arguments else {
            continue;
        };
        let what = map
            .get("what")
            .and_then(|what| what.as_str())
            .unwrap_or("?")
            .to_owned();
        match counted.iter_mut().find(|(seen, _)| *seen == what) {
            Some((_, n)) => *n += 1,
            None => counted.push((what, 1)),
        }
    }
    counted
}

/// Answers whatever `root`'s turn, or a subagent's, waits for, until the turn has ended, or
/// until `dead` says its server can record nothing more. Answered with every wait it met.
async fn answer_until_done<L: EventLog + 'static>(
    server: &Tested<L>,
    root: ThreadId,
    dead: impl Fn() -> bool,
) {
    for _ in 0..100_000 {
        if dead() {
            return;
        }
        let ended = events(server, root)
            .await
            .last()
            .is_some_and(|event| event.kind.as_str() == "turn.ended");
        if ended {
            return;
        }
        if let Some((at, pending)) = waiting(server, root).await {
            // Refused when it was answered meanwhile, or the server can no longer record it.
            let _ = server.request(answering(at, pending, false)).await;
        }
        tokio::task::yield_now().await;
    }
    panic!("the turn never ended");
}

/// Whether every open turn of `root` and its subagents waits, as their views say after a
/// restart: none is left running.
async fn none_left_running<L: EventLog + 'static>(server: &Tested<L>, root: ThreadId) -> bool {
    let mut threads = vec![root];
    threads.extend(super::children_of(server, root).await);
    for thread in threads {
        let got = server.request(Request::ThreadGet { thread }).await.unwrap();
        let Reply::Thread { thread: summary } = got else {
            return false;
        };
        let events = events(server, thread).await;
        let open = events
            .iter()
            .rev()
            .find(|event| matches!(event.kind.as_str(), "turn.started" | "turn.ended"))
            .is_some_and(|event| event.kind.as_str() == "turn.started");
        let waits = summary.status == glave_session::protocol::Status::Waiting;
        // A parent whose subagent waits is waiting too.
        if open && !waits {
            return false;
        }
    }
    true
}

/// `script` stopped after `appends` appends, then taken to its end by a server on the same
/// store: whether every turn was waiting or closed, how often each deploy ran, and how many
/// appends the first server made.
async fn stopped_after(
    name: &str,
    script: fn() -> Vec<(&'static str, Vec<Completion>)>,
    appends: usize,
) -> (bool, Vec<(String, usize)>, usize) {
    let dir = folder(&format!("{name}-{appends}"));
    let tools = deploys();
    let shared = Memory::new();
    let (_, own_leases) = shared_leases();
    let doomed = Doomed {
        log: Dying::after(&shared.log, appends),
        projects: Arc::clone(&shared.projects),
        leases: own_leases,
    };
    let dead = Arc::clone(&doomed.log.died);
    let left = Arc::clone(&doomed.log.left);
    let sources: Vec<Arc<dyn ToolSource>> = vec![Arc::clone(&tools) as Arc<dyn ToolSource>];
    let first = server_on(
        &dir,
        &doomed,
        ScriptedModel::keyed(script()),
        sources.clone(),
        1,
    )
    .await;
    let started = first
        .request(Request::ThreadStart {
            project: Some(scope().project()),
            background: false,
            worktree: None,
            computer: None,
            model: None,
            permissions: None,
            effort: None,
        })
        .await;
    let Ok(Reply::Thread { thread }) = started else {
        // Stopped before its thread was kept: nothing ran.
        first.stop(Duration::ZERO).await;
        let used = appends.saturating_sub(left.load(Ordering::SeqCst));
        return (true, runs(&tools), used);
    };
    let root = thread.id;
    let stream = stream_turn(&first, root, "PARENT: go", None).await;
    answer_until_done(&first, root, || dead.load(Ordering::SeqCst)).await;
    first.stop(Duration::ZERO).await;
    drop((stream, first));

    let second = server_on(&dir, &shared, ScriptedModel::keyed(script()), sources, 2).await;
    second.repair_left_open().await.unwrap();
    let settled = none_left_running(&second, root).await;
    answer_until_done(&second, root, || false).await;
    second.stop(Duration::from_secs(3_600)).await;
    let used = appends.saturating_sub(left.load(Ordering::SeqCst));
    (settled, runs(&tools), used)
}

/// Every boundary of `script`'s run: how many appends a whole run makes, deploying each of
/// `parts` once, then a run stopped after each count.
async fn every_boundary(
    name: &str,
    script: fn() -> Vec<(&'static str, Vec<Completion>)>,
    parts: &[&str],
) {
    let whole = usize::MAX / 2;
    let (_, mut all, total) = stopped_after(name, script, whole).await;
    all.sort();
    let once: Vec<(String, usize)> = parts.iter().map(|part| ((*part).to_owned(), 1)).collect();
    assert_eq!(all, once, "a whole run deploys each part once");
    assert!(total > 10, "a whole run appends {total} times");
    for appends in 0..total {
        let (settled, ran, _) = stopped_after(name, script, appends).await;
        assert!(
            settled,
            "stopped after {appends} appends: a turn was left running"
        );
        assert!(
            ran.iter().all(|(_, n)| *n <= 1),
            "stopped after {appends} appends: a call ran twice: {ran:?}"
        );
    }
}

#[test]
fn a_turn_stopped_at_any_boundary_is_waiting_or_interrupted_and_no_call_runs_twice() {
    block_on!(every_boundary("crashes-alone", alone, &["one", "two"]));
}

#[test]
fn subagents_stopped_at_any_boundary_never_run_a_call_twice() {
    block_on!(every_boundary(
        "crashes-children",
        with_children,
        &["a", "b", "p"]
    ));
}
