//! A turn stopped at every event boundary (`specs/one-server.md`, "The pull requests", 5): its
//! server stops after its first `n` appends, and nothing it appends after lands, for every `n`
//! up to what a whole run makes; a server started on the same store finds each turn waiting or
//! closes it `interrupted`, and answering what waits then takes every turn to its end. A turn
//! whose start never landed has nothing to repair: its request failed. Alone and with two
//! subagents side by side, no call ever runs twice.

use std::sync::Arc;
use std::time::Duration;

use glave_event::{Arguments, ThreadId};
use glave_ports::testing::{FaultyLog, ScriptedModel};
use glave_ports::{
    BoxFuture, Completion, Effect, EventLog, InMemoryLeases, InMemoryProjects, InMemoryTools,
    Leases, Projects, ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use glave_session::protocol::{Reply, Request, Status};
use serde_json::json;

use super::{Memory, Opened, Store, answering, calling, calling_all, server_on, subagent, waiting};
use crate::support::*;

/// More appends than a whole run of either script makes.
const BOUNDARIES: usize = 100;

/// The server that stops: the store's log, through a log that stops landing its appends, and
/// its projects; its leases its own, so they are gone with it, as a stopped server's lapse.
struct Doomed {
    log: FaultyLog,
    projects: Arc<InMemoryProjects>,
    leases: Arc<InMemoryLeases>,
}

impl Store for Doomed {
    type Log = FaultyLog;
    fn open(&self) -> BoxFuture<'_, Opened<FaultyLog>> {
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
/// until `stopped` says its server can record nothing more.
async fn answer_until_done<L: EventLog + 'static>(
    server: &Tested<L>,
    root: ThreadId,
    stopped: impl Fn() -> bool,
) {
    for _ in 0..100_000 {
        if stopped() {
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

/// The open turns of `root` and its subagents, each by whether it waits as its view says: a
/// parent whose subagent waits is waiting too.
async fn left_open<L: EventLog + 'static>(server: &Tested<L>, root: ThreadId) -> Vec<bool> {
    let mut threads = vec![root];
    threads.extend(super::children_of(server, root).await);
    let mut open = Vec::new();
    for thread in threads {
        let got = server.request(Request::ThreadGet { thread }).await.unwrap();
        let Reply::Thread { thread: summary } = got else {
            panic!("a thread, not {got:?}");
        };
        let turn_open = events(server, thread)
            .await
            .iter()
            .rev()
            .find(|event| matches!(event.kind.as_str(), "turn.started" | "turn.ended"))
            .is_some_and(|event| event.kind.as_str() == "turn.started");
        if turn_open {
            open.push(summary.status == Status::Waiting);
        }
    }
    open
}

/// `script` stopped after `appends` appends, then taken to its end by a server on the same
/// store: every turn left open waits or is closed, then ends, and no deploy runs twice. How
/// often each deploy ran when the run was whole, its server never stopped.
async fn stopped_after(
    name: &str,
    script: fn() -> Vec<(&'static str, Vec<Completion>)>,
    appends: usize,
) -> Option<Vec<(String, usize)>> {
    let dir = folder(&format!("{name}-{appends}"));
    let tools = deploys();
    let shared = Memory::new();
    let (_, own_leases) = shared_leases();
    let log = FaultyLog::over(Arc::clone(&shared.log.0));
    log.stop_after(appends);
    let doomed = Doomed {
        log: log.clone(),
        projects: Arc::clone(&shared.projects),
        leases: own_leases,
    };
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
        assert!(log.stopped(), "a thread refused with {started:?}");
        return None;
    };
    let root = thread.id;
    let mut stream = stream_turn(&first, root, "PARENT: go", None).await;
    answer_until_done(&first, root, || log.stopped()).await;
    let begun = events(&first, root)
        .await
        .iter()
        .any(|event| event.kind.as_str() == "turn.started");
    if !begun {
        // Stopped before its turn was kept: nothing to repair, and the request failed.
        let failed = match &mut stream {
            Ok(stream) => stream.finish().await.is_err(),
            Err(_) => true,
        };
        assert!(
            failed,
            "stopped after {appends} appends: a turn never kept ended well"
        );
    }
    first.stop(Duration::ZERO).await;
    drop((stream, first));

    let second = server_on(&dir, &shared, ScriptedModel::keyed(script()), sources, 2).await;
    second.repair_left_open().await.unwrap();
    assert!(
        left_open(&second, root).await.iter().all(|waits| *waits),
        "stopped after {appends} appends: a turn was left running"
    );
    if begun {
        answer_until_done(&second, root, || false).await;
    }
    second.stop(Duration::from_secs(3_600)).await;
    assert_eq!(
        left_open(&second, root).await,
        Vec::<bool>::new(),
        "stopped after {appends} appends: a turn never ended"
    );
    let ran = runs(&tools);
    assert!(
        ran.iter().all(|(_, n)| *n <= 1),
        "stopped after {appends} appends: a call ran twice: {ran:?}"
    );
    (!log.stopped()).then_some(ran)
}

/// `script`'s run stopped at every boundary, one append after another, until a run is whole:
/// it deploys each of `parts` once.
async fn every_boundary(
    name: &str,
    script: fn() -> Vec<(&'static str, Vec<Completion>)>,
    parts: &[&str],
) {
    for appends in 0..BOUNDARIES {
        let Some(mut all) = stopped_after(name, script, appends).await else {
            continue;
        };
        all.sort();
        let once: Vec<(String, usize)> = parts.iter().map(|part| ((*part).to_owned(), 1)).collect();
        assert_eq!(all, once, "a whole run deploys each part once");
        assert!(appends > 10, "a whole run appends {appends} times");
        return;
    }
    panic!("no run was whole within {BOUNDARIES} appends");
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
