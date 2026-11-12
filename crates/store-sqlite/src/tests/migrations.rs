//! A file carried forward (`specs/storage.md`, "Migrations"): from each earlier version, with
//! what it held; refused, unchanged, when a newer glave saved it or it is from before the
//! baseline; left at its version when a migration fails; copied first when one reshapes it; and
//! migrated once by processes opening it at once.

use std::path::Path;
use std::sync::Barrier;

use glave_ports::conformance::{StoreSuite, store_suite};
use glave_ports::migrations::{Change, Migration, Schema};
use rusqlite::Connection;

use super::*;
use crate::copy::copy_of;
use crate::schema::{self, SCHEMA};

/// The version the file at `path` is at.
fn version(path: &Path) -> u32 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

/// A file at `version` of glave's schema, made by its own migrations, holding a setting.
fn file_at(version: u32, path: &Path) {
    let count = usize::try_from(version - SCHEMA.baseline).unwrap();
    let earlier = Schema {
        migrations: &SCHEMA.migrations[..count],
        ..SCHEMA
    };
    let mut connection = Connection::open(path).unwrap();
    schema::migrate(&mut connection, Some(path), &earlier).unwrap();
    connection
        .execute(
            "INSERT INTO settings (key, value) VALUES ('kept', 'yes')",
            [],
        )
        .unwrap();
}

/// Each earlier version, built from the recorded migrations, opens at the latest and passes
/// every suite of a whole store, each on a file of its own.
#[test]
fn a_file_of_each_earlier_version_is_brought_up_to_date_keeping_what_it_held() {
    for version_then in SCHEMA.baseline..SCHEMA.latest() {
        for suite in StoreSuite::ALL {
            let (_dir, path) = temp_db(&format!("from-{version_then}-{suite:?}"));
            file_at(version_then, &path);
            block_on!(async {
                let store = Store::open(&path, file_keys(&path), objects())
                    .await
                    .unwrap();
                let (log, projects) = (store.log(), store.projects());
                let (directory, leases) = (store.directory(), store.leases());
                let verdict = store_suite(suite, &log, &projects, &directory, &leases).await;
                assert_eq!(verdict, Ok(()), "{suite:?}");
            });
            assert_eq!(version(&path), SCHEMA.latest());
            let kept: String = Connection::open(&path)
                .unwrap()
                .query_row("SELECT value FROM settings WHERE key = 'kept'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                kept, "yes",
                "from {version_then}, what the file held is kept"
            );
        }
    }
}

/// A file glave saved at 18 had its search index unsealed, which 19 makes again sealed.
#[test]
fn a_file_saved_at_18_loses_its_unsealed_index_for_the_sealed_one() {
    let (_dir, path) = temp_db("unsealed");
    file_at(18, &path);
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE VIRTUAL TABLE event_search USING fts5(terms, org UNINDEXED,
                 project UNINDEXED, thread UNINDEXED, seq UNINDEXED, tokenize = 'ascii');
             INSERT INTO event_search VALUES ('0123abcd', 'o', 'p', 't', 1);",
        )
        .unwrap();
    block_on!(async {
        let store = Store::open(&path, file_keys(&path), objects())
            .await
            .unwrap();
        assert_eq!(conformance::search(&store.log()).await, Ok(()));
    });
    let unsealed: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE sql LIKE '%fts5%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unsealed, 0, "the unsealed index is gone");
}

#[test]
fn a_new_file_is_made_at_the_latest_version_and_opens_again() {
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
    assert_eq!(version(&path), SCHEMA.latest());
    assert!(!copy_of(&path).exists(), "a new file is never copied");
}

