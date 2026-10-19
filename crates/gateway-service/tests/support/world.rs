//! A gateway service for tests, on 127.0.0.1: a fake identity provider signs people in, a fake
//! OpenAI-compatible provider answers behind the gateway with a key only the gateway holds,
//! the log is in memory, and the catalog is glave's MCP test server, started twice.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use glave_event::{Body, RawEvent, Seq, ThreadId, decode};
use glave_gateway::{Budgets, Gateway, Model, Routes, Target};
use glave_gateway_service::{Policy, Service, Wiring, serve, stream_id};
use glave_identity::signin::{SignIn, Trust, person};
use glave_mcp::{Limits, McpServer, ServerConfig};
use glave_ports::Caller;
pub use glave_ports::block_on;
use glave_ports::testing::{CountingEntropy, FAKE_SECRET};
use glave_ports::{EventLog, InMemoryLog, LogError, OrgId, ProjectId, Purpose, Scope};
use glave_provider_openai::fake::{FakeServer, Reply};
use glave_provider_openai::{Config, OpenAiCompatible};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::issuer::{AUDIENCE, Issuer, Signer, fixed_clock};

/// The key the gateway holds for its model provider.
pub const KEY: &str = FAKE_SECRET;

/// One log shared by every service a test starts, as a restart would find it.
#[derive(Clone, Default)]
pub struct SharedLog(pub Arc<InMemoryLog>);

impl EventLog for SharedLog {
    async fn append(&self, scope: &Scope, events: &[RawEvent]) -> Result<(), LogError> {
        self.0.append(scope, events).await
    }

    async fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> Result<Vec<RawEvent>, LogError> {
        self.0.read(scope, thread, after, limit).await
    }

    async fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<glave_ports::search::Found>, LogError> {
        self.0.search(scope, query, limit).await
    }

    async fn locate(&self, thread: ThreadId) -> Result<Option<Scope>, LogError> {
        self.0.locate(thread).await
    }

    async fn thread_views(
        &self,
        scope: &Scope,
        query: &glave_ports::ThreadQuery,
    ) -> Result<Vec<glave_ports::ThreadView>, LogError> {
        self.0.thread_views(scope, query).await
    }

    async fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> Result<Option<glave_ports::ThreadView>, LogError> {
        self.0.thread_view(scope, thread).await
    }

    async fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> Result<Option<glave_ports::ThreadView>, LogError> {
        self.0.rebuild_thread_view(scope, thread).await
    }
}

/// Bob may not use the judge's model, nor any tool of the `tracker` server.
pub struct Rules;

impl Policy for Rules {
    fn refuses_model(&self, caller: &Caller, target: &str) -> Option<String> {
        (subject(caller) == "bob" && target == "acme/small")
            .then(|| "[models] deny = [\"acme/small\"] for bob in the test policy".into())
    }

    fn refuses_tool(&self, caller: &Caller, server: &str, _tool: &str) -> Option<String> {
        (subject(caller) == "bob" && server == "tracker")
            .then(|| "the tracker is not in bob's catalog".into())
    }
}

fn subject(caller: &Caller) -> &str {
    caller
        .auth
        .as_ref()
        .and_then(|auth| auth.subject.as_deref())
        .unwrap_or("")
}

/// glave's MCP test server, built beside this test by `cargo xtask ci`.
pub fn test_server() -> PathBuf {
    let path = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(|deps| deps.parent())
        .unwrap()
        .join("glave-mcp-test-server");
    assert!(
        path.exists(),
        "build the test server first: cargo build -p glave-mcp --features test-server"
    );
    path
}

/// The catalog: the test server as `docs` and as `tracker`.
pub fn catalog() -> Vec<Arc<McpServer>> {
    let config = ServerConfig::Stdio {
        command: test_server().display().to_string(),
        args: Vec::new(),
        env: BTreeMap::new(),
    };
    let dir = std::env::temp_dir();
    ["docs", "tracker"]
        .into_iter()
        .map(|name| {
            Arc::new(McpServer::start(
                name,
                &config,
                &BTreeMap::new(),
                &dir,
                Limits::default(),
                &glave_process::Processes::unrecorded(),
            ))
        })
        .collect()
}

