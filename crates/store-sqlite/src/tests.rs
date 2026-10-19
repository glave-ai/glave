use std::path::PathBuf;
use std::sync::Arc;

use glave_keys::{InMemoryRoot, KeyFileRoot, KeySettings};
use glave_ports::{EventLog, Projects as _, Standing, ThreadQuery, block_on, conformance};
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
    let path = temp_db("identity");
    block_on!(async {
        let store = Store::open(&path, file_keys(&path)).await.unwrap();
        let first = (principal(1), principal(2));
        assert_eq!(store.identity(first).await.unwrap(), first);
        assert_eq!(
            store.identity((principal(3), principal(4))).await.unwrap(),
            first,
            "a second identity never replaces the first"
        );
        drop(store);
        let reopened = Store::open(&path, file_keys(&path)).await.unwrap();
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
    let path = temp_db("reopen");
    block_on!(async {
        let store = Store::open(&path, file_keys(&path)).await.unwrap();
        assert_eq!(conformance::event_log(&store.log()).await, Ok(()));
        let org = store.org(Uuid::from_u128(1)).await.unwrap();
        let key = [glave_ports::FolderKey::Path("/work/app".into())];
        let project = store
            .projects()
            .folder(org, &key, &record(2, &ada()))
            .await
            .unwrap();
        drop(store);
        let reopened = Store::open(&path, file_keys(&path)).await.unwrap();
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

#[test]
fn projects_from_before_records_become_records_and_are_claimed() {
    let path = temp_db("projects-migration");
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
        let projects = store.projects();
        let app = glave_ports::ProjectId::from_uuid(Uuid::from_u128(2));
        let before = projects.get(org, app).await.unwrap().unwrap();
        assert_eq!(before.name, "app");
        assert_eq!(before.kind, glave_ports::ProjectKind::Project);
        assert!(!before.admits(&ada()), "no one's until claimed");
        assert_eq!(store.claim_projects(&ada()).await.unwrap(), 1);
        let found = projects
            .folder(
                org,
                &[glave_ports::FolderKey::Path("/work/app".into())],
                &record(9, &ada()),
            )
            .await
            .unwrap();
        assert_eq!(found.id, app, "the folder still leads to its project");
        assert!(found.admits(&ada()));
        let mut inbox = record(10, &ada());
        inbox.kind = glave_ports::ProjectKind::Personal;
        let personal = projects.personal(org, &inbox).await.unwrap();
        assert_eq!(
            personal.id,
            glave_ports::ProjectId::from_uuid(Uuid::from_u128(3)),
            "the stand-in personal project is the person's personal project, memories and all"
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
}

/// What a database at schema 5 kept under spaces' names reads the same under projects': its
/// records, members and folders through the store, and its search index row for row.
#[test]
fn a_database_at_schema_5_keeps_its_rows_under_projects_names() {
    let path = temp_db("projects-rename");
    let owner = projects::principal_text(&ada()).unwrap();
    let org = "00000000-0000-0000-0000-000000000001";
    let app = "00000000-0000-0000-0000-000000000002";
    {
        let mut connection = rusqlite::Connection::open(&path).unwrap();
        let transaction = connection.transaction().unwrap();
        for sql in schema::MIGRATIONS.iter().take(5) {
            transaction.execute_batch(sql).unwrap();
        }
        transaction.pragma_update(None, "user_version", 5).unwrap();
        transaction
            .execute_batch(&format!(
                "INSERT INTO settings (key, value) VALUES ('org', '{org}');
                 INSERT INTO space_records VALUES
                     ('{org}', '{app}', 'project', 'app', '{owner}', '2026-10-01T00:00:00.000Z',
                      '2026-10-01T00:00:00.000Z');
                 INSERT INTO space_members VALUES ('{org}', '{app}', 0, '{owner}');
                 INSERT INTO space_folders VALUES ('{org}', 'path:/work/app', '{app}');
                 INSERT INTO event_search (terms, org, space, thread, seq)
                     VALUES ('f00d', '{org}', '{app}', 't', 1);"
            ))
            .unwrap();
        transaction.commit().unwrap();
    }
    block_on!(async {
        let store = Store::open(&path, keys()).await.unwrap();
        let org = store.org(Uuid::max()).await.unwrap();
        let id = glave_ports::ProjectId::from_uuid(Uuid::from_u128(2));
        let kept = store.projects().get(org, id).await.unwrap().unwrap();
        assert_eq!((kept.name.as_str(), kept.members.len()), ("app", 1));
        let key = [glave_ports::FolderKey::Path("/work/app".into())];
        let projects = store.projects();
        let found = projects.folder(org, &key, &record(9, &ada())).await;
        assert_eq!(found.unwrap().id, id, "the folder still leads to it");
    });
    let raw = rusqlite::Connection::open(&path).unwrap();
    let searched: String = raw
        .query_row(
            "SELECT project FROM event_search WHERE event_search MATCH 'f00d'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(searched, app, "the search index kept its rows");
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
        let scope = glave_ports::Scope { org, project: a.id };
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
