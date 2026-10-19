//! The threads view on Postgres (`threads`): one row per thread, folded in the same transaction
//! as its events are appended, read a page at a time through an index, and rebuilt from the
//! log when the fold changes (`THREAD_VIEW_VERSION`) or when asked.

use deadpool_postgres::{Pool, Transaction};
use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::{
    LogError, OrgId, ProjectId, Scope, THREAD_VIEW_VERSION, ThreadQuery, ThreadView, fold_from_log,
};
use tokio_postgres::GenericClient;
use uuid::Uuid;

use crate::log::PostgresLog;
use crate::store::{StoreError, connection, describe};

/// The setting that records which version of the fold built the stored views.
const VERSION_SETTING: &str = "threads_view";

/// How many times a rebuild tries again when the thread was appended to meanwhile.
const REBUILD_TRIES: usize = 3;

fn failed(error: &tokio_postgres::Error) -> LogError {
    LogError::Storage(format!("the threads view failed: {}", describe(error)))
}

fn corrupt(error: impl std::fmt::Display) -> LogError {
    LogError::Storage(format!("a stored thread view is corrupt: {error}"))
}

/// Folds `events`, just appended to one thread inside `transaction`, into its view, holding
/// the view's row until the transaction ends. A thread with no view yet is given one only from
/// its first event, so a view is never folded from part of a thread; a rebuild makes the rest.
pub(crate) async fn fold_in(
    transaction: &Transaction<'_>,
    scope: &Scope,
    events: &[RawEvent],
    position: i64,
) -> Result<(), LogError> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    let before = stored(&**transaction, scope, first.thread, true).await?;
    if before.is_none() && first.seq != Seq::FIRST {
        return Ok(());
    }
    match ThreadView::fold(before, events) {
        Some(mut view) => {
            view.position = position;
            write(&**transaction, scope, &view).await
        }
        None => Ok(()),
    }
}

/// A stored row as a view: its JSON, and its position beside it.
fn read_row(row: &tokio_postgres::Row) -> Option<ThreadView> {
    let json: String = row.try_get(0).ok()?;
    let mut view: ThreadView = serde_json::from_str(&json).ok()?;
    view.position = row.try_get(1).ok()?;
    Some(view)
}

/// The stored view of `thread`; with `lock`, its row is held until the transaction ends.
async fn stored<C: GenericClient + Sync>(
    client: &C,
    scope: &Scope,
    thread: ThreadId,
    lock: bool,
) -> Result<Option<ThreadView>, LogError> {
    let sql = if lock {
        "SELECT view, position FROM threads WHERE org = $1 AND project = $2 AND thread = $3
         FOR UPDATE"
    } else {
        "SELECT view, position FROM threads WHERE org = $1 AND project = $2 AND thread = $3"
    };
    let row = client
        .query_opt(
            sql,
            &[
                &scope.org().as_uuid(),
                &scope.project().as_uuid(),
                &thread.as_uuid(),
            ],
        )
        .await
        .map_err(|e| failed(&e))?;
    // A view that no longer reads is missing: the next rebuild makes it again.
    Ok(row.as_ref().and_then(read_row))
}

async fn write<C: GenericClient + Sync>(
    client: &C,
    scope: &Scope,
    view: &ThreadView,
) -> Result<(), LogError> {
    let json = serde_json::to_string(view).map_err(corrupt)?;
    client
        .execute(
            "INSERT INTO threads (org, project, thread, standing, position, view)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (org, project, thread) DO UPDATE SET
                 standing = excluded.standing, position = excluded.position,
                 view = excluded.view",
            &[
                &scope.org().as_uuid(),
                &scope.project().as_uuid(),
                &view.thread.as_uuid(),
                &view.standing.as_str(),
                &view.position,
                &json,
            ],
        )
        .await
        .map_err(|e| failed(&e))?;
    Ok(())
}

/// A page of views, most recently written first, through the `threads_by_position` index.
pub(crate) async fn page(
    pool: &Pool,
    scope: &Scope,
    query: &ThreadQuery,
) -> Result<Vec<ThreadView>, LogError> {
    let client = connection(pool)
        .await
        .map_err(|e| LogError::Storage(e.to_string()))?;
    let statement = client
        .prepare_cached(
            "SELECT view, position FROM threads
             WHERE org = $1 AND project = $2 AND standing = $3
               AND ($4::bigint IS NULL OR position < $4::bigint)
             ORDER BY position DESC LIMIT $5",
        )
        .await
        .map_err(|e| failed(&e))?;
    let rows = client
        .query(
            &statement,
            &[
                &scope.org().as_uuid(),
                &scope.project().as_uuid(),
                &query.standing.as_str(),
                &query.after.map(|after| after.0),
                &i64::try_from(query.limit).unwrap_or(i64::MAX),
            ],
        )
        .await
        .map_err(|e| failed(&e))?;
    rows.iter()
        .map(|row| read_row(row).ok_or_else(|| corrupt("it does not read")))
        .collect()
}

