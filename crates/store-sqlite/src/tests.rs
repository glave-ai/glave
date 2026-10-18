use std::path::PathBuf;
use std::sync::Arc;

use glave_keys::{InMemoryRoot, KeyFileRoot, KeySettings};
use glave_ports::{EventLog, Spaces as _, Standing, ThreadQuery, block_on, conformance};
use uuid::Uuid;

use super::*;

mod sealing;

/// Keys under a root that lives only in this process.
fn keys() -> KeySettings {
    KeySettings::new(Arc::new(InMemoryRoot::new().unwrap()))
}

/// Keys under the key file beside `database`, so a reopened database can read its content.
fn file_keys(database: &std::path::Path) -> KeySettings {
    let root = KeyFileRoot::open_or_create(database.with_extension("key")).unwrap();
    KeySettings::new(Arc::new(root))
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
    assert_eq!(block_on!(conformance::event_log(&store.log())), Ok(()));
}

#[test]
fn the_log_passes_the_search_conformance_suite() {
    let store = Store::open_in_memory(keys()).unwrap();
    assert_eq!(block_on!(conformance::search(&store.log())), Ok(()));
}

#[test]
fn the_search_index_holds_no_word_in_the_clear() {
    let path = temp_db("search");
    block_on!(async {
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
    assert_eq!(block_on!(conformance::blob_store(&store.blobs())), Ok(()));
}

/// A project space for `path`'s folder, with ids from `n`.
fn project(n: u128, owner: &glave_event::Principal) -> glave_ports::Space {
    let at = glave_event::At::try_from("2026-10-11T09:00:00.000Z".to_owned()).unwrap();
    glave_ports::Space {
        id: glave_ports::SpaceId::from_uuid(Uuid::from_u128(n)),
        kind: glave_ports::SpaceKind::Project,
        name: "app".into(),
        owner: owner.clone(),
        members: Vec::new(),
        created_at: at,
        updated_at: at,
    }
}

fn ada() -> glave_event::Principal {
    glave_event::Principal::person(
        glave_event::PrincipalId::try_from(Uuid::from_u128(
            0x0192_f5c0_11aa_7e21_8c3d_5f9b_2e4a_6c10,
        ))
        .unwrap(),
    )
}

#[test]
fn events_and_spaces_survive_closing_and_reopening() {
    let path = temp_db("reopen");
    block_on!(async {
        let store = Store::open(&path, file_keys(&path)).await.unwrap();
        assert_eq!(conformance::event_log(&store.log()).await, Ok(()));
        let org = store.org(Uuid::from_u128(1)).await.unwrap();
        let key = [glave_ports::FolderKey::Path("/work/app".into())];
        let space = store
            .spaces()
            .folder(org, &key, &project(2, &ada()))
            .await
            .unwrap();
        drop(store);
        let reopened = Store::open(&path, file_keys(&path)).await.unwrap();
        assert_eq!(reopened.org(Uuid::max()).await.unwrap(), org);
        let again = reopened
            .spaces()
            .folder(org, &key, &project(3, &ada()))
            .await
            .unwrap();
        assert_eq!(space, again, "the folder keeps its space across runs");
    });
}

#[test]
fn the_spaces_pass_the_spaces_conformance_suite() {
    let store = Store::open_in_memory(keys()).unwrap();
    assert_eq!(block_on!(conformance::spaces(&store.spaces())), Ok(()));
}

#[test]
fn spaces_from_before_records_become_records_and_are_claimed() {
    let path = temp_db("spaces-migration");
    let person = "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10";
    {
        let mut connection = rusqlite::Connection::open(&path).unwrap();
        let transaction = connection.transaction().unwrap();
        for sql in schema::MIGRATIONS.iter().take(2) {
            transaction.execute_batch(sql).unwrap();
        }
        transaction.pragma_update(None, "user_version", 2).unwrap();
        transaction
            .execute_batch(&format!(
                "INSERT INTO settings (key, value) VALUES ('org', '00000000-0000-0000-0000-000000000001');
                 INSERT INTO spaces (path, id) VALUES
                     ('/work/app', '00000000-0000-0000-0000-000000000002'),
                     ('glave:personal/{person}', '00000000-0000-0000-0000-000000000003');"
            ))
            .unwrap();
        transaction.commit().unwrap();
    }
    block_on!(async {
        let store = Store::open(&path, keys()).await.unwrap();
        let org = store.org(Uuid::max()).await.unwrap();
        let spaces = store.spaces();
        let app = glave_ports::SpaceId::from_uuid(Uuid::from_u128(2));
        let before = spaces.get(org, app).await.unwrap().unwrap();
        assert_eq!(before.name, "app");
        assert_eq!(before.kind, glave_ports::SpaceKind::Project);
        assert!(!before.admits(&ada()), "no one's until claimed");
        assert_eq!(store.claim_spaces(&ada()).await.unwrap(), 1);
        let found = spaces
            .folder(
                org,
                &[glave_ports::FolderKey::Path("/work/app".into())],
                &project(9, &ada()),
            )
            .await
            .unwrap();
        assert_eq!(found.id, app, "the folder still leads to its space");
        assert!(found.admits(&ada()));
        let mut inbox = project(10, &ada());
        inbox.kind = glave_ports::SpaceKind::Personal;
        let personal = spaces.personal(org, &inbox).await.unwrap();
        assert_eq!(
            personal.id,
            glave_ports::SpaceId::from_uuid(Uuid::from_u128(3)),
            "the stand-in personal space is the person's personal space, memories and all"
        );
        assert_eq!(personal.name, "Chats");
    });
}

#[test]
fn a_database_at_schema_4_gains_the_search_index_and_searches_new_events() {
    let path = temp_db("search-migration");
    {
        let mut connection = rusqlite::Connection::open(&path).unwrap();
        let transaction = connection.transaction().unwrap();
        for sql in schema::MIGRATIONS.iter().take(4) {
            transaction.execute_batch(sql).unwrap();
        }
        transaction.pragma_update(None, "user_version", 4).unwrap();
        transaction.commit().unwrap();
    }
    block_on!(async {
        let store = Store::open(&path, file_keys(&path)).await.unwrap();
        assert_eq!(conformance::search(&store.log()).await, Ok(()));
    });
    let raw = rusqlite::Connection::open(&path).unwrap();
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, schema::current());
    assert_eq!(version, 5, "the search index is migration 5");
}

#[test]
fn a_newer_schema_is_refused() {
    let path = temp_db("newer");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 999).unwrap();
    }
    let result = block_on!(Store::open(&path, keys()));
    assert!(matches!(result, Err(StoreError::Schema(message)) if message.contains("newer glave")));
}

