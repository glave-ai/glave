//! The database schema, carried forward (`specs/storage.md`, "Migrations"): a new database is
//! made at the baseline and brought through every migration since, an older one through those
//! it lacks, all in one transaction that records each version in `glave_migrations`. Servers
//! starting at once queue on a lock, so one migrates and the rest find it done. A database a
//! newer glave set up, or one from before the baseline, is refused and left as it was.

use std::time::Duration;

use glave_ports::migrations::{Change, Migration, Refused, Schema};
use tokio_postgres::Client;
use tokio_postgres::error::SqlState;

use crate::store::{StoreError, describe};

/// The schema this build makes and reads. A migration that has shipped is never edited: a
/// change to the schema is a new one, appended.
pub const SCHEMA: Schema<'static> = Schema {
    baseline: 20,
    made: BASELINE,
    migrations: &[Migration {
        change: Change::Adds,
        sql: SEALED_INDEX,
    }],
};

/// How long a server waits for another to finish bringing the database up to date: minutes,
/// since a migration is real work.
const MIGRATED_WITHIN: Duration = Duration::from_mins(5);

/// How long a migration waits for a table it changes that servers of the release before are
/// using: moments, since every lock they take is held for one statement or transaction, and a
/// migration waiting for one holds up every query of that table behind it.
const TABLES_WITHIN: Duration = Duration::from_secs(5);

/// Every table as of version 20, but its unsealed search index, which 21 makes again sealed.
const BASELINE: &str = "
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

    -- Row-level security (`specs/storage.md`): every table of organizations' rows shows and
    -- takes only the rows of the organization the connection is set to (`glave.org`), and
    -- none while it is set to none. `FORCE`, since glave owns the tables it reads.
    CREATE FUNCTION glave_org() RETURNS uuid LANGUAGE sql STABLE
        AS $$ SELECT NULLIF(current_setting('glave.org', true), '')::uuid $$;
    DO $$
    DECLARE kept text;
    BEGIN
        FOREACH kept IN ARRAY ARRAY['events', 'blobs', 'blob_threads', 'org_keys',
            'project_keys', 'project_records', 'project_folders', 'threads', 'leases']
        LOOP
            EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', kept);
            EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', kept);
            EXECUTE format('CREATE POLICY glave_org ON %I USING (org = glave_org())
                WITH CHECK (org = glave_org())', kept);
        END LOOP;
    END $$;

    -- The directory sign-in finds people in (`specs/storage.md`, \"Sign-in is the one lookup
    -- before an organization is known\"), ids and hashes only, read before anyone's
    -- organization is known, so no policy, as `glave_orgs` has none. A person's organizations
    -- come in the order they joined.
    CREATE TABLE identities (
        identity  text PRIMARY KEY,
        person    uuid NOT NULL
    );

    CREATE TABLE memberships (
        joined  bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
        person  uuid   NOT NULL,
        org     uuid   NOT NULL,
        UNIQUE (person, org)
    );
";

/// 21: the search index sealed (`specs/storage.md`, "Search index"). Each searchable event's
/// words, as keyed hashes under its project key (`Keys::seal_terms`), sealed under the data key
/// `key_id` names, written in the same transaction as the event; `pos` is the write order, so a
/// search reads only the rows after the last it read. A database of 20 loses its unsealed index,
/// derived from the events, so its chats from before are not searched.
const SEALED_INDEX: &str = "
    DROP TABLE IF EXISTS event_search;

    CREATE TABLE event_search (
        pos      bigint GENERATED ALWAYS AS IDENTITY,
        org      uuid   NOT NULL,
        project  uuid   NOT NULL,
        thread   uuid   NOT NULL,
        seq      bigint NOT NULL,
        key_id   text   NOT NULL,
        terms    bytea  NOT NULL,
        PRIMARY KEY (org, project, thread, seq)
    );
    CREATE INDEX event_search_by_pos ON event_search (org, project, pos);
    ALTER TABLE event_search ENABLE ROW LEVEL SECURITY;
    ALTER TABLE event_search FORCE ROW LEVEL SECURITY;
    CREATE POLICY glave_org ON event_search USING (org = glave_org())
        WITH CHECK (org = glave_org());
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

/// Brings the database in `client`'s search path up to `schema` ([`SCHEMA`] but in tests),
/// making it the first time: every step it lacks, in one transaction that records each version
/// it reaches, so one that fails leaves it at the version it had. Servers starting at once queue
/// on a lock for the schema they share, taken before the version is read, so no step runs twice.
pub async fn migrate(client: &mut Client, schema: &Schema<'_>) -> Result<(), StoreError> {
    let failed = |e: tokio_postgres::Error| StoreError::Schema(describe(&e));
    let transaction = client.transaction().await.map_err(failed)?;
    transaction
        .batch_execute(&format!(
            "SET LOCAL lock_timeout = {};
             SELECT pg_advisory_xact_lock(hashtext('glave_migrations:' || current_schema()));
             SET LOCAL lock_timeout = {};
             CREATE TABLE IF NOT EXISTS glave_migrations (
                 version  integer     PRIMARY KEY,
                 applied  timestamptz NOT NULL DEFAULT now()
             );",
            MIGRATED_WITHIN.as_millis(),
            TABLES_WITHIN.as_millis()
        ))
        .await
        .map_err(|e| match e.code() {
            Some(&SqlState::LOCK_NOT_AVAILABLE) => StoreError::Schema(format!(
                "another glave server was bringing the database up to date and had not \
                 finished within {}",
                glave_ports::spoken(MIGRATED_WITHIN)
            )),
            _ => failed(e),
        })?;
    let found: i32 = transaction
        .query_one(
            "SELECT COALESCE(MAX(version), 0) FROM glave_migrations",
            &[],
        )
        .await
        .map_err(failed)?
        .try_get(0)
        .map_err(failed)?;
    let found = u32::try_from(found)
        .map_err(|_| StoreError::Schema(format!("the database records schema {found}")))?;
    let plan = schema.plan(found).map_err(refused)?;
    for step in &plan.steps {
        let version = i32::try_from(step.version)
            .map_err(|_| StoreError::Schema(format!("schema {} is too large", step.version)))?;
        transaction.batch_execute(step.sql).await.map_err(|e| {
            let why = match e.code() {
                Some(&SqlState::LOCK_NOT_AVAILABLE) => format!(
                    "a table it changes stayed in use for {}; starting again tries again",
                    glave_ports::spoken(TABLES_WITHIN)
                ),
                _ => describe(&e),
            };
            StoreError::Schema(format!(
                "could not bring the database up to date (schema {version}): {why}"
            ))
        })?;
        transaction
            .execute(
                "INSERT INTO glave_migrations (version) VALUES ($1)",
                &[&version],
            )
            .await
            .map_err(failed)?;
    }
    transaction.commit().await.map_err(failed)
}

/// Why a database is not opened, with what to do about it.
fn refused(refused: Refused) -> StoreError {
    let what = match refused {
        Refused::Newer { .. } => "Update glave on every server that uses it and start them again",
        Refused::Older { .. } => {
            "Empty it (drop glave's tables, or the schema they are in) and start glave again"
        }
    };
    StoreError::Schema(format!("this database was {refused}. {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_migration_keeps_the_rules_of_a_migration() {
        assert_eq!(SCHEMA.broken_rule(), None);
    }
}
