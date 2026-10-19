//! The database schema: one, made whole in a new database. Until glave 0.1 ships it changes in
//! place, with [`VERSION`] bumped, and a database of another version is refused (AGENTS.md,
//! "Types"). `glave_migrations` records the version.

use tokio_postgres::Client;

use crate::store::{StoreError, describe};

/// The schema this build makes and reads. Bumped whenever [`SCHEMA`] changes. It starts past
/// every version the migrations it replaced wrote, so a database one of them made is refused.
pub(crate) const VERSION: i32 = 9;

/// Every table, made in one transaction in a new database.
const SCHEMA: &str = "
    -- The event log. `pos` is the write order across threads, so searches can put the most
    -- recent first. `data` is the payload sealed under the data key `key_id` names. One event
    -- per position: a second writer fails on the key instead of scrambling a thread.
    CREATE TABLE events (
        pos      bigint      GENERATED ALWAYS AS IDENTITY,
        org      uuid        NOT NULL,
        project  uuid        NOT NULL,
        thread   uuid        NOT NULL,
        seq      bigint      NOT NULL CHECK (seq > 0),
        id       uuid        NOT NULL,
        at       timestamptz NOT NULL,
        type     text        NOT NULL,
        v        bigint      NOT NULL,
        glave    bigint      NOT NULL,
        data     bytea       NOT NULL,
        key_id   text        NOT NULL,
        PRIMARY KEY (org, project, thread, seq)
    );

    -- Threads found by id alone (`EventLog::locate`).
    CREATE INDEX events_by_thread ON events (thread);

    -- Blobs, sealed, stored under a keyed name derived from their hash, with the threads that
    -- use each.
    CREATE TABLE blobs (
        org         uuid  NOT NULL,
        project     uuid  NOT NULL,
        name        text  NOT NULL,
        media_type  text  NOT NULL,
        bytes       bytea NOT NULL,
        key_id      text  NOT NULL,
        PRIMARY KEY (org, project, name)
    );

    CREATE TABLE blob_threads (
        org      uuid NOT NULL,
        project  uuid NOT NULL,
        name     text NOT NULL,
        thread   uuid NOT NULL,
        PRIMARY KEY (org, project, name, thread)
    );

    -- The wrapped keys: one per organization, and one record of keys per project.
    CREATE TABLE org_keys (
        org      uuid  PRIMARY KEY,
        wrapped  bytea NOT NULL
    );

    CREATE TABLE project_keys (
        org       uuid   NOT NULL,
        project   uuid   NOT NULL,
        revision  bigint NOT NULL,
        record    bytea  NOT NULL,
        PRIMARY KEY (org, project)
    );

    -- Projects as records: a kind, a name, an owner (a principal as JSON text), members in
    -- order, and times; one personal project per person; and the folder keys (`path:<path>`
    -- or `remote:<url>`) that lead to them.
    CREATE TABLE project_records (
        org         uuid        NOT NULL,
        id          uuid        NOT NULL,
        kind        text        NOT NULL,
        name        text        NOT NULL,
        owner       text        NOT NULL,
        created_at  timestamptz NOT NULL,
        updated_at  timestamptz NOT NULL,
        -- Updates so far: each names the one it read, so none is lost (`Projects::update`).
        revision    bigint      NOT NULL,
        PRIMARY KEY (org, id)
    );

    CREATE UNIQUE INDEX one_personal_project ON project_records (org, owner)
        WHERE kind = 'personal';

    CREATE TABLE project_members (
        org       uuid   NOT NULL,
        project   uuid   NOT NULL,
        position  bigint NOT NULL,
        member    text   NOT NULL,
        PRIMARY KEY (org, project, position)
    );

    CREATE INDEX project_members_by_member ON project_members (org, member);

    CREATE TABLE project_folders (
        org      uuid NOT NULL,
        key      text NOT NULL,
        project  uuid NOT NULL,
        PRIMARY KEY (org, key)
    );

    -- The threads view: one row per thread, folded from its events as they are appended
    -- (`glave_ports::ThreadView`, as JSON), listed by standing, most recently updated first.
    -- Content-free. `updated` is when its latest event was recorded, in milliseconds; a page
    -- starts after the last row's own `(updated, thread)`, never a position across the store.
    CREATE TABLE threads (
        org       uuid   NOT NULL,
        project   uuid   NOT NULL,
        thread    uuid   NOT NULL,
        standing  text   NOT NULL,
        updated   bigint NOT NULL,
        view      text   NOT NULL,
        PRIMARY KEY (org, project, thread)
    );

    CREATE INDEX threads_by_update ON threads (org, project, standing, updated DESC, thread DESC);

    -- The organization, the server, the local person and their agent, and which fold built
    -- the threads view.
    CREATE TABLE glave_settings (
        key    text PRIMARY KEY,
        value  text NOT NULL
    );

    -- Keyword search over conversations (`specs/memory.md`, \"Recall\"): the words of each
    -- message and tool output, as keyed hashes under its project key (`Keys::search_terms`),
    -- so the index holds no word in the clear. Written in the same transaction as the events.
    CREATE TABLE event_search (
        pos      bigint   GENERATED ALWAYS AS IDENTITY,
        org      uuid     NOT NULL,
        project  uuid     NOT NULL,
        thread   uuid     NOT NULL,
        seq      bigint   NOT NULL,
        terms    tsvector NOT NULL,
        PRIMARY KEY (org, project, thread, seq)
    );
    CREATE INDEX event_search_terms ON event_search USING gin (terms);
";

/// Makes the schema in a new database, and checks an existing one is this build's. Openers
/// queue on a lock for the schema they share, so two never make it at once. A database of any
/// other version is refused with what to do about it, never changed.
pub(crate) async fn prepare(client: &mut Client) -> Result<(), StoreError> {
    let schema = |e: tokio_postgres::Error| StoreError::Schema(describe(&e));
    let transaction = client.transaction().await.map_err(schema)?;
    transaction
        .batch_execute(
            "SELECT pg_advisory_xact_lock(hashtext('glave_migrations:' || current_schema()));
             CREATE TABLE IF NOT EXISTS glave_migrations (
                 version  integer     PRIMARY KEY,
                 applied  timestamptz NOT NULL DEFAULT now()
             );",
        )
        .await
        .map_err(schema)?;
    let found: i32 = transaction
        .query_one(
            "SELECT COALESCE(MAX(version), 0) FROM glave_migrations",
            &[],
        )
        .await
        .map_err(schema)?
        .try_get(0)
        .map_err(schema)?;
    if found == VERSION {
        return transaction.commit().await.map_err(schema);
    }
    if found != 0 {
        return Err(StoreError::Schema(format!(
            "this database was set up by another version of glave, which this one cannot read \
             before glave 0.1 (schema {found}, not {VERSION}). Empty it (drop glave's tables, or \
             the schema they are in) and start glave again"
        )));
    }
    transaction.batch_execute(SCHEMA).await.map_err(schema)?;
    transaction
        .execute(
            "INSERT INTO glave_migrations (version) VALUES ($1)",
            &[&VERSION],
        )
        .await
        .map_err(schema)?;
    transaction.commit().await.map_err(schema)
}