/// The view of one thread.
pub(crate) async fn one(
    pool: &Pool,
    scope: &Scope,
    thread: ThreadId,
) -> Result<Option<ThreadView>, LogError> {
    let client = connection(pool)
        .await
        .map_err(|e| LogError::Storage(e.to_string()))?;
    stored(&**client, scope, thread, false).await
}

/// Folds the view of `thread` from the log again and stores it, unless the thread was appended
/// to meanwhile, in which case it tries again.
pub(crate) async fn rebuild(
    log: &PostgresLog,
    pool: &Pool,
    scope: &Scope,
    thread: ThreadId,
) -> Result<Option<ThreadView>, LogError> {
    for _ in 0..REBUILD_TRIES {
        let folded = fold_from_log(log, scope, thread).await?;
        let mut client = connection(pool)
            .await
            .map_err(|e| LogError::Storage(e.to_string()))?;
        let transaction = client.transaction().await.map_err(|e| failed(&e))?;
        // Holds the row, so an append waits and folds its events onto this view.
        stored(&*transaction, scope, thread, true).await?;
        let row = transaction
            .query_one(
                "SELECT MAX(seq), MAX(pos) FROM events
                 WHERE org = $1 AND project = $2 AND thread = $3",
                &[
                    &scope.org().as_uuid(),
                    &scope.project().as_uuid(),
                    &thread.as_uuid(),
                ],
            )
            .await
            .map_err(|e| failed(&e))?;
        let last: Option<i64> = row.try_get(0).map_err(corrupt)?;
        let position: Option<i64> = row.try_get(1).map_err(corrupt)?;
        let folded_last = folded
            .as_ref()
            .and_then(|view| i64::try_from(view.last_seq.get()).ok());
        if folded.is_some() && last != folded_last {
            continue;
        }
        let view = folded.map(|mut view| {
            view.position = position.unwrap_or_default();
            view
        });
        match &view {
            Some(view) => write(&*transaction, scope, view).await?,
            None => {
                transaction
                    .execute(
                        "DELETE FROM threads WHERE org = $1 AND project = $2 AND thread = $3",
                        &[
                            &scope.org().as_uuid(),
                            &scope.project().as_uuid(),
                            &thread.as_uuid(),
                        ],
                    )
                    .await
                    .map_err(|e| failed(&e))?;
            }
        }
        transaction.commit().await.map_err(|e| failed(&e))?;
        return Ok(view);
    }
    Err(LogError::Storage(format!(
        "thread {thread} kept changing while its view was rebuilt"
    )))
}

/// Rebuilds the view of every thread in the database. A thread that cannot be read, such as
/// one sealed under an erased key, gets no view and is not listed.
pub(crate) async fn rebuild_all(log: &PostgresLog, pool: &Pool) -> Result<(), StoreError> {
    let client = connection(pool).await?;
    let rows = client
        .query(
            "SELECT org, project, thread FROM events WHERE seq = 1 AND type = 'thread.started'",
            &[],
        )
        .await
        .map_err(|e| StoreError::Storage(describe(&e)))?;
    drop(client);
    for row in rows {
        let found = (|| {
            let org: Uuid = row.try_get(0).ok()?;
            let project: Uuid = row.try_get(1).ok()?;
            let thread: Uuid = row.try_get(2).ok()?;
            // Glave's own: views rebuilt in the scope each thread is stored in.
            let scope = Scope::glaves_own(OrgId::from_uuid(org), ProjectId::from_uuid(project));
            Some((scope, ThreadId::try_from(thread).ok()?))
        })();
        if let Some((scope, thread)) = found {
            let _ = rebuild(log, pool, &scope, thread).await;
        }
    }
    Ok(())
}

/// Rebuilds every view when the stored ones were built by an older fold (or none was built
/// yet, as in a new database), then records the version.
pub(crate) async fn refresh(log: &PostgresLog, pool: &Pool) -> Result<(), StoreError> {
    let client = connection(pool).await?;
    let built = client
        .query_opt(
            "SELECT value FROM glave_settings WHERE key = $1",
            &[&VERSION_SETTING],
        )
        .await
        .map_err(|e| StoreError::Storage(describe(&e)))?
        .and_then(|row| row.try_get::<_, String>(0).ok());
    drop(client);
    if built.and_then(|built| built.parse::<u32>().ok()) == Some(THREAD_VIEW_VERSION) {
        return Ok(());
    }
    rebuild_all(log, pool).await?;
    let client = connection(pool).await?;
    client
        .execute(
            "INSERT INTO glave_settings (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            &[&VERSION_SETTING, &THREAD_VIEW_VERSION.to_string()],
        )
        .await
        .map_err(|e| StoreError::Storage(describe(&e)))?;
    Ok(())
}
