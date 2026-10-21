//! The event log on Postgres: one row per event, the envelope as readable columns, the
//! payload sealed under its project's data key.

use std::future::Future;
use std::sync::Arc;

use deadpool_postgres::Pool;
use glave_event::{At, EventId, PayloadVersion, RawEvent, Seq, ThreadId, TypeName};
use glave_keys::{EventFields, erased_payload};
use glave_ports::search::{Found, indexed, query_terms};
use glave_ports::{
    EventLog, Located, LogError, OrgId, ProjectId, Scope, ThreadQuery, ThreadView, check_append,
};
use jiff::Timestamp;
use serde_json::value::RawValue;
use tokio_postgres::Row;
use tokio_postgres::error::SqlState;
use uuid::Uuid;

use crate::store::{PostgresKeyring, connection, describe};

/// The event log in the database.
#[derive(Clone)]
pub struct PostgresLog {
    pool: Pool,
    keys: Arc<PostgresKeyring>,
}

impl PostgresLog {
    pub(crate) fn new(pool: Pool, keys: Arc<PostgresKeyring>) -> Self {
        Self { pool, keys }
    }
}

/// The whole batch in one statement: each column travels as one array.
const INSERT: &str = "
    INSERT INTO events (org, project, thread, seq, id, at, type, v, glave, data, key_id)
    SELECT $1, $2, $3, seq, id, at, type, v, glave, data, key_id
    FROM UNNEST($4::bigint[], $5::uuid[], $6::timestamptz[], $7::text[], $8::bigint[],
                $9::bigint[], $10::bytea[], $11::text[])
        AS batch (seq, id, at, type, v, glave, data, key_id)";

/// The batch's searchable events into the index, each word exactly as hashed.
const INDEX: &str = "
    INSERT INTO event_search (org, project, thread, seq, terms)
    SELECT $1, $2, $3, seq, array_to_tsvector(string_to_array(words, ' '))
    FROM UNNEST($4::bigint[], $5::text[]) AS batch (seq, words)
    WHERE words IS NOT NULL";

/// The events matching any hashed word, best match first, then newest first.
const SEARCH: &str = "
    SELECT thread, seq FROM event_search
    WHERE org = $1 AND project = $2 AND terms @@ $3::text::tsquery
    ORDER BY ts_rank(terms, $3::text::tsquery) DESC, pos DESC
    LIMIT $4";

/// The thread's latest event of any of the types: one index lookup per type, from the newest
/// (`events_by_type`), then the newest of those, so no other event is read or opened.
const LATEST_OF: &str = "
    SELECT latest.* FROM UNNEST($4::text[]) AS wanted (type)
    CROSS JOIN LATERAL (
        SELECT glave, id, thread, seq, at, type, v, data, key_id FROM events
        WHERE org = $1 AND project = $2 AND thread = $3 AND type = wanted.type
        ORDER BY seq DESC LIMIT 1
    ) AS latest
    ORDER BY latest.seq DESC LIMIT 1";

