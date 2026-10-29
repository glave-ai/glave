//! The database schema: one, made whole in a new database. Until glave 0.1 ships it changes in
//! place, with [`VERSION`] bumped, and a database of another version is refused (AGENTS.md,
//! "Types"). `glave_migrations` records the version.

use tokio_postgres::Client;

use crate::store::{StoreError, describe};

/// The schema this build makes and reads. Bumped whenever [`SCHEMA`] changes. It starts past
/// every version the migrations it replaced wrote, so a database one of them made is refused.
pub(crate) const VERSION: i32 = 19;

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

    -- A thread's latest event of a type, found without reading the rest (`EventLog::latest_of`).
    CREATE INDEX events_by_type ON events (org, project, thread, type, seq);

    -- Blobs' records: each under a keyed name derived from its hash, with its media type, size,
    -- the data key that sealed it and the object its bytes are kept as, in the deployment's
    -- objects (never in the database); whether a thread ever referred to it, and when it was
    -- last uploaded with none (milliseconds), for the sweep; and the threads that refer to each.
    CREATE TABLE blobs (
        org         uuid   NOT NULL,
        project     uuid   NOT NULL,
        name        text   NOT NULL,
        media_type  text   NOT NULL,
        size        bigint NOT NULL,
        key_id      text   NOT NULL,
        object      text   NOT NULL,
        referred    boolean NOT NULL,
        uploaded    bigint NOT NULL,
        PRIMARY KEY (org, project, name)
    );

    CREATE TABLE blob_threads (
        org      uuid NOT NULL,
        project  uuid NOT NULL,
        name     text NOT NULL,
        thread   uuid NOT NULL,
        PRIMARY KEY (org, project, name, thread)
    );

    CREATE INDEX blob_threads_by_thread ON blob_threads (org, project, thread);

    -- The organizations the store holds, ids only, each added with its key: what work that runs
    -- once per organization goes through. No organization's data, so no policy.
    CREATE TABLE glave_orgs (
        org  uuid PRIMARY KEY
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

    -- Projects as records: a kind, a name, an owner (a principal as JSON text) and times; one
    -- personal project per person; and the folder keys (`path:<path>` or `remote:<url>`) that
    -- lead to them.
    CREATE TABLE project_records (
        org         uuid        NOT NULL,
        id          uuid        NOT NULL,
        kind        text        NOT NULL,
        name        text        NOT NULL,
        owner       text        NOT NULL,
        created_at  timestamptz NOT NULL,
        updated_at  timestamptz NOT NULL,
        PRIMARY KEY (org, id)
    );

    CREATE UNIQUE INDEX one_personal_project ON project_records (org, owner)
        WHERE kind = 'personal';

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
    -- `open` is whether a turn is open in it, for a server that starts to find those a glave
    -- before it left open; a subagent's thread has the thread and call that started it
    -- (`parent`, `parent_call`), which only it names.
    CREATE TABLE threads (
        org          uuid    NOT NULL,
        project      uuid    NOT NULL,
        thread       uuid    NOT NULL,
        standing     text    NOT NULL,
        updated      bigint  NOT NULL,
        open         boolean NOT NULL,
        parent       uuid,
        parent_call  text,
        view         text    NOT NULL,
        PRIMARY KEY (org, project, thread)
    );

    CREATE INDEX threads_by_update ON threads (org, project, standing, updated DESC, thread DESC);
    CREATE INDEX threads_open ON threads (org, project, standing, updated DESC, thread DESC)
        WHERE open;
    CREATE INDEX threads_by_parent ON threads (org, project, parent, parent_call)
        WHERE parent IS NOT NULL;

    -- The organization, the server, the local person and their agent, and which fold built
    -- the threads view.
    CREATE TABLE glave_settings (
        key    text PRIMARY KEY,
        value  text NOT NULL
    );

    -- Work in flight (`glave_ports::Leases`): what each lease keeps (`Leased::key`), in an
    -- organization, its holder, and when it lapses, in milliseconds on the database's clock.
    CREATE TABLE leases (
        org      uuid   NOT NULL,
        key      text   NOT NULL,
        holder   uuid   NOT NULL,
        expires  bigint NOT NULL,
        PRIMARY KEY (org, key)
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

    -- Row-level security (`specs/storage.md`): every table of organizations' rows shows and
    -- takes only the rows of the organization the connection is set to (`glave.org`), and
    -- none while it is set to none. `FORCE`, since glave owns the tables it reads.
    CREATE FUNCTION glave_org() RETURNS uuid LANGUAGE sql STABLE
        AS $$ SELECT NULLIF(current_setting('glave.org', true), '')::uuid $$;
    DO $$
    DECLARE kept text;
    BEGIN
        FOREACH kept IN ARRAY ARRAY['events', 'blobs', 'blob_threads', 'org_keys',
            'project_keys', 'project_records', 'project_folders', 'threads', 'leases',
            'event_search']
        LOOP
            EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', kept);
            EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', kept);
            EXECUTE format('CREATE POLICY glave_org ON %I USING (org = glave_org())
                WITH CHECK (org = glave_org())', kept);
        END LOOP;
    END $$;
";

/// Refuses a connection whose role row-level security does not bind: a superuser, or a role
/// with `BYPASSRLS`, for which every organization's rows would be one.
pub(crate) async fn refuse_bypassing(client: &Client) -> Result<(), StoreError> {
    let bypasses: bool = client
        .query_one(
            "SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .and_then(|row| row.try_get(0))
        .map_err(|e| StoreError::Schema(describe(&e)))?;
    if bypasses {
        return Err(StoreError::Schema(
            "glave signs in to the database as a role that skips row-level security (a \
             superuser, or one with BYPASSRLS), so nothing would keep one organization's data \
             from another's. Give glave a role of its own without either, owning its tables, \
             and name it in the store's URL"
                .into(),
        ));
    }
    Ok(())
}

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