/// A file a newer glave saved is never downgraded, and one from before the baseline is never
/// guessed at: each is refused with what to do, and left as it was.
#[test]
fn a_file_a_newer_glave_saved_or_from_before_the_baseline_is_refused_and_left_as_it_was() {
    let cases = [
        (SCHEMA.latest() + 1, "Update glave"),
        (SCHEMA.baseline - 1, "Remove glave's data folder"),
    ];
    for (found, what) in cases {
        let (_dir, path) = temp_db(&format!("refused-{found}"));
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", found)
            .unwrap();
        let result = block_on!(Store::open(&path, keys(), objects()));
        assert!(
            matches!(&result, Err(StoreError::Schema(message)) if message.contains(what)),
            "schema {found}: {:?}",
            result.err()
        );
        assert_eq!(version(&path), found, "the file is not changed");
        assert!(!copy_of(&path).exists());
    }
}

/// Notes, at version 1 of a schema of the test's own.
const NOTES: &str = "CREATE TABLE notes (text TEXT NOT NULL) STRICT;";

/// 2: notes' text renamed.
const RENAMED: Migration = Migration {
    change: Change::Reshapes,
    sql: "ALTER TABLE notes RENAME COLUMN text TO body;",
};

/// 3: notes indexed, which only adds.
const INDEXED: Migration = Migration {
    change: Change::Adds,
    sql: "CREATE INDEX notes_by_body ON notes (body);",
};

/// 4: notes dated, which fails if it runs twice.
const DATED: Migration = Migration {
    change: Change::Reshapes,
    sql: "ALTER TABLE notes ADD COLUMN at INTEGER NOT NULL DEFAULT 0;",
};

fn notes(migrations: &[Migration]) -> Schema<'_> {
    Schema {
        baseline: 1,
        made: NOTES,
        migrations,
    }
}

/// A file of notes at version 1 holding one, and the connection that made it.
fn notes_file(path: &Path) -> Connection {
    let mut connection = Connection::open(path).unwrap();
    schema::migrate(&mut connection, Some(path), &notes(&[])).unwrap();
    connection
        .execute("INSERT INTO notes (text) VALUES ('first')", [])
        .unwrap();
    connection
}

