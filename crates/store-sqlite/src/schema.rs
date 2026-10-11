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
    // 3: spaces as records. Each space has a kind, a name, an owner (a principal as JSON; ''
    // for a space made before records, until the local person claims it), members, and
    // times. Folders lead to spaces through `space_folders` (`path:<path>` or
    // `remote:<url>`), so a folder is an index, not the space. The old path-to-space table's
    // rows become records and folder keys; the stand-in personal spaces (`glave:personal/<id>`)
    // become their person's personal space. Threads are found by id alone through an index.
    "
    CREATE INDEX events_by_thread ON events (thread);

    CREATE TABLE space_records (
        org         TEXT NOT NULL,
        id          TEXT NOT NULL,
        kind        TEXT NOT NULL,
        name        TEXT NOT NULL,
        owner       TEXT NOT NULL,
        created_at  TEXT NOT NULL,
        updated_at  TEXT NOT NULL,
        PRIMARY KEY (org, id)
    ) STRICT;

    CREATE UNIQUE INDEX one_personal_space ON space_records (org, owner)
        WHERE kind = 'personal';

    CREATE TABLE space_members (
        org       TEXT    NOT NULL,
        space     TEXT    NOT NULL,
        position  INTEGER NOT NULL,
        member    TEXT    NOT NULL,
        PRIMARY KEY (org, space, position)
    ) STRICT;

    CREATE INDEX space_members_by_member ON space_members (org, member);

    CREATE TABLE space_folders (
        org    TEXT NOT NULL,
        key    TEXT NOT NULL,
        space  TEXT NOT NULL,
        PRIMARY KEY (org, key)
    ) STRICT;

    INSERT INTO space_records (org, id, kind, name, owner, created_at, updated_at)
    SELECT
        (SELECT value FROM settings WHERE key = 'org'),
        id,
        CASE WHEN path LIKE 'glave:personal/%' THEN 'personal' ELSE 'project' END,
        CASE WHEN path LIKE 'glave:personal/%' THEN 'Inbox'
             ELSE COALESCE(NULLIF(replace(path, rtrim(path, replace(path, '/', '')), ''), ''), path)
        END,
        CASE WHEN path LIKE 'glave:personal/%'
             THEN '{\"kind\":\"person\",\"id\":\"' || substr(path, 16) || '\"}'
             ELSE ''
        END,
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    FROM spaces
    WHERE EXISTS (SELECT 1 FROM settings WHERE key = 'org');

    INSERT INTO space_folders (org, key, space)
    SELECT (SELECT value FROM settings WHERE key = 'org'), 'path:' || path, id
    FROM spaces
    WHERE path NOT LIKE 'glave:personal/%'
      AND EXISTS (SELECT 1 FROM settings WHERE key = 'org');

    DROP TABLE spaces;
    ",
    // 4: the threads view: one row per thread, folded from its events as they are appended
    // (`glave_ports::ThreadView`, as JSON), listed by standing, most recently written first.
    // Content-free. The store fills it from the log when it opens (`views::refresh`).
    "
    CREATE TABLE threads (
        org       TEXT    NOT NULL,
        space     TEXT    NOT NULL,
        thread    TEXT    NOT NULL,
        standing  TEXT    NOT NULL,
        -- The rowid of the thread's latest event: the order of writes.
        position  INTEGER NOT NULL,
        view      TEXT    NOT NULL,
        PRIMARY KEY (org, space, thread)
    ) STRICT;

    CREATE INDEX threads_by_position ON threads (org, space, standing, position DESC);
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