impl EventLog for PostgresLog {
    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send {
        let scope = *scope;
        let events = events.to_vec();
        let pool = self.pool.clone();
        let keys = Arc::clone(&self.keys);
        let span = tracing::debug_span!("store.append", store = "postgres", events = events.len());
        tracing::Instrument::instrument(
            async move {
                let Some(first) = events.first() else {
                    return Ok(());
                };
                let (org, project, thread) = (
                    scope.org().as_uuid(),
                    scope.project().as_uuid(),
                    first.thread.as_uuid(),
                );
                let columns = Columns::of(&keys, &scope, &events).await?;
                let mut client = connection(&pool).await.map_err(storage)?;
                let transaction = client.transaction().await.map_err(database)?;
                let last: Option<i64> = transaction
                    .query_one(
                        "SELECT MAX(seq) FROM events WHERE org = $1 AND project = $2 AND thread = $3",
                        &[&org, &project, &thread],
                    )
                    .await
                    .map_err(database)?
                    .try_get(0)
                    .map_err(database)?;
                let last = last
                    .map(|seq| {
                        u64::try_from(seq)
                            .map_err(LogError::corrupt)
                            .and_then(stored_seq)
                    })
                    .transpose()?;
                check_append(last, &events)?;
                let insert = transaction.prepare_cached(INSERT).await.map_err(database)?;
                let params: [&(dyn tokio_postgres::types::ToSql + Sync); 11] = [
                    &org,
                    &project,
                    &thread,
                    &columns.seq,
                    &columns.id,
                    &columns.at,
                    &columns.kind,
                    &columns.v,
                    &columns.glave,
                    &columns.data,
                    &columns.key_id,
                ];
                if let Err(error) = transaction.execute(&insert, &params).await {
                    let raced = error.code() == Some(&SqlState::UNIQUE_VIOLATION);
                    transaction.rollback().await.map_err(database)?;
                    return Err(if raced {
                        taken(&client, (org, project), first).await
                    } else {
                        database(error)
                    });
                }
                if columns.words.iter().any(Option::is_some) {
                    let index = transaction.prepare_cached(INDEX).await.map_err(database)?;
                    transaction
                        .execute(
                            &index,
                            &[&org, &project, &thread, &columns.seq, &columns.words],
                        )
                        .await
                        .map_err(database)?;
                }
                crate::views::fold_in(&transaction, &scope, &events).await?;
                transaction.commit().await.map_err(database)
            },
            span,
        )
    }

    fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Found>, LogError>> + Send {
        let scope = *scope;
        let words = query_terms(query);
        let pool = self.pool.clone();
        let keys = Arc::clone(&self.keys);
        async move {
            if words.is_empty() || limit == 0 {
                return Ok(Vec::new());
            }
            let Some(hashed) = keys
                .search_terms(&scope, &words)
                .await
                .map_err(LogError::sealing)?
            else {
                return Ok(Vec::new());
            };
            // Hashes are hex, so quoting each is all the query syntax needs.
            let query = hashed
                .iter()
                .map(|word| format!("'{word}'"))
                .collect::<Vec<_>>()
                .join(" | ");
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let client = connection(&pool).await.map_err(storage)?;
            let statement = client.prepare_cached(SEARCH).await.map_err(database)?;
            let rows = client
                .query(
                    &statement,
                    &[
                        &scope.org().as_uuid(),
                        &scope.project().as_uuid(),
                        &query,
                        &limit,
                    ],
                )
                .await
                .map_err(database)?;
            rows.iter()
                .map(|row| {
                    let thread: Uuid = row.try_get(0).map_err(LogError::corrupt)?;
                    let seq: i64 = row.try_get(1).map_err(LogError::corrupt)?;
                    Ok(Found {
                        thread: ThreadId::try_from(thread).map_err(LogError::corrupt)?,
                        seq: stored_seq(u64::try_from(seq).map_err(LogError::corrupt)?)?,
                    })
                })
                .collect()
        }
    }

