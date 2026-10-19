//! The threads view on SQLite (`threads`): one row per thread, folded in the same transaction
//! as its events are appended, read a page at a time through an index, and rebuilt from the
//! log when the fold changes (`THREAD_VIEW_VERSION`) or when asked.

use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::{
    LogError, OrgId, ProjectId, Scope, THREAD_VIEW_VERSION, ThreadQuery, ThreadView, fold_from_log,
};
use rusqlite::{Connection, OptionalExtension as _, Transaction, params};
use uuid::Uuid;

use crate::log::SqliteLog;
use crate::store::{Shared, StoreError, run};

/// The setting that records which version of the fold built the stored views.
const VERSION_SETTING: &str = "threads_view";

/// How many times a rebuild tries again when the thread was appended to meanwhile.
const REBUILD_TRIES: usize = 3;

/// Folds `events`, just appended to one thread inside `transaction` with the last at row
/// `position`, into its view. A thread with no view yet is given one only from its first
/// event, so a view is never folded from part of a thread; a rebuild makes the rest.
pub(crate) fn fold_in(
    transaction: &Transaction<'_>,
    scope: &Scope,
    events: &[RawEvent],
    position: i64,
) -> rusqlite::Result<()> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    let before = stored(transaction, scope, first.thread)?;
    if before.is_none() && first.seq != Seq::FIRST {
        return Ok(());
    }
    match ThreadView::fold(before, events) {
        Some(mut view) => {
            view.position = position;
            write(transaction, scope, &view)
        }
        None => Ok(()),
    }
}

/// A stored row as a view: its JSON, and its position beside it.
fn read_row(json: &str, position: i64) -> Option<ThreadView> {
    let mut view: ThreadView = serde_json::from_str(json).ok()?;
    view.position = position;
    Some(view)
}

