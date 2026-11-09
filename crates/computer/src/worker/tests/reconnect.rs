//! A worker whose connection dies is reached again through a new bridge, with a fresh token,
//! so its work can still be saved; when it cannot be, the call says why, and the next call
//! tries again.

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

fn shared(dir: &Path) -> (Arc<InProcess>, WorkerComputers) {
    let backend = Arc::new(InProcess::new(&dir.join("copies")));
    let workers = WorkerComputers::on(Box::new(Shared(Arc::clone(&backend))), Limits::default());
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
        workers.destroy(made.id).await.unwrap();
    });
}

#[test]
fn a_worker_that_cannot_be_reached_again_says_so_and_the_next_call_tries_again() {
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
                    if m.contains("could not be opened again: it did not answer")
                        && !m.contains("no worker w0")),
                "the cluster's own words go to the log, not the person: {failed:?}"
            );
        }
        assert_eq!(
            backend.renewed.load(Ordering::SeqCst),
            3,
            "tried again at each call"
        );
        workers.destroy(made.id).await.unwrap();
    });
}