    fn thread_views(
        &self,
        scope: &Scope,
        query: &ThreadQuery,
    ) -> impl Future<Output = Result<Vec<ThreadView>, LogError>> + Send {
        let (scope, query, pool) = (*scope, query.clone(), self.pool.clone());
        async move { crate::views::page(&pool, &scope, &query).await }
    }

    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        let (scope, pool) = (*scope, self.pool.clone());
        async move { crate::views::one(&pool, &scope, thread).await }
    }

    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        let (scope, this) = (*scope, self.clone());
        async move { crate::views::rebuild(&this, &this.pool, &scope, thread).await }
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        let scope = *scope;
        let pool = self.pool.clone();
        let keys = Arc::clone(&self.keys);
        async move {
            let after = i64::try_from(after.map_or(0, Seq::get)).unwrap_or(i64::MAX);
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let client = connection(&pool).await.map_err(storage)?;
            let statement = client
                .prepare_cached(
                    "SELECT glave, id, thread, seq, at, type, v, data, key_id FROM events
                     WHERE org = $1 AND project = $2 AND thread = $3 AND seq > $4
                     ORDER BY seq LIMIT $5",
                )
                .await
                .map_err(database)?;
            let rows = client
                .query(
                    &statement,
                    &[
                        &scope.org().as_uuid(),
                        &scope.project().as_uuid(),
                        &thread.as_uuid(),
                        &after,
                        &limit,
                    ],
                )
                .await
                .map_err(database)?;
            let mut events = Vec::with_capacity(rows.len());
            for row in &rows {
                events.push(event(&keys, &scope, row).await?);
            }
            Ok(events)
        }
    }

    fn latest_of(
        &self,
        scope: &Scope,
        thread: ThreadId,
        kinds: &[&str],
    ) -> impl Future<Output = Result<Option<RawEvent>, LogError>> + Send {
        let scope = *scope;
        let kinds: Vec<String> = kinds.iter().map(|&kind| kind.to_owned()).collect();
        let pool = self.pool.clone();
        let keys = Arc::clone(&self.keys);
        async move {
            let client = connection(&pool).await.map_err(storage)?;
            let statement = client.prepare_cached(LATEST_OF).await.map_err(database)?;
            let row = client
                .query_opt(
                    &statement,
                    &[
                        &scope.org().as_uuid(),
                        &scope.project().as_uuid(),
                        &thread.as_uuid(),
                        &kinds,
                    ],
                )
                .await
                .map_err(database)?;
            match row {
                Some(row) => Ok(Some(event(&keys, &scope, &row).await?)),
                None => Ok(None),
            }
        }
    }

    fn locate(
        &self,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Located>, LogError>> + Send {
        let pool = self.pool.clone();
        async move {
            let client = connection(&pool).await.map_err(storage)?;
            let statement = client
                .prepare_cached(
                    "SELECT org, project FROM events WHERE thread = $1
                     ORDER BY org, project LIMIT 1",
                )
                .await
                .map_err(database)?;
            let row = client
                .query_opt(&statement, &[&thread.as_uuid()])
                .await
                .map_err(database)?;
            row.map(|row| {
                let org: Uuid = row.try_get(0).map_err(LogError::corrupt)?;
                let project: Uuid = row.try_get(1).map_err(LogError::corrupt)?;
                Ok(Located {
                    org: OrgId::from_uuid(org),
                    project: ProjectId::from_uuid(project),
                })
            })
            .transpose()
        }
    }
}

/// A batch of events turned inside out: one array per column, as `INSERT` takes them.
struct Columns {
    seq: Vec<i64>,
    id: Vec<Uuid>,
    at: Vec<Timestamp>,
    kind: Vec<String>,
    v: Vec<i64>,
    glave: Vec<i64>,
    data: Vec<Vec<u8>>,
    key_id: Vec<String>,
    /// Each event's hashed words, for the search index; `None` for one never searched.
    words: Vec<Option<String>>,
}

impl Columns {
    /// The columns of `events`, each payload sealed.
    async fn of(
        keys: &PostgresKeyring,
        scope: &Scope,
        events: &[RawEvent],
    ) -> Result<Self, LogError> {
        let mut columns = Self {
            seq: Vec::with_capacity(events.len()),
            id: Vec::with_capacity(events.len()),
            at: Vec::with_capacity(events.len()),
            kind: Vec::with_capacity(events.len()),
            v: Vec::with_capacity(events.len()),
            glave: Vec::with_capacity(events.len()),
            data: Vec::with_capacity(events.len()),
            key_id: Vec::with_capacity(events.len()),
            words: Vec::with_capacity(events.len()),
        };
        for event in events {
            let seq = i64::try_from(event.seq.get())
                .map_err(|_| LogError::Storage("seq is too large to store".into()))?;
            columns.seq.push(seq);
            columns.id.push(event.id.as_uuid());
            columns.at.push(event.at.timestamp());
            columns.kind.push(event.kind.as_str().to_owned());
            columns.v.push(i64::from(event.v.get()));
            columns.glave.push(i64::from(event.glave));
            let sealed = keys
                .seal_event(scope, event)
                .await
                .map_err(LogError::sealing)?;
            columns.data.push(sealed.bytes);
            columns.key_id.push(sealed.key_id);
            let words = indexed(event);
            let hashed = if words.is_empty() {
                None
            } else {
                keys.search_terms(scope, &words)
                    .await
                    .map_err(LogError::sealing)?
            };
            columns.words.push(hashed.map(|hashed| hashed.join(" ")));
        }
        Ok(columns)
    }
}

