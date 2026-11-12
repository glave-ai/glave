//! The event log on SQLite: one row per event, the envelope as readable columns, the payload
//! sealed under its project's data key.

use std::future::Future;
use std::sync::Arc;

use glave_event::{At, EventId, PayloadVersion, RawEvent, Seq, ThreadId, TypeName};
use glave_keys::{EventFields, Sealed, erased_payload};
use glave_ports::search::{Found, SearchCache, deletes_thread, indexed};
use glave_ports::{
    EventLog, Located, LogError, OrgId, ProjectId, Scope, ThreadQuery, ThreadView, check_append,
};
use rusqlite::{Connection, ErrorCode, OptionalExtension as _, TransactionBehavior, params};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::store::{Shared, SqliteKeyring, StoreError, run};

/// The event log in the local database.
#[derive(Clone)]
pub struct SqliteLog {
    pub(crate) connection: Shared,
    pub(crate) keys: Arc<SqliteKeyring>,
    index: Arc<SearchCache>,
}

impl SqliteLog {
    pub(crate) fn new(
        connection: Shared,
        keys: Arc<SqliteKeyring>,
        index: Arc<SearchCache>,
    ) -> Self {
        Self {
            connection,
            keys,
            index,
        }
    }

    /// The events `query` finds in `scope`, given its organization and project as the
    /// database keeps them, each opened.
    fn events(
        &self,
        scope: Scope,
        query: impl FnOnce(&Connection, [String; 2]) -> rusqlite::Result<Vec<Row>> + Send + 'static,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        let this = self.clone();
        async move {
            let ids = [scope.org().to_string(), scope.project().to_string()];
            let rows = run(&this.connection, move |connection| query(connection, ids))
                .await
                .map_err(|e| LogError::Storage(e.to_string()))?;
            let mut events = Vec::with_capacity(rows.len());
            for row in rows {
                events.push(row.into_event(&this.keys, &scope).await?);
            }
            Ok(events)
        }
    }
}

/// Why an append failed inside the transaction: the shared rule, or the database.
enum AppendFailure {
    Rule(LogError),
    Database(rusqlite::Error),
}

