//! Workers without a cluster: a backend that serves this machine's computers in-process, on a
//! Unix socket reached through bridges as a Pod is, so the adapter's lifecycle, limits and
//! routing are tested on every machine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use glave_ports::block_on;
use glave_ports::computer::{Bytes, Glob, Image, Isolation, Network, Save};
use glave_ports::testing::{ManualClock, SocketPath, TempDir};
use tokio::task::JoinHandle;

use super::backend::Listed;
use super::*;
use crate::{CancelSwitch, LocalComputers, bridge, listen_socket, serve_socket};

mod reconnect;

const IMAGE: &str =
    "alpine@sha256:ce64758a109eb420d874a118f87920e625e12d3634e03b4a5573fd9f6e5d3507";

fn scratch(name: &str) -> TempDir {
    TempDir::new(&format!("worker-{name}")).unwrap()
}

/// Workers served in-process: what a Pod would run, without the Pod. A worker reads its token
/// from a file, as glave in a Pod does, and is reached through bridges whose connections a test
/// can cut, as a fault of the cluster's API server would.
struct InProcess {
    copies: PathBuf,
    serving: Mutex<HashMap<String, JoinHandle<()>>>,
    started: Mutex<Vec<Spec>>,
    /// Bridges open into each worker.
    links: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// Times a worker was to be given a new token after it started.
    renewed: AtomicU64,
    /// Whether giving a worker a new token fails, as it does when the worker is gone.
    renewing_fails: std::sync::atomic::AtomicBool,
    /// Each worker running, with the glave it is labelled with.
    labelled: Mutex<std::collections::BTreeMap<String, Option<String>>>,
    /// The socket each worker listens at.
    sockets: Mutex<std::collections::BTreeMap<String, SocketPath>>,
}

impl InProcess {
    fn new(copies: &Path) -> Self {
        Self {
            copies: copies.to_owned(),
            serving: Mutex::new(HashMap::new()),
            started: Mutex::new(Vec::new()),
            links: Arc::default(),
            renewed: AtomicU64::new(0),
            renewing_fails: std::sync::atomic::AtomicBool::new(false),
            labelled: Mutex::default(),
            sockets: Mutex::default(),
        }
    }

    /// Serves `computers` as the worker `name` on a socket, its token in a file, reached
    /// through bridges ([`Self::bridged_into`]).
    fn on_socket(&self, name: &str, spec: &Spec, computers: Arc<LocalComputers>) -> JoinHandle<()> {
        std::fs::create_dir_all(&self.copies).unwrap();
        std::fs::write(self.token_file(name), spec.token.to_file()).unwrap();
        let socket = SocketPath::new(&self.copies, &format!("{name}.sock")).unwrap();
        let listener = listen_socket(&socket).unwrap();
        self.sockets.lock().unwrap().insert(name.to_owned(), socket);
        let tokens = crate::Tokens::file(self.token_file(name));
        tokio::spawn(async move {
            let _ = serve_socket(computers, listener, tokens, None, None).await;
        })
    }

    fn socket(&self, name: &str) -> PathBuf {
        self.sockets
            .lock()
            .unwrap()
            .get(name)
            .unwrap()
            .to_path_buf()
    }

    /// A new bridge into the worker `name`, a connection the test can cut.
    fn bridged_into(&self, name: &str) -> backend::Stream {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(theirs);
        let socket = self.socket(name);
        let carried = tokio::spawn(async move {
            let _ = bridge(&socket, read, write).await;
        });
        self.links.lock().unwrap().push(carried);
        let (read, write) = tokio::io::split(ours);
        (Box::new(read), Box::new(write))
    }

    fn token_file(&self, name: &str) -> PathBuf {
        self.copies.join(format!("{name}.token"))
    }

    /// Cuts every connection open to every worker, leaving the workers running.
    fn cut(&self) {
        for link in self.links.lock().unwrap().drain(..) {
            link.abort();
        }
    }

    /// Ends glave in the worker `name`, as if its process died; the worker stays held.
    fn kill(&self, name: &str) {
        if let Some(task) = self.serving.lock().unwrap().get(name) {
            task.abort();
        }
    }
}

/// A backend shared with the test, so it can cut connections while the adapter holds it.
struct Shared(Arc<InProcess>);

impl Backend for Shared {
    fn start<'a>(&'a self, spec: &'a Spec) -> BoxFuture<'a, Result<Started, ComputerError>> {
        self.0.start(spec)
    }

    fn renew<'a>(
        &'a self,
        name: &'a str,
        token: &'a Token,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        self.0.renew(name, token)
    }

    fn bridge<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, Result<backend::Stream, ComputerError>> {
        self.0.bridge(name)
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<Listed>, ComputerError>> {
        self.0.list()
    }

    fn remove<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<(), ComputerError>> {
        self.0.remove(name)
    }
}

