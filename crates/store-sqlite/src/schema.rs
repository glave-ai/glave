//! The database schema, carried forward (`specs/storage.md`, "Migrations"): a new file is made
//! at the baseline and brought through every migration since, an older one through those it
//! lacks, all in one transaction with the version it reaches, SQLite's `user_version`. A file a
//! newer glave saved, or one from before the baseline, is refused and left as it was. Before a
//! migration that reshapes what the file holds, a copy of the file is kept beside it.

use std::path::Path;
use std::time::Duration;

use glave_ports::migrations::{Change, Migration, Refused, Schema};
use rusqlite::{Connection, ErrorCode, TransactionBehavior};

use crate::copy;
use crate::store::{BUSY, StoreError};

/// How long a process opening the file waits for another that is bringing it up to date: minutes,
/// since a migration is real work.
const MIGRATED_WITHIN: Duration = Duration::from_mins(5);

/// The schema this build makes and reads. A migration that has shipped is never edited: a
/// change to the schema is a new one, appended.
pub(crate) const SCHEMA: Schema<'static> = Schema {
    baseline: 18,
    made: BASELINE,
    migrations: &[Migration {
        change: Change::Adds,
        sql: SEALED_INDEX,
    }],
};

/// Every table as of version 18, but its unsealed search index, which 19 makes again sealed.
const BASELINE: &str = "
    -- The event log. `data` is the payload sealed under the data key `key_id` names.
    CREATE TABLE events (
        org      TEXT    NOT NULL,
        project  TEXT    NOT NULL,
        thread   TEXT    NOT NULL,
        seq      INTEGER NOT NULL,
        id       TEXT    NOT NULL,
        at       TEXT    NOT NULL,
        type     TEXT    NOT NULL,
        v        INTEGER NOT NULL,
        glave    INTEGER NOT NULL,
        data     BLOB    NOT NULL,
        key_id   TEXT    NOT NULL,
        UNIQUE (org, project, thread, seq)
    ) STRICT;

    -- Threads found by id alone (`EventLog::locate`).
    CREATE INDEX events_by_thread ON events (thread);

    -- Blobs' records: each under a keyed name derived from its hash, with its media type, size,
    -- the data key that sealed it and the object its bytes are kept as, in the objects beside
    -- the database (never in it); whether a thread ever referred to it, and when it was last
    -- uploaded with none (milliseconds), for the sweep; and the threads that refer to each.
    CREATE TABLE blobs (
        org         TEXT    NOT NULL,
        project     TEXT    NOT NULL,
        name        TEXT    NOT NULL,
        media_type  TEXT    NOT NULL,
        size        INTEGER NOT NULL,
        key_id      TEXT    NOT NULL,
        object      TEXT    NOT NULL,
        referred    INTEGER NOT NULL,
        uploaded    INTEGER NOT NULL,
        PRIMARY KEY (org, project, name)
    ) STRICT;

    CREATE TABLE blob_threads (
        org      TEXT NOT NULL,
        project  TEXT NOT NULL,
        name     TEXT NOT NULL,
        thread   TEXT NOT NULL,
        PRIMARY KEY (org, project, name, thread)
    ) STRICT;

    CREATE INDEX blob_threads_by_thread ON blob_threads (org, project, thread);

    -- The local settings: the organization, the server, the local person and their agent, and
    -- which fold built the threads view.
    CREATE TABLE settings (
        key    TEXT PRIMARY KEY,
        value  TEXT NOT NULL
    ) STRICT;

    -- The wrapped keys: one per organization, and one record of keys per project.
    CREATE TABLE org_keys (
        org      TEXT PRIMARY KEY,
        wrapped  BLOB NOT NULL
    ) STRICT;

    CREATE TABLE project_keys (
        org       TEXT    NOT NULL,
        project   TEXT    NOT NULL,
        revision  INTEGER NOT NULL,
        record    BLOB    NOT NULL,
        PRIMARY KEY (org, project)
    ) STRICT;

    -- Projects as records: a kind, a name, an owner (a principal as JSON) and times; one
    -- personal project per person; and the folder keys (`path:<path>` or `remote:<url>`) that
    -- lead to them.
    CREATE TABLE project_records (
        org         TEXT NOT NULL,
        id          TEXT NOT NULL,
        kind        TEXT NOT NULL,
        name        TEXT NOT NULL,
        owner       TEXT NOT NULL,
        created_at  TEXT NOT NULL,
        updated_at  TEXT NOT NULL,
        PRIMARY KEY (org, id)
    ) STRICT;

    CREATE UNIQUE INDEX one_personal_project ON project_records (org, owner)
        WHERE kind = 'personal';

    CREATE TABLE project_folders (
        org      TEXT NOT NULL,
        key      TEXT NOT NULL,
        project  TEXT NOT NULL,
        PRIMARY KEY (org, key)
    ) STRICT;

    -- The threads view: one row per thread, folded from its events as they are appended
    -- (`glave_ports::ThreadView`, as JSON), listed by standing, most recently updated first.
    -- Content-free. `updated` is when its latest event was recorded, in milliseconds; a page
    -- starts after the last row's own `(updated, thread)`, never a position across the store.
    -- `open` is whether a turn is open in it, for a server that starts to find those a glave
    -- before it left open; a subagent's thread has the thread and call that started it
    -- (`parent`, `parent_call`), which only it names.
    CREATE TABLE threads (
        org          TEXT    NOT NULL,
        project      TEXT    NOT NULL,
        thread       TEXT    NOT NULL,
        standing     TEXT    NOT NULL,
        updated      INTEGER NOT NULL,
        open         INTEGER NOT NULL,
        parent       TEXT,
        parent_call  TEXT,
        view         TEXT    NOT NULL,
        PRIMARY KEY (org, project, thread)
    ) STRICT;

    CREATE INDEX threads_by_update ON threads (org, project, standing, updated DESC, thread DESC);
    CREATE INDEX threads_open ON threads (org, project, standing, updated DESC, thread DESC)
        WHERE open;
    CREATE INDEX threads_by_parent ON threads (org, project, parent, parent_call)
        WHERE parent IS NOT NULL;

    -- Work in flight (`glave_ports::Leases`): what each lease keeps (`Leased::key`), in an
    -- organization, its holder, and when it lapses, in milliseconds on this file's clock.
    CREATE TABLE leases (
        org      TEXT    NOT NULL,
        key      TEXT    NOT NULL,
        holder   TEXT    NOT NULL,
        expires  INTEGER NOT NULL,
        PRIMARY KEY (org, key)
    ) STRICT;

    -- The directory sign-in finds people in (`specs/storage.md`, \"Sign-in is the one lookup
    -- before an organization is known\"), ids and hashes only. A person's organizations come in
    -- the order of their rows, the first joined first.
    CREATE TABLE identities (
        identity TEXT NOT NULL PRIMARY KEY,
        person   TEXT NOT NULL
    ) STRICT;

    CREATE TABLE memberships (
        joined INTEGER PRIMARY KEY,
        person TEXT NOT NULL,
        org    TEXT NOT NULL,
        UNIQUE (person, org)
    ) STRICT;
";

/// 19: the search index sealed (`specs/storage.md`, "Search index"). Each searchable event's
/// words, as keyed hashes under its project key (`Keys::seal_terms`), sealed under the data key
/// `key_id` names, written in the same transaction as the event; `pos` is the write order, never
/// reused (`AUTOINCREMENT`), so a search reads only the rows after the last it read. A file of
/// 18 loses its unsealed index, derived from the events, so its chats from before are not
/// searched.
const SEALED_INDEX: &str = "
    DROP TABLE IF EXISTS event_search;

    CREATE TABLE event_search (
        pos      INTEGER PRIMARY KEY AUTOINCREMENT,
        org      TEXT    NOT NULL,
        project  TEXT    NOT NULL,
        thread   TEXT    NOT NULL,
        seq      INTEGER NOT NULL,
        key_id   TEXT    NOT NULL,
        terms    BLOB    NOT NULL
    ) STRICT;
    CREATE INDEX event_search_by_pos ON event_search (org, project, pos);
    CREATE INDEX event_search_by_thread ON event_search (org, project, thread);
";

/// Brings the database up to [`SCHEMA`], making it the first time. `file` is where it is kept,
/// none for a database in memory.
pub(crate) fn prepare(connection: &mut Connection, file: Option<&Path>) -> Result<(), StoreError> {
    migrate(connection, file, &SCHEMA)
}

/// Brings the database up to `schema`: every step it lacks, in one transaction with the version
/// it reaches, so one that fails leaves it at the version it had. Before a step that reshapes
/// what it holds, `file` is copied beside itself.
pub(crate) fn migrate(
    connection: &mut Connection,
    file: Option<&Path>,
    schema: &Schema<'_>,
) -> Result<(), StoreError> {
    // Most opens find the file up to date and take no lock.
    if schema
        .plan(version(connection)?)
        .map_err(refused)?
        .steps
        .is_empty()
    {
        return Ok(());
    }
    // The write lock first, waiting for another process that is migrating the file, then the
    // version again: that process may have brought it forward meanwhile, and no step runs twice.
    let set = |connection: &Connection, wait| {
        connection
            .busy_timeout(wait)
            .map_err(|e| StoreError::schema(&e))
    };
    set(connection, MIGRATED_WITHIN)?;
    let began = connection.transaction_with_behavior(TransactionBehavior::Immediate);
    let transaction = began.map_err(|e| match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => StoreError::Schema(format!(
            "another glave was bringing glave's data up to date and had not finished within {}",
            glave_ports::spoken(MIGRATED_WITHIN)
        )),
        _ => StoreError::schema(&e),
    })?;
    set(&transaction, BUSY)?;
    let plan = schema.plan(version(&transaction)?).map_err(refused)?;
    if let (true, Some(file)) = (plan.reshapes, file) {
        copy::take(file)?;
    }
    for step in &plan.steps {
        transaction.execute_batch(step.sql).map_err(|e| {
            StoreError::Schema(format!(
                "could not bring glave's data up to date (schema {}): {e}",
                step.version
            ))
        })?;
    }
    if let Some(last) = plan.steps.last() {
        transaction
            .pragma_update(None, "user_version", last.version)
            .map_err(|e| StoreError::schema(&e))?;
    }
    transaction.commit().map_err(|e| StoreError::schema(&e))
}

/// The version the database is at, 0 for one with nothing in it yet.
fn version(connection: &Connection) -> Result<u32, StoreError> {
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| StoreError::schema(&e))
}

/// Why a database is not opened, with what to do about it.
fn refused(refused: Refused) -> StoreError {
    let what = match refused {
        Refused::Newer { .. } => "Update glave and start it again",
        Refused::Older { .. } => {
            "Remove glave's data folder (~/.glave unless you moved it) and start glave again"
        }
    };
    StoreError::Schema(format!("glave's data was {refused}. {what}"))
}