impl From<rusqlite::Error> for AppendFailure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl EventLog for SqliteLog {
    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send {
        let scope = *scope;
        let events = events.to_vec();
        let this = self.clone();
        let span = tracing::debug_span!("store.append", store = "sqlite", events = events.len());
        tracing::Instrument::instrument(
            async move {
                if events.is_empty() {
                    return Ok(());
                }
                // Sealing may read keys from the database, so it happens before the transaction,
                // and so does sealing the words the search index keeps.
                let mut sealed = Vec::with_capacity(events.len());
                let mut words = Vec::with_capacity(events.len());
                for event in &events {
                    sealed.push(
                        this.keys
                            .seal_event(&scope, event)
                            .await
                            .map_err(LogError::sealing)?,
                    );
                    words.push(
                        this.keys
                            .seal_terms(&scope, event, &indexed(event))
                            .await
                            .map_err(LogError::sealing)?,
                    );
                }
                let outcome = run(&this.connection, move |connection| {
                    Ok(append_now(connection, &scope, &events, &sealed, &words))
                })
                .await;
                match outcome {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(AppendFailure::Rule(error))) => Err(error),
                    Ok(Err(AppendFailure::Database(error))) => Err(storage(&error)),
                    Err(error) => Err(LogError::Storage(error.to_string())),
                }
            },
            span,
        )
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        let after = i64::try_from(after.map_or(0, Seq::get)).unwrap_or(i64::MAX);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.events(*scope, move |connection, [org, project]| {
            let mut statement = connection.prepare_cached(
                "SELECT glave, id, thread, seq, at, type, v, data, key_id FROM events
                 WHERE org = ?1 AND project = ?2 AND thread = ?3 AND seq > ?4
                 ORDER BY seq LIMIT ?5",
            )?;
            let rows = statement.query_map(
                params![org, project, thread.to_string(), after, limit],
                Row::read,
            )?;
            rows.collect()
        })
    }

    fn read_back(
        &self,
        scope: &Scope,
        thread: ThreadId,
        kinds: Option<&[&str]>,
        before: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        let before = i64::try_from(before.map_or(u64::MAX, Seq::get)).unwrap_or(i64::MAX);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        // The types travel as one JSON array, so the statement is the same for any number;
        // none (`NULL`) means every type.
        let kinds = kinds.map(|kinds| serde_json::to_string(kinds).unwrap_or_default());
        self.events(*scope, move |connection, [org, project]| {
            let mut statement = connection.prepare_cached(
                "SELECT glave, id, thread, seq, at, type, v, data, key_id FROM events
                 WHERE org = ?1 AND project = ?2 AND thread = ?3 AND seq < ?4
                   AND (?5 IS NULL OR type IN (SELECT value FROM json_each(?5)))
                 ORDER BY seq DESC LIMIT ?6",
            )?;
            let rows = statement.query_map(
                params![org, project, thread.to_string(), before, kinds, limit],
                Row::read,
            )?;
            rows.collect()
        })
    }

    fn search(
        &self,
        scope: &Scope,
        query: &str,
        except: Option<ThreadId>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Found>, LogError>> + Send {
        let (scope, query, log) = (*scope, query.to_owned(), self.clone());
        async move {
            let cache = Arc::clone(&log.index);
            let asked = (&scope, query.as_str(), except);
            log.keys.search(&cache, &log, asked, limit).await
        }
    }

    fn thread_views(
        &self,
        scope: &Scope,
        query: &ThreadQuery,
    ) -> impl Future<Output = Result<Vec<ThreadView>, LogError>> + Send {
        let (scope, query, connection) = (*scope, query.clone(), Arc::clone(&self.connection));
        async move { crate::views::page(&connection, &scope, &query).await }
    }

    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        let (scope, connection) = (*scope, Arc::clone(&self.connection));
        async move { crate::views::one(&connection, &scope, thread).await }
    }

    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        let (scope, this) = (*scope, self.clone());
        async move { crate::views::rebuild(&this, &this.connection, &scope, thread).await }
    }

    fn locate(
        &self,
        org: OrgId,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Located>, LogError>> + Send {
        let connection = Arc::clone(&self.connection);
        async move {
            let found = run(&connection, move |connection| {
                let mut statement = connection.prepare_cached(
                    "SELECT org, project FROM events WHERE thread = ?1 AND org = ?2
                     ORDER BY project LIMIT 1",
                )?;
                statement
                    .query_row(params![thread.to_string(), org.to_string()], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .optional()
            })
            .await
            .map_err(|e| LogError::Storage(e.to_string()))?;
            found
                .map(|(org, project)| {
                    Ok(Located {
                        org: OrgId::from_uuid(parse_uuid(&org)?),
                        project: ProjectId::from_uuid(parse_uuid(&project)?),
                    })
                })
                .transpose()
        }
    }
}

fn append_now(
    connection: &mut Connection,
    scope: &Scope,
    events: &[RawEvent],
    sealed: &[Sealed],
    words: &[Option<Sealed>],
) -> Result<(), AppendFailure> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    let (org, project, thread) = (
        scope.org().to_string(),
        scope.project().to_string(),
        first.thread.to_string(),
    );
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let last: Option<i64> = transaction.query_row(
        "SELECT MAX(seq) FROM events WHERE org = ?1 AND project = ?2 AND thread = ?3",
        params![org, project, thread],
        |row| row.get(0),
    )?;
    let last = match last.map(|seq| u64::try_from(seq).map(Seq::new)) {
        None => None,
        Some(Ok(Ok(seq))) => Some(seq),
        Some(_) => {
            return Err(AppendFailure::Rule(LogError::Storage(
                "a stored seq is invalid".into(),
            )));
        }
    };
    check_append(last, events).map_err(AppendFailure::Rule)?;
    {
        let mut insert = transaction.prepare_cached(
            "INSERT INTO events (org, project, thread, seq, id, at, type, v, glave, data, key_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for (event, sealed) in events.iter().zip(sealed) {
            let seq = i64::try_from(event.seq.get()).map_err(|_| {
                AppendFailure::Rule(LogError::Storage("seq is too large to store".into()))
            })?;
            insert.execute(params![
                org,
                project,
                event.thread.to_string(),
                seq,
                event.id.to_string(),
                event.at.to_string(),
                event.kind.as_str(),
                i64::from(event.v.get()),
                i64::from(event.glave),
                sealed.bytes,
                sealed.key_id,
            ])?;
        }
        let mut index = transaction.prepare_cached(
            "INSERT INTO event_search (org, project, thread, seq, key_id, terms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for (event, words) in events.iter().zip(words) {
            if let Some(words) = words {
                let seq = i64::try_from(event.seq.get()).unwrap_or(i64::MAX);
                let thread = event.thread.to_string();
                index.execute(params![
                    org,
                    project,
                    thread,
                    seq,
                    words.key_id,
                    words.bytes
                ])?;
            }
        }
        if deletes_thread(events) {
            transaction
                .prepare_cached(
                    "DELETE FROM event_search WHERE org = ?1 AND project = ?2 AND thread = ?3",
                )?
                .execute(params![org, project, thread])?;
        }
    }
    crate::views::fold_in(&transaction, scope, events)?;
    transaction.commit()?;
    Ok(())
}

