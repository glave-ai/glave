//! The event log on SQLite: one row per event, the envelope as columns, the payload as the
//! exact bytes written.

use std::future::Future;

use glave_event::{At, EventId, PayloadVersion, RawEvent, Seq, ThreadId, TypeName};
use glave_ports::{EventLog, LogError, Scope, check_append};
use rusqlite::{Connection, ErrorCode, TransactionBehavior, params};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::store::{Shared, StoreError, run};

/// The event log in the local database.
#[derive(Clone)]
pub struct SqliteLog {
    connection: Shared,
}

impl SqliteLog {
    pub(crate) fn new(connection: Shared) -> Self {
        Self { connection }
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
        let connection = self.connection.clone();
        async move {
            if events.is_empty() {
                return Ok(());
            }
            let outcome = run(&connection, move |connection| {
                Ok(append_now(connection, &scope, &events))
            })
            .await;
            match outcome {
                Ok(Ok(())) => Ok(()),
                Ok(Err(AppendFailure::Rule(error))) => Err(error),
                Ok(Err(AppendFailure::Database(error))) => Err(storage(&error)),
                Err(error) => Err(LogError::Storage(error.to_string())),
            }
        }
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        let scope = *scope;
        let connection = self.connection.clone();
        async move {
            let after = i64::try_from(after.map_or(0, Seq::get)).unwrap_or(i64::MAX);
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let rows = run(&connection, move |connection| {
                let mut statement = connection.prepare_cached(
                    "SELECT glave, id, thread, seq, at, type, v, data FROM events
                     WHERE org = ?1 AND space = ?2 AND thread = ?3 AND seq > ?4
                     ORDER BY seq LIMIT ?5",
                )?;
                let rows = statement.query_map(
                    params![
                        scope.org.to_string(),
                        scope.space.to_string(),
                        thread.to_string(),
                        after,
                        limit
                    ],
                    |row| {
                        Ok(Row {
                            glave: row.get(0)?,
                            id: row.get(1)?,
                            thread: row.get(2)?,
                            seq: row.get(3)?,
                            at: row.get(4)?,
                            kind: row.get(5)?,
                            v: row.get(6)?,
                            data: row.get(7)?,
                        })
                    },
                )?;
                rows.collect::<rusqlite::Result<Vec<Row>>>()
            })
            .await
            .map_err(|e| LogError::Storage(e.to_string()))?;
            rows.into_iter().map(Row::into_event).collect()
        }
    }

    fn threads(
        &self,
        scope: &Scope,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<ThreadId>, LogError>> + Send {
        let scope = *scope;
        let connection = self.connection.clone();
        async move {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let ids = run(&connection, move |connection| {
                let mut statement = connection.prepare_cached(
                    "SELECT thread FROM events WHERE org = ?1 AND space = ?2
                     GROUP BY thread ORDER BY MAX(rowid) DESC LIMIT ?3",
                )?;
                let ids = statement.query_map(
                    params![scope.org.to_string(), scope.space.to_string(), limit],
                    |row| row.get::<_, String>(0),
                )?;
                ids.collect::<rusqlite::Result<Vec<String>>>()
            })
            .await
            .map_err(|e| LogError::Storage(e.to_string()))?;
            ids.iter()
                .map(|id| parse_uuid(id).and_then(|uuid| ThreadId::try_from(uuid).map_err(corrupt)))
                .collect()
        }
    }
}

fn append_now(
    connection: &mut Connection,
    scope: &Scope,
    events: &[RawEvent],
) -> Result<(), AppendFailure> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    let (org, space, thread) = (
        scope.org.to_string(),
        scope.space.to_string(),
        first.thread.to_string(),
    );
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let last: Option<i64> = transaction.query_row(
        "SELECT MAX(seq) FROM events WHERE org = ?1 AND space = ?2 AND thread = ?3",
        params![org, space, thread],
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
            "INSERT INTO events (org, space, thread, seq, id, at, type, v, glave, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for event in events {
            let seq = i64::try_from(event.seq.get()).map_err(|_| {
                AppendFailure::Rule(LogError::Storage("seq is too large to store".into()))
            })?;
            insert.execute(params![
                org,
                space,
                event.thread.to_string(),
                seq,
                event.id.to_string(),
                event.at.to_string(),
                event.kind.as_str(),
                i64::from(event.v.get()),
                i64::from(event.glave),
                event.data.get().as_bytes(),
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

/// A stored event before its columns are checked.
struct Row {
    glave: i64,
    id: String,
    thread: String,
    seq: i64,
    at: String,
    kind: String,
    v: i64,
    data: Vec<u8>,
}

impl Row {
    fn into_event(self) -> Result<RawEvent, LogError> {
        let data = String::from_utf8(self.data).map_err(corrupt)?;
        Ok(RawEvent {
            glave: u32::try_from(self.glave).map_err(corrupt)?,
            id: EventId::try_from(parse_uuid(&self.id)?).map_err(corrupt)?,
            thread: ThreadId::try_from(parse_uuid(&self.thread)?).map_err(corrupt)?,
            seq: Seq::new(u64::try_from(self.seq).map_err(corrupt)?).map_err(corrupt)?,
            at: At::try_from(self.at).map_err(corrupt)?,
            kind: TypeName::new(self.kind).map_err(corrupt)?,
            v: PayloadVersion::new(u32::try_from(self.v).map_err(corrupt)?).map_err(corrupt)?,
            data: RawValue::from_string(data).map_err(corrupt)?,
        })
    }
}

fn parse_uuid(text: &str) -> Result<Uuid, LogError> {
    Uuid::parse_str(text).map_err(corrupt)
}

fn corrupt(error: impl std::fmt::Display) -> LogError {
    LogError::Storage(format!("a stored event is corrupt: {error}"))
}

/// A uniqueness violation means another writer took the position first.
fn storage(error: &rusqlite::Error) -> LogError {
    if error.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) {
        LogError::Storage(format!("another writer appended first: {error}"))
    } else {
        LogError::Storage(StoreError::Storage(error.to_string()).to_string())
    }
}