/// A running gateway service and what stands behind it.
pub struct World {
    pub url: String,
    pub issuer: Issuer,
    pub signer: Signer,
    pub upstream: FakeServer,
    pub log: SharedLog,
    pub scope: Scope,
    pub notices: Arc<Mutex<Vec<String>>>,
    stop: Option<oneshot::Sender<()>>,
    serving: Option<JoinHandle<()>>,
}

impl World {
    /// A gateway over `replies`, with `budgets`, its catalog empty unless `with_catalog`.
    pub async fn start(replies: Vec<Reply>, budgets: Budgets, with_catalog: bool) -> Self {
        let issuer = Issuer::start().await;
        let signer = Signer::es256("k1");
        issuer.publish(&signer);
        let upstream = FakeServer::start(replies).unwrap();
        let scope = Scope {
            org: OrgId::from_uuid(uuid(1)),
            project: ProjectId::from_uuid(uuid(2)),
        };
        let mut world = Self {
            url: String::new(),
            issuer,
            signer,
            upstream,
            log: SharedLog::default(),
            scope,
            notices: Arc::new(Mutex::new(Vec::new())),
            stop: None,
            serving: None,
        };
        world.serve(budgets, with_catalog).await;
        world
    }

    /// Starts (or starts again, on the same log) the service.
    pub async fn serve(&mut self, budgets: Budgets, with_catalog: bool) {
        self.halt().await;
        let config = Config::for_url(self.upstream.url(), Some(KEY.to_owned())).unwrap();
        let handle: Arc<dyn Model> = Arc::new(OpenAiCompatible::new(config).unwrap());
        let main = vec![Target::new(Arc::clone(&handle), "acme", "big")];
        let judge = vec![Target::new(handle, "acme", "small")];
        let routes = Routes::new(main)
            .unwrap()
            .with_route(Purpose::Judge, judge)
            .unwrap();
        let trust = Trust {
            issuers: vec![self.issuer.url.clone()],
            audience: AUDIENCE.into(),
        };
        let notices = Arc::clone(&self.notices);
        let service = Service::new(Wiring {
            gateway: Gateway::new(routes).with_budgets(budgets),
            policy: Arc::new(Rules),
            signin: SignIn::new(trust, Box::new(fixed_clock())).unwrap(),
            log: self.log.clone(),
            scope: self.scope,
            clock: Box::new(fixed_clock()),
            entropy: Box::new(CountingEntropy::starting_at(1)),
            catalog: if with_catalog { catalog() } else { Vec::new() },
            secrets: vec![KEY.to_owned()],
            notices: Arc::new(move |notice: &str| notices.lock().unwrap().push(notice.into())),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        self.url = format!("http://{}", listener.local_addr().unwrap());
        let (stop, stopped) = oneshot::channel::<()>();
        self.stop = Some(stop);
        self.serving = Some(tokio::spawn(async move {
            serve(service, listener, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
        }));
    }

    async fn halt(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(serving) = self.serving.take() {
            let _ = serving.await;
        }
    }

    /// A good token for `subject`.
    pub fn token(&self, subject: &str) -> String {
        self.signer.sign(&self.issuer.claims(subject))
    }

    /// The events recorded for `subject`, decoded.
    pub async fn recorded(&self, subject: &str) -> Vec<Body> {
        let person = person(&self.issuer.url, subject).unwrap();
        let thread = stream_id(person).unwrap();
        let raw = self
            .log
            .read(&self.scope, thread, None, 1_000)
            .await
            .unwrap();
        raw.into_iter()
            .map(|raw| decode(raw).unwrap().body)
            .collect()
    }
}

fn uuid(n: u8) -> uuid::Uuid {
    uuid::Uuid::from_bytes([n; 16])
}