#[test]
fn folders_get_their_own_spaces_in_one_organization() {
    let store = Store::open_in_memory(keys()).unwrap();
    block_on!(async {
        let org = store.org(Uuid::from_u128(1)).await.unwrap();
        let spaces = store.spaces();
        let path = |p: &str| [glave_ports::FolderKey::Path(p.into())];
        let a = spaces
            .folder(org, &path("/work/a"), &project(2, &ada()))
            .await
            .unwrap();
        let b = spaces
            .folder(org, &path("/work/b"), &project(4, &ada()))
            .await
            .unwrap();
        assert_ne!(a.id, b.id, "each folder is its own space");
        let scope = glave_ports::Scope { org, space: a.id };
        let query = ThreadQuery {
            standing: Standing::Active,
            limit: 10,
            after: None,
        };
        let threads = store.log().thread_views(&scope, &query).await.unwrap();
        assert!(threads.is_empty(), "a new space has no threads");
    });
}

#[test]
fn the_log_passes_the_threads_view_conformance_suite() {
    let store = Store::open_in_memory(keys()).unwrap();
    assert_eq!(block_on!(conformance::thread_views(&store.log())), Ok(()));
}

/// Every row of the threads view, in a stable order.
fn views(path: &std::path::Path) -> Vec<(String, String, String, String)> {
    let connection = rusqlite::Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("SELECT org, space, thread, view FROM threads ORDER BY org, space, thread")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn views_rebuilt_from_the_log_equal_the_kept_ones() {
    let path = temp_db("views");
    block_on!(async {
        let store = Store::open(&path, file_keys(&path)).await.unwrap();
        assert_eq!(conformance::thread_views(&store.log()).await, Ok(()));
        drop(store);
        let kept = views(&path);
        assert_eq!(kept.len(), 4);

        // As after the upgrade that made the table: no views, and no version recorded.
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch("DELETE FROM threads; DELETE FROM settings WHERE key = 'threads_view';")
            .unwrap();
        drop(connection);
        let _reopened = Store::open(&path, file_keys(&path)).await.unwrap();
        assert_eq!(views(&path), kept, "opening fills the view from the log");
    });
}