/// A stored row back into an event, checking every column and opening the payload.
async fn event(keys: &PostgresKeyring, scope: &Scope, row: &Row) -> Result<RawEvent, LogError> {
    let glave: i64 = row.try_get("glave").map_err(LogError::corrupt)?;
    let id: Uuid = row.try_get("id").map_err(LogError::corrupt)?;
    let thread: Uuid = row.try_get("thread").map_err(LogError::corrupt)?;
    let seq: i64 = row.try_get("seq").map_err(LogError::corrupt)?;
    let at: Timestamp = row.try_get("at").map_err(LogError::corrupt)?;
    let kind: String = row.try_get("type").map_err(LogError::corrupt)?;
    let v: i64 = row.try_get("v").map_err(LogError::corrupt)?;
    let data: Vec<u8> = row.try_get("data").map_err(LogError::corrupt)?;
    let key_id: String = row.try_get("key_id").map_err(LogError::corrupt)?;
    let glave = u32::try_from(glave).map_err(LogError::corrupt)?;
    let thread = ThreadId::try_from(thread).map_err(LogError::corrupt)?;
    let seq = stored_seq(u64::try_from(seq).map_err(LogError::corrupt)?)?;
    let kind = TypeName::new(kind).map_err(LogError::corrupt)?;
    let v = PayloadVersion::new(u32::try_from(v).map_err(LogError::corrupt)?)
        .map_err(LogError::corrupt)?;
    let fields = EventFields {
        glave,
        thread,
        seq,
        kind: kind.as_str(),
        v: v.get(),
    };
    let data = match keys.open_event(scope, &fields, &key_id, &data).await {
        Ok(opened) => opened,
        Err(error) => {
            erased_payload(&key_id, &error).ok_or_else(|| LogError::opening(seq, &error))?
        }
    };
    let data = String::from_utf8(data).map_err(LogError::corrupt)?;
    Ok(RawEvent {
        glave,
        id: EventId::try_from(id).map_err(LogError::corrupt)?,
        thread,
        seq,
        at: At::from(at),
        kind,
        v,
        data: RawValue::from_string(data).map_err(LogError::corrupt)?,
    })
}

fn stored_seq(seq: u64) -> Result<Seq, LogError> {
    Seq::new(seq).map_err(LogError::corrupt)
}

fn storage(error: impl std::fmt::Display) -> LogError {
    LogError::Storage(error.to_string())
}

/// What a writer that lost the race for `first`'s place is told: a conflict, as when it reads
/// the place taken before writing, naming where the thread now ends.
async fn taken(
    client: &tokio_postgres::Client,
    (org, project): (Uuid, Uuid),
    first: &RawEvent,
) -> LogError {
    let last = client
        .query_one(
            "SELECT MAX(seq) FROM events WHERE org = $1 AND project = $2 AND thread = $3",
            &[&org, &project, &first.thread.as_uuid()],
        )
        .await
        .and_then(|row| row.try_get::<_, Option<i64>>(0));
    match last {
        Ok(last) => LogError::Conflict {
            thread: first.thread,
            expected: last
                .and_then(|last| u64::try_from(last).ok())
                .map_or(1, |last| last.saturating_add(1)),
            found: first.seq.get(),
        },
        Err(error) => database(error),
    }
}

/// A uniqueness violation means another writer took the position first.
#[expect(
    clippy::needless_pass_by_value,
    reason = "taken by value to pass straight to map_err"
)]
fn database(error: tokio_postgres::Error) -> LogError {
    if error.code() == Some(&SqlState::UNIQUE_VIOLATION) {
        LogError::Storage(format!(
            "another writer appended first: {}",
            describe(&error)
        ))
    } else {
        LogError::Storage(format!("the database failed: {}", describe(&error)))
    }
}
