//! Shared helpers for the task and schedule tests: a server on a clock that stands still until
//! the test moves it, model answers that cost something, and starting, listing and waiting for
//! tasks.

use std::future::Future;
use std::sync::Arc;

use glave_computer::LocalComputers;
use glave_event::{Cost, Currency, Money, ThreadId, TokenCounts, Usage};
use glave_event::{RawEvent, Seq};
use glave_harness::{Settings, Workspace};
use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{Completion, EventLog, LogError, MemoryBlobs, MemoryLog, Scope};
use glave_session::protocol::{Reply, Request, TaskInfo, TaskStatus};
use glave_session::{Answering, Server, TaskLimits, Wiring};
use jiff::Timestamp;
use tokio::sync::Notify;

use crate::support::{Script, scope, script};

pub type Tested = Server<Script, Shared, MemoryBlobs>;

/// A log in memory that two servers can share, as two processes share a database.
#[derive(Clone, Default)]
pub struct Shared(pub Arc<MemoryLog>);

impl EventLog for Shared {
    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send {
        self.0.append(scope, events)
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        self.0.read(scope, thread, after, limit)
    }

    fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<glave_ports::search::Found>, LogError>> + Send {
        self.0.search(scope, query, limit)
    }

    fn locate(
        &self,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Scope>, LogError>> + Send {
        self.0.locate(thread)
    }

    fn thread_views(
        &self,
        scope: &Scope,
        query: &glave_ports::ThreadQuery,
    ) -> impl Future<Output = Result<Vec<glave_ports::ThreadView>, LogError>> + Send {
        self.0.thread_views(scope, query)
    }

    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<glave_ports::ThreadView>, LogError>> + Send {
        self.0.thread_view(scope, thread)
    }

    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<glave_ports::ThreadView>, LogError>> + Send {
        self.0.rebuild_thread_view(scope, thread)
    }
}

/// What a test server is built from.
pub struct Setup {
    pub answers: Vec<Completion>,
    pub gate: Option<Arc<Notify>>,
    pub log: Shared,
    pub answering: Answering,
    pub settings: Settings,
    pub limits: TaskLimits,
    pub now: ManualClock,
    /// Models offered beside the settings' own.
    pub models: Vec<glave_session::protocol::ModelInfo>,
    /// The approval modes a thread may choose.
    pub modes: Vec<glave_harness::ApprovalMode>,
}

impl Setup {
    pub fn new(answers: Vec<Completion>) -> Self {
        Self {
            answers,
            gate: None,
            log: Shared::default(),
            answering: Answering::Clients,
            settings: Settings::new("test-model", "script"),
            limits: TaskLimits::default(),
            now: ManualClock::at_millis(
                "2026-10-10T10:03:00Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_millisecond(),
            ),
            models: Vec::new(),
            modes: vec![
                glave_harness::ApprovalMode::Ask,
                glave_harness::ApprovalMode::Auto,
            ],
        }
    }

    pub async fn build(
        self,
        dir: &std::path::PathBuf,
    ) -> (Tested, Arc<std::sync::Mutex<Vec<String>>>) {
        let (provider, seen) = script(self.answers, self.gate);
        let (computers, info) = LocalComputers::for_here(std::env::vars_os())
            .open_here(dir)
            .await
            .unwrap();
        let wiring = Wiring {
            provider,
            log: self.log,
            blobs: MemoryBlobs::new(),
            clock: Box::new(self.now),
            entropy: Box::new(CountingEntropy::starting_at(1 << 32)),
            workspace: Workspace::new(computers, info),
            tools: Vec::new(),
            worktree: false,
            hooks: glave_harness::Hooks::default(),
            workers: None,
            tasks: self.limits,
            memory: None,
            spaces: Arc::new(glave_ports::MemorySpaces::new()),
            models: self.models,
            modes: self.modes,
        };
        (
            Server::new(wiring, self.settings, scope(), self.answering),
            seen,
        )
    }
}

/// A model answer that used `tokens` in and out and cost `micros`.
pub fn costing(mut completion: Completion, input: u64, output: u64, micros: i64) -> Completion {
    completion.usage = Usage::new(
        TokenCounts {
            input,
            output,
            ..TokenCounts::default()
        },
        false,
        None,
    )
    .unwrap();
    completion.cost = Some(Cost {
        amount: Money {
            micros,
            currency: Currency::USD,
        },
        price_list: "test".into(),
    });
    completion
}

pub async fn background(server: &Tested, thread: ThreadId, budget: Option<u64>) -> TaskInfo {
    let request = Request::TurnStart {
        thread,
        prompt: "tidy the notes".into(),
        background: true,
        budget,
        meta: glave_session::protocol::Meta::default(),
        model: None,
        approval: None,
    };
    match server.request(request).await.unwrap() {
        Reply::Task { task } => task,
        other => panic!("a task, not {other:?}"),
    }
}

pub async fn tasks(server: &Tested) -> Vec<TaskInfo> {
    match server
        .request(Request::TasksList { limit: None })
        .await
        .unwrap()
    {
        Reply::Tasks { tasks } => tasks,
        other => panic!("tasks, not {other:?}"),
    }
}

/// Lets the server's own work run until no task runs here any more.
pub async fn settle(server: &Tested) {
    for _ in 0..100_000 {
        if tasks(server)
            .await
            .iter()
            .all(|task| task.status != TaskStatus::Running)
        {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("a task never finished");
}
