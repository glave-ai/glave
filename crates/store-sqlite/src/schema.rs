//! The database schema: one, made whole in a new file. Until glave 0.1 ships it changes in
//! place, with [`VERSION`] bumped, and a file of another version is refused (AGENTS.md,
//! "Types"). The version is SQLite's `user_version`.

use rusqlite::Connection;

use crate::store::StoreError;

/// The schema this build makes and reads. Bumped whenever [`SCHEMA`] changes. It starts past
/// every version the migrations it replaced wrote, so a file one of them made is refused.
pub(crate) const VERSION: u32 = 15;

/// Every table, made in one transaction in a new file.
const SCHEMA: &str = "
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

    -- Projects as records: a kind, a name, an owner (a principal as JSON), members in order,
    -- and times; one personal project per person; and the folder keys (`path:<path>` or
    -- `remote:<url>`) that lead to them.
    CREATE TABLE project_records (
        org         TEXT NOT NULL,
        id          TEXT NOT NULL,
        kind        TEXT NOT NULL,
        name        TEXT NOT NULL,
        owner       TEXT NOT NULL,
        created_at  TEXT NOT NULL,
        updated_at  TEXT NOT NULL,
        -- Updates so far: each names the one it read, so none is lost (`Projects::update`).
        revision    INTEGER NOT NULL,
        PRIMARY KEY (org, id)
    ) STRICT;

    CREATE UNIQUE INDEX one_personal_project ON project_records (org, owner)
        WHERE kind = 'personal';

    CREATE TABLE project_members (
        org       TEXT    NOT NULL,
        project   TEXT    NOT NULL,
        position  INTEGER NOT NULL,
        member    TEXT    NOT NULL,
        PRIMARY KEY (org, project, position)
    ) STRICT;

    CREATE INDEX project_members_by_member ON project_members (org, member);

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

    -- Keyword search over conversations (`specs/memory.md`, \"Recall\"): the words of each
    -- message and tool output, as keyed hashes under its project key (`Keys::search_terms`),
    -- so the index holds no word in the clear. Written in the same transaction as the events.
    CREATE VIRTUAL TABLE event_search USING fts5(
        terms,
        org UNINDEXED,
        project UNINDEXED,
        thread UNINDEXED,
        seq UNINDEXED,
        tokenize = 'ascii'
    );
";

/// Makes the schema in a new file, and checks an existing one is this build's. A file of any
/// other version is refused with what to do about it, never changed.
pub(crate) fn prepare(connection: &mut Connection) -> Result<(), StoreError> {
    let found: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| StoreError::schema(&e))?;
    if found == VERSION {
        return Ok(());
    }
    if found != 0 {
        return Err(StoreError::Schema(format!(
            "glave's data was saved by another version of glave, which this one cannot read \
             before glave 0.1 (schema {found}, not {VERSION}). Remove glave's data folder \
             (~/.glave unless you moved it) and start glave again"
        )));
    }
    let transaction = connection
        .transaction()
        .map_err(|e| StoreError::schema(&e))?;
    transaction
        .execute_batch(SCHEMA)
        .map_err(|e| StoreError::schema(&e))?;
    transaction
        .pragma_update(None, "user_version", VERSION)
        .map_err(|e| StoreError::schema(&e))?;
    transaction.commit().map_err(|e| StoreError::schema(&e))
}