impl Backend for InProcess {
    fn start<'a>(&'a self, spec: &'a Spec) -> BoxFuture<'a, Result<Started, ComputerError>> {
        Box::pin(async move {
            let name = {
                let mut started = self.started.lock().unwrap();
                started.push(spec.clone());
                format!("w{}", started.len() - 1)
            };
            let label = spec
                .starter
                .as_ref()
                .map(|starter| serde_json::to_string(starter).unwrap());
            self.labelled.lock().unwrap().insert(name.clone(), label);
            let environment = std::env::vars_os().filter(|(n, _)| n == "PATH" || n == "HOME");
            let computers = LocalComputers::new(self.copies.join(&name), environment)
                .with_worktrees(self.copies.join(format!("{name}-worktrees")))
                .with_projects(self.copies.join(format!("{name}-projects")));
            let computers = Arc::new(computers);
            let task = self.on_socket(&name, spec, computers);
            self.serving.lock().unwrap().insert(name.clone(), task);
            Ok(Started { name })
        })
    }

    fn renew<'a>(
        &'a self,
        name: &'a str,
        token: &'a Token,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            self.renewed.fetch_add(1, Ordering::SeqCst);
            if self.renewing_fails.load(Ordering::SeqCst) {
                return Err(ComputerError::Failed(format!("no worker {name}")));
            }
            std::fs::write(self.token_file(name), token.to_file()).unwrap();
            Ok(())
        })
    }

    fn bridge<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, Result<backend::Stream, ComputerError>> {
        Box::pin(std::future::ready(Ok(self.bridged_into(name))))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<Listed>, ComputerError>> {
        let listed = self.labelled.lock().unwrap().clone();
        Box::pin(std::future::ready(Ok(listed
            .into_iter()
            .map(|(name, started_by)| Listed { name, started_by })
            .collect())))
    }

    fn remove<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            self.labelled.lock().unwrap().remove(name);
            if let Some(task) = self.serving.lock().unwrap().remove(name) {
                task.abort();
            }
            Ok(())
        })
    }
}

/// A bare repository with one commit on `main`, as a worker would clone it.
fn origin(dir: &Path) -> String {
    let origin = dir.join("origin.git");
    let script = format!(
        "git init -q --bare {o} && git init -q src && cd src && echo hi > a.txt && git add a.txt \
         && git -c user.name=t -c user.email=t@example.com commit -q -m one && \
         git push -q {o} HEAD:refs/heads/main",
        o = origin.display()
    );
    let made = std::process::Command::new("bash")
        .args(["-c", &script])
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(made.success());
    origin.to_string_lossy().into_owned()
}

fn environment() -> Environment {
    Environment {
        image: Image::Oci(IMAGE.into()),
        setup: vec!["echo ready > setup.txt".into()],
        ..Environment::host()
    }
}

struct Quiet;

impl Output for Quiet {
    fn write<'a>(&'a self, _: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(std::future::ready(()))
    }
}

#[test]
fn workers_pass_the_computers_suite() {
    let dir = scratch("suite");
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let backend = InProcess::new(&dir.join("copies"));
    let workers = WorkerComputers::on(Box::new(backend), Limits::default());
    let image = Environment {
        image: Image::Oci(IMAGE.into()),
        ..Environment::host()
    };
    let verdict = block_on!(glave_ports::conformance::computers(
        &workers,
        &image,
        &work.to_string_lossy()
    ));
    assert_eq!(verdict, Ok(()));
}

#[test]
fn a_worker_is_made_cloned_into_set_up_saved_and_removed() {
    let dir = scratch("lifecycle");
    let url = origin(&dir);
    let backend = InProcess::new(&dir.join("copies"));
    let limits = Limits {
        cpus: Some(2),
        memory_gb: Some(4),
        ..Limits::default()
    };
    let workers = WorkerComputers::on(Box::new(backend), limits);
    block_on!(async {
        let made = workers.create(&environment()).await.unwrap();
        assert_eq!(made.environment, Some(environment()));
        let sources = Sources::Git {
            url,
            at: Some("main".into()),
        };
        let opened = workers.open(made.id, &sources).await.unwrap();
        let setup = workers.read(opened.id, "setup.txt").await.unwrap();
        assert_eq!(setup, Bytes::new(b"ready\n"), "setup ran in the workspace");
        let glob = Glob {
            pattern: "*.txt".into(),
            path: ".".into(),
            limit: 10,
        };
        let found = workers.glob(opened.id, &glob).await.unwrap();
        assert_eq!(found.total, 2, "{found:?}");
        let finished = workers
            .run(
                opened.id,
                &Command::new("echo more > b.txt"),
                &Quiet,
                &CancelSwitch::default(),
            )
            .await
            .unwrap();
        assert_eq!(finished.ending, Ending::Exited { code: 0 });
        let save = Save {
            branch: "glave/wip/abc".into(),
            message: "wip".into(),
        };
        assert!(workers.save(opened.id, &save).await.unwrap().pushed);
        workers.destroy(made.id).await.unwrap();
        let after = workers.status(made.id).await.unwrap();
        assert_eq!(after.state, glave_ports::computer::State::Gone);
        assert!(workers.read(opened.id, "a.txt").await.is_err());
    });
}