/// The names in the folder `path` is in, sorted.
fn beside(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_failed_migration_leaves_the_file_at_its_version_with_none_of_its_steps() {
    let (_dir, path) = temp_db("failed");
    let mut connection = notes_file(&path);
    let failing = Migration {
        change: Change::Adds,
        sql: "CREATE TABLE tags (name TEXT) STRICT; INSERT INTO missing VALUES (1);",
    };
    let failed = schema::migrate(&mut connection, Some(&path), &notes(&[RENAMED, failing]));
    assert!(
        matches!(&failed, Err(StoreError::Schema(message))
            if message.starts_with("could not bring glave's data up to date (schema 3)")),
        "{failed:?}"
    );
    drop(connection);
    assert_eq!(version(&path), 1);
    let raw = Connection::open(&path).unwrap();
    let text: String = raw
        .query_row("SELECT text FROM notes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(text, "first", "the step before it is undone too");
    let tables: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'tags'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
}

#[test]
fn a_migration_that_reshapes_keeps_one_copy_of_the_file_from_before_it() {
    let (_dir, path) = temp_db("copied");
    let mut connection = notes_file(&path);
    let copy = copy_of(&path);
    schema::migrate(&mut connection, Some(&path), &notes(&[RENAMED])).unwrap();
    assert_eq!(version(&copy), 1, "copied before it was reshaped");
    let text: String = Connection::open(&copy)
        .unwrap()
        .query_row("SELECT text FROM notes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(text, "first");

    schema::migrate(&mut connection, Some(&path), &notes(&[RENAMED, INDEXED])).unwrap();
    assert_eq!(version(&copy), 1, "one that only adds takes no copy");

    // As a copy that stopped partway, when glave was killed taking it, leaves.
    glave_ports::file::create_until(&copy, b"half", glave_ports::file::Stop::Written).unwrap();
    let all = [RENAMED, INDEXED, DATED];
    schema::migrate(&mut connection, Some(&path), &notes(&all)).unwrap();
    assert_eq!(version(&path), 4);
    assert_eq!(version(&copy), 3, "the next copy replaces the one before");
    assert_eq!(
        beside(&path),
        ["glave.db", "glave.db.backup"],
        "one copy, and nothing half written"
    );
}

/// A store on a fresh file at `path`, on `clock`, with a copy beside it from before a migration.
fn copied(path: &Path, clock: &glave_ports::testing::ManualClock) -> Store {
    let mut settings = file_keys(path);
    settings.clock = Arc::new(clock.clone());
    let store = block_on!(Store::open(path, settings, objects())).unwrap();
    std::fs::write(copy_of(path), b"what the file held before").unwrap();
    store
}

/// The copy still holds every key the file held, so what is erased after it was taken would
/// open from it again: destroying a key, or rotating a project key, lets it go, and making one
/// does not (`invariants.md`, 19).
#[test]
fn destroying_a_key_lets_go_of_the_copy_and_making_one_does_not() {
    use glave_ports::{Erasable, Eraser as _};
    let (_dir, path) = temp_db("erased");
    let store = copied(&path, &glave_ports::testing::ManualClock::at_millis(0));
    let scope = Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(2)),
    );
    let bytes = Box::new(Pieces::from(b"a logo".to_vec()));
    block_on!(store.blobs().put(&scope, None, "image/png", 1024, bytes)).unwrap();
    assert!(copy_of(&path).exists(), "a key made keeps the copy");
    block_on!(store.keys().erase_project(&scope)).unwrap();
    assert!(!copy_of(&path).exists(), "a project erased lets it go");

    std::fs::write(copy_of(&path), b"a copy taken again").unwrap();
    let account = Erasable::Account(Uuid::from_u128(5));
    block_on!(store.keyring().destroy(scope, account)).unwrap();
    assert!(!copy_of(&path).exists(), "an account removed lets it go");
}

#[test]
fn a_copy_a_week_old_is_let_go_of() {
    let (_dir, path) = temp_db("aged");
    let clock = glave_ports::testing::ManualClock::at_millis(0);
    let store = copied(&path, &clock);
    let taken = std::fs::metadata(copy_of(&path))
        .unwrap()
        .modified()
        .unwrap();
    let taken = jiff::Timestamp::try_from(taken).unwrap().as_millisecond();
    let day = 24 * 3_600 * 1_000;
    clock.set_millis(taken + 7 * day - 1);
    assert!(!block_on!(store.forget_old_copy()).unwrap());
    assert!(copy_of(&path).exists(), "kept for a week");
    // A millisecond past, as the file's time is finer than the clock's.
    clock.set_millis(taken + 7 * day + 1);
    assert!(block_on!(store.forget_old_copy()).unwrap());
    assert!(!copy_of(&path).exists(), "let go of after it");
}

#[test]
fn every_migration_keeps_the_rules_of_a_migration() {
    assert_eq!(SCHEMA.broken_rule(), None);
}

/// Processes opening one older file at once, as a person's server and a glave run beside it
/// after an update: one migrates it, and the others find it brought forward.
#[test]
fn processes_opening_one_older_file_at_once_migrate_it_once() {
    let (_dir, path) = temp_db("racing");
    notes_file(&path)
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    let schema = notes(&[DATED]);
    // Every connection opened first, as the store opens its own before it migrates, so nothing
    // a thread does before the barrier can fail and leave the others waiting at it.
    let connections: Vec<Connection> = (0..4)
        .map(|_| {
            let connection = Connection::open(&path).unwrap();
            connection.busy_timeout(store::BUSY).unwrap();
            connection
        })
        .collect();
    let starting = Barrier::new(connections.len());
    let results: Vec<Result<(), StoreError>> = std::thread::scope(|threads| {
        let opening: Vec<_> = connections
            .into_iter()
            .map(|mut connection| {
                let (path, schema, starting) = (&path, &schema, &starting);
                threads.spawn(move || {
                    starting.wait();
                    schema::migrate(&mut connection, Some(path), schema)
                })
            })
            .collect();
        opening.into_iter().map(|one| one.join().unwrap()).collect()
    });
    for result in &results {
        assert!(result.is_ok(), "{result:?}");
    }
    assert_eq!(version(&path), 2);
}
