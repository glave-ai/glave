use std::path::PathBuf;
use std::sync::Arc;

use glave_keys::{InMemoryRoot, KeyFileRoot, KeySettings};
use glave_ports::testing::TempDir;
use glave_ports::{
    EventLog, InMemoryObjects, Objects, Projects as _, Standing, ThreadQuery, block_on, conformance,
};
use uuid::Uuid;

use super::*;

mod sealing;

/// Objects that live only in this process.
fn objects() -> Arc<dyn Objects> {
    Arc::new(InMemoryObjects::new())
}

/// Keys under a root that lives only in this process.
fn keys() -> KeySettings {
    KeySettings::new(Arc::new(InMemoryRoot::new().unwrap()))
}

/// Keys under the key file beside `database`, so a reopened database can read its content.
fn file_keys(database: &std::path::Path) -> KeySettings {
    let root = KeyFileRoot::open_or_create(database.with_extension("key")).unwrap();
    KeySettings::new(Arc::new(root))
}

/// A database path in a fresh folder, removed when the first drops.
fn temp_db(name: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new(&format!("sqlite-{name}")).unwrap();
    let path = dir.join("glave.db");
    (dir, path)
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
    let (_dir, path) = temp_db("search");
    block_on!(async {
        let store = Store::open(&path, file_keys(&path), objects())
            .await
            .unwrap();
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
    let (_dir, path) = temp_db("blobs");
    let clock = glave_ports::testing::ManualClock::at_millis(1_791_000_000_000);
    let mut settings = keys();
    settings.clock = Arc::new(clock.clone());
    let objects = InMemoryObjects::with_clock(Arc::new(clock.clone()));
    block_on!(async {
        let store = Store::open(&path, settings, Arc::new(objects.clone()))
            .await
            .unwrap();
        let verdict = conformance::blob_store(&store.blobs(), &clock, Some(&objects)).await;
        assert_eq!(verdict, Ok(()));
    });
}

/// A project for `path`'s folder, with ids from `n`.
fn record(n: u128, owner: &glave_event::Principal) -> glave_ports::Project {
    let at = glave_event::At::try_from("2026-10-11T09:00:00.000Z".to_owned()).unwrap();
    glave_ports::Project {
        id: glave_ports::ProjectId::from_uuid(Uuid::from_u128(n)),
        kind: glave_ports::ProjectKind::Project,
        name: "app".into(),
        owner: owner.clone(),
        members: Vec::new(),
        created_at: at,
        updated_at: at,
        revision: 0,
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

fn principal(n: u8) -> glave_event::PrincipalId {
    glave_event::PrincipalId::try_from(
        uuid::Builder::from_unix_timestamp_millis(1, &[n; 10]).into_uuid(),
    )
    .unwrap()
}

#[test]
fn the_identity_is_kept_the_first_time_and_survives_reopening() {
    let (_dir, path) = temp_db("identity");
    block_on!(async {
        let store = Store::open(&path, file_keys(&path), objects())
            .await
            .unwrap();
        let first = (principal(1), principal(2));
        assert_eq!(store.identity(first).await.unwrap(), first);
        assert_eq!(
            store.identity((principal(3), principal(4))).await.unwrap(),
            first,
            "a second identity never replaces the first"
        );
        drop(store);
        let reopened = Store::open(&path, file_keys(&path), objects())
            .await
            .unwrap();
        assert_eq!(
            reopened
                .identity((principal(5), principal(6)))
                .await
                .unwrap(),
            first
        );
    });
}

#[test]
fn events_and_projects_survive_closing_and_reopening() {
    let (_dir, path) = temp_db("reopen");
    block_on!(async {
        let store = Store::open(&path, file_keys(&path), objects())
            .await
            .unwrap();
        assert_eq!(conformance::event_log(&store.log()).await, Ok(()));
        let org = store.org(Uuid::from_u128(1)).await.unwrap();
        let key = [glave_ports::FolderKey::Path("/work/app".into())];
        let project = store
            .projects()
            .folder(org, &key, &record(2, &ada()))
            .await
            .unwrap();
        drop(store);
        let reopened = Store::open(&path, file_keys(&path), objects())
            .await
            .unwrap();
        assert_eq!(reopened.org(Uuid::max()).await.unwrap(), org);
        let again = reopened
            .projects()
            .folder(org, &key, &record(3, &ada()))
            .await
            .unwrap();
        assert_eq!(project, again, "the folder keeps its project across runs");
    });
}

#[test]
fn the_projects_pass_the_projects_conformance_suite() {
    let store = Store::open_in_memory(keys()).unwrap();
    assert_eq!(block_on!(conformance::projects(&store.projects())), Ok(()));
}

/// A file another version of glave wrote, older or newer, is refused with what to do, and left
/// as it was: until glave 0.1 there is one schema and no migration.
#[test]
fn a_file_of_another_schema_is_refused_and_left_as_it_was() {
    for found in [6, schema::VERSION + 1] {
        let (_dir, path) = temp_db(&format!("schema-{found}"));
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .pragma_update(None, "user_version", found)
                .unwrap();
        }
        let result = block_on!(Store::open(&path, keys(), objects()));
        assert!(
            matches!(&result, Err(StoreError::Schema(message))
                if message.contains("Remove glave's data folder")),
            "schema {found}: {:?}",
            result.err()
        );
        let raw = rusqlite::Connection::open(&path).unwrap();
        let version: u32 = raw
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, found, "the file is not changed");
    }
}

#[test]
fn a_new_file_gets_the_schema_and_opens_again() {
    let (_dir, path) = temp_db("fresh");
    block_on!(async {
        drop(
            Store::open(&path, file_keys(&path), objects())
                .await
                .unwrap(),
        );
        let again = Store::open(&path, file_keys(&path), objects())
            .await
            .unwrap();
        assert_eq!(conformance::search(&again.log()).await, Ok(()));
    });
    let raw = rusqlite::Connection::open(&path).unwrap();
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, schema::VERSION);
}

#[test]
fn folders_get_their_own_projects_in_one_organization() {
    let store = Store::open_in_memory(keys()).unwrap();
    block_on!(async {
        let org = store.org(Uuid::from_u128(1)).await.unwrap();
        let projects = store.projects();
        let path = |p: &str| [glave_ports::FolderKey::Path(p.into())];
        let a = projects
            .folder(org, &path("/work/a"), &record(2, &ada()))
            .await
            .unwrap();
        let b = projects
            .folder(org, &path("/work/b"), &record(4, &ada()))
            .await
            .unwrap();
        assert_ne!(a.id, b.id, "each folder is its own project");
        let scope = glave_ports::Scope::glaves_own(org, a.id);
        let query = ThreadQuery {
            standing: Standing::Active,
            limit: 10,
            after: None,
        };
        let threads = store.log().thread_views(&scope, &query).await.unwrap();
        assert!(threads.is_empty(), "a new project has no threads");
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
        .prepare("SELECT org, project, thread, view FROM threads ORDER BY org, project, thread")
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
    let (_dir, path) = temp_db("views");
    block_on!(async {
        let store = Store::open(&path, file_keys(&path), objects())
            .await
            .unwrap();
        assert_eq!(conformance::thread_views(&store.log()).await, Ok(()));
        drop(store);
        let kept = views(&path);
        assert_eq!(kept.len(), 4);

        // As if no fold had built them: no views, and no version recorded.
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch("DELETE FROM threads; DELETE FROM settings WHERE key = 'threads_view';")
            .unwrap();
        drop(connection);
        let _reopened = Store::open(&path, file_keys(&path), objects())
            .await
            .unwrap();
        assert_eq!(views(&path), kept, "opening fills the view from the log");
    });
}
