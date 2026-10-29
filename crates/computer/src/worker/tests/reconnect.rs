//! A worker whose connection dies is reached again once, with a fresh token, so its work
//! can still be saved, over TLS or through a new bridge; when it cannot be, every call says
//! why, and it is not tried again.

use super::*;

/// Reads `path` until a call goes through: the first after a connection died may fail (it
/// was sent before the death was noticed), and then says it may or may not have run.
async fn read_after_death(
    workers: &WorkerComputers,
    workspace: WorkspaceId,
    path: &str,
) -> Result<Bytes, ComputerError> {
    match workers.read(workspace, path).await {
        Err(ComputerError::Unreachable(first)) if first.contains("may or may not") => {
            workers.read(workspace, path).await
        }
        other => other,
    }
}

/// A worker without a network, made by `workers`, with a folder in `dir` holding `a.txt`
/// opened on it.
async fn offline(workers: &WorkerComputers, dir: &Path) -> WorkspaceInfo {
    let mut offline = environment();
    offline.network = Some(Network::Off);
    offline.setup.clear();
    let made = workers.create(&offline).await.unwrap();
    let folder = dir.join("here");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("a.txt"), "a").unwrap();
    let path = folder.to_string_lossy().into_owned();
    workers
        .open(made.id, &Sources::Folder { path })
        .await
        .unwrap()
}

fn shared(dir: &Path) -> (Arc<InProcess>, WorkerComputers) {
    let backend = Arc::new(InProcess::new(&dir.join("copies")));
    let workers = WorkerComputers::on(
        Box::new(Shared(Arc::clone(&backend))),
        Limits::default(),
        WorkerCredentials::None,
    );
    (backend, workers)
}

#[test]
fn a_worker_whose_connection_dies_is_reached_again_with_a_fresh_token() {
    let dir = scratch("reconnect");
    let url = origin(&dir);
    let (backend, workers) = shared(&dir);
    block_on!(async {
        let made = workers.create(&environment()).await.unwrap();
        let sources = Sources::Git { url, at: None };
        let opened = workers.open(made.id, &sources).await.unwrap();
        let first = backend.started.lock().unwrap()[0].token.clone();

        backend.cut();
        let read = read_after_death(&workers, opened.id, "setup.txt").await;
        assert_eq!(read.unwrap(), Bytes::new(b"ready\n"));
        assert_eq!(backend.renewed.load(Ordering::SeqCst), 1, "one fresh token");
        let file = std::fs::read_to_string(backend.token_file("w0")).unwrap();
        assert!(
            !file.contains(first.secret()),
            "the first token is replaced"
        );

        // The new connection dies too: it is opened again, once more.
        backend.cut();
        read_after_death(&workers, opened.id, "setup.txt")
            .await
            .unwrap();
        let save = Save {
            branch: "glave/wip/again".into(),
            message: "wip".into(),
        };
        assert!(workers.save(opened.id, &save).await.unwrap().pushed);
        assert_eq!(backend.renewed.load(Ordering::SeqCst), 2);
        workers.destroy_all().await.unwrap();
    });
}

#[test]
fn a_worker_that_cannot_be_reached_again_says_so_and_is_not_tried_again() {
    let dir = scratch("unreachable");
    let url = origin(&dir);
    let (backend, workers) = shared(&dir);
    block_on!(async {
        let made = workers.create(&environment()).await.unwrap();
        let sources = Sources::Git { url, at: None };
        let opened = workers.open(made.id, &sources).await.unwrap();
        backend.renewing_fails.store(true, Ordering::SeqCst);
        backend.kill("w0");
        for _ in 0..3 {
            let failed = read_after_death(&workers, opened.id, "setup.txt").await;
            assert!(
                matches!(&failed, Err(ComputerError::Unreachable(m))
                    if m.contains("could not be opened again") && m.contains("no worker w0")),
                "{failed:?}"
            );
        }
        assert_eq!(backend.renewed.load(Ordering::SeqCst), 1, "tried once");
        workers.destroy_all().await.unwrap();
    });
}

#[test]
fn a_worker_without_a_network_whose_stream_dies_says_it_cannot_be_reached_again() {
    let dir = scratch("offline-dies");
    let (backend, workers) = shared(&dir);
    block_on!(async {
        let opened = offline(&workers, &dir).await;
        backend.kill("w0");
        let failed = read_after_death(&workers, opened.id, "a.txt").await;
        assert!(
            matches!(&failed, Err(ComputerError::Unreachable(m)) if m.contains("standard input")),
            "{failed:?}"
        );
        assert_eq!(backend.renewed.load(Ordering::SeqCst), 0);
        workers.destroy_all().await.unwrap();
    });
}

#[test]
fn a_worker_reached_through_a_bridge_is_reached_again_through_a_new_one() {
    let dir = scratch("bridged");
    let backend = Arc::new(InProcess::bridged(&dir.join("copies")));
    let workers = WorkerComputers::on(
        Box::new(Shared(Arc::clone(&backend))),
        Limits::default(),
        WorkerCredentials::None,
    );
    block_on!(async {
        // Without a network too: the socket needs none.
        let opened = offline(&workers, &dir).await;
        // The bridge drops, as an exec does when the API server restarts: the computer goes
        // on, and the workspace it opened is still there through the next bridge.
        backend.cut();
        let read = read_after_death(&workers, opened.id, "a.txt").await;
        assert_eq!(read.unwrap(), Bytes::new(b"a"));
        assert_eq!(backend.renewed.load(Ordering::SeqCst), 1, "one fresh token");
        workers.destroy_all().await.unwrap();
    });
}