/// A stored event before its columns are checked and its payload opened.
struct Row {
    glave: i64,
    id: String,
    thread: String,
    seq: i64,
    at: String,
    kind: String,
    v: i64,
    data: Vec<u8>,
    key_id: String,
}

impl Row {
    /// The row a statement selecting `glave, id, thread, seq, at, type, v, data, key_id` read.
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            glave: row.get(0)?,
            id: row.get(1)?,
            thread: row.get(2)?,
            seq: row.get(3)?,
            at: row.get(4)?,
            kind: row.get(5)?,
            v: row.get(6)?,
            data: row.get(7)?,
            key_id: row.get(8)?,
        })
    }

    async fn into_event(self, keys: &SqliteKeyring, scope: &Scope) -> Result<RawEvent, LogError> {
        let glave = u32::try_from(self.glave).map_err(LogError::corrupt)?;
        let thread = ThreadId::try_from(parse_uuid(&self.thread)?).map_err(LogError::corrupt)?;
        let seq = Seq::new(u64::try_from(self.seq).map_err(LogError::corrupt)?)
            .map_err(LogError::corrupt)?;
        let kind = TypeName::new(self.kind).map_err(LogError::corrupt)?;
        let v = PayloadVersion::new(u32::try_from(self.v).map_err(LogError::corrupt)?)
            .map_err(LogError::corrupt)?;
        let fields = EventFields {
            glave,
            thread,
            seq,
            kind: kind.as_str(),
            v: v.get(),
        };
        let data = match keys
            .open_event(scope, &fields, &self.key_id, &self.data)
            .await
        {
            Ok(opened) => opened,
            Err(error) => erased_payload(&self.key_id, &error)
                .ok_or_else(|| LogError::opening(seq, &error))?,
        };
        let data = String::from_utf8(data).map_err(LogError::corrupt)?;
        Ok(RawEvent {
            glave,
            id: EventId::try_from(parse_uuid(&self.id)?).map_err(LogError::corrupt)?,
            thread,
            seq,
            at: At::try_from(self.at).map_err(LogError::corrupt)?,
            kind,
            v,
            data: RawValue::from_string(data).map_err(LogError::corrupt)?,
        })
    }
}

pub(crate) fn parse_uuid(text: &str) -> Result<Uuid, LogError> {
    Uuid::parse_str(text).map_err(LogError::corrupt)
}

/// A uniqueness violation means another writer took the position first.
fn storage(error: &rusqlite::Error) -> LogError {
    if error.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) {
        LogError::Storage(format!("another writer appended first: {error}"))
    } else {
        LogError::Storage(StoreError::Storage(error.to_string()).to_string())
    }
}
