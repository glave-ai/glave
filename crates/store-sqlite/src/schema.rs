//! The database schema and its migrations. Each migration runs once, in order, inside a
//! transaction; the schema version is SQLite's `user_version`.

use rusqlite::Connection;

use crate::store::StoreError;

/// Every migration ever shipped, in order. Never edit or remove one; add the next.
pub(crate) const MIGRATIONS: &[&str] = &[
    // 1: the event log, the blob store, and the local settings and spaces.
    "
    CREATE TABLE events (
        org     TEXT    NOT NULL,
        space   TEXT    NOT NULL,
        thread  TEXT    NOT NULL,
        seq     INTEGER NOT NULL,
        id      TEXT    NOT NULL,
        at      TEXT    NOT NULL,
        type    TEXT    NOT NULL,
        v       INTEGER NOT NULL,
        glave   INTEGER NOT NULL,
        -- The payload exactly as written. Sealed with `key_id` once encryption lands;
        -- NULL `key_id` means stored as written.
        data    BLOB    NOT NULL,
        key_id  TEXT,
        UNIQUE (org, space, thread, seq)
    ) STRICT;

    CREATE TABLE blobs (
        org         TEXT NOT NULL,
        space       TEXT NOT NULL,
        sha256      TEXT NOT NULL,
        media_type  TEXT NOT NULL,
        bytes       BLOB NOT NULL,
        PRIMARY KEY (org, space, sha256)
    ) STRICT;

    CREATE TABLE settings (
        key    TEXT PRIMARY KEY,
        value  TEXT NOT NULL
    ) STRICT;

    CREATE TABLE spaces (
        path  TEXT PRIMARY KEY,
        id    TEXT NOT NULL UNIQUE
    ) STRICT;
    ",
    // 2: sealing. Blobs are stored under a name derived from their hash, sealed with
    // `key_id` (NULL: stored as written, before sealing); the threads that use each blob; and
    // the wrapped keys: one per organization, and one record of keys per space.
    "
    ALTER TABLE blobs RENAME COLUMN sha256 TO name;
    ALTER TABLE blobs ADD COLUMN key_id TEXT;

    CREATE TABLE blob_threads (
        org     TEXT NOT NULL,
        space   TEXT NOT NULL,
        name    TEXT NOT NULL,
        thread  TEXT NOT NULL,
        PRIMARY KEY (org, space, name, thread)
    ) STRICT;

    CREATE TABLE org_keys (
        org      TEXT PRIMARY KEY,
        wrapped  BLOB NOT NULL
    ) STRICT;

    CREATE TABLE space_keys (
        org       TEXT    NOT NULL,
        space     TEXT    NOT NULL,
        revision  INTEGER NOT NULL,
        record    BLOB    NOT NULL,
        PRIMARY KEY (org, space)
    ) STRICT;
    ",
    // 3: keyword search over conversations (`specs/memory.md`, "Recall"): the words of each
    // message and tool output, as keyed hashes under its space key (`Keys::search_terms`), so
    // the index holds no word in the clear. Written in the same transaction as the events.
    "
    CREATE VIRTUAL TABLE event_search USING fts5(
        terms,
        org UNINDEXED,
        space UNINDEXED,
        thread UNINDEXED,
        seq UNINDEXED,
        tokenize = 'ascii'
    );
    ",
];

/// The schema version this build writes.
pub(crate) fn current() -> u32 {
    u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX)
}

/// Brings the database up to the current schema. A database from a newer glave is refused
/// rather than misread.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let found: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| StoreError::schema(&e))?;
    let current = current();
    if found > current {
        return Err(StoreError::Schema(format!(
            "this database uses schema {found}, written by a newer glave; this one knows up to {current}"
        )));
    }
    let transaction = connection
        .transaction()
        .map_err(|e| StoreError::schema(&e))?;
    for (index, sql) in MIGRATIONS
        .iter()
        .enumerate()
        .skip(usize::try_from(found).unwrap_or(usize::MAX))
    {
        transaction
            .execute_batch(sql)
            .map_err(|e| StoreError::schema(&e))?;
        let version = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
        transaction
            .pragma_update(None, "user_version", version)
            .map_err(|e| StoreError::schema(&e))?;
    }
    transaction.commit().map_err(|e| StoreError::schema(&e))
}