fn stored(
    connection: &Connection,
    scope: &Scope,
    thread: ThreadId,
) -> rusqlite::Result<Option<ThreadView>> {
    let row: Option<(String, i64)> = connection
        .prepare_cached(
            "SELECT view, position FROM threads WHERE org = ?1 AND project = ?2 AND thread = ?3",
        )?
        .query_row(
            params![
                scope.org().to_string(),
                scope.project().to_string(),
                thread.to_string()
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    // A view that no longer reads is missing: the next rebuild makes it again.
    Ok(row.and_then(|(json, position)| read_row(&json, position)))
}

fn write(connection: &Connection, scope: &Scope, view: &ThreadView) -> rusqlite::Result<()> {
    let json = serde_json::to_string(view)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    connection
        .prepare_cached(
            "INSERT INTO threads (org, project, thread, standing, position, view)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (org, project, thread) DO UPDATE SET
                 standing = excluded.standing, position = excluded.position,
                 view = excluded.view",
        )?
        .execute(params![
            scope.org().to_string(),
            scope.project().to_string(),
            view.thread.to_string(),
            view.standing.as_str(),
            view.position,
            json,
        ])?;
    Ok(())
}

/// A page of views, most recently written first, through the `threads_by_position` index.
pub(crate) async fn page(
    connection: &Shared,
    scope: &Scope,
    query: &ThreadQuery,
) -> Result<Vec<ThreadView>, LogError> {
    let (scope, query) = (*scope, *query);
    let rows = run(connection, move |connection| {
        let mut statement = connection.prepare_cached(
            "SELECT view, position FROM threads
             WHERE org = ?1 AND project = ?2 AND standing = ?3 AND (?4 IS NULL OR position < ?4)
             ORDER BY position DESC LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![
                scope.org().to_string(),
                scope.project().to_string(),
                query.standing.as_str(),
                query.after.map(|after| after.0),
                i64::try_from(query.limit).unwrap_or(i64::MAX),
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .await
    .map_err(|e| LogError::Storage(e.to_string()))?;
    rows.iter()
        .map(|(json, position)| {
            read_row(json, *position)
                .ok_or_else(|| LogError::Storage("a stored thread view is corrupt".into()))
        })
        .collect()
}

/// The view of one thread.
pub(crate) async fn one(
    connection: &Shared,
    scope: &Scope,
    thread: ThreadId,
) -> Result<Option<ThreadView>, LogError> {
    let scope = *scope;
    run(connection, move |connection| {
        stored(connection, &scope, thread)
    })
    .await
    .map_err(|e| LogError::Storage(e.to_string()))
}

/// Folds the view of `thread` from the log again and stores it, unless the thread was appended
/// to meanwhile, in which case it tries again.
pub(crate) async fn rebuild(
    log: &SqliteLog,
    connection: &Shared,
    scope: &Scope,
    thread: ThreadId,
) -> Result<Option<ThreadView>, LogError> {
    let scope = *scope;
    for _ in 0..REBUILD_TRIES {
        let folded = fold_from_log(log, &scope, thread).await?;
        // `None` when the thread was appended to meanwhile; else the view stored.
        let stored = run(connection, move |connection| {
            let transaction = connection.transaction()?;
            let (last, position): (Option<i64>, Option<i64>) = transaction.query_row(
                "SELECT MAX(seq), MAX(rowid) FROM events
                 WHERE org = ?1 AND project = ?2 AND thread = ?3",
                params![
                    scope.org().to_string(),
                    scope.project().to_string(),
                    thread.to_string()
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let folded_last = folded
                .as_ref()
                .and_then(|view| i64::try_from(view.last_seq.get()).ok());
            if folded.is_some() && last != folded_last {
                return Ok(None);
            }
            let folded = folded.map(|mut view| {
                view.position = position.unwrap_or_default();
                view
            });
            match &folded {
                Some(view) => write(&transaction, &scope, view)?,
                None => {
                    transaction.execute(
                        "DELETE FROM threads WHERE org = ?1 AND project = ?2 AND thread = ?3",
                        params![
                            scope.org().to_string(),
                            scope.project().to_string(),
                            thread.to_string()
                        ],
                    )?;
                }
            }
            transaction.commit()?;
            Ok(Some(folded))
        })
        .await
        .map_err(|e| LogError::Storage(e.to_string()))?;
        if let Some(view) = stored {
            return Ok(view);
        }
    }
    Err(LogError::Storage(format!(
        "thread {thread} kept changing while its view was rebuilt"
    )))
}

/// Rebuilds the view of every thread in the database. A thread that cannot be read, such as
/// one sealed under an erased key, gets no view and is not listed.
pub(crate) async fn rebuild_all(log: &SqliteLog, connection: &Shared) -> Result<(), StoreError> {
    let threads = run(connection, |connection| {
        let mut statement = connection.prepare(
            "SELECT org, project, thread FROM events WHERE seq = 1 AND type = 'thread.started'",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .await?;
    for (org, project, thread) in threads {
        let parsed = (|| {
            // Glave's own: views rebuilt in the scope each thread is stored in.
            let scope = Scope::glaves_own(
                OrgId::from_uuid(Uuid::parse_str(&org).ok()?),
                ProjectId::from_uuid(Uuid::parse_str(&project).ok()?),
            );
            let thread = ThreadId::try_from(Uuid::parse_str(&thread).ok()?).ok()?;
            Some((scope, thread))
        })();
        if let Some((scope, thread)) = parsed {
            let _ = rebuild(log, connection, &scope, thread).await;
        }
    }
    Ok(())
}

/// Rebuilds every view when the stored ones were built by an older fold (or none was built
/// yet, as in a new database), then records the version.
pub(crate) async fn refresh(log: &SqliteLog, connection: &Shared) -> Result<(), StoreError> {
    let built: Option<String> = run(connection, |connection| {
        connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![VERSION_SETTING],
                |row| row.get(0),
            )
            .optional()
    })
    .await?;
    if built.and_then(|built| built.parse::<u32>().ok()) == Some(THREAD_VIEW_VERSION) {
        return Ok(());
    }
    rebuild_all(log, connection).await?;
    run(connection, |connection| {
        connection.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![VERSION_SETTING, THREAD_VIEW_VERSION.to_string()],
        )
    })
    .await?;
    Ok(())
}
