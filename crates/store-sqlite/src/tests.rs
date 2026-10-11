use std::path::PathBuf;
use std::sync::Arc;

use glave_keys::{KeyFileRoot, KeySettings, MemoryRoot};
use glave_ports::{EventLog, conformance};
use uuid::Uuid;

use super::*;

mod sealing;

/// Keys under a root that lives only in this process.
fn keys() -> KeySettings {
    KeySettings::new(Arc::new(MemoryRoot::new().unwrap()))
}

/// Keys under the key file beside `database`, so a reopened database can read its content.
fn file_keys(database: &std::path::Path) -> KeySettings {
    let root = KeyFileRoot::open_or_create(database.with_extension("key")).unwrap();
    KeySettings::new(Arc::new(root))
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-sqlite-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("glave.db")
}

#[test]
fn the_log_passes_the_event_log_conformance_suite() {
    let store = Store::open_in_memory(keys()).unwrap();
    assert_eq!(block_on(conformance::event_log(&store.log())), Ok(()));
}

#[test]
fn the_log_passes_the_search_conformance_suite() {
    let store = Store::open_in_memory(keys()).unwrap();
    assert_eq!(block_on(conformance::search(&store.log())), Ok(()));
}

#[test]
fn the_search_index_holds_no_word_in_the_clear() {
    let path = temp_db("search");
    block_on(async {
        let store = Store::open(&path, file_keys(&path)).await.unwrap();
        assert_eq!(conformance::search(&store.log()).await, Ok(()));
    });
    let raw = rusqlite::Connection::open(&path).unwrap();
    let terms: Vec<String> = raw
        .prepare("SELECT terms FROM event_search")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!terms.is_empty(), "messages and outputs are indexed");
    for words in &terms {
        assert!(
            words
                .split(' ')
                .all(|word| word.len() == 32 && word.bytes().all(|b| b.is_ascii_hexdigit())),
            "{words}"
        );
        assert!(!words.contains("deploy") && !words.contains("postgres"));
    }
}

#[test]
fn the_blobs_pass_the_blob_store_conformance_suite() {
    let store = Store::open_in_memory(keys()).unwrap();
    assert_eq!(block_on(conformance::blob_store(&store.blobs())), Ok(()));
}

#[test]
fn events_survive_closing_and_reopening() {
    let path = temp_db("reopen");
    block_on(async {
        let store = Store::open(&path, file_keys(&path)).await.unwrap();
        assert_eq!(conformance::event_log(&store.log()).await, Ok(()));
        let scope = store
            .local_scope(&path, Uuid::nil(), Uuid::nil())
            .await
            .unwrap();
        drop(store);
        let reopened = Store::open(&path, file_keys(&path)).await.unwrap();
        let again = reopened
            .local_scope(&path, Uuid::max(), Uuid::max())
            .await
            .unwrap();
        assert_eq!(scope, again, "the folder keeps its space across runs");
    });
}

#[test]
fn a_newer_schema_is_refused() {
    let path = temp_db("newer");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 999).unwrap();
    }
    let result = block_on(Store::open(&path, keys()));
    assert!(matches!(result, Err(StoreError::Schema(message)) if message.contains("newer glave")));
}

#[test]
fn folders_get_their_own_spaces_in_one_organization() {
    let store = Store::open_in_memory(keys()).unwrap();
    block_on(async {
        let a = store
            .local_scope(
                std::path::Path::new("/work/a"),
                Uuid::from_u128(1),
                Uuid::from_u128(2),
            )
            .await
            .unwrap();
        let b = store
            .local_scope(
                std::path::Path::new("/work/b"),
                Uuid::from_u128(3),
                Uuid::from_u128(4),
            )
            .await
            .unwrap();
        assert_eq!(a.org, b.org, "one local organization");
        assert_ne!(a.space, b.space, "each folder is its own space");
        let threads = store.log().threads(&a, 10).await.unwrap();
        assert_eq!(threads.len(), 0, "a new space has no threads");
    });
}
