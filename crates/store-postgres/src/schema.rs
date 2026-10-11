//! The database schema and its migrations. Each migration runs once, in order, inside one
//! transaction; `glave_migrations` records every version applied.

use tokio_postgres::Client;

use crate::store::{StoreError, describe};

/// Every migration ever shipped, in order. Never edit or remove one; add the next.
const MIGRATIONS: &[&str] = &[
    // 1: the event log and the blob store.
    "
    CREATE TABLE events (
        -- Write order across threads, so listings can put the most recent first.
        pos     bigint      GENERATED ALWAYS AS IDENTITY,
        org     uuid        NOT NULL,
        space   uuid        NOT NULL,
        thread  uuid        NOT NULL,
        seq     bigint      NOT NULL CHECK (seq > 0),
        id      uuid        NOT NULL,
        at      timestamptz NOT NULL,
        type    text        NOT NULL,
        v       bigint      NOT NULL,
        glave   bigint      NOT NULL,
        -- The payload exactly as written. Sealed with `key_id` once encryption lands;
        -- NULL `key_id` means stored as written.
        data    bytea       NOT NULL,
        key_id  text,
        -- One event per position: a second writer fails here instead of scrambling a thread.
        PRIMARY KEY (org, space, thread, seq)
    );

    CREATE TABLE blobs (
        org         uuid  NOT NULL,
        space       uuid  NOT NULL,
        sha256      text  NOT NULL,
        media_type  text  NOT NULL,
        bytes       bytea NOT NULL,
        PRIMARY KEY (org, space, sha256)
    );
    ",
    // 2: sealing. Blobs are stored under a name derived from their hash, sealed with
    // `key_id` (NULL: stored as written, before sealing); the threads that use each blob; and
    // the wrapped keys: one per organization, and one record of keys per space.
    "
    ALTER TABLE blobs RENAME COLUMN sha256 TO name;
    ALTER TABLE blobs ADD COLUMN key_id text;

    CREATE TABLE blob_threads (
        org     uuid NOT NULL,
        space   uuid NOT NULL,
        name    text NOT NULL,
        thread  uuid NOT NULL,
        PRIMARY KEY (org, space, name, thread)
    );

    CREATE TABLE org_keys (
        org      uuid  PRIMARY KEY,
        wrapped  bytea NOT NULL
    );

    CREATE TABLE space_keys (
        org       uuid   NOT NULL,
        space     uuid   NOT NULL,
        revision  bigint NOT NULL,
        record    bytea  NOT NULL,
        PRIMARY KEY (org, space)
    );
    ",
    // 3: spaces as records: kind, name, owner (a principal as JSON text), members in order,
    // and times; the folder keys (`path:<path>` or `remote:<url>`) that lead to them; one
    // personal space per person; and threads found by id alone through an index.
    "
    CREATE INDEX events_by_thread ON events (thread);

    CREATE TABLE space_records (
        org         uuid        NOT NULL,
        id          uuid        NOT NULL,
        kind        text        NOT NULL,
        name        text        NOT NULL,
        owner       text        NOT NULL,
        created_at  timestamptz NOT NULL,
        updated_at  timestamptz NOT NULL,
        PRIMARY KEY (org, id)
    );

    CREATE UNIQUE INDEX one_personal_space ON space_records (org, owner)
        WHERE kind = 'personal';

    CREATE TABLE space_members (
        org       uuid   NOT NULL,
        space     uuid   NOT NULL,
        position  bigint NOT NULL,
        member    text   NOT NULL,
        PRIMARY KEY (org, space, position)
    );

    CREATE INDEX space_members_by_member ON space_members (org, member);

    CREATE TABLE space_folders (
        org    uuid NOT NULL,
        key    text NOT NULL,
        space  uuid NOT NULL,
        PRIMARY KEY (org, key)
    );
    ",
    // 4: the threads view: one row per thread, folded from its events as they are appended
    // (`glave_ports::ThreadView`, as JSON), listed by standing, most recently written first.
    // Content-free. The store fills it from the log when it connects (`views::refresh`), and
    // records which fold built it in `glave_settings`.
    "
    CREATE TABLE threads (
        org       uuid   NOT NULL,
        space     uuid   NOT NULL,
        thread    uuid   NOT NULL,
        standing  text   NOT NULL,
        -- The `pos` of the thread's latest event: the order of writes.
        position  bigint NOT NULL,
        view      text   NOT NULL,
        PRIMARY KEY (org, space, thread)
    );

    CREATE INDEX threads_by_position ON threads (org, space, standing, position DESC);

    CREATE TABLE glave_settings (
        key    text PRIMARY KEY,
        value  text NOT NULL
    );
    ",
    // 5: keyword search over conversations (`specs/memory.md`, "Recall"): the words of each
    // message and tool output, as keyed hashes under its space key (`Keys::search_terms`), so
    // the index holds no word in the clear. Written in the same transaction as the events.
    "
    CREATE TABLE event_search (
        pos     bigint   GENERATED ALWAYS AS IDENTITY,
        org     uuid     NOT NULL,
        space   uuid     NOT NULL,
        thread  uuid     NOT NULL,
        seq     bigint   NOT NULL,
        terms   tsvector NOT NULL,
        PRIMARY KEY (org, space, thread, seq)
    );
    CREATE INDEX event_search_terms ON event_search USING gin (terms);
    ",
];

/// The schema version this build writes.
pub(crate) fn current() -> i32 {
    i32::try_from(MIGRATIONS.len()).unwrap_or(i32::MAX)
}

/// Brings the database up to the current schema. Openers queue on a lock for the schema they
/// share, so two never migrate at once. A database from a newer glave is refused rather than
/// misread.
pub(crate) async fn migrate(client: &mut Client) -> Result<(), StoreError> {
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
    let current = current();
    if found > current {
        return Err(StoreError::Schema(format!(
            "this database uses schema {found}, written by a newer glave; this one knows up to {current}"
        )));
    }
    for (version, sql) in (1_i32..)
        .zip(MIGRATIONS)
        .skip(usize::try_from(found).unwrap_or(0))
    {
        transaction.batch_execute(sql).await.map_err(schema)?;
        transaction
            .execute(
                "INSERT INTO glave_migrations (version) VALUES ($1)",
                &[&version],
            )
            .await
            .map_err(schema)?;
    }
    transaction.commit().await.map_err(schema)
}