#[test]
fn a_worker_with_no_network_clones_nothing() {
    let dir = scratch("offline");
    let backend = InProcess::new(&dir.join("copies"));
    let workers = WorkerComputers::on(Box::new(backend), Limits::default());
    let mut offline = environment();
    offline.network = Some(Network::Off);
    offline.setup.clear();
    block_on!(async {
        let made = workers.create(&offline).await.unwrap();
        let sources = Sources::Git {
            url: "https://example.com/r.git".into(),
            at: None,
        };
        let refused = workers.open(made.id, &sources).await;
        assert!(
            matches!(&refused, Err(ComputerError::Refused(m)) if m.contains("no network")),
            "{refused:?}"
        );
        let folder = dir.join("here");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.to_string_lossy().into_owned();
        let opened = workers
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap();
        let finished = workers
            .run(
                opened.id,
                &Command::new("true"),
                &Quiet,
                &CancelSwitch::default(),
            )
            .await
            .unwrap();
        assert_eq!(finished.ending, Ending::Exited { code: 0 });
        workers.destroy(made.id).await.unwrap();
    });
}

#[test]
fn a_policy_requiring_vm_isolation_refuses_a_backend_without_it() {
    let dir = scratch("isolation");
    let backend = InProcess::new(&dir.join("copies"));
    let limits = Limits {
        isolation: Isolation::Vm,
        ..Limits::default()
    };
    let workers = WorkerComputers::on(Box::new(backend), limits);
    block_on!(async {
        let refused = workers.create(&environment()).await;
        assert!(
            matches!(&refused, Err(ComputerError::Refused(m)) if m.contains("isolation = \"vm\"")),
            "{refused:?}"
        );
    });
}

#[test]
fn a_failed_setup_says_why() {
    let dir = scratch("setup");
    let url = origin(&dir);
    let backend = InProcess::new(&dir.join("copies"));
    let limits = Limits {
        cpus: Some(2),
        ..Limits::default()
    };
    let workers = WorkerComputers::on(Box::new(backend), limits);
    let mut broken = environment();
    broken.setup = vec!["echo missing tool >&2; exit 3".into()];
    block_on!(async {
        let made = workers.create(&broken).await.unwrap();
        let failed = workers.open(made.id, &Sources::Git { url, at: None }).await;
        assert!(
            matches!(&failed, Err(ComputerError::Failed(m)) if m.contains("missing tool")),
            "{failed:?}"
        );
        workers.destroy(made.id).await.unwrap();
    });
}

/// `glave` with `field` changed, as a glave elsewhere or one that ended would be labelled.
pub(crate) fn changed(glave: &Glave, field: &str, value: &str) -> Glave {
    let mut changed = serde_json::to_value(glave).unwrap();
    changed[field] = serde_json::Value::from(value);
    serde_json::from_value(changed).unwrap()
}

#[test]
fn a_glave_starting_removes_the_workers_of_a_glave_that_is_gone() {
    let dir = scratch("reap");
    let backend = Arc::new(InProcess::new(&dir.join("copies")));
    let workers = |starter: Option<Glave>| {
        WorkerComputers::on(Box::new(Shared(Arc::clone(&backend))), Limits::default())
        .started_by(starter)
    };
    let this = Glave::this().unwrap();
    let ended = changed(&this, "start", "the start of a glave that ended");
    let elsewhere = changed(&ended, "host", "another machine");
    let mut offline = environment();
    offline.network = Some(Network::Off);
    block_on!(async {
        // Glaves that ended without removing their workers, as one killed outright does.
        for starter in [Some(ended), Some(elsewhere), None] {
            workers(starter).create(&offline).await.unwrap();
        }
        let next = workers(Some(this));
        let own = next.create(&offline).await.unwrap();
        assert_eq!(next.reap().await.unwrap(), 1);
        let left: Vec<String> = backend.labelled.lock().unwrap().keys().cloned().collect();
        assert_eq!(
            left,
            ["w1", "w2", "w3"],
            "only the worker of the glave that ended goes"
        );
        assert_eq!(next.reap().await.unwrap(), 0);
        next.destroy(own.id).await.unwrap();
        for name in ["w1", "w2"] {
            backend.remove(name).await.unwrap();
        }
    });
}
